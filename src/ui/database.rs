use iced::{
    widget::{button, column, container, row, text, text_input, Space, scrollable},
    Color, Element, Length, Task,
};
use crate::system::{
    list_databases, create_database, drop_database, backup_database, restore_database, import_sql,
    rename_database, list_users, create_user, drop_user, rename_user, change_user_password, grant_privileges, DbUser,
    DbCredentials, DbEngine, load_db_connections, save_db_connection, ensure_adminer_site, open_url,
};
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
}

impl DatabaseState {
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
                if self.user == "root" && engine == DbEngine::PostgreSql {
                    self.user = "postgres".to_string();
                } else if self.user == "postgres" && engine == DbEngine::MariaDb {
                    self.user = "root".to_string();
                }
                self.new_user_host = if engine == DbEngine::PostgreSql {
                    "local".to_string()
                } else {
                    "localhost".to_string()
                };
                Task::none()
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
                    if let Ok(list) = save_db_connection(self.engine, &self.user, &self.password) {
                        self.saved_connections = list;
                    }
                    Some(Ok("연결 성공 (접속 목록에 저장됨)".to_string()))
                } else {
                    Some(Err("연결 실패 또는 DB 없음".to_string()))
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
                    Err(e) => self.status = Some(Err(e)),
                }
                Task::none()
            }
            DatabaseMessage::Done(result) => {
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
        let conn_panel = container(
            column![
                row![
                    column![
                        text("엔진").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        row![
                            engine_btn("MariaDB", self.engine == DbEngine::MariaDb, DatabaseMessage::EngineSelected(DbEngine::MariaDb)),
                            Space::with_width(6),
                            engine_btn("PostgreSQL", self.engine == DbEngine::PostgreSql, DatabaseMessage::EngineSelected(DbEngine::PostgreSql)),
                        ],
                    ].width(230),
                    Space::with_width(10),
                    column![
                        text("사용자").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        text_input("root", &self.user)
                            .on_input(DatabaseMessage::UserChanged)
                            .padding(10),
                    ].width(150),
                    Space::with_width(10),
                    column![
                        text("비밀번호").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        row![
                            text_input("password", &self.password)
                                .on_input(DatabaseMessage::PasswordChanged)
                                .secure(!self.show_password)
                                .padding(10)
                                .width(Length::Fill),
                            Space::with_width(6),
                            button(text(if self.show_password { "숨기기" } else { "보기" }).size(12))
                                .on_press(DatabaseMessage::TogglePasswordVisibility)
                                .padding([10, 12])
                                .style(|_, _| button::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.2, 0.25))),
                                    border: iced::Border { radius: 6.0.into(), ..Default::default() },
                                    text_color: Color::WHITE,
                                    ..Default::default()
                                }),
                        ],
                        Space::with_height(4),
                        text(match self.engine {
                            DbEngine::MariaDb => "기본 root 비밀번호: root",
                            DbEngine::PostgreSql => "기본 사용자: postgres, 포트: 5432",
                        }).size(11).color(Color::from_rgb(0.45,0.5,0.45)),
                    ].width(260),
                    Space::with_width(10),
                    column![
                        Space::with_height(18),
                        button(text("연결").size(13))
                            .on_press(DatabaseMessage::Connect)
                            .padding([10, 20])
                            .style(|_, _| button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.45, 0.7))),
                                border: iced::Border { radius: 6.0.into(), ..Default::default() },
                                text_color: Color::WHITE,
                                ..Default::default()
                            }),
                    ],
                ]
                .align_y(iced::Alignment::End),
                saved_connections_view(&self.saved_connections),
            ]
        )
        .padding(16)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
            ..Default::default()
        });

        let subtab_row = row![
            subtab_btn("DB 목록", self.subtab == SubTab::Databases, DatabaseMessage::SubTabSelected(SubTab::Databases)),
            Space::with_width(8),
            subtab_btn("사용자 관리", self.subtab == SubTab::Users, DatabaseMessage::SubTabSelected(SubTab::Users)),
        ];

        let body: Element<DatabaseMessage> = match self.subtab {
            SubTab::Databases => self.view_databases(),
            SubTab::Users => self.view_users(),
        };

        let header_row = row![
            column![
                text("데이터베이스").size(22),
                Space::with_height(8),
                text(format!("{} 데이터베이스를 관리하고 백업/복원합니다.", self.engine.label())).size(13).color(Color::from_rgb(0.6,0.6,0.6)),
            ].width(Length::Fill),
            button(text("🛢 Adminer 열기").size(13))
                .on_press(DatabaseMessage::OpenAdminer)
                .padding([10, 18])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.45, 0.35))),
                    border: iced::Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
        ].align_y(iced::Alignment::Center);

        let mut col = column![
            header_row,
            Space::with_height(20),
            conn_panel,
            Space::with_height(16),
            subtab_row,
            Space::with_height(16),
            body,
        ];

        if let Some(status) = &self.status {
            let (msg, color) = match status {
                Ok(m) => (m.as_str(), Color::from_rgb(0.2, 0.9, 0.4)),
                Err(e) => (e.as_str(), Color::from_rgb(1.0, 0.4, 0.4)),
            };
            col = col.push(Space::with_height(10)).push(
                row![
                    text(msg).size(13).color(color).width(Length::Fill),
                    Space::with_width(8),
                    copy_btn(msg.to_string()),
                ].align_y(iced::Alignment::Center)
            );
        }

        col.into()
    }

    fn view_databases(&self) -> Element<'_, DatabaseMessage> {
        let create_panel = container(
            row![
                text_input("새 데이터베이스 이름", &self.new_db_name)
                    .on_input(DatabaseMessage::NewDbNameChanged)
                    .padding(10)
                    .width(Length::Fill),
                Space::with_width(8),
                button(text("생성").size(13))
                    .on_press(DatabaseMessage::CreateDb)
                    .padding([10, 18])
                    .style(|_, _| button::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.5, 0.3))),
                        border: iced::Border { radius: 6.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        ..Default::default()
                    }),
            ]
        )
        .padding(14)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
            ..Default::default()
        });

        let import_panel = container(
            column![
                text("SQL 가져오기 / 복원").size(14),
                Space::with_height(4),
                text(".sql 또는 .sql.gz 덤프를 선택하면 대상 DB로 가져옵니다. DB가 없으면 자동 생성됩니다.")
                    .size(11).color(Color::from_rgb(0.55,0.55,0.55)),
                Space::with_height(10),
                row![
                    text_input("대상 DB 이름 (비우면 파일명 사용)", &self.import_db_name)
                        .on_input(DatabaseMessage::ImportDbNameChanged)
                        .padding(10)
                        .width(Length::Fill),
                    Space::with_width(8),
                    button(text("SQL 파일 선택 후 가져오기").size(13))
                        .on_press(DatabaseMessage::ImportSql)
                        .padding([10, 18])
                        .style(|_, _| button::Style {
                            background: Some(iced::Background::Color(Color::from_rgb(0.4, 0.25, 0.1))),
                            border: iced::Border { radius: 6.0.into(), ..Default::default() },
                            text_color: Color::WHITE,
                            ..Default::default()
                        }),
                ]
                .align_y(iced::Alignment::Center),
            ]
        )
        .padding(14)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
            ..Default::default()
        });

        let db_list: Element<DatabaseMessage> = if self.databases.is_empty() {
            container(
                text(if self.connected { "데이터베이스 없음" } else { "연결 후 목록이 표시됩니다." })
                    .size(13).color(Color::from_rgb(0.5,0.5,0.5))
            ).padding(16).into()
        } else {
            let items: Vec<Element<DatabaseMessage>> = self.databases.iter().map(|db| {
                db_row(db, &self.db_users, self.editing_db.as_deref(), &self.edit_db_name)
            }).collect();
            scrollable(column(items).spacing(6)).into()
        };

        column![create_panel, Space::with_height(12), import_panel, Space::with_height(12), db_list].into()
    }

    fn view_users(&self) -> Element<'_, DatabaseMessage> {
        let create_panel = container(
            column![
                text("새 사용자 추가").size(14),
                Space::with_height(10),
                row![
                    column![
                        text("사용자명").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        text_input("dbuser", &self.new_user_name)
                            .on_input(DatabaseMessage::NewUserNameChanged)
                            .padding(9),
                    ].width(Length::FillPortion(2)),
                    Space::with_width(10),
                    column![
                        text("비밀번호").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        text_input("password", &self.new_user_password)
                            .on_input(DatabaseMessage::NewUserPasswordChanged)
                            .secure(true)
                            .padding(9),
                    ].width(Length::FillPortion(2)),
                    Space::with_width(10),
                    column![
                        text("호스트").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        text_input("localhost", &self.new_user_host)
                            .on_input(DatabaseMessage::NewUserHostChanged)
                            .padding(9),
                    ].width(120),
                    Space::with_width(10),
                    column![
                        Space::with_height(18),
                        button(text("추가").size(13))
                            .on_press(DatabaseMessage::CreateUser)
                            .padding([9, 18])
                            .style(|_, _| button::Style {
                                background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.5, 0.3))),
                                border: iced::Border { radius: 6.0.into(), ..Default::default() },
                                text_color: Color::WHITE,
                                ..Default::default()
                            }),
                    ],
                ],
            ]
        )
        .padding(16)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
            ..Default::default()
        });

        let user_list: Element<DatabaseMessage> = if self.db_users.is_empty() {
            container(
                text(if self.connected { "사용자 없음" } else { "연결 후 목록이 표시됩니다." })
                    .size(13).color(Color::from_rgb(0.5,0.5,0.5))
            ).padding(16).into()
        } else {
            let items: Vec<Element<DatabaseMessage>> = self.db_users.iter().map(|u| {
                user_row(u, &self.databases, self.editing_user.as_ref(), &self.edit_user_name, &self.edit_user_password)
            }).collect();
            scrollable(column(items).spacing(6)).into()
        };

        let mut col = column![create_panel, Space::with_height(12), user_list];

        if let Some(status) = &self.user_status {
            let (msg, color) = match status {
                Ok(m) => (m.as_str(), Color::from_rgb(0.2, 0.9, 0.4)),
                Err(e) => (e.as_str(), Color::from_rgb(1.0, 0.4, 0.4)),
            };
            col = col.push(Space::with_height(8)).push(
                row![
                    text(msg).size(13).color(color).width(Length::Fill),
                    Space::with_width(8),
                    copy_btn(msg.to_string()),
                ].align_y(iced::Alignment::Center)
            );
        }

        col.into()
    }
}

/// 메시지를 클립보드로 복사하는 작은 버튼
fn copy_btn(text_to_copy: String) -> Element<'static, DatabaseMessage> {
    button(text("복사").size(11))
        .on_press(DatabaseMessage::CopyText(text_to_copy))
        .padding([4, 10])
        .style(|_, _| button::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.35, 0.5))),
            border: iced::Border { radius: 5.0.into(), ..Default::default() },
            text_color: Color::WHITE,
            ..Default::default()
        })
        .into()
}

fn subtab_btn(label: &str, active: bool, msg: DatabaseMessage) -> Element<'_, DatabaseMessage> {
    let bg = if active {
        Color::from_rgb(0.15, 0.35, 0.55)
    } else {
        Color::from_rgb(0.13, 0.13, 0.16)
    };
    button(text(label).size(13))
        .on_press(msg)
        .padding([8, 18])
        .style(move |_, _| button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 6.0.into(),
                color: Color::from_rgb(0.2, 0.2, 0.25),
                width: 1.0,
            },
            text_color: Color::WHITE,
            ..Default::default()
        })
        .into()
}

fn engine_btn(label: &str, active: bool, msg: DatabaseMessage) -> Element<'_, DatabaseMessage> {
    let bg = if active {
        Color::from_rgb(0.1, 0.45, 0.7)
    } else {
        Color::from_rgb(0.18, 0.18, 0.22)
    };
    button(text(label).size(12))
        .on_press(msg)
        .padding([10, 14])
        .style(move |_, _| button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                radius: 6.0.into(),
                color: Color::from_rgb(0.24, 0.24, 0.3),
                width: 1.0,
            },
            text_color: Color::WHITE,
            ..Default::default()
        })
        .into()
}

fn saved_connections_view(saved: &[DbCredentials]) -> Element<'_, DatabaseMessage> {
    if saved.is_empty() {
        return Space::with_height(0).into();
    }
    let buttons: Vec<Element<DatabaseMessage>> = saved.iter().enumerate().map(|(i, c)| {
        button(text(format!("{} / {}", c.engine.label(), c.user)).size(11))
            .on_press(DatabaseMessage::SavedConnectionSelected(i))
            .padding([5, 10])
            .style(|_, _| button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.18, 0.18, 0.22))),
                border: iced::Border { radius: 5.0.into(), color: Color::from_rgb(0.25,0.25,0.3), width: 1.0 },
                text_color: Color::WHITE,
                ..Default::default()
            })
            .into()
    }).collect();

    column![
        Space::with_height(10),
        text("저장된 연결").size(11).color(Color::from_rgb(0.55, 0.55, 0.55)),
        Space::with_height(6),
        row(buttons).spacing(6),
    ].into()
}

fn db_row<'a>(
    db: &'a str,
    users: &'a [DbUser],
    editing_db: Option<&'a str>,
    edit_db_name: &'a str,
) -> Element<'a, DatabaseMessage> {
    let db_name = db.to_string();
    let db_backup = db.to_string();
    let db_restore = db.to_string();
    let is_editing = editing_db == Some(db);

    let grant_buttons: Vec<Element<DatabaseMessage>> = users.iter().map(|u| {
        let un = u.username.clone();
        let uh = u.host.clone();
        let dn = db.to_string();
        button(text(format!("{} 권한", u.username)).size(11))
            .on_press(DatabaseMessage::GrantPrivileges(un, uh, dn))
            .padding([5, 10])
            .style(|_, _| button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.25, 0.2, 0.45))),
                border: iced::Border { radius: 5.0.into(), ..Default::default() },
                text_color: Color::WHITE,
                ..Default::default()
            })
            .into()
    }).collect();

    let grant_row: Element<DatabaseMessage> = if grant_buttons.is_empty() {
        Space::with_width(0).into()
    } else {
        row(grant_buttons).spacing(4).into()
    };

    let main_row: Element<DatabaseMessage> = if is_editing {
        row![
            text_input("DB 이름", edit_db_name)
                .on_input(DatabaseMessage::EditDbNameChanged)
                .padding(8)
                .width(Length::Fill),
            Space::with_width(6),
            button(text("저장").size(12))
                .on_press(DatabaseMessage::SaveDbEdit(db.to_string()))
                .padding([6, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.5, 0.3))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("취소").size(12))
                .on_press(DatabaseMessage::CancelDbEdit)
                .padding([6, 12])
        ].align_y(iced::Alignment::Center).into()
    } else {
        row![
            text(db).size(14).width(Length::Fill),
            button(text("편집").size(12))
                .on_press(DatabaseMessage::EditDb(db.to_string()))
                .padding([6, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.25, 0.35))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("백업").size(12))
                .on_press(DatabaseMessage::BackupDb(db_backup))
                .padding([6, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.35, 0.6))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("복원").size(12))
                .on_press(DatabaseMessage::RestoreDb(db_restore))
                .padding([6, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.4, 0.25, 0.1))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("삭제").size(12))
                .on_press(DatabaseMessage::DropDb(db_name))
                .padding([6, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.5, 0.1, 0.1))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
        ]
        .align_y(iced::Alignment::Center).into()
    };

    let mut card_col = column![main_row];

    if !users.is_empty() {
        card_col = card_col.push(Space::with_height(6)).push(
            row![
                text("권한 부여:").size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
                Space::with_width(6),
                grant_row,
            ]
            .align_y(iced::Alignment::Center)
        );
    }

    container(card_col)
    .padding(14)
    .width(Length::Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
        border: iced::Border { radius: 8.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
        ..Default::default()
    })
    .into()
}

fn user_row<'a>(
    u: &'a DbUser,
    _databases: &'a [String],
    editing_user: Option<&'a (String, String)>,
    edit_user_name: &'a str,
    edit_user_password: &'a str,
) -> Element<'a, DatabaseMessage> {
    let un = u.username.clone();
    let uh = u.host.clone();
    let is_editing = editing_user
        .map(|(name, host)| name == &u.username && host == &u.host)
        .unwrap_or(false);

    let content: Element<DatabaseMessage> = if is_editing {
        row![
            column![
                text("사용자명").size(11).color(Color::from_rgb(0.55, 0.55, 0.55)),
                text_input("사용자명", edit_user_name)
                    .on_input(DatabaseMessage::EditUserNameChanged)
                    .padding(8),
            ].width(Length::FillPortion(2)),
            Space::with_width(8),
            column![
                text("새 비밀번호").size(11).color(Color::from_rgb(0.55, 0.55, 0.55)),
                text_input("비워두면 유지", edit_user_password)
                    .on_input(DatabaseMessage::EditUserPasswordChanged)
                    .secure(true)
                    .padding(8),
            ].width(Length::FillPortion(2)),
            Space::with_width(8),
            button(text("저장").size(12))
                .on_press(DatabaseMessage::SaveUserEdit(u.username.clone(), u.host.clone()))
                .padding([7, 12])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.5, 0.3))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("취소").size(12))
                .on_press(DatabaseMessage::CancelUserEdit)
                .padding([7, 12]),
        ]
        .align_y(iced::Alignment::End)
        .into()
    } else {
        row![
            column![
                text(&u.username).size(14),
                Space::with_height(2),
                text(format!("호스트: {}", u.host)).size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
            ].width(Length::Fill),
            button(text("편집").size(12))
                .on_press(DatabaseMessage::EditUser(u.username.clone(), u.host.clone()))
                .padding([6, 14])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.25, 0.35))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
            Space::with_width(6),
            button(text("삭제").size(12))
                .on_press(DatabaseMessage::DropUser(un, uh))
                .padding([6, 14])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.5, 0.1, 0.1))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
        ]
        .align_y(iced::Alignment::Center)
        .into()
    };

    container(content)
    .padding(14)
    .width(Length::Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
        border: iced::Border { radius: 8.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
        ..Default::default()
    })
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
