use iced::{
    widget::{column, row, text, Space},
    Element, Length, Task,
};
use crate::platform::{ServiceStatus, get_service_status, install_service, toggle_service};
use super::theme::{self, Icon, Kind, Tone, btn, card, chip, group, icon, muted, p, page_header, result_line, section_label, setting_row};
use crate::domain::tools::{TOOLS, Tool, install as install_tool, installed_version, latest_lts};

#[derive(Debug, Clone)]
pub enum ServicesMessage {
    Toggle(String, bool),
    Install(String),
    Refresh,
    Toggled(String, Result<(), String>),
    Installed(String, Result<(), String>),
    ToolsChecked(Vec<(Tool, Option<String>)>),
    LtsFetched(Option<(String, String)>),
    InstallTool(Tool),
    ToolInstalled(Tool, Result<String, String>),
}

pub struct ServicesState {
    apache_status: ServiceStatus,
    mariadb_status: ServiceStatus,
    postgresql_status: ServiceStatus,
    installing: Option<String>,
    error: Option<String>,
    tools: Vec<(Tool, Option<String>)>,
    /// nodejs.org 의 지금 LTS (버전, 이름)
    lts: Option<(String, String)>,
    tool_installing: Option<Tool>,
    tool_log: Vec<String>,
}

fn check_tools() -> Task<ServicesMessage> {
    Task::perform(
        async { tokio::task::spawn_blocking(|| TOOLS.iter().map(|t| (*t, installed_version(*t))).collect()).await.unwrap_or_default() },
        ServicesMessage::ToolsChecked,
    )
}

/// "24.21.0" 의 24
fn major(v: &str) -> Option<u32> {
    v.trim_start_matches('v').split('.').next()?.parse().ok()
}

/// (UI 이름, 서비스 id, 설명, 아이콘)
const SERVICES: [(&str, &str, &str, Icon); 3] = [
    ("Apache", "apache2", "웹 서버 · *.localhost", Icon::Globe),
    ("MariaDB", "mariadb", "데이터베이스 서버", Icon::Database),
    ("PostgreSQL", "postgresql", "데이터베이스 서버", Icon::Layers),
];

pub fn status_tone(s: &ServiceStatus) -> (&'static str, Tone) {
    match s {
        ServiceStatus::Running => ("실행 중", Tone::Success),
        ServiceStatus::Stopped => ("중지됨", Tone::Neutral),
        ServiceStatus::NotInstalled => ("미설치", Tone::Warning),
        ServiceStatus::Unknown => ("알 수 없음", Tone::Neutral),
    }
}

impl ServicesState {
    pub fn new() -> Self {
        let mut s = Self {
            apache_status: ServiceStatus::Unknown,
            mariadb_status: ServiceStatus::Unknown,
            postgresql_status: ServiceStatus::Unknown,
            installing: None,
            error: None,
            tools: Vec::new(),
            lts: None,
            tool_installing: None,
            tool_log: Vec::new(),
        };
        s.refresh();
        s
    }

    /// 앱 시작 때: 도구 버전과 Node LTS 를 백그라운드로 확인한다
    pub fn init_task() -> Task<ServicesMessage> {
        Task::batch([
            check_tools(),
            Task::perform(
                async { tokio::task::spawn_blocking(|| latest_lts().ok()).await.ok().flatten() },
                ServicesMessage::LtsFetched,
            ),
        ])
    }

    pub fn refresh(&mut self) {
        self.apache_status = get_service_status("apache2");
        self.mariadb_status = get_service_status("mariadb");
        self.postgresql_status = get_service_status("postgresql");
    }

    fn status_of(&self, id: &str) -> &ServiceStatus {
        match id {
            "apache2" => &self.apache_status,
            "mariadb" => &self.mariadb_status,
            _ => &self.postgresql_status,
        }
    }

    /// 사이드바 아래 요약용: (이름, 상태)
    pub fn summary(&self) -> Vec<(&'static str, &ServiceStatus)> {
        SERVICES.iter().map(|(name, id, ..)| (*name, self.status_of(id))).collect()
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
            ServicesMessage::ToolsChecked(list) => {
                self.tools = list;
                Task::none()
            }
            ServicesMessage::LtsFetched(lts) => {
                self.lts = lts;
                Task::none()
            }
            ServicesMessage::InstallTool(t) => {
                self.tool_installing = Some(t);
                self.tool_log.clear();
                Task::perform(
                    async move { tokio::task::spawn_blocking(move || install_tool(t)).await.unwrap_or_else(|e| Err(e.to_string())) },
                    move |r| ServicesMessage::ToolInstalled(t, r),
                )
            }
            ServicesMessage::ToolInstalled(_, r) => {
                self.tool_installing = None;
                self.tool_log = vec![match r {
                    Ok(m) => format!("✓ {m}"),
                    Err(e) => format!("✗ {e}"),
                }];
                check_tools()
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
        let refresh = btn("새로고침", Some(Icon::Refresh), Kind::Surface).on_press(ServicesMessage::Refresh);

        let tiles = SERVICES.iter().fold(row![].spacing(12), |r, (name, id, desc, ic)| {
            r.push(self.tile(name, id, desc, *ic))
        });

        let mut col = column![
            page_header("서비스", "웹 서버와 데이터베이스를 켜고 끕니다", Some(refresh.into())),
            Space::with_height(20),
            tiles,
            Space::with_height(18),
            section_label("개발 도구"),
            self.tools_view(),
        ];

        if let Some(err) = &self.error {
            col = col.push(Space::with_height(16)).push(card(
                row![icon(Icon::X, 14.0, p().danger_fg), text(err).size(13).color(p().danger_fg)].spacing(8),
            ));
        }
        col.into()
    }

    fn tools_view(&self) -> Element<'_, ServicesMessage> {
        let mut rows: Vec<Element<ServicesMessage>> = Vec::new();
        for (t, ver) in &self.tools {
            let lts = self.lts.as_ref().filter(|_| *t == Tool::Node);
            // Node 는 LTS 보다 오래된 판이면 LTS 설치를 권한다
            let outdated = match (lts, ver) {
                (Some((l, _)), Some(v)) => major(v).zip(major(l)).is_some_and(|(a, b)| a < b),
                _ => false,
            };
            let status: Element<ServicesMessage> = match ver {
                Some(v) if outdated => chip(format!("v{v} · LTS 아님"), Tone::Warning),
                Some(v) => chip(format!("v{v}"), Tone::Success),
                None => chip("미설치", Tone::Warning),
            };
            let label: String = match (t, lts) {
                (Tool::Node, Some((l, name))) => format!("{l} {name} LTS 설치"),
                (Tool::Node, None) => "LTS 설치".into(),
                _ => "설치".into(),
            };
            let action: Element<ServicesMessage> = if self.tool_installing == Some(*t) {
                theme::status("설치 중…", Tone::Primary)
            } else if ver.is_none() || outdated {
                let b = btn(label, Some(Icon::Download), Kind::Primary);
                if self.tool_installing.is_none() { b.on_press(ServicesMessage::InstallTool(*t)).into() } else { b.into() }
            } else {
                Space::with_width(0).into()
            };
            rows.push(setting_row(
                t.label(),
                Some(t.help()),
                row![status, action].spacing(10).align_y(iced::Alignment::Center).into(),
            ));
        }
        if rows.is_empty() {
            rows.push(muted("도구 버전을 확인하는 중…").into());
        }
        if !self.tool_log.is_empty() {
            rows.push(self.tool_log.iter().fold(column![].spacing(4), |c, l| c.push(result_line(l))).into());
        }
        group(rows)
    }

    fn tile<'a>(&'a self, name: &'a str, id: &'a str, desc: &'a str, ic: Icon) -> Element<'a, ServicesMessage> {
        let status = self.status_of(id);
        let (label, tone) = status_tone(status);
        let installing = self.installing.as_deref() == Some(id);
        let running = matches!(status, ServiceStatus::Running);
        let not_installed = matches!(status, ServiceStatus::NotInstalled);

        let action = if installing {
            btn("설치 중…", Some(Icon::Download), Kind::Flat)
        } else if not_installed {
            btn("설치하기", Some(Icon::Download), Kind::Primary).on_press(ServicesMessage::Install(id.to_string()))
        } else if running {
            btn("중지", Some(Icon::Square), Kind::Danger).on_press(ServicesMessage::Toggle(id.to_string(), false))
        } else {
            btn("시작", Some(Icon::Play), Kind::Success).on_press(ServicesMessage::Toggle(id.to_string(), true))
        };

        card(
            column![
                row![
                    icon(ic, 14.0, p().fg3),
                    text(name).size(13).font(theme::MEDIUM).color(p().fg3),
                    Space::with_width(Length::Fill),
                    chip(label, tone),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
                Space::with_height(10),
                text(label).size(26).font(theme::BOLD).color(p().fg),
                muted(desc),
                Space::with_height(14),
                action,
            ]
            .spacing(2),
        )
    }
}
