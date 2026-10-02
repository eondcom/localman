mod services;
mod projects;
mod database;
mod transfer;

use iced::{
    keyboard,
    widget::{button, column, container, row, text, Space},
    Color, Element, Length, Subscription, Task,
};

pub use services::ServicesMessage;
pub use projects::ProjectsMessage;
pub use database::DatabaseMessage;
pub use transfer::TransferMessage;

#[derive(Debug, Clone)]
pub enum Tab {
    Services,
    Projects,
    Database,
    Transfer,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Services(ServicesMessage),
    Projects(ProjectsMessage),
    Database(DatabaseMessage),
    Transfer(TransferMessage),
    #[allow(dead_code)]
    RefreshServices,
    FocusNext,
    FocusPrevious,
}

pub struct App {
    active_tab: Tab,
    services: services::ServicesState,
    projects: projects::ProjectsState,
    database: database::DatabaseState,
    transfer: transfer::TransferState,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let (database, db_task) = database::DatabaseState::new();
        let (transfer, transfer_task) = transfer::TransferState::new();
        let app = Self {
            active_tab: Tab::Services,
            services: services::ServicesState::new(),
            projects: projects::ProjectsState::new(),
            database,
            transfer,
        };
        (
            app,
            Task::batch([db_task.map(Message::Database), transfer_task.map(Message::Transfer)]),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
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
            Message::Transfer(msg) => {
                // 가져오기로 프로젝트가 바뀌면 프로젝트 탭 목록도 다시 읽는다
                let imported = matches!(msg, TransferMessage::Imported(_));
                let task = self.transfer.update(msg).map(Message::Transfer);
                if imported {
                    let refresh = self.projects.update(ProjectsMessage::Refresh).map(Message::Projects);
                    return Task::batch([task, refresh]);
                }
                task
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
            tab_button("백업·이전", matches!(self.active_tab, Tab::Transfer), Message::TabSelected(Tab::Transfer)),
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
            Tab::Transfer => self.transfer.view().map(Message::Transfer),
        };

        let content = container(content)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill);

        row![sidebar, content].into()
    }
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
