//! 백업·이전 탭: 같은 네트워크의 다른 PC로 바로 보내고 받거나(lan.rs),
//! 설정·DB를 한 파일로 내보내고 가져온다(transfer.rs). 리눅스 ↔ 맥 어느 방향이든 같다.

use iced::{
    widget::{button, checkbox, column, container, pick_list, progress_bar, row, scrollable, text, text_input, Space},
    Color, Element, Length, Task,
};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::domain::history::{load_history, summary_line};
use crate::domain::lan::{
    LanEvent, Peer, ReceiveOptions, SendPlan, TRANSFER_PORT, discover, human_bytes, receive, send,
};
use crate::domain::list_projects;

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
    // 같은 네트워크로 바로 이전
    ScopeSelected(Scope),
    /// 프로젝트 목록의 더보기 → "다른 PC로 보내기"
    PresetProject(String),
    FindPeers,
    PeersFound(Vec<Peer>),
    PickPeer(usize),
    PeerAddrChanged(String),
    CodeChanged(String),
    Send,
    SendEvent(LanEvent),
    ToggleRecvOverwrite(bool),
    StartReceive,
    StopReceive,
    RecvEvent(LanEvent),
}

/// 보낼 범위
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Project(String),
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::All => write!(f, "전체 (모든 프로젝트)"),
            Scope::Project(id) => write!(f, "{id}"),
        }
    }
}

/// 보내기·받기 한쪽의 진행 상태
#[derive(Default)]
struct LanJob {
    running: bool,
    status: String,
    /// (보낸 파일 수, 보낸 바이트, 보낼 바이트)
    progress: Option<(u64, u64, u64)>,
    log: Vec<String>,
    result: Option<Result<Vec<String>, String>>,
}

impl LanJob {
    fn start(&mut self) {
        *self = LanJob { running: true, ..Default::default() };
    }

    /// 이벤트를 반영한다. 끝났으면 true.
    fn apply(&mut self, e: LanEvent) -> bool {
        match e {
            LanEvent::Listening(..) => {}
            LanEvent::Status(s) => self.status = s,
            LanEvent::Progress { files, bytes, total_bytes } => self.progress = Some((files, bytes, total_bytes)),
            LanEvent::Log(l) => self.log.push(l),
            LanEvent::Done(r) => {
                self.running = false;
                self.status.clear();
                self.progress = None;
                self.result = Some(r);
                return true;
            }
        }
        false
    }
}

/// 블로킹 네트워크 작업을 스레드에서 돌리고 진행 상황을 화면으로 흘려보낸다.
fn run_lan(
    work: impl FnOnce(&dyn Fn(LanEvent)) -> Result<Vec<String>, String> + Send + 'static,
) -> iced::futures::channel::mpsc::UnboundedReceiver<LanEvent> {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let r = work(&move |e| {
            let _ = tx2.unbounded_send(e);
        });
        let _ = tx.unbounded_send(LanEvent::Done(r));
    });
    rx
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
    // 같은 네트워크로 바로 이전
    scope: Scope,
    peers: Vec<Peer>,
    searching: bool,
    peer_addr: String,
    code: String,
    sending: LanJob,
    recv_overwrite: bool,
    recv_stop: Option<Arc<AtomicBool>>,
    /// 받기 대기 중일 때 (코드, 이 PC 이름)
    recv_code: Option<(String, String)>,
    receiving: LanJob,
    history: Vec<String>,
}

fn recent_history() -> Vec<String> {
    load_history()
        .iter()
        .rev()
        .take(10)
        .map(|r| {
            let what = if r.projects.is_empty() { "설정".to_string() } else { r.projects.join(", ") };
            format!("{} — {what}", summary_line(r))
        })
        .collect()
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
            scope: Scope::All,
            peers: Vec::new(),
            searching: false,
            peer_addr: String::new(),
            code: String::new(),
            sending: LanJob::default(),
            recv_overwrite: false,
            recv_stop: None,
            recv_code: None,
            receiving: LanJob::default(),
            history: recent_history(),
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
                    projects: None,
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
            TransferMessage::ScopeSelected(sc) => {
                self.scope = sc;
                Task::none()
            }
            TransferMessage::PresetProject(id) => {
                self.scope = Scope::Project(id);
                // 프로젝트 하나를 보낼 때는 DB 를 기본으로 담지 않는다 (어느 DB 가 그 프로젝트 것인지 모른다)
                self.dbs.iter_mut().for_each(|d| d.2 = false);
                Task::done(TransferMessage::FindPeers)
            }
            TransferMessage::FindPeers => {
                self.searching = true;
                Task::perform(
                    blocking(|| discover(std::time::Duration::from_millis(1500))),
                    TransferMessage::PeersFound,
                )
            }
            TransferMessage::PeersFound(peers) => {
                self.searching = false;
                if self.peer_addr.is_empty() {
                    if let Some(p) = peers.first() {
                        self.peer_addr = p.addr.ip().to_string();
                    }
                }
                self.peers = peers;
                Task::none()
            }
            TransferMessage::PickPeer(i) => {
                if let Some(p) = self.peers.get(i) {
                    self.peer_addr = p.addr.ip().to_string();
                }
                Task::none()
            }
            TransferMessage::PeerAddrChanged(v) => {
                self.peer_addr = v;
                Task::none()
            }
            TransferMessage::CodeChanged(v) => {
                self.code = v.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
                Task::none()
            }
            TransferMessage::Send => {
                let addr = match parse_addr(&self.peer_addr) {
                    Some(a) => a,
                    None => {
                        self.sending.result = Some(Err("받는 PC 주소가 올바르지 않습니다.".into()));
                        return Task::none();
                    }
                };
                let plan = SendPlan {
                    projects: match &self.scope {
                        Scope::All => None,
                        Scope::Project(id) => Some(vec![id.clone()]),
                    },
                    databases: self.dbs.iter().filter(|d| d.2).map(|d| (d.0, d.1.clone())).collect(),
                    include_credentials: self.export_credentials,
                };
                let code = self.code.clone();
                self.sending.start();
                Task::run(run_lan(move |ev| send(addr, &code, plan, ev)), TransferMessage::SendEvent)
            }
            TransferMessage::SendEvent(e) => {
                if self.sending.apply(e) {
                    self.code.clear();
                    self.history = recent_history();
                }
                Task::none()
            }
            TransferMessage::ToggleRecvOverwrite(on) => {
                self.recv_overwrite = on;
                Task::none()
            }
            TransferMessage::StartReceive => {
                let stop = Arc::new(AtomicBool::new(false));
                self.recv_stop = Some(stop.clone());
                self.receiving.start();
                let opts = ReceiveOptions { overwrite: self.recv_overwrite };
                Task::run(run_lan(move |ev| receive(opts, stop, ev)), TransferMessage::RecvEvent)
            }
            TransferMessage::StopReceive => {
                if let Some(s) = &self.recv_stop {
                    s.store(true, Ordering::Relaxed);
                }
                Task::none()
            }
            TransferMessage::RecvEvent(e) => {
                if let LanEvent::Listening(code, name) = &e {
                    self.recv_code = Some((code.clone(), name.clone()));
                }
                if self.receiving.apply(e) {
                    self.recv_code = None;
                    self.recv_stop = None;
                    self.history = recent_history();
                    // 받은 프로젝트·DB 가 생겼을 수 있다
                    return Task::done(TransferMessage::LoadDatabases);
                }
                Task::none()
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
            text("프로젝트 파일·설정·DB를 다른 PC(리눅스 ↔ 맥)로 옮깁니다. 같은 와이파이면 바로 보내고, 아니면 파일로 옮기세요.")
                .size(13)
                .color(muted()),
            Space::with_height(20),
            card(self.send_view()),
            Space::with_height(16),
            card(self.receive_view()),
            Space::with_height(16),
            card(self.selection_view()),
            Space::with_height(16),
            card(self.export_view()),
            Space::with_height(16),
            card(self.import_view()),
            Space::with_height(16),
            card(self.history_view()),
        ];
        if let Some(e) = &self.error {
            col = col.push(Space::with_height(12)).push(text(e).size(13).color(red()));
        }
        scrollable(col.spacing(0).padding(iced::Padding { right: 12.0, ..Default::default() })).into()
    }

    /// 보내기·파일 내보내기에 함께 쓰는 선택: DB 와 접속 정보
    fn selection_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![
            text("함께 옮길 데이터베이스").size(16),
            Space::with_height(4),
            text("바로 보내기와 파일 내보내기에 모두 적용됩니다.").size(12).color(muted()),
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

        c.into()
    }

    fn export_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![
            text("파일로 내보내기").size(16),
            Space::with_height(4),
            text("설정과 위에서 고른 DB를 한 파일로 묶습니다. 프로젝트 소스 폴더는 담지 않습니다.").size(12).color(muted()),
        ];
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
        let mut c = column![text("파일에서 가져오기").size(16), Space::with_height(14)];

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
                line.push_str(&format!("  (포트 {old} → {})", p.project.port));
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

impl TransferState {
    fn send_view(&self) -> Element<'_, TransferMessage> {
        let mut scopes = vec![Scope::All];
        scopes.extend(list_projects().into_iter().map(|p| Scope::Project(p.id)));
        let busy = self.sending.running;

        let mut c = column![
            text("다른 PC로 보내기 (같은 네트워크)").size(16),
            Space::with_height(4),
            text("바뀐 파일만 보냅니다. node_modules·venv 같은 의존성 폴더는 빼고 보내니 받은 PC에서 패키지를 설치하세요.")
                .size(12)
                .color(muted()),
            Space::with_height(14),
            row![
                text("보낼 것").size(13).width(80),
                pick_list(scopes, Some(self.scope.clone()), TransferMessage::ScopeSelected).text_size(13),
            ]
            .align_y(iced::Alignment::Center),
            Space::with_height(10),
        ];

        // 받는 PC
        let mut find = button(text(if self.searching { "찾는 중…" } else { "받는 PC 찾기" }).size(12)).padding([6, 12]);
        if !self.searching && !busy {
            find = find.on_press(TransferMessage::FindPeers);
        }
        let mut peers_row = row![text("받는 PC").size(13).width(80), find].spacing(8).align_y(iced::Alignment::Center);
        for (i, p) in self.peers.iter().enumerate() {
            let selected = self.peer_addr == p.addr.ip().to_string();
            peers_row = peers_row.push(
                button(text(format!("{} ({})", p.name, os_label(&p.os))).size(12))
                    .on_press(TransferMessage::PickPeer(i))
                    .padding([6, 12])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(if selected {
                            Color::from_rgb(0.2, 0.4, 0.6)
                        } else {
                            Color::from_rgb(0.2, 0.2, 0.25)
                        })),
                        border: iced::Border { radius: 5.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        ..Default::default()
                    }),
            );
        }
        c = c.push(peers_row).push(Space::with_height(8)).push(
            row![
                text("").width(80),
                text_input("또는 IP 직접 입력 (예: 192.168.0.12)", &self.peer_addr)
                    .on_input(TransferMessage::PeerAddrChanged)
                    .size(13)
                    .padding(6)
                    .width(260),
                Space::with_width(16),
                text("코드").size(13),
                text_input("받는 PC의 6자리", &self.code)
                    .on_input(TransferMessage::CodeChanged)
                    .size(13)
                    .padding(6)
                    .width(140),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
        if !self.searching && self.peers.is_empty() {
            c = c.push(
                text("받는 PC에서 아래 [받기 대기]를 먼저 누르세요. 찾지 못하면 그 PC의 IP를 직접 입력하세요.")
                    .size(12)
                    .color(muted()),
            );
        }

        let ready = !busy && self.code.len() == 6 && !self.peer_addr.trim().is_empty();
        let mut go = primary_button(if busy { "보내는 중…" } else { "보내기" });
        if ready {
            go = go.on_press(TransferMessage::Send);
        }
        c = c.push(Space::with_height(12)).push(go);
        c = c.push(job_view(&self.sending));
        c.into()
    }

    fn receive_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![
            text("이 PC에서 받기").size(16),
            Space::with_height(4),
            text("받기 대기를 누르면 코드가 나옵니다. 보내는 PC에 그 코드를 입력하세요. 코드는 한 번만 쓸 수 있습니다.")
                .size(12)
                .color(muted()),
            Space::with_height(12),
        ];
        let waiting = self.recv_stop.is_some();
        c = c.push(
            checkbox("이미 있는 프로젝트·DB, 이 PC가 더 최신인 파일도 보내는 쪽 기준으로 덮어쓰기", self.recv_overwrite)
                .on_toggle_maybe((!waiting).then_some(TransferMessage::ToggleRecvOverwrite))
                .size(14)
                .text_size(13),
        );
        let btn = if waiting {
            button(text("멈추기").size(13)).padding([8, 16]).on_press(TransferMessage::StopReceive)
        } else {
            primary_button("받기 대기").on_press(TransferMessage::StartReceive)
        };
        c = c.push(Space::with_height(12)).push(btn);

        if let Some((code, name)) = &self.recv_code {
            if self.receiving.status.is_empty() {
                c = c.push(Space::with_height(14)).push(
                    row![
                        column![
                            text("코드").size(12).color(muted()),
                            text(format!("{} {}", &code[..3], &code[3..])).size(34).color(Color::from_rgb(0.4, 0.8, 1.0)),
                        ],
                        Space::with_width(30),
                        column![
                            text("이 PC").size(12).color(muted()),
                            text(name).size(16),
                            text(format!("포트 {TRANSFER_PORT}")).size(11).color(muted()),
                        ],
                    ]
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        c = c.push(job_view(&self.receiving));
        c.into()
    }

    fn history_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![text("최근 이전 기록").size(16), Space::with_height(10)].spacing(3);
        if self.history.is_empty() {
            c = c.push(text("아직 다른 PC와 주고받은 기록이 없습니다.").size(12).color(muted()));
        }
        for l in &self.history {
            let color = if l.contains("일부 실패") { amber() } else { Color::from_rgb(0.75, 0.75, 0.8) };
            c = c.push(text(l).size(12).color(color));
        }
        c.into()
    }
}

/// "192.168.0.12" 또는 "192.168.0.12:47801"
fn parse_addr(s: &str) -> Option<SocketAddr> {
    let s = s.trim();
    s.parse::<SocketAddr>()
        .ok()
        .or_else(|| s.parse::<std::net::IpAddr>().ok().map(|ip| SocketAddr::new(ip, TRANSFER_PORT)))
}

fn job_view(job: &LanJob) -> Element<'_, TransferMessage> {
    let mut c = column![].spacing(4);
    if !job.status.is_empty() {
        c = c.push(Space::with_height(10)).push(text(&job.status).size(13));
    }
    if let Some((files, done, total)) = job.progress {
        if total > 0 {
            c = c.push(progress_bar(0.0..=total as f32, done as f32).height(8)).push(
                text(format!("{} / {} · 파일 {files}개 완료", human_bytes(done), human_bytes(total)))
                    .size(11)
                    .color(muted()),
            );
        }
    }
    if !job.log.is_empty() {
        c = c.push(result_lines(job.log.iter().map(|s| s.as_str())));
    }
    match &job.result {
        Some(Ok(lines)) => {
            c = c.push(Space::with_height(8)).push(result_lines(lines.iter().map(|s| s.as_str())));
        }
        Some(Err(e)) => {
            c = c.push(Space::with_height(8)).push(text(e).size(13).color(red()));
        }
        None => {}
    }
    c.into()
}

fn os_label(os: &str) -> &str {
    match os {
        "linux" => "리눅스",
        "macos" => "맥",
        "windows" => "윈도우",
        other => other,
    }
}

fn result_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Element<'a, TransferMessage> {
    let mut col = column![].spacing(4);
    for l in lines {
        col = col.push(super::status_line(l));
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
