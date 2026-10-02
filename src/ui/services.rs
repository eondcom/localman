use iced::{
    widget::{button, column, container, row, text, Space},
    Color, Element, Length, Task,
};
use crate::platform::{ServiceStatus, ca_trusted, get_service_status, install_service, toggle_service};
use crate::domain::apache::{renew_certs, set_https};
use crate::domain::settings::load_settings;
use crate::domain::tls::{ca_cert_path, ca_exists};

#[derive(Debug, Clone)]
pub enum ServicesMessage {
    Toggle(String, bool),
    Install(String),
    Refresh,
    Toggled(String, Result<(), String>),
    Installed(String, Result<(), String>),
    SetHttps(bool),
    HttpsSet(Result<Vec<String>, String>),
    RenewCerts,
    CertsRenewed(Vec<String>),
}

pub struct ServicesState {
    apache_status: ServiceStatus,
    mariadb_status: ServiceStatus,
    postgresql_status: ServiceStatus,
    installing: Option<String>,
    error: Option<String>,
    https: bool,
    ca_trusted: bool,
    https_busy: bool,
    https_log: Vec<String>,
}

impl ServicesState {
    pub fn new() -> Self {
        let mut s = Self {
            apache_status: ServiceStatus::Unknown,
            mariadb_status: ServiceStatus::Unknown,
            postgresql_status: ServiceStatus::Unknown,
            installing: None,
            error: None,
            https: false,
            ca_trusted: false,
            https_busy: false,
            https_log: Vec::new(),
        };
        s.refresh();
        s
    }

    pub fn refresh(&mut self) {
        self.apache_status = get_service_status("apache2");
        self.mariadb_status = get_service_status("mariadb");
        self.postgresql_status = get_service_status("postgresql");
        self.https = load_settings().https;
        self.ca_trusted = ca_exists() && ca_trusted(&ca_cert_path());
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
            ServicesMessage::Install(name) => {
                self.installing = Some(name.clone());
                let service = name.clone();
                Task::perform(
                    async move { install_service(&service) },
                    move |result| ServicesMessage::Installed(name.clone(), result),
                )
            }
            ServicesMessage::Toggled(name, result) => {
                match result {
                    Ok(_) => {
                        self.error = None;
                        match name.as_str() {
                            "apache2" => self.apache_status = get_service_status("apache2"),
                            "mariadb" => self.mariadb_status = get_service_status("mariadb"),
                            "postgresql" => self.postgresql_status = get_service_status("postgresql"),
                            _ => {}
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
                Task::none()
            }
            ServicesMessage::SetHttps(on) => {
                self.https_busy = true;
                self.https_log.clear();
                Task::perform(
                    async move { tokio::task::spawn_blocking(move || set_https(on)).await.unwrap_or_else(|e| Err(e.to_string())) },
                    ServicesMessage::HttpsSet,
                )
            }
            ServicesMessage::HttpsSet(result) => {
                self.https_busy = false;
                match result {
                    Ok(log) => self.https_log = log,
                    Err(e) => self.https_log = vec![format!("✗ {e}")],
                }
                self.refresh();
                Task::none()
            }
            ServicesMessage::RenewCerts => Task::perform(
                async { tokio::task::spawn_blocking(renew_certs).await.unwrap_or_default() },
                ServicesMessage::CertsRenewed,
            ),
            ServicesMessage::CertsRenewed(log) => {
                if !log.is_empty() {
                    self.https_log = log;
                }
                Task::none()
            }
            ServicesMessage::Installed(name, result) => {
                // 설치 중이던 그 서비스의 응답일 때만 잠금을 푼다. 순서가 엇갈려도
                // GUI 가 죽어서는 안 되므로 단언하지 않고 조용히 넘긴다.
                if self.installing.as_deref() == Some(name.as_str()) {
                    self.installing = None;
                }
                match result {
                    Ok(_) => {
                        self.error = None;
                        self.refresh();
                    }
                    Err(error) => self.error = Some(error),
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
            self.installing.as_deref() == Some("apache2"),
        );

        let mariadb = service_card(
            "MariaDB",
            "데이터베이스 서버",
            &self.mariadb_status,
            "mariadb",
            self.installing.as_deref() == Some("mariadb"),
        );

        let postgresql = service_card(
            "PostgreSQL",
            "데이터베이스 서버",
            &self.postgresql_status,
            "postgresql",
            self.installing.as_deref() == Some("postgresql"),
        );

        let refresh_btn = button(text("새로고침").size(13))
            .on_press(ServicesMessage::Refresh)
            .padding([8, 16]);

        let mut col = column![
            text("서비스 관리").size(22),
            Space::with_height(8),
            text("Apache, MariaDB, PostgreSQL 서비스를 제어합니다.").size(13).color(Color::from_rgb(0.6, 0.6, 0.6)),
            Space::with_height(24),
            apache,
            Space::with_height(12),
            mariadb,
            Space::with_height(12),
            postgresql,
            Space::with_height(12),
            self.https_card(),
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

impl ServicesState {
    fn https_card(&self) -> Element<'_, ServicesMessage> {
        let muted = Color::from_rgb(0.6, 0.6, 0.6);
        let (status, color) = if !self.https {
            ("꺼짐", muted)
        } else if self.ca_trusted {
            ("켜짐 · 인증기관 신뢰됨", Color::from_rgb(0.2, 0.9, 0.4))
        } else {
            ("켜짐 · 인증기관 신뢰 안 됨 (브라우저 경고)", Color::from_rgb(0.85, 0.6, 0.2))
        };
        let label = match (self.https_busy, self.https) {
            (true, _) => "처리 중…",
            (false, true) => "끄기",
            (false, false) => "켜기",
        };
        let mut btn = button(text(label).size(13)).padding([8, 16]);
        if !self.https_busy {
            btn = btn.on_press(ServicesMessage::SetHttps(!self.https));
        }
        let mut info = column![
            text("HTTPS").size(16),
            Space::with_height(2),
            text("로컬 인증기관으로 모든 프로젝트를 https://도메인 으로도 엽니다. 인증서는 만료 30일 전에 자동 갱신됩니다.")
                .size(12)
                .color(muted),
        ];
        if self.https && !self.ca_trusted {
            info = info.push(Space::with_height(6)).push(
                text("끄고 다시 켜면 인증기관 신뢰 등록을 다시 시도합니다.").size(12).color(muted),
            );
        }
        for l in &self.https_log {
            let c = if l.starts_with('✗') { Color::from_rgb(1.0, 0.4, 0.4) } else { muted };
            info = info.push(text(l).size(12).color(c));
        }
        container(
            row![
                info.width(Length::Fill),
                Space::with_width(16),
                column![text(status).size(13).color(color), Space::with_height(8), btn]
                    .align_x(iced::Alignment::End),
            ]
            .align_y(iced::Alignment::Center),
        )
        .padding(20)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13, 0.13, 0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2, 0.2, 0.25), width: 1.0 },
            ..Default::default()
        })
        .into()
    }
}

fn service_card<'a>(
    name: &'a str,
    desc: &'a str,
    status: &'a ServiceStatus,
    service_id: &'a str,
    installing: bool,
) -> Element<'a, ServicesMessage> {
    let (status_text, status_color, is_running) = match status {
        ServiceStatus::Running => ("실행 중", Color::from_rgb(0.2, 0.9, 0.4), true),
        ServiceStatus::Stopped => ("중지됨", Color::from_rgb(0.9, 0.3, 0.3), false),
        ServiceStatus::NotInstalled => ("설치되지 않음", Color::from_rgb(0.85, 0.6, 0.2), false),
        ServiceStatus::Unknown => ("알 수 없음", Color::from_rgb(0.6, 0.6, 0.6), false),
    };

    let is_not_installed = matches!(status, ServiceStatus::NotInstalled);
    let toggle_label = if installing {
        "설치 중…"
    } else if is_not_installed {
        "설치하기"
    } else if is_running {
        "중지"
    } else {
        "시작"
    };
    let sid = service_id.to_string();

    let mut toggle_btn = button(text(toggle_label).size(13))
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

    if !installing {
        let message = if is_not_installed {
            ServicesMessage::Install(sid)
        } else {
            ServicesMessage::Toggle(sid, !is_running)
        };
        toggle_btn = toggle_btn.on_press(message);
    }

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
