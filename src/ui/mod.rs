mod services;
mod projects;
mod database;
mod settings;
mod transfer;
pub mod theme;

use iced::{
    keyboard,
    widget::{column, container, row, scrollable, stack, text, Space},
    Background, Element, Length, Subscription, Task, Theme,
};
use std::time::Duration;

/// 화면 오른쪽 아래에 잠깐 떴다 사라지는 알림. 오류는 오래, 성공은 짧게 띄운다.
struct Toast {
    id: u64,
    result: Result<String, String>,
}

pub use services::ServicesMessage;
pub use projects::ProjectsMessage;
pub use database::DatabaseMessage;
pub use settings::SettingsMessage;
pub use transfer::TransferMessage;
use theme::{Icon, Tone, p};
use crate::i18n::tr;

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
    /// 사이드바 [후원하기]
    Donate,
    DismissToast(u64),
    CopyToast(String),
}

pub struct App {
    active_tab: Tab,
    services: services::ServicesState,
    projects: projects::ProjectsState,
    database: database::DatabaseState,
    transfer: transfer::TransferState,
    settings: settings::SettingsState,
    toasts: Vec<Toast>,
    next_toast_id: u64,
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
            toasts: Vec::new(),
            next_toast_id: 0,
        };
        let mut services_task = app.services.init_task().map(Message::Services);
        if crate::domain::settings::load_settings().autostart_services {
            services_task = Task::batch([services_task, services::ServicesState::autostart_task().map(Message::Services)]);
        }
        (
            app,
            Task::batch([
                db_task.map(Message::Database),
                transfer_task.map(Message::Transfer),
                // 앱을 켤 때마다 만료가 다가온 인증서를 갱신한다
                Task::done(Message::Settings(SettingsMessage::RenewCerts)),
                // 사이트별 용량은 폴더를 훑어야 하므로 시작할 때 한 번 백그라운드로 잰다
                Task::done(Message::Projects(ProjectsMessage::ComputeUsage)),
                services_task,
            ]),
        )
    }

    pub fn theme(&self) -> Theme {
        theme::iced_theme()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.update_inner(message);
        // 각 탭이 쌓아둔 알림을 꺼내 토스트로 띄우고, 일정 시간 뒤 자동으로 닫는다.
        let pending: Vec<_> = self.projects.take_toasts().into_iter().chain(self.database.take_toasts()).collect();
        if pending.is_empty() {
            return task;
        }
        let mut tasks = vec![task];
        for result in pending {
            let id = self.next_toast_id;
            self.next_toast_id += 1;
            let secs = if result.is_err() { 8 } else { 3 };
            self.toasts.push(Toast { id, result });
            tasks.push(Task::perform(tokio::time::sleep(Duration::from_secs(secs)), move |_| Message::DismissToast(id)));
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
                // 서비스·프로젝트 화면은 Apache 등 상태를 보여주므로 들어갈 때 백그라운드로 다시 확인한다
                let services = matches!(tab, Tab::Services | Tab::Projects);
                self.active_tab = tab;
                if services {
                    self.services.refresh().map(Message::Services)
                } else {
                    Task::none()
                }
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
            Message::Donate => {
                self.settings.open_donate();
                self.active_tab = Tab::Settings;
                Task::none()
            }
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

        let base = row![self.sidebar(), theme::vdivider(), content];
        // 토스트가 없어도 항상 stack 으로 감싼다 — 루트 위젯 종류가 바뀌면
        // 입력 중인 text_input 의 포커스·커서 상태가 초기화된다.
        let toasts = column(self.toasts.iter().map(toast_view)).spacing(8).width(380);
        let layer = container(toasts)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom);
        stack![base, layer].into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let nav = |i, label, tab: Tab| theme::nav_item(i, label, self.active_tab == tab, Message::TabSelected(tab));

        let logo = row![
            container(theme::icon(Icon::HousePlug, 18.0, p().on_primary))
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
            nav(Icon::Server, tr("서비스"), Tab::Services),
            nav(Icon::Folder, tr("프로젝트"), Tab::Projects),
            nav(Icon::Database, tr("데이터베이스"), Tab::Database),
            nav(Icon::ArrowLeftRight, tr("백업·이전"), Tab::Transfer),
            nav(Icon::Settings, tr("설정"), Tab::Settings),
            Space::with_height(Length::Fill),
            // 사이드바 아래 작은 후원 줄 (mac-fan-control 과 같은 자리)
            iced::widget::button(
                row![theme::icon(Icon::Heart, 13.0, p().fg3), text(tr("후원하기")).size(12).color(p().fg3)]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            )
            .on_press(Message::Donate)
            .padding([6, 4])
            .style(|_, s| iced::widget::button::Style {
                background: (s == iced::widget::button::Status::Hovered).then(|| Background::Color(p().hover)),
                border: iced::Border { radius: theme::R_ROW.into(), ..Default::default() },
                ..Default::default()
            }),
            Space::with_height(8),
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

fn toast_view(t: &Toast) -> Element<'_, Message> {
    use theme::{Kind, Tone, btn};
    let (msg, tone, title) = match &t.result {
        Ok(m) => (m.as_str(), Tone::Success, tr("완료")),
        Err(e) => (e.as_str(), Tone::Danger, tr("오류")),
    };
    let mut actions = row![].spacing(4);
    if t.result.is_err() {
        actions = actions.push(btn(tr("복사"), Some(Icon::Copy), Kind::Ghost).on_press(Message::CopyToast(msg.to_string())));
    }
    actions = actions.push(btn(tr("닫기"), Some(Icon::X), Kind::Ghost).on_press(Message::DismissToast(t.id)));
    let accent = theme::tone_dot_color(tone);
    container(
        column![
            row![theme::status(title, tone), Space::with_width(Length::Fill), actions].align_y(iced::Alignment::Center),
            text(msg).size(13).color(p().fg),
        ]
        .spacing(4),
    )
    .padding(iced::Padding { top: 8.0, bottom: 14.0, left: 14.0, right: 8.0 })
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(p().c1)),
        border: iced::Border { color: accent, width: 1.0, radius: theme::R_CARD.into() },
        shadow: iced::Shadow { color: p().shadow, offset: iced::Vector::new(0.0, 4.0), blur_radius: 16.0 },
        ..Default::default()
    })
    .into()
}
