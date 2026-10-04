use iced::{
    widget::{column, container, row, text, Space},
    Element, Length, Task,
};
use super::theme::{
    self, Icon, Kind, btn, card, group, icon, input, muted, p, page_header, result_line, segmented,
};
use crate::domain::{
    list_databases, create_database, drop_database, backup_database, restore_database, import_sql,
    rename_database, list_users, create_user, drop_user, rename_user, change_user_password, grant_privileges, DbUser,
    DbCredentials, DbEngine, load_db_connections, save_db_connection, ensure_adminer_site,
};
use crate::platform::open_url;
use rfd;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SubTab {
    Databases,
    Users,
}

#[derive(Debug, Clone)]
pub enum DatabaseMessage {
    EngineSelected(DbEngine),
    SavedConnectionSelected(usize),
    UserChanged(String),
    PasswordChanged(String),
    TogglePasswordVisibility,
    Connect,
    Connected(Vec<String>, Vec<DbUser>),
    NewDbNameChanged(String),
    CreateDb,
    DropDb(String),
    EditDb(String),
    EditDbNameChanged(String),
    SaveDbEdit(String),
    CancelDbEdit,
    BackupDb(String),
    BackupPathSelected(String, Option<String>),
    RestoreDb(String),
    RestorePathSelected(String, Option<String>),
    ImportDbNameChanged(String),
    ImportSql,
    ImportPathSelected(Option<String>),
    CopyText(String),
    OpenAdminer,
    AdminerReady(Result<String, String>),
    Done(Result<String, String>),
    SubTabSelected(SubTab),
    NewUserNameChanged(String),
    NewUserPasswordChanged(String),
    NewUserHostChanged(String),
    CreateUser,
    DropUser(String, String),
    EditUser(String, String),
    EditUserNameChanged(String),
    EditUserPasswordChanged(String),
    SaveUserEdit(String, String),
    CancelUserEdit,
    GrantPrivileges(String, String, String),
    UserActionDone(Result<String, String>),
}

pub struct DatabaseState {
    engine: DbEngine,
    user: String,
    password: String,
    saved_connections: Vec<DbCredentials>,
    show_password: bool,
    databases: Vec<String>,
    db_users: Vec<DbUser>,
    new_db_name: String,
    import_db_name: String,
    connected: bool,
    status: Option<Result<String, String>>,
    editing_db: Option<String>,
    edit_db_name: String,
    subtab: SubTab,
    new_user_name: String,
    new_user_password: String,
    new_user_host: String,
    editing_user: Option<(String, String)>,
    edit_user_name: String,
    edit_user_password: String,
    user_status: Option<Result<String, String>>,
    // App 이 꺼내 토스트로 띄울 알림
    toasts: Vec<Result<String, String>>,
}

impl DatabaseState {
    pub fn take_toasts(&mut self) -> Vec<Result<String, String>> {
        std::mem::take(&mut self.toasts)
    }

    pub fn new() -> (Self, Task<DatabaseMessage>) {
        // 저장된 자격증명이 있으면 미리 채운다. 없으면 기본 root.
        let saved_connections = load_db_connections();
        let (engine, user, password, autoconnect) = match saved_connections.first() {
            Some(c) if !c.user.is_empty() => (c.engine, c.user.clone(), c.password.clone(), true),
            _ => (DbEngine::MariaDb, "root".to_string(), String::new(), false),
        };
        let state = Self {
            engine,
            user,
            password,
            saved_connections,
            show_password: false,
            databases: vec![],
            db_users: vec![],
            new_db_name: String::new(),
            import_db_name: String::new(),
            connected: false,
            status: None,
            editing_db: None,
            edit_db_name: String::new(),
            subtab: SubTab::Databases,
            new_user_name: String::new(),
            new_user_password: String::new(),
            new_user_host: "localhost".to_string(),
            editing_user: None,
            edit_user_name: String::new(),
            edit_user_password: String::new(),
            user_status: None,
            toasts: Vec::new(),
        };
        // 저장된 자격증명이 있으면 시작 시 자동으로 연결한다.
        let task = if autoconnect {
            Task::done(DatabaseMessage::Connect)
        } else {
            Task::none()
        };
        (state, task)
    }

    pub fn update(&mut self, msg: DatabaseMessage) -> Task<DatabaseMessage> {
        match msg {
            DatabaseMessage::EngineSelected(engine) => {
                self.engine = engine;
                self.connected = false;
                self.databases.clear();
                self.db_users.clear();
                self.status = None;
                // 바꾼 엔진으로 저장된 접속이 있으면 그 자격증명을 쓴다. 없으면 기본 관리자명으로
                // 되돌리고, 다른 엔진의 비밀번호를 그대로 들고 가지 않는다.
                match self.saved_connections.iter().find(|c| c.engine == engine && !c.user.is_empty()) {
                    Some(saved) => {
                        self.user = saved.user.clone();
                        self.password = saved.password.clone();
                    }
                    None => {
                        self.user = match engine {
                            DbEngine::PostgreSql => "postgres".to_string(),
                            DbEngine::MariaDb => "root".to_string(),
                        };
                        self.password.clear();
                    }
                }
                self.new_user_host = if engine == DbEngine::PostgreSql {
                    "local".to_string()
                } else {
                    "localhost".to_string()
                };
                // 자격증명이 갖춰졌으면 Connect 를 누르지 않아도 바로 목록을 불러온다.
                if self.password.is_empty() {
                    Task::none()
                } else {
                    Task::done(DatabaseMessage::Connect)
                }
            }
            DatabaseMessage::SavedConnectionSelected(index) => {
                if let Some(saved) = self.saved_connections.get(index).cloned() {
                    self.engine = saved.engine;
                    self.user = saved.user;
                    self.password = saved.password;
                    self.connected = false;
                    self.databases.clear();
                    self.db_users.clear();
                    self.status = Some(Ok(format!("저장된 연결 선택: {} / {}", self.engine.label(), self.user)));
                    self.new_user_host = if self.engine == DbEngine::PostgreSql {
                        "local".to_string()
                    } else {
                        "localhost".to_string()
                    };
                    // 고른 접속으로 바로 연결한다.
                    if !self.password.is_empty() {
                        return Task::done(DatabaseMessage::Connect);
                    }
                }
                Task::none()
            }
            DatabaseMessage::UserChanged(v) => { self.user = v; Task::none() }
            DatabaseMessage::PasswordChanged(v) => { self.password = v; Task::none() }
            DatabaseMessage::TogglePasswordVisibility => {
                self.show_password = !self.show_password;
                Task::none()
            }
            DatabaseMessage::Connect => {
                let engine = self.engine;
                let u = self.user.clone();
                let p = self.password.clone();
                Task::perform(
                    async move {
                        let dbs = list_databases(engine, &u, &p).iter().map(|d| d.name.clone()).collect();
                        let users = list_users(engine, &u, &p);
                        (dbs, users)
                    },
                    |(dbs, users)| DatabaseMessage::Connected(dbs, users),
                )
            }
            DatabaseMessage::Connected(dbs, users) => {
                self.connected = !dbs.is_empty() || !users.is_empty();
                self.databases = dbs;
                self.db_users = users;
                self.status = if self.connected {
                    // 연결 성공 시 자격증명 저장 (다음 실행 때 자동 입력)
                    match save_db_connection(self.engine, &self.user, &self.password) {
                        Ok(list) => self.saved_connections = list,
                        Err(e) => self.toasts.push(Err(format!("접속 정보 저장 실패: {e}"))),
                    }
                    Some(Ok(format!(
                        "{} / {} 연결 성공 (접속 목록에 저장됨)",
                        self.engine.label(),
                        self.user
                    )))
                } else {
                    Some(Err(format!(
                        "{} / {} 연결 실패 또는 DB 없음",
                        self.engine.label(),
                        self.user
                    )))
                };
                Task::none()
            }
            DatabaseMessage::NewDbNameChanged(v) => { self.new_db_name = v; Task::none() }
            DatabaseMessage::CreateDb => {
                let u = self.user.clone();
                let p = self.password.clone();
                let name = self.new_db_name.clone();
                let engine = self.engine;
                self.new_db_name.clear();
                Task::perform(
                    async move {
                        create_database(engine, &u, &p, &name)
                            .map(|_| format!("'{name}' 생성 완료"))
                    },
                    DatabaseMessage::Done,
                )
            }
            DatabaseMessage::DropDb(name) => {
                let u = self.user.clone();
                let p = self.password.clone();
                let n = name.clone();
                let engine = self.engine;
                Task::perform(
                    async move {
                        drop_database(engine, &u, &p, &n)
                            .map(|_| format!("'{n}' 삭제 완료"))
                    },
                    DatabaseMessage::Done,
                )
            }
            DatabaseMessage::EditDb(name) => {
                self.editing_db = Some(name.clone());
                self.edit_db_name = name;
                Task::none()
            }
            DatabaseMessage::EditDbNameChanged(v) => { self.edit_db_name = v; Task::none() }
            DatabaseMessage::SaveDbEdit(old_name) => {
                let new_name = self.edit_db_name.trim().to_string();
                if new_name.is_empty() {
                    self.status = Some(Err("새 DB 이름을 입력하세요.".to_string()));
                    self.toasts.push(Err("새 DB 이름을 입력하세요.".to_string()));
                    return Task::none();
                }
                let u = self.user.clone();
                let p = self.password.clone();
                let engine = self.engine;
                self.editing_db = None;
                self.edit_db_name.clear();
                Task::perform(
                    async move {
                        rename_database(engine, &u, &p, &old_name, &new_name)
                            .map(|_| format!("'{old_name}' → '{new_name}' 이름 변경 완료"))
                    },
                    DatabaseMessage::Done,
                )
            }
            DatabaseMessage::CancelDbEdit => {
                self.editing_db = None;
                self.edit_db_name.clear();
                Task::none()
            }
            DatabaseMessage::BackupDb(name) => {
                Task::perform(pick_save_file(name.clone()), move |path| {
                    DatabaseMessage::BackupPathSelected(name.clone(), path)
                })
            }
            DatabaseMessage::BackupPathSelected(name, path) => {
                if let Some(path) = path {
                    let u = self.user.clone();
                    let p = self.password.clone();
                    let engine = self.engine;
                    return Task::perform(
                        async move {
                            backup_database(engine, &u, &p, &name, &path)
                                .map(|_| format!("백업 완료: {path}"))
                        },
                        DatabaseMessage::Done,
                    );
                }
                Task::none()
            }
            DatabaseMessage::RestoreDb(name) => {
                Task::perform(pick_open_file(), move |path| {
                    DatabaseMessage::RestorePathSelected(name.clone(), path)
                })
            }
            DatabaseMessage::RestorePathSelected(name, path) => {
                if let Some(path) = path {
                    let u = self.user.clone();
                    let p = self.password.clone();
                    let engine = self.engine;
                    return Task::perform(
                        async move {
                            restore_database(engine, &u, &p, &name, &path)
                                .map(|_| format!("복원 완료: {name}"))
                        },
                        DatabaseMessage::Done,
                    );
                }
                Task::none()
            }
            DatabaseMessage::ImportDbNameChanged(v) => { self.import_db_name = v; Task::none() }
            DatabaseMessage::ImportSql => {
                Task::perform(pick_open_file(), DatabaseMessage::ImportPathSelected)
            }
            DatabaseMessage::ImportPathSelected(path) => {
                if let Some(path) = path {
                    // 대상 DB 이름이 비어 있으면 파일명에서 유추한다.
                    let mut db_name = self.import_db_name.trim().to_string();
                    if db_name.is_empty() {
                        db_name = derive_db_name(&path);
                    }
                    let u = self.user.clone();
                    let p = self.password.clone();
                    let engine = self.engine;
                    return Task::perform(
                        async move {
                            import_sql(engine, &u, &p, &db_name, &path, true)
                                .map(|_| format!("가져오기 완료: {db_name} ← {path}"))
                        },
                        DatabaseMessage::Done,
                    );
                }
                Task::none()
            }
            DatabaseMessage::CopyText(s) => {
                return iced::clipboard::write(s);
            }
            DatabaseMessage::OpenAdminer => {
                self.status = Some(Ok("Adminer 준비 중...".to_string()));
                Task::perform(
                    async move { ensure_adminer_site() },
                    DatabaseMessage::AdminerReady,
                )
            }
            DatabaseMessage::AdminerReady(result) => {
                match result {
                    Ok(url) => {
                        open_url(&url);
                        self.status = Some(Ok(format!("Adminer 열림: {url}")));
                    }
                    Err(e) => {
                        self.toasts.push(Err(e.clone()));
                        self.status = Some(Err(e));
                    }
                }
                Task::none()
            }
            DatabaseMessage::Done(result) => {
                self.toasts.push(result.clone());
                if result.is_ok() {
                    let u = self.user.clone();
                    let p = self.password.clone();
                    self.databases = list_databases(self.engine, &u, &p)
                        .iter().map(|d| d.name.clone()).collect();
                    self.import_db_name.clear();
                }
                self.status = Some(result);
                Task::none()
            }
            DatabaseMessage::SubTabSelected(t) => {
                self.subtab = t;
                Task::none()
            }
            DatabaseMessage::NewUserNameChanged(v) => { self.new_user_name = v; Task::none() }
            DatabaseMessage::NewUserPasswordChanged(v) => { self.new_user_password = v; Task::none() }
            DatabaseMessage::NewUserHostChanged(v) => { self.new_user_host = v; Task::none() }
            DatabaseMessage::CreateUser => {
                if self.new_user_name.is_empty() || self.new_user_password.is_empty() {
                    self.user_status = Some(Err("사용자명과 비밀번호를 입력하세요.".to_string()));
                    return Task::none();
                }
                let admin = self.user.clone();
                let admin_pw = self.password.clone();
                let new_user = self.new_user_name.clone();
                let new_pw = self.new_user_password.clone();
                let host = self.new_user_host.clone();
                let engine = self.engine;
                self.new_user_name.clear();
                self.new_user_password.clear();
                Task::perform(
                    async move {
                        create_user(engine, &admin, &admin_pw, &new_user, &new_pw, &host)
                            .map(|_| format!("'{new_user}'@'{host}' 생성 완료"))
                    },
                    DatabaseMessage::UserActionDone,
                )
            }
            DatabaseMessage::DropUser(target, host) => {
                let admin = self.user.clone();
                let admin_pw = self.password.clone();
                let t = target.clone();
                let h = host.clone();
                let engine = self.engine;
                Task::perform(
                    async move {
                        drop_user(engine, &admin, &admin_pw, &t, &h)
                            .map(|_| format!("'{t}'@'{h}' 삭제 완료"))
                    },
                    DatabaseMessage::UserActionDone,
                )
            }
            DatabaseMessage::EditUser(target, host) => {
                self.editing_user = Some((target.clone(), host));
                self.edit_user_name = target;
                self.edit_user_password.clear();
                Task::none()
            }
            DatabaseMessage::EditUserNameChanged(v) => { self.edit_user_name = v; Task::none() }
            DatabaseMessage::EditUserPasswordChanged(v) => { self.edit_user_password = v; Task::none() }
            DatabaseMessage::SaveUserEdit(target, host) => {
                let new_user = self.edit_user_name.trim().to_string();
                let new_password = self.edit_user_password.clone();
                if new_user.is_empty() {
                    self.user_status = Some(Err("새 사용자명을 입력하세요.".to_string()));
                    self.toasts.push(Err("새 사용자명을 입력하세요.".to_string()));
                    return Task::none();
                }
                let admin = self.user.clone();
                let admin_pw = self.password.clone();
                let engine = self.engine;
                self.editing_user = None;
                self.edit_user_name.clear();
                self.edit_user_password.clear();
                Task::perform(
                    async move {
                        if new_user != target {
                            rename_user(engine, &admin, &admin_pw, &target, &host, &new_user)?;
                        }
                        if !new_password.is_empty() {
                            change_user_password(engine, &admin, &admin_pw, &new_user, &host, &new_password)?;
                        }
                        Ok(format!("'{target}' 사용자 편집 완료"))
                    },
                    DatabaseMessage::UserActionDone,
                )
            }
            DatabaseMessage::CancelUserEdit => {
                self.editing_user = None;
                self.edit_user_name.clear();
                self.edit_user_password.clear();
                Task::none()
            }
            DatabaseMessage::GrantPrivileges(target, host, db) => {
                let admin = self.user.clone();
                let admin_pw = self.password.clone();
                let t = target.clone();
                let h = host.clone();
                let d = db.clone();
                let engine = self.engine;
                Task::perform(
                    async move {
                        grant_privileges(engine, &admin, &admin_pw, &t, &h, &d)
                            .map(|_| format!("'{t}'@'{h}' → {d} 권한 부여 완료"))
                    },
                    DatabaseMessage::UserActionDone,
                )
            }
            DatabaseMessage::UserActionDone(result) => {
                self.toasts.push(result.clone());
                if result.is_ok() {
                    let u = self.user.clone();
                    let p = self.password.clone();
                    self.db_users = list_users(self.engine, &u, &p);
                }
                self.user_status = Some(result);
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, DatabaseMessage> {
        let adminer = btn("Adminer 열기", Some(Icon::ExternalLink), Kind::Surface).on_press(DatabaseMessage::OpenAdminer);

        let conn = card(
            column![
                row![
                    field(
                        "엔진",
                        segmented(
                            &[(DbEngine::MariaDb, "MariaDB·MySQL"), (DbEngine::PostgreSql, "PostgreSQL")],
                            &self.engine,
                            DatabaseMessage::EngineSelected,
                        ),
                    ),
                    container(field("사용자", input("root", &self.user).on_input(DatabaseMessage::UserChanged).into())).width(160),
                    container(field(
                        "비밀번호",
                        row![
                            input("password", &self.password)
                                .on_input(DatabaseMessage::PasswordChanged)
                                .secure(!self.show_password)
                                .width(Length::Fill),
                            btn(if self.show_password { "숨기기" } else { "보기" }, None, Kind::Ghost)
                                .on_press(DatabaseMessage::TogglePasswordVisibility),
                        ]
                        .spacing(4)
                        .align_y(iced::Alignment::Center)
                        .into(),
                    ))
                    .width(Length::Fill),
                    column![Space::with_height(19), btn("연결", Some(Icon::Zap), Kind::Primary).on_press(DatabaseMessage::Connect)],
                ]
                .spacing(12)
                .align_y(iced::Alignment::End),
                Space::with_height(8),
                muted(match self.engine {
                    DbEngine::MariaDb => "기본 root 비밀번호: root",
                    DbEngine::PostgreSql => "기본 사용자: postgres · 포트 5432",
                }),
                saved_connections_view(&self.saved_connections),
            ]
            .spacing(0),
        );

        let tabs = segmented(
            &[(SubTab::Databases, "데이터베이스"), (SubTab::Users, "사용자")],
            &self.subtab,
            DatabaseMessage::SubTabSelected,
        );

        let body: Element<DatabaseMessage> = match self.subtab {
            SubTab::Databases => self.view_databases(),
            SubTab::Users => self.view_users(),
        };

        let subtitle: &'static str = match self.engine {
            DbEngine::MariaDb => "MariaDB 데이터베이스와 사용자를 관리하고 백업·복원합니다",
            DbEngine::PostgreSql => "PostgreSQL 데이터베이스와 사용자를 관리하고 백업·복원합니다",
        };
        let mut col = column![
            page_header("데이터베이스", subtitle, Some(adminer.into())),
            Space::with_height(20),
            conn,
            Space::with_height(16),
            tabs,
            Space::with_height(14),
        ];
        if let Some(status) = &self.status {
            col = col.push(status_card(status)).push(Space::with_height(12));
        }
        col.push(body).into()
    }

    fn view_databases(&self) -> Element<'_, DatabaseMessage> {
        let create = card(
            row![
                input("새 데이터베이스 이름", &self.new_db_name)
                    .on_input(DatabaseMessage::NewDbNameChanged)
                    .width(Length::Fill),
                btn("만들기", Some(Icon::Plus), Kind::Primary).on_press(DatabaseMessage::CreateDb),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );

        let import = card(
            column![
                row![icon(Icon::Upload, 14.0, p().fg2), theme::title("SQL 가져오기 · 복원")]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                muted(".sql 또는 .sql.gz 덤프를 고르면 대상 DB로 가져옵니다. DB가 없으면 새로 만듭니다."),
                Space::with_height(8),
                row![
                    input("대상 DB 이름 (비우면 파일 이름)", &self.import_db_name)
                        .on_input(DatabaseMessage::ImportDbNameChanged)
                        .width(Length::Fill),
                    btn("파일 골라 가져오기", Some(Icon::FolderOpen), Kind::Flat).on_press(DatabaseMessage::ImportSql),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(4),
        );

        let list: Element<DatabaseMessage> = if self.databases.is_empty() {
            card(empty_state(if self.connected { "데이터베이스가 없습니다" } else { "연결하면 목록이 나옵니다" }))
        } else {
            group(
                self.databases
                    .iter()
                    .map(|db| db_row(db, &self.db_users, self.editing_db.as_deref(), &self.edit_db_name))
                    .collect(),
            )
        };

        column![
            row![create, import].spacing(12),
            Space::with_height(14),
            theme::section_label("데이터베이스 목록"),
            list,
        ]
        .into()
    }

    fn view_users(&self) -> Element<'_, DatabaseMessage> {
        let create = card(
            column![
                theme::title("새 사용자"),
                Space::with_height(10),
                row![
                    field("사용자 이름", input("dbuser", &self.new_user_name).on_input(DatabaseMessage::NewUserNameChanged).into()),
                    field(
                        "비밀번호",
                        input("password", &self.new_user_password)
                            .on_input(DatabaseMessage::NewUserPasswordChanged)
                            .secure(true)
                            .into(),
                    ),
                    container(field("호스트", input("localhost", &self.new_user_host).on_input(DatabaseMessage::NewUserHostChanged).into()))
                        .width(140),
                    column![Space::with_height(19), btn("추가", Some(Icon::Plus), Kind::Primary).on_press(DatabaseMessage::CreateUser)],
                ]
                .spacing(12)
                .align_y(iced::Alignment::End),
            ],
        );

        let list: Element<DatabaseMessage> = if self.db_users.is_empty() {
            card(empty_state(if self.connected { "사용자가 없습니다" } else { "연결하면 목록이 나옵니다" }))
        } else {
            group(
                self.db_users
                    .iter()
                    .map(|u| user_row(u, self.editing_user.as_ref(), &self.edit_user_name, &self.edit_user_password))
                    .collect(),
            )
        };

        let mut col = column![create, Space::with_height(14)];
        if let Some(status) = &self.user_status {
            col = col.push(status_card(status)).push(Space::with_height(12));
        }
        col.push(theme::section_label("사용자 목록")).push(list).into()
    }
}

fn field<'a>(label: &'a str, control: Element<'a, DatabaseMessage>) -> Element<'a, DatabaseMessage> {
    column![text(label).size(12).font(theme::MEDIUM).color(p().fg3), control]
        .spacing(6)
        .width(Length::Fill)
        .into()
}

fn empty_state<'a>(msg: &'a str) -> Element<'a, DatabaseMessage> {
    column![icon(Icon::Database, 22.0, p().fg4), text(msg).size(13).color(p().fg3)]
        .spacing(6)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill)
        .into()
}

fn status_card(status: &Result<String, String>) -> Element<'_, DatabaseMessage> {
    let (line, raw) = match status {
        Ok(m) => (format!("✓ {m}"), m.clone()),
        Err(e) => (format!("✗ {e}"), e.clone()),
    };
    card(
        row![
            container(result_line(line)).width(Length::Fill),
            btn("복사", Some(Icon::Copy), Kind::Ghost).on_press(DatabaseMessage::CopyText(raw)),
        ]
        .align_y(iced::Alignment::Center),
    )
}

fn saved_connections_view(saved: &[DbCredentials]) -> Element<'_, DatabaseMessage> {
    if saved.is_empty() {
        return Space::with_height(0).into();
    }
    let chips = saved.iter().enumerate().fold(row![].spacing(6), |r, (i, c)| {
        r.push(
            theme::chip_btn(format!("{} · {}", c.engine.label(), c.user), Kind::Flat)
                .on_press(DatabaseMessage::SavedConnectionSelected(i)),
        )
    });
    column![
        Space::with_height(12),
        text("저장된 연결").size(12).font(theme::MEDIUM).color(p().fg3),
        Space::with_height(6),
        chips,
    ]
    .into()
}

fn db_row<'a>(db: &'a str, users: &'a [DbUser], editing_db: Option<&'a str>, edit_db_name: &'a str) -> Element<'a, DatabaseMessage> {
    if editing_db == Some(db) {
        return row![
            input("DB 이름", edit_db_name).on_input(DatabaseMessage::EditDbNameChanged).width(Length::Fill),
            btn("저장", Some(Icon::Check), Kind::Primary).on_press(DatabaseMessage::SaveDbEdit(db.to_string())),
            btn("취소", None, Kind::Ghost).on_press(DatabaseMessage::CancelDbEdit),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into();
    }

    let mut left = column![
        row![icon(Icon::Database, 14.0, p().fg3), text(db).size(14).font(theme::MEDIUM).color(p().fg)]
            .spacing(8)
            .align_y(iced::Alignment::Center),
    ]
    .spacing(6)
    .width(Length::Fill);
    if !users.is_empty() {
        let grants = users.iter().fold(row![muted("권한 주기")].spacing(4).align_y(iced::Alignment::Center), |r, u| {
            r.push(
                theme::chip_btn(u.username.clone(), Kind::Flat)
                    .on_press(DatabaseMessage::GrantPrivileges(u.username.clone(), u.host.clone(), db.to_string())),
            )
        });
        left = left.push(grants);
    }

    row![
        left,
        btn("이름 변경", Some(Icon::Pencil), Kind::Flat).on_press(DatabaseMessage::EditDb(db.to_string())),
        btn("백업", Some(Icon::Download), Kind::Flat).on_press(DatabaseMessage::BackupDb(db.to_string())),
        btn("복원", Some(Icon::Upload), Kind::Flat).on_press(DatabaseMessage::RestoreDb(db.to_string())),
        btn("삭제", Some(Icon::Trash), Kind::Danger).on_press(DatabaseMessage::DropDb(db.to_string())),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

fn user_row<'a>(
    u: &'a DbUser,
    editing_user: Option<&'a (String, String)>,
    edit_user_name: &'a str,
    edit_user_password: &'a str,
) -> Element<'a, DatabaseMessage> {
    let is_editing = editing_user.map(|(name, host)| name == &u.username && host == &u.host).unwrap_or(false);
    if is_editing {
        return row![
            field("사용자 이름", input("사용자 이름", edit_user_name).on_input(DatabaseMessage::EditUserNameChanged).into()),
            field(
                "새 비밀번호",
                input("비워 두면 그대로", edit_user_password)
                    .on_input(DatabaseMessage::EditUserPasswordChanged)
                    .secure(true)
                    .into(),
            ),
            btn("저장", Some(Icon::Check), Kind::Primary).on_press(DatabaseMessage::SaveUserEdit(u.username.clone(), u.host.clone())),
            btn("취소", None, Kind::Ghost).on_press(DatabaseMessage::CancelUserEdit),
        ]
        .spacing(8)
        .align_y(iced::Alignment::End)
        .into();
    }
    row![
        column![
            text(&u.username).size(14).font(theme::MEDIUM).color(p().fg),
            muted(format!("호스트 {}", u.host)),
        ]
        .spacing(2)
        .width(Length::Fill),
        btn("수정", Some(Icon::Pencil), Kind::Flat).on_press(DatabaseMessage::EditUser(u.username.clone(), u.host.clone())),
        btn("삭제", Some(Icon::Trash), Kind::Danger).on_press(DatabaseMessage::DropUser(u.username.clone(), u.host.clone())),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

async fn pick_save_file(db_name: String) -> Option<String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("백업 파일 저장 위치")
        .set_file_name(&format!("{db_name}.sql"))
        .add_filter("SQL", &["sql"])
        .save_file()
        .await;

    match result {
        Some(handle) => {
            let path = handle.path().to_string_lossy().to_string();
            eprintln!("[localman] 백업 경로 선택: {path}");
            Some(path)
        }
        None => {
            eprintln!("[localman] 백업 경로 선택 취소");
            None
        }
    }
}

/// 파일 경로에서 DB 이름을 유추한다.
/// 예: `/path/eond.sql` → `eond`, `/path/eond.sql.gz` → `eond`
fn derive_db_name(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("imported");
    // .sql.gz / .sql / .gz 등 확장자를 모두 제거
    let mut name = stem;
    for ext in [".sql.gz", ".sql", ".gz", ".dump"] {
        if let Some(stripped) = name.strip_suffix(ext) {
            name = stripped;
            break;
        }
    }
    if name.is_empty() { "imported".to_string() } else { name.to_string() }
}

async fn pick_open_file() -> Option<String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("복원할 SQL 파일 선택")
        .add_filter("SQL", &["sql"])
        .pick_file()
        .await;

    match result {
        Some(handle) => {
            let path = handle.path().to_string_lossy().to_string();
            eprintln!("[localman] 복원 파일 선택: {path}");
            Some(path)
        }
        None => {
            eprintln!("[localman] 복원 파일 선택 취소");
            None
        }
    }
}
