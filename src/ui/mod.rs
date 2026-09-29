mod services;
mod projects;
mod database;

use iced::{
    keyboard,
    widget::{button, column, container, row, stack, text, Space},
    Color, Element, Length, Subscription, Task,
};
use std::time::Duration;

/// 화면 우하단에 잠깐 떴다 사라지는 알림. 오류는 오래, 성공은 짧게 띄운다.
struct Toast {
    id: u64,
    result: Result<String, String>,
}

pub use services::ServicesMessage;
pub use projects::ProjectsMessage;
pub use database::DatabaseMessage;

#[derive(Debug, Clone)]
pub enum Tab {
    Services,
    Projects,
    Database,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Services(ServicesMessage),
    Projects(ProjectsMessage),
    Database(DatabaseMessage),
    #[allow(dead_code)]
    RefreshServices,
    FocusNext,
    FocusPrevious,
    DismissToast(u64),
    CopyToast(String),
}

pub struct App {
    active_tab: Tab,
    services: services::ServicesState,
    projects: projects::ProjectsState,
    database: database::DatabaseState,
    toasts: Vec<Toast>,
    next_toast_id: u64,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let (database, db_task) = database::DatabaseState::new();
        let app = Self {
            active_tab: Tab::Services,
            services: services::ServicesState::new(),
            projects: projects::ProjectsState::new(),
            database,
            toasts: Vec::new(),
            next_toast_id: 0,
        };
        (app, db_task.map(Message::Database))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.update_inner(message);
        // 각 탭이 쌓아둔 알림을 꺼내 토스트로 띄우고, 일정 시간 뒤 자동으로 닫는다.
        let pending: Vec<_> = self.projects.take_toasts().into_iter()
            .chain(self.database.take_toasts())
            .collect();
        if pending.is_empty() {
            return task;
        }
        let mut tasks = vec![task];
        for result in pending {
            let id = self.next_toast_id;
            self.next_toast_id += 1;
            let secs = if result.is_err() { 8 } else { 3 };
            self.toasts.push(Toast { id, result });
            tasks.push(Task::perform(
                tokio::time::sleep(Duration::from_secs(secs)),
                move |_| Message::DismissToast(id),
            ));
        }
        // 너무 많이 쌓이면 오래된 것부터 버린다.
        if self.toasts.len() > 4 {
            let extra = self.toasts.len() - 4;
            self.toasts.drain(..extra);
        }
        Task::batch(tasks)
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DismissToast(id) => {
                self.toasts.retain(|t| t.id != id);
                Task::none()
            }
            Message::CopyToast(s) => iced::clipboard::write(s),
            Message::TabSelected(tab) => {
                self.active_tab = tab;
                Task::none()
            }
            Message::RefreshServices => {
                self.services.refresh();
                Task::none()
            }
            Message::Services(msg) => {
                self.services.update(msg).map(Message::Services)
            }
            Message::Projects(msg) => {
                self.projects.update(msg).map(Message::Projects)
            }
            Message::Database(msg) => {
                self.database.update(msg).map(Message::Database)
            }
            Message::FocusNext => iced::widget::focus_next(),
            Message::FocusPrevious => iced::widget::focus_previous(),
        }
    }

    /// Tab/Shift+Tab으로 입력창 간 포커스 이동.
    /// text_input이 키를 소비하지 않을 때만(Status::Ignored) 들어오므로
    /// 입력 중인 텍스트를 가로채지 않는다.
    pub fn subscription(&self) -> Subscription<Message> {
        keyboard::on_key_press(|key, modifiers| match key {
            keyboard::Key::Named(keyboard::key::Named::Tab) => {
                if modifiers.shift() {
                    Some(Message::FocusPrevious)
                } else {
                    Some(Message::FocusNext)
                }
            }
            _ => None,
        })
    }

    pub fn view(&self) -> Element<'_, Message> {
        let sidebar = column![
            sidebar_logo(),
            Space::with_height(20),
            tab_button("서비스", matches!(self.active_tab, Tab::Services), Message::TabSelected(Tab::Services)),
            tab_button("프로젝트", matches!(self.active_tab, Tab::Projects), Message::TabSelected(Tab::Projects)),
            tab_button("데이터베이스", matches!(self.active_tab, Tab::Database), Message::TabSelected(Tab::Database)),
        ]
        .width(200)
        .padding(12)
        .spacing(4);

        let sidebar = container(sidebar)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.1, 0.12))),
                ..Default::default()
            })
            .height(Length::Fill);

        let content = match self.active_tab {
            Tab::Services => self.services.view().map(Message::Services),
            Tab::Projects => self.projects.view().map(Message::Projects),
            Tab::Database => self.database.view().map(Message::Database),
        };

        let content = container(content)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill);

        let base = row![sidebar, content];

        // 토스트가 없어도 항상 stack 으로 감싼다 — 루트 위젯 종류가 바뀌면
        // 입력 중인 text_input 의 포커스·커서 상태가 초기화된다.
        let toasts = column(self.toasts.iter().map(toast_view)).spacing(8).width(380);
        let toast_layer = container(toasts)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom);

        stack![base, toast_layer].into()
    }
}

fn toast_view(t: &Toast) -> Element<'_, Message> {
    let (msg, accent, title) = match &t.result {
        Ok(m) => (m.as_str(), Color::from_rgb(0.2, 0.8, 0.4), "완료"),
        Err(e) => (e.as_str(), Color::from_rgb(0.95, 0.35, 0.35), "오류"),
    };
    let mut actions = row![].spacing(6);
    if t.result.is_err() {
        actions = actions.push(
            button(text("복사").size(12))
                .on_press(Message::CopyToast(msg.to_string()))
                .padding([2, 8]),
        );
    }
    actions = actions.push(
        button(text("닫기").size(12))
            .on_press(Message::DismissToast(t.id))
            .padding([2, 8]),
    );

    container(
        column![
            row![
                text(title).size(13).color(accent).width(Length::Fill),
                actions,
            ]
            .align_y(iced::Alignment::Center),
            text(msg).size(13).color(Color::WHITE),
        ]
        .spacing(6),
    )
    .padding(12)
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(iced::Background::Color(Color::from_rgb(0.14, 0.14, 0.17))),
        border: iced::Border { color: accent, width: 1.0, radius: 8.0.into() },
        ..Default::default()
    })
    .into()
}

fn sidebar_logo<'a>() -> Element<'a, Message> {
    container(
        text("LocalMan")
            .size(20)
            .color(Color::from_rgb(0.4, 0.8, 1.0)),
    )
    .padding([12, 8])
    .into()
}

fn tab_button(label: &str, active: bool, msg: Message) -> Element<'_, Message> {
    let bg = if active {
        Color::from_rgb(0.2, 0.4, 0.6)
    } else {
        Color::TRANSPARENT
    };

    button(
        text(label)
            .size(14)
            .width(Length::Fill),
    )
    .on_press(msg)
    .width(Length::Fill)
    .padding([10, 14])
    .style(move |_, _| button::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            radius: 6.0.into(),
            ..Default::default()
        },
        text_color: Color::WHITE,
        ..Default::default()
    })
    .into()
}
