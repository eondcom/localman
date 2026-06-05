mod services;
mod projects;
mod database;

use iced::{
    widget::{button, column, container, row, text, Space}, Color, Element, Length, Task,
};

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
}

pub struct App {
    active_tab: Tab,
    services: services::ServicesState,
    projects: projects::ProjectsState,
    database: database::DatabaseState,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            active_tab: Tab::Services,
            services: services::ServicesState::new(),
            projects: projects::ProjectsState::new(),
            database: database::DatabaseState::new(),
        };
        (app, Task::none())
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
        }
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
