mod services;
mod projects;
mod database;
mod settings;
mod transfer;
pub mod theme;

use iced::{
    keyboard,
    widget::{column, container, row, scrollable, text, Space},
    Background, Element, Length, Subscription, Task, Theme,
};

pub use services::ServicesMessage;
pub use projects::ProjectsMessage;
pub use database::DatabaseMessage;
pub use settings::SettingsMessage;
pub use transfer::TransferMessage;
use theme::{Icon, Tone, p};

#[derive(Debug, Clone, PartialEq)]
pub enum Tab {
    Services,
    Projects,
    Database,
    Transfer,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Services(ServicesMessage),
    Projects(ProjectsMessage),
    Database(DatabaseMessage),
    Transfer(TransferMessage),
    Settings(SettingsMessage),
    FocusNext,
    FocusPrevious,
}

pub struct App {
    active_tab: Tab,
    services: services::ServicesState,
    projects: projects::ProjectsState,
    database: database::DatabaseState,
    transfer: transfer::TransferState,
    settings: settings::SettingsState,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        // 테마를 가장 먼저 정한다 (다른 화면의 색이 여기에 따라 결정된다)
        let settings = settings::SettingsState::new();
        let (database, db_task) = database::DatabaseState::new();
        let (transfer, transfer_task) = transfer::TransferState::new();
        let app = Self {
            active_tab: Tab::Services,
            services: services::ServicesState::new(),
            projects: projects::ProjectsState::new(),
            database,
            transfer,
            settings,
        };
        (
            app,
            Task::batch([
                db_task.map(Message::Database),
                transfer_task.map(Message::Transfer),
                // 앱을 켤 때마다 만료가 다가온 인증서를 갱신한다
                Task::done(Message::Settings(SettingsMessage::RenewCerts)),
                // 사이트별 용량은 폴더를 훑어야 하므로 시작할 때 한 번 백그라운드로 잰다
                Task::done(Message::Projects(ProjectsMessage::ComputeUsage)),
                services::ServicesState::init_task().map(Message::Services),
            ]),
        )
    }

    pub fn theme(&self) -> Theme {
        theme::iced_theme()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TabSelected(tab) => {
                if tab == Tab::Services {
                    self.services.refresh();
                }
                self.active_tab = tab;
                Task::none()
            }
            Message::Services(msg) => self.services.update(msg).map(Message::Services),
            Message::Projects(msg) => {
                // 더보기 → "다른 PC로 보내기": 백업·이전 탭으로 넘어가 그 프로젝트를 골라 둔다
                if let ProjectsMessage::SendToPc(id) = &msg {
                    let id = id.clone();
                    self.active_tab = Tab::Transfer;
                    let t = self.projects.update(msg).map(Message::Projects);
                    let preset = self.transfer.update(TransferMessage::PresetProject(id)).map(Message::Transfer);
                    return Task::batch([t, preset]);
                }
                self.projects.update(msg).map(Message::Projects)
            }
            Message::Database(msg) => self.database.update(msg).map(Message::Database),
            Message::Transfer(msg) => {
                // 가져오기·받기·보내기가 끝나면 프로젝트 목록(과 이전 기록 표시)을 다시 읽는다
                let imported = matches!(
                    msg,
                    TransferMessage::Imported(_)
                        | TransferMessage::RecvEvent(crate::domain::lan::LanEvent::Done(_))
                        | TransferMessage::SendEvent(crate::domain::lan::LanEvent::Done(_))
                );
                let task = self.transfer.update(msg).map(Message::Transfer);
                if imported {
                    let refresh = self.projects.update(ProjectsMessage::Refresh).map(Message::Projects);
                    return Task::batch([task, refresh]);
                }
                task
            }
            Message::Settings(msg) => self.settings.update(msg).map(Message::Settings),
            Message::FocusNext => iced::widget::focus_next(),
            Message::FocusPrevious => iced::widget::focus_previous(),
        }
    }

    /// Tab/Shift+Tab으로 입력창 간 포커스 이동.
    /// text_input이 키를 소비하지 않을 때만(Status::Ignored) 들어오므로
    /// 입력 중인 텍스트를 가로채지 않는다.
    pub fn subscription(&self) -> Subscription<Message> {
        // 앱을 오래 켜 두는 경우를 위해 6시간마다 인증서 만료를 확인한다
        let renew = iced::time::every(std::time::Duration::from_secs(6 * 3600))
            .map(|_| Message::Settings(SettingsMessage::RenewCerts));
        // 테마가 "시스템"이면 OS 의 다크/라이트 전환을 따라간다
        let system_theme = iced::time::every(std::time::Duration::from_secs(5))
            .map(|_| Message::Settings(SettingsMessage::CheckSystemTheme));
        let keys = keyboard::on_key_press(|key, modifiers| match key {
            keyboard::Key::Named(keyboard::key::Named::Tab) => {
                if modifiers.shift() {
                    Some(Message::FocusPrevious)
                } else {
                    Some(Message::FocusNext)
                }
            }
            _ => None,
        });
        Subscription::batch([keys, renew, system_theme])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = match self.active_tab {
            Tab::Services => self.services.view().map(Message::Services),
            Tab::Projects => self.projects.view().map(Message::Projects),
            Tab::Database => self.database.view().map(Message::Database),
            Tab::Transfer => self.transfer.view().map(Message::Transfer),
            Tab::Settings => self.settings.view().map(Message::Settings),
        };

        let content = container(scrollable(container(content).padding(iced::Padding {
            top: 28.0,
            bottom: 28.0,
            left: 28.0,
            right: 28.0,
        })))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(p().app_bg)), ..Default::default() });

        row![self.sidebar(), theme::vdivider(), content].into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let nav = |i, label, tab: Tab| theme::nav_item(i, label, self.active_tab == tab, Message::TabSelected(tab));

        let logo = row![
            container(theme::icon(Icon::Server, 18.0, p().on_primary))
                .padding(8)
                .style(|_| container::Style {
                    background: Some(Background::Color(p().primary)),
                    border: iced::Border { radius: 9.0.into(), ..Default::default() },
                    ..Default::default()
                }),
            column![
                text("LocalMan").size(16).font(theme::BOLD).color(p().fg),
                text(concat!("v", env!("CARGO_PKG_VERSION"))).size(11).color(p().fg4),
            ],
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        // 아래쪽 요약: 서비스 상태와 HTTPS
        let mut summary = column![].spacing(6);
        for (name, st) in self.services.summary() {
            let (label, tone) = services::status_tone(st);
            summary = summary.push(
                row![
                    theme::dot(tone),
                    text(name).size(12).color(p().fg3).width(Length::Fill),
                    text(label).size(12).font(theme::MEDIUM).color(if tone == Tone::Success { p().fg2 } else { p().fg4 }),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        let (https_label, https_tone) = settings::https_summary(self.settings.https_on());
        summary = summary.push(
            row![theme::dot(https_tone), text(https_label).size(12).color(p().fg3)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
        );

        let col = column![
            logo,
            Space::with_height(22),
            nav(Icon::Server, "서비스", Tab::Services),
            nav(Icon::Folder, "프로젝트", Tab::Projects),
            nav(Icon::Database, "데이터베이스", Tab::Database),
            nav(Icon::ArrowLeftRight, "백업·이전", Tab::Transfer),
            nav(Icon::Settings, "설정", Tab::Settings),
            Space::with_height(Length::Fill),
            summary,
        ]
        .spacing(4)
        .padding(iced::Padding { top: 20.0, bottom: 20.0, left: 14.0, right: 14.0 })
        .width(220)
        .height(Length::Fill);

        container(col)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(p().chrome)),
                border: iced::Border { width: 0.0, ..Default::default() },
                ..Default::default()
            })
            .into()
    }
}
