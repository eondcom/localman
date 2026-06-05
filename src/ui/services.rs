use iced::{
    widget::{button, column, container, row, text, Space},
    Color, Element, Length, Task,
};
use crate::system::{ServiceStatus, get_service_status, toggle_service};

#[derive(Debug, Clone)]
pub enum ServicesMessage {
    Toggle(String, bool),
    Refresh,
    Toggled(String, Result<(), String>),
}

pub struct ServicesState {
    apache_status: ServiceStatus,
    mariadb_status: ServiceStatus,
    error: Option<String>,
}

impl ServicesState {
    pub fn new() -> Self {
        let mut s = Self {
            apache_status: ServiceStatus::Unknown,
            mariadb_status: ServiceStatus::Unknown,
            error: None,
        };
        s.refresh();
        s
    }

    pub fn refresh(&mut self) {
        self.apache_status = get_service_status("apache2");
        self.mariadb_status = get_service_status("mariadb");
    }

    pub fn update(&mut self, msg: ServicesMessage) -> Task<ServicesMessage> {
        match msg {
            ServicesMessage::Refresh => {
                self.refresh();
                Task::none()
            }
            ServicesMessage::Toggle(name, start) => {
                let n = name.clone();
                Task::perform(
                    async move { toggle_service(&n, start) },
                    move |r| ServicesMessage::Toggled(name.clone(), r),
                )
            }
            ServicesMessage::Toggled(name, result) => {
                match result {
                    Ok(_) => {
                        self.error = None;
                        match name.as_str() {
                            "apache2" => self.apache_status = get_service_status("apache2"),
                            "mariadb" => self.mariadb_status = get_service_status("mariadb"),
                            _ => {}
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, ServicesMessage> {
        let apache = service_card(
            "Apache2",
            "웹 서버",
            &self.apache_status,
            "apache2",
        );

        let mariadb = service_card(
            "MariaDB",
            "데이터베이스 서버",
            &self.mariadb_status,
            "mariadb",
        );

        let refresh_btn = button(text("새로고침").size(13))
            .on_press(ServicesMessage::Refresh)
            .padding([8, 16]);

        let mut col = column![
            text("서비스 관리").size(22),
            Space::with_height(8),
            text("Apache와 MariaDB 서비스를 제어합니다.").size(13).color(Color::from_rgb(0.6, 0.6, 0.6)),
            Space::with_height(24),
            apache,
            Space::with_height(12),
            mariadb,
            Space::with_height(20),
            refresh_btn,
        ]
        .spacing(0);

        if let Some(err) = &self.error {
            col = col.push(Space::with_height(12)).push(
                container(text(format!("오류: {err}")).size(13).color(Color::from_rgb(1.0, 0.4, 0.4)))
                    .padding(12),
            );
        }

        col.into()
    }
}

fn service_card<'a>(
    name: &'a str,
    desc: &'a str,
    status: &'a ServiceStatus,
    service_id: &'a str,
) -> Element<'a, ServicesMessage> {
    let (status_text, status_color, is_running) = match status {
        ServiceStatus::Running => ("실행 중", Color::from_rgb(0.2, 0.9, 0.4), true),
        ServiceStatus::Stopped => ("중지됨", Color::from_rgb(0.9, 0.3, 0.3), false),
        ServiceStatus::Unknown => ("알 수 없음", Color::from_rgb(0.6, 0.6, 0.6), false),
    };

    let toggle_label = if is_running { "중지" } else { "시작" };
    let sid = service_id.to_string();

    let toggle_btn = button(text(toggle_label).size(13))
        .on_press(ServicesMessage::Toggle(sid, !is_running))
        .padding([8, 20])
        .style(move |_, _| button::Style {
            background: Some(iced::Background::Color(if is_running {
                Color::from_rgb(0.7, 0.2, 0.2)
            } else {
                Color::from_rgb(0.1, 0.5, 0.3)
            })),
            border: iced::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            text_color: Color::WHITE,
            ..Default::default()
        });

    let dot = container(Space::with_width(10))
        .width(10)
        .height(10)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(status_color)),
            border: iced::Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let info = column![
        text(name).size(16),
        Space::with_height(2),
        text(desc).size(12).color(Color::from_rgb(0.6, 0.6, 0.6)),
    ];

    let status_row = row![
        dot,
        Space::with_width(6),
        text(status_text).size(13).color(status_color),
    ]
    .align_y(iced::Alignment::Center);

    let card_content = row![
        info,
        Space::with_width(Length::Fill),
        column![status_row, Space::with_height(8), toggle_btn]
            .align_x(iced::Alignment::End),
    ]
    .align_y(iced::Alignment::Center);

    container(card_content)
        .padding(20)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13, 0.13, 0.16))),
            border: iced::Border {
                radius: 10.0.into(),
                color: Color::from_rgb(0.2, 0.2, 0.25),
                width: 1.0,
            },
            ..Default::default()
        })
        .into()
}
