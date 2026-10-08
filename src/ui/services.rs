use iced::{
    widget::{column, row, text, Space},
    Element, Length, Task,
};
use crate::i18n::{tr, trf};
use crate::platform::{ServiceStatus, get_service_status, install_service, toggle_service};
use super::theme::{self, Icon, Kind, Tone, btn, card, chip, group, icon, muted, p, page_header, result_line, section_label, setting_row};
use crate::domain::tools::{NodeSupport, TOOLS, Tool, install as install_tool, installed_version, latest_lts, node_schedule, node_support, today};

#[derive(Debug, Clone)]
pub enum ServicesMessage {
    Toggle(String, bool),
    Install(String),
    Refresh,
    Toggled(String, Result<(), String>),
    Installed(String, Result<(), String>),
    ToolsChecked(Vec<(Tool, Option<String>)>, bool),
    LtsFetched(Option<(String, String)>, Option<serde_json::Value>),
    InstallTool(Tool),
    ToolInstalled(Result<String, String>),
}

pub struct ServicesState {
    apache_status: ServiceStatus,
    mariadb_status: ServiceStatus,
    postgresql_status: ServiceStatus,
    installing: Option<String>,
    error: Option<String>,
    /// DB 서버 이름 — OS·설치 상태에 따라 다르다 (맥 Intel 은 앱이 설치한 MySQL).
    /// brew 조회가 느려 새로 고칠 때만 정한다.
    db_label: &'static str,
    tools: Vec<(Tool, Option<String>)>,
    /// nodejs.org 의 지금 LTS (버전, 이름)
    lts: Option<(String, String)>,
    /// nodejs/Release 의 릴리스 일정 (설치된 Node 가 지원 중인지 판단)
    node_schedule: Option<serde_json::Value>,
    /// Apache 용 PHP 모듈이 있는지 (php 명령만 있고 모듈이 없을 수 있다 — MAMP 등)
    php_apache: bool,
    tool_installing: Option<Tool>,
    tool_log: Vec<String>,
}

fn check_tools() -> Task<ServicesMessage> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                (TOOLS.iter().map(|t| (*t, installed_version(*t))).collect(), crate::platform::php_module_ready())
            })
            .await
            .unwrap_or_default()
        },
        |(list, php)| ServicesMessage::ToolsChecked(list, php),
    )
}


/// (UI 이름, 서비스 id, 설명, 아이콘) — 설명은 그릴 때 tr() 로 번역한다
const SERVICES: [(&str, &str, &str, Icon); 3] = [
    ("Apache", "apache2", "웹 서버 · *.localhost", Icon::Globe),
    ("MariaDB", "mariadb", "데이터베이스 서버", Icon::Database),
    ("PostgreSQL", "postgresql", "데이터베이스 서버", Icon::Layers),
];

pub fn status_tone(s: &ServiceStatus) -> (&'static str, Tone) {
    match s {
        ServiceStatus::Running => (tr("실행 중"), Tone::Success),
        ServiceStatus::Stopped => (tr("중지됨"), Tone::Neutral),
        ServiceStatus::NotInstalled => (tr("미설치"), Tone::Warning),
        ServiceStatus::Unknown => (tr("알 수 없음"), Tone::Neutral),
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
            db_label: "MariaDB",
            tools: Vec::new(),
            lts: None,
            node_schedule: None,
            php_apache: false,
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
                async {
                    tokio::task::spawn_blocking(|| (latest_lts().ok(), node_schedule().ok()))
                        .await
                        .unwrap_or((None, None))
                },
                |(lts, sch)| ServicesMessage::LtsFetched(lts, sch),
            ),
        ])
    }

    pub fn refresh(&mut self) {
        self.apache_status = get_service_status("apache2");
        self.mariadb_status = get_service_status("mariadb");
        self.postgresql_status = get_service_status("postgresql");
        self.db_label = crate::platform::db_service_label();
    }

    fn display_name(&self, name: &'static str, id: &str) -> &'static str {
        if id == "mariadb" { self.db_label } else { name }
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
        SERVICES.iter().map(|(name, id, ..)| (self.display_name(name, id), self.status_of(id))).collect()
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
            ServicesMessage::ToolsChecked(list, php) => {
                self.tools = list;
                self.php_apache = php;
                Task::none()
            }
            ServicesMessage::LtsFetched(lts, sch) => {
                self.lts = lts;
                self.node_schedule = sch;
                Task::none()
            }
            ServicesMessage::InstallTool(t) => {
                self.tool_installing = Some(t);
                self.tool_log.clear();
                Task::perform(
                    async move { tokio::task::spawn_blocking(move || install_tool(t)).await.unwrap_or_else(|e| Err(e.to_string())) },
                    ServicesMessage::ToolInstalled,
                )
            }
            ServicesMessage::ToolInstalled(r) => {
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
        let refresh = btn(tr("새로고침"), Some(Icon::Refresh), Kind::Surface).on_press(ServicesMessage::Refresh);

        let tiles = SERVICES.iter().fold(row![].spacing(12), |r, (name, id, desc, ic)| {
            r.push(self.tile(self.display_name(name, id), id, tr(desc), *ic))
        });

        let mut col = column![
            page_header(tr("서비스"), tr("웹 서버와 데이터베이스를 켜고 끕니다"), Some(refresh.into())),
            Space::with_height(20),
            tiles,
            Space::with_height(18),
            section_label(tr("개발 도구")),
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
            // Node 는 공식 릴리스 일정으로 지원 상태를 본다 (일정을 못 받으면 버전만 보여준다)
            let support = match (t, ver, &self.node_schedule) {
                (Tool::Node, Some(v), Some(sch)) => Some(node_support(sch, v, &today())),
                _ => None,
            };
            let php_unlinked = *t == Tool::Php && ver.is_some() && !self.php_apache;
            let status: Element<ServicesMessage> = match (ver, &support) {
                (None, _) => chip(tr("미설치"), Tone::Warning),
                (Some(v), _) if php_unlinked => chip(trf("v{0} · Apache 연결 안 됨", &[v]), Tone::Warning),
                (Some(v), Some(NodeSupport::ActiveLts(n))) => chip(format!("v{v} · {n} LTS"), Tone::Success),
                (Some(v), Some(NodeSupport::MaintenanceLts(n, end))) => {
                    chip(trf("v{0} · {1} 유지보수 LTS ({2} 종료)", &[v, n, &&end[..end.len().min(7)]]), Tone::Warning)
                }
                (Some(v), Some(NodeSupport::Current)) => chip(trf("v{0} · LTS 아님", &[v]), Tone::Warning),
                (Some(v), Some(NodeSupport::Eol)) => chip(trf("v{0} · 지원 종료", &[v]), Tone::Danger),
                (Some(v), _) => chip(format!("v{v}"), Tone::Success),
            };
            let label: String = match (t, lts) {
                (Tool::Node, Some((l, name))) => trf("{0} {1} LTS 설치", &[l, name]),
                (Tool::Node, None) => tr("LTS 설치").into(),
                (Tool::Php, _) if php_unlinked => tr("Apache용 PHP 설치").into(),
                _ => tr("설치").into(),
            };
            // 없거나·지원 종료·LTS 아님 → 주 버튼, 유지보수 LTS → 보조 버튼
            let kind = match (ver, &support) {
                _ if php_unlinked => Some(Kind::Primary),
                (None, _) | (_, Some(NodeSupport::Eol | NodeSupport::Current)) => Some(Kind::Primary),
                (_, Some(NodeSupport::MaintenanceLts(..))) => Some(Kind::Flat),
                _ => None,
            };
            let action: Element<ServicesMessage> = if self.tool_installing == Some(*t) {
                theme::status(tr("설치 중…"), Tone::Primary)
            } else if let Some(kind) = kind {
                let b = btn(label, Some(Icon::Download), kind);
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
            rows.push(muted(tr("도구 버전을 확인하는 중…")).into());
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
            btn(tr("설치 중…"), Some(Icon::Download), Kind::Flat)
        } else if not_installed {
            btn(tr("설치하기"), Some(Icon::Download), Kind::Primary).on_press(ServicesMessage::Install(id.to_string()))
        } else if running {
            btn(tr("중지"), Some(Icon::Square), Kind::Danger).on_press(ServicesMessage::Toggle(id.to_string(), false))
        } else {
            btn(tr("시작"), Some(Icon::Play), Kind::Success).on_press(ServicesMessage::Toggle(id.to_string(), true))
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
