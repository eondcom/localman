//! 백업·이전 탭: 이 PC의 localman 설정(프로젝트·DB 접속)과 DB를 한 파일로 내보내고,
//! 다른 PC(리눅스 ↔ 맥)에서 그 파일을 가져온다.

use iced::{
    widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space},
    Color, Element, Length, Task,
};
use std::path::PathBuf;

use crate::domain::transfer::{
    Bundle, ExportOptions, ImportOptions, PlannedProject, default_bundle_name, export_bundle,
    import_bundle, open_bundle, plan_projects,
};
use crate::domain::{DbEngine, list_databases, load_db_connections};

#[derive(Debug, Clone)]
pub enum TransferMessage {
    LoadDatabases,
    DatabasesLoaded(Vec<(DbEngine, String)>),
    ToggleDb(usize, bool),
    ToggleAllDbs(bool),
    ToggleExportCredentials(bool),
    Export,
    Exported(Result<String, String>),
    PickBundle,
    BundleOpened(Option<Result<Bundle, String>>),
    PathFromChanged(String),
    PathToChanged(String),
    ToggleOverwrite(bool),
    ToggleRestoreDbs(bool),
    ToggleImportCredentials(bool),
    Import,
    Imported(Vec<String>),
    CloseBundle,
}

pub struct TransferState {
    // 내보내기
    dbs: Vec<(DbEngine, String, bool)>,
    dbs_loading: bool,
    export_credentials: bool,
    exporting: bool,
    export_result: Option<Result<String, String>>,
    // 가져오기
    bundle: Option<Bundle>,
    opening: bool,
    path_from: String,
    path_to: String,
    overwrite: bool,
    restore_dbs: bool,
    import_credentials: bool,
    plan: Vec<PlannedProject>,
    importing: bool,
    import_log: Vec<String>,
    error: Option<String>,
}

/// 오래 걸리는 동기 작업(덤프·압축·sudo)을 GUI 스레드 밖에서 돌린다.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("작업 스레드가 비정상 종료했습니다")
}

impl TransferState {
    pub fn new() -> (Self, Task<TransferMessage>) {
        let s = Self {
            dbs: Vec::new(),
            dbs_loading: false,
            export_credentials: false,
            exporting: false,
            export_result: None,
            bundle: None,
            opening: false,
            path_from: String::new(),
            path_to: dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            overwrite: false,
            restore_dbs: true,
            import_credentials: true,
            plan: Vec::new(),
            importing: false,
            import_log: Vec::new(),
            error: None,
        };
        (s, Task::done(TransferMessage::LoadDatabases))
    }

    fn import_options(&self) -> ImportOptions {
        ImportOptions {
            path_from: self.path_from.clone(),
            path_to: self.path_to.clone(),
            overwrite: self.overwrite,
            restore_databases: self.restore_dbs,
            import_credentials: self.import_credentials,
        }
    }

    fn replan(&mut self) {
        self.plan = match &self.bundle {
            Some(b) => plan_projects(b, &self.import_options()),
            None => Vec::new(),
        };
    }

    pub fn update(&mut self, msg: TransferMessage) -> Task<TransferMessage> {
        match msg {
            TransferMessage::LoadDatabases => {
                self.dbs_loading = true;
                Task::perform(
                    blocking(|| {
                        // 엔진마다 가장 최근 저장한 접속으로 목록을 읽는다 (내보낼 때와 같은 접속)
                        let mut out = Vec::new();
                        for engine in [DbEngine::MariaDb, DbEngine::PostgreSql] {
                            if let Some(c) = load_db_connections().into_iter().find(|c| c.engine == engine) {
                                for db in list_databases(engine, &c.user, &c.password) {
                                    out.push((engine, db.name));
                                }
                            }
                        }
                        out
                    }),
                    TransferMessage::DatabasesLoaded,
                )
            }
            TransferMessage::DatabasesLoaded(list) => {
                self.dbs_loading = false;
                self.dbs = list.into_iter().map(|(e, n)| (e, n, true)).collect();
                Task::none()
            }
            TransferMessage::ToggleDb(i, on) => {
                if let Some(d) = self.dbs.get_mut(i) {
                    d.2 = on;
                }
                Task::none()
            }
            TransferMessage::ToggleAllDbs(on) => {
                self.dbs.iter_mut().for_each(|d| d.2 = on);
                Task::none()
            }
            TransferMessage::ToggleExportCredentials(on) => {
                self.export_credentials = on;
                Task::none()
            }
            TransferMessage::Export => {
                let opts = ExportOptions {
                    databases: self.dbs.iter().filter(|d| d.2).map(|d| (d.0, d.1.clone())).collect(),
                    include_credentials: self.export_credentials,
                };
                self.exporting = true;
                self.export_result = None;
                Task::perform(
                    async move {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("백업 파일 저장 위치")
                            .set_file_name(default_bundle_name())
                            .add_filter("localman 백업", &["gz"])
                            .save_file()
                            .await;
                        let Some(h) = handle else {
                            return Err(String::new()); // 취소
                        };
                        let dest = h.path().to_path_buf();
                        blocking(move || export_bundle(&dest, &opts)).await
                    },
                    TransferMessage::Exported,
                )
            }
            TransferMessage::Exported(r) => {
                self.exporting = false;
                // 빈 에러는 저장 창에서 취소한 것
                self.export_result = match r {
                    Err(e) if e.is_empty() => None,
                    other => Some(other),
                };
                Task::none()
            }
            TransferMessage::PickBundle => {
                self.opening = true;
                self.error = None;
                Task::perform(
                    async {
                        let h = rfd::AsyncFileDialog::new()
                            .set_title("가져올 localman 백업 파일")
                            .add_filter("localman 백업", &["gz", "tgz"])
                            .pick_file()
                            .await?;
                        let path: PathBuf = h.path().to_path_buf();
                        Some(blocking(move || open_bundle(&path)).await)
                    },
                    TransferMessage::BundleOpened,
                )
            }
            TransferMessage::BundleOpened(r) => {
                self.opening = false;
                match r {
                    None => {}
                    Some(Err(e)) => self.error = Some(e),
                    Some(Ok(b)) => {
                        if let Some(old) = self.bundle.take() {
                            old.close();
                        }
                        // 경로 바꾸기 기본값: 만든 PC의 홈 → 이 PC의 홈
                        self.path_from = b.manifest.source_home.clone();
                        self.import_credentials = !b.credentials.is_empty();
                        self.restore_dbs = !b.manifest.databases.is_empty();
                        self.import_log.clear();
                        self.bundle = Some(b);
                        self.replan();
                    }
                }
                Task::none()
            }
            TransferMessage::PathFromChanged(v) => {
                self.path_from = v;
                self.replan();
                Task::none()
            }
            TransferMessage::PathToChanged(v) => {
                self.path_to = v;
                self.replan();
                Task::none()
            }
            TransferMessage::ToggleOverwrite(on) => {
                self.overwrite = on;
                self.replan();
                Task::none()
            }
            TransferMessage::ToggleRestoreDbs(on) => {
                self.restore_dbs = on;
                Task::none()
            }
            TransferMessage::ToggleImportCredentials(on) => {
                self.import_credentials = on;
                Task::none()
            }
            TransferMessage::Import => {
                let Some(b) = self.bundle.clone() else {
                    return Task::none();
                };
                let opts = self.import_options();
                self.importing = true;
                self.import_log.clear();
                Task::perform(blocking(move || import_bundle(&b, &opts)), TransferMessage::Imported)
            }
            TransferMessage::Imported(log) => {
                self.importing = false;
                self.import_log = log;
                self.replan();
                // DB 목록도 바뀌었을 수 있다
                Task::done(TransferMessage::LoadDatabases)
            }
            TransferMessage::CloseBundle => {
                if let Some(b) = self.bundle.take() {
                    b.close();
                }
                self.plan.clear();
                self.import_log.clear();
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, TransferMessage> {
        let mut col = column![
            text("백업·이전").size(22),
            Space::with_height(8),
            text("프로젝트 설정, DB 접속 정보, 데이터베이스를 한 파일로 묶어 다른 PC(리눅스 ↔ 맥)로 옮깁니다.")
                .size(13)
                .color(muted()),
            Space::with_height(20),
            card(self.export_view()),
            Space::with_height(16),
            card(self.import_view()),
        ];
        if let Some(e) = &self.error {
            col = col.push(Space::with_height(12)).push(text(e).size(13).color(red()));
        }
        scrollable(col.spacing(0).padding(iced::Padding { right: 12.0, ..Default::default() })).into()
    }

    fn export_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![
            text("내보내기").size(16),
            Space::with_height(4),
            text("프로젝트 소스 폴더는 담지 않습니다. git 등으로 따로 옮기세요.").size(12).color(muted()),
            Space::with_height(14),
        ];

        let all_on = !self.dbs.is_empty() && self.dbs.iter().all(|d| d.2);
        let mut db_header = row![text("데이터베이스").size(13)].align_y(iced::Alignment::Center);
        if !self.dbs.is_empty() {
            db_header = db_header
                .push(Space::with_width(12))
                .push(checkbox("전체", all_on).on_toggle(TransferMessage::ToggleAllDbs).size(14).text_size(12));
        }
        db_header = db_header
            .push(Space::with_width(Length::Fill))
            .push(button(text("다시 읽기").size(12)).on_press(TransferMessage::LoadDatabases).padding([4, 10]));
        c = c.push(db_header).push(Space::with_height(8));

        if self.dbs_loading {
            c = c.push(text("DB 목록을 읽는 중…").size(12).color(muted()));
        } else if self.dbs.is_empty() {
            c = c.push(
                text("담을 DB가 없습니다. 데이터베이스 탭에서 접속 정보를 저장하면 목록이 나옵니다.")
                    .size(12)
                    .color(muted()),
            );
        } else {
            let mut list = column![].spacing(4);
            for (i, (engine, name, on)) in self.dbs.iter().enumerate() {
                list = list.push(
                    checkbox(format!("{name}  ({})", engine.label()), *on)
                        .on_toggle(move |v| TransferMessage::ToggleDb(i, v))
                        .size(14)
                        .text_size(13),
                );
            }
            c = c.push(list);
        }

        c = c
            .push(Space::with_height(14))
            .push(
                checkbox("DB 접속 정보(비밀번호 포함)도 담기", self.export_credentials)
                    .on_toggle(TransferMessage::ToggleExportCredentials)
                    .size(14)
                    .text_size(13),
            );
        if self.export_credentials {
            c = c.push(text("비밀번호가 평문으로 들어갑니다. 파일을 공유하지 마세요.").size(12).color(amber()));
        }

        let label = if self.exporting { "내보내는 중…" } else { "백업 파일 만들기" };
        let mut btn = primary_button(label);
        if !self.exporting {
            btn = btn.on_press(TransferMessage::Export);
        }
        c = c.push(Space::with_height(14)).push(btn);

        match &self.export_result {
            Some(Ok(s)) => c = c.push(Space::with_height(10)).push(result_lines(s.lines())),
            Some(Err(e)) => c = c.push(Space::with_height(10)).push(text(e).size(13).color(red())),
            None => {}
        }
        c.into()
    }

    fn import_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![text("가져오기").size(16), Space::with_height(14)];

        let Some(b) = &self.bundle else {
            let label = if self.opening { "여는 중…" } else { "백업 파일 열기" };
            let mut btn = primary_button(label);
            if !self.opening {
                btn = btn.on_press(TransferMessage::PickBundle);
            }
            return c.push(btn).into();
        };

        let m = &b.manifest;
        c = c.push(
            text(format!(
                "{} ({})에서 만든 백업 · 프로젝트 {}개 · DB {}개{}",
                m.hostname,
                os_label(&m.source_os),
                b.projects.len(),
                m.databases.len(),
                if m.includes_credentials { " · 접속 정보 포함" } else { "" },
            ))
            .size(13),
        );

        // 경로 바꾸기
        c = c
            .push(Space::with_height(14))
            .push(text("프로젝트 경로 바꾸기").size(13))
            .push(Space::with_height(6))
            .push(
                row![
                    text_input("원래 경로 앞부분", &self.path_from)
                        .on_input(TransferMessage::PathFromChanged)
                        .size(13)
                        .padding(6),
                    text("→").size(14),
                    text_input("이 PC 경로 앞부분", &self.path_to)
                        .on_input(TransferMessage::PathToChanged)
                        .size(13)
                        .padding(6),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );

        // 프로젝트 미리보기
        c = c.push(Space::with_height(12));
        let mut list = column![].spacing(3);
        for p in &self.plan {
            let (mark, color) = if p.exists_here && !self.overwrite {
                ("건너뜀(이미 있음)", muted())
            } else if !p.folder_exists {
                ("폴더 없음", amber())
            } else if p.exists_here {
                ("덮어씀", amber())
            } else {
                ("추가", green())
            };
            let mut line = format!("{}  {}", p.project.id, p.project.path);
            if let Some(old) = p.port_changed_from {
                line.push_str(&format!("  (포트 {old}→{})", p.project.port));
            }
            list = list.push(
                row![
                    text(mark).size(12).color(color).width(110),
                    text(line).size(12),
                ]
                .spacing(6),
            );
        }
        c = c.push(list);

        // 옵션
        c = c
            .push(Space::with_height(14))
            .push(
                checkbox("이미 있는 프로젝트·DB 덮어쓰기", self.overwrite)
                    .on_toggle(TransferMessage::ToggleOverwrite)
                    .size(14)
                    .text_size(13),
            );
        if !m.databases.is_empty() {
            let names: Vec<String> = m.databases.iter().map(|d| d.name.clone()).collect();
            c = c.push(
                checkbox(format!("DB 복원: {}", names.join(", ")), self.restore_dbs)
                    .on_toggle(TransferMessage::ToggleRestoreDbs)
                    .size(14)
                    .text_size(13),
            );
        }
        if !b.credentials.is_empty() {
            c = c.push(
                checkbox("DB 접속 정보 가져오기 (이 PC에 같은 사용자가 있으면 이 PC 설정 유지)", self.import_credentials)
                    .on_toggle(TransferMessage::ToggleImportCredentials)
                    .size(14)
                    .text_size(13),
            );
        }
        c = c.push(
            text("프로젝트를 추가하면 이 PC의 Apache 가상호스트와 /etc/hosts가 새로 만들어집니다.")
                .size(12)
                .color(muted()),
        );

        let label = if self.importing { "가져오는 중…" } else { "가져오기" };
        let mut go = primary_button(label);
        let mut close = button(text("닫기").size(13)).padding([8, 16]);
        if !self.importing {
            go = go.on_press(TransferMessage::Import);
            close = close.on_press(TransferMessage::CloseBundle);
        }
        c = c.push(Space::with_height(14)).push(row![go, close].spacing(8));

        if !self.import_log.is_empty() {
            c = c.push(Space::with_height(10)).push(result_lines(self.import_log.iter().map(|s| s.as_str())));
        }
        c.into()
    }
}

fn os_label(os: &str) -> &str {
    match os {
        "linux" => "리눅스",
        "macos" => "맥",
        "windows" => "윈도우",
        other => other,
    }
}

/// "✓ / ✗ / ·" 로 시작하는 결과 줄을 색으로 구분해 보여준다.
fn result_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Element<'a, TransferMessage> {
    let mut col = column![].spacing(3);
    for l in lines {
        let color = if l.starts_with('✓') {
            green()
        } else if l.starts_with('✗') {
            red()
        } else {
            muted()
        };
        col = col.push(text(l).size(12).color(color));
    }
    col.into()
}

fn card<'a>(content: Element<'a, TransferMessage>) -> Element<'a, TransferMessage> {
    container(content)
        .padding(20)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.13, 0.13, 0.16))),
            border: iced::Border {
                radius: 10.0.into(),
                color: Color::from_rgb(0.2, 0.2, 0.25),
                width: 1.0,
            },
            ..Default::default()
        })
        .into()
}

fn primary_button(label: &str) -> button::Button<'_, TransferMessage> {
    button(text(label).size(13)).padding([8, 16])
}

fn muted() -> Color {
    Color::from_rgb(0.6, 0.6, 0.6)
}
fn green() -> Color {
    Color::from_rgb(0.45, 0.85, 0.5)
}
fn amber() -> Color {
    Color::from_rgb(0.9, 0.7, 0.3)
}
fn red() -> Color {
    Color::from_rgb(0.95, 0.4, 0.4)
}
