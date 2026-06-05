use iced::{
    widget::{button, column, container, row, text, text_input, Space, scrollable},
    Color, Element, Length, Task,
};
use crate::system::{list_databases, create_database, drop_database, backup_database, restore_database};

#[derive(Debug, Clone)]
pub enum DatabaseMessage {
    UserChanged(String),
    PasswordChanged(String),
    Connect,
    Connected(Vec<String>),
    NewDbNameChanged(String),
    CreateDb,
    DropDb(String),
    BackupDb(String),
    BackupPathSelected(String, Option<String>),
    RestoreDb(String),
    RestorePathSelected(String, Option<String>),
    Done(Result<String, String>),
}

pub struct DatabaseState {
    user: String,
    password: String,
    databases: Vec<String>,
    new_db_name: String,
    connected: bool,
    status: Option<Result<String, String>>,
}

impl DatabaseState {
    pub fn new() -> Self {
        Self {
            user: "root".to_string(),
            password: String::new(),
            databases: vec![],
            new_db_name: String::new(),
            connected: false,
            status: None,
        }
    }

    pub fn update(&mut self, msg: DatabaseMessage) -> Task<DatabaseMessage> {
        match msg {
            DatabaseMessage::UserChanged(v) => { self.user = v; Task::none() }
            DatabaseMessage::PasswordChanged(v) => { self.password = v; Task::none() }
            DatabaseMessage::Connect => {
                let u = self.user.clone();
                let p = self.password.clone();
                Task::perform(
                    async move {
                        list_databases(&u, &p).iter().map(|d| d.name.clone()).collect()
                    },
                    DatabaseMessage::Connected,
                )
            }
            DatabaseMessage::Connected(dbs) => {
                self.connected = !dbs.is_empty();
                self.databases = dbs;
                self.status = if self.connected {
                    Some(Ok("연결 성공".to_string()))
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
                self.new_db_name.clear();
                Task::perform(
                    async move {
                        create_database(&u, &p, &name)
                            .map(|_| format!("'{name}' 생성 완료"))
                    },
                    DatabaseMessage::Done,
                )
            }
            DatabaseMessage::DropDb(name) => {
                let u = self.user.clone();
                let p = self.password.clone();
                let n = name.clone();
                Task::perform(
                    async move {
                        drop_database(&u, &p, &n)
                            .map(|_| format!("'{n}' 삭제 완료"))
                    },
                    DatabaseMessage::Done,
                )
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
                    return Task::perform(
                        async move {
                            backup_database(&u, &p, &name, &path)
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
                    return Task::perform(
                        async move {
                            restore_database(&u, &p, &name, &path)
                                .map(|_| format!("복원 완료: {name}"))
                        },
                        DatabaseMessage::Done,
                    );
                }
                Task::none()
            }
            DatabaseMessage::Done(result) => {
                match &result {
                    Ok(_) => {
                        let u = self.user.clone();
                        let p = self.password.clone();
                        self.databases = list_databases(&u, &p)
                            .iter().map(|d| d.name.clone()).collect();
                    }
                    Err(_) => {}
                }
                self.status = Some(result);
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, DatabaseMessage> {
        let conn_panel = container(
            row![
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
                    text_input("password", &self.password)
                        .on_input(DatabaseMessage::PasswordChanged)
                        .secure(true)
                        .padding(10),
                ].width(200),
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
            .align_y(iced::Alignment::End)
        )
        .padding(16)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
            border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
            ..Default::default()
        });

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

        let db_list: Element<DatabaseMessage> = if self.databases.is_empty() {
            container(
                text(if self.connected { "데이터베이스 없음" } else { "연결 후 목록이 표시됩니다." })
                    .size(13).color(Color::from_rgb(0.5,0.5,0.5))
            ).padding(16).into()
        } else {
            let items: Vec<Element<DatabaseMessage>> = self.databases.iter().map(|db| {
                db_row(db)
            }).collect();
            scrollable(column(items).spacing(6)).into()
        };

        let mut col = column![
            text("데이터베이스").size(22),
            Space::with_height(8),
            text("MariaDB 데이터베이스를 관리하고 백업/복원합니다.").size(13).color(Color::from_rgb(0.6,0.6,0.6)),
            Space::with_height(20),
            conn_panel,
            Space::with_height(12),
            create_panel,
            Space::with_height(16),
            db_list,
        ];

        if let Some(status) = &self.status {
            let (msg, color) = match status {
                Ok(m) => (m.as_str(), Color::from_rgb(0.2, 0.9, 0.4)),
                Err(e) => (e.as_str(), Color::from_rgb(1.0, 0.4, 0.4)),
            };
            col = col.push(Space::with_height(10)).push(
                text(msg).size(13).color(color)
            );
        }

        col.into()
    }
}

fn db_row(db: &str) -> Element<'_, DatabaseMessage> {
    let db_name = db.to_string();
    let db_backup = db.to_string();
    let db_restore = db.to_string();

    container(
        row![
            text(db).size(14).width(Length::Fill),
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
        .align_y(iced::Alignment::Center)
    )
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
    let default = format!("{}.sql", db_name);
    let output = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--save",
            "--confirm-overwrite",
            &format!("--filename={}", default),
            "--title=백업 파일 저장 위치",
            "--file-filter=SQL files (*.sql) | *.sql",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => None,
    }
}

async fn pick_open_file() -> Option<String> {
    let output = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--title=복원할 SQL 파일 선택",
            "--file-filter=SQL files (*.sql) | *.sql",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => None,
    }
}
