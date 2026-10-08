//! 백업·이전 탭: 같은 네트워크의 다른 PC로 바로 보내고 받거나(lan.rs),
//! 설정·DB를 한 파일로 내보내고 가져온다(transfer.rs). 리눅스 ↔ 맥 어느 방향이든 같다.

use iced::{
    widget::{column, container, pick_list, row, text, Space},
    Element, Length, Task,
};
use super::theme::{
    self, Icon, Kind, Tone, btn, card, check, chip, icon, input, muted, p, page_header, section_label,
};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::domain::history::{load_history, summary_line};
use crate::i18n::{tr, trf};
use crate::domain::lan::{
    LanEvent, Peer, ReceiveOptions, SendPlan, TRANSFER_PORT, discover, human_bytes, receive, send,
};
use crate::domain::list_projects;
use crate::domain::mamp::{MampExport, MampInfo, detect as detect_mamp, export_bundle as export_mamp, vhost_to_project};

use crate::domain::transfer::{
    Bundle, ExportOptions, ImportOptions, PlannedProject, default_bundle_name, export_bundle,
    import_bundle, open_bundle, plan_projects,
};
use crate::domain::{DbEngine, list_databases, load_db_connections};

#[derive(Debug, Clone)]
pub enum TransferMessage {
    CopyLog(String),
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
    // MAMP 에서 옮기기
    MampDetected(Option<MampInfo>),
    MampToggleDb(usize, bool),
    MampToggleAllDbs(bool),
    MampToggleVhost(usize, bool),
    MampUserChanged(String),
    MampPasswordChanged(String),
    MampToggleUsers(bool),
    MampToggleShowPw,
    MampExport,
    MampEvent(LanEvent),
    /// 방금 만든 MAMP 백업 파일을 이 PC 가져오기로 연다
    MampImportHere,
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
            Scope::All => write!(f, "{}", tr("전체 (모든 프로젝트)")),
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
    /// (기록 한 줄, 성공 여부) — 색은 글자가 아니라 기록의 ok 로 정한다
    history: Vec<(String, bool)>,
    // MAMP 에서 옮기기
    mamp: Option<MampInfo>,
    mamp_dbs: Vec<bool>,
    mamp_vhosts: Vec<bool>,
    mamp_user: String,
    mamp_password: String,
    mamp_users: bool,
    mamp_show_pw: bool,
    mamp_job: LanJob,
    mamp_output: Option<PathBuf>,
}

fn recent_history() -> Vec<(String, bool)> {
    load_history()
        .iter()
        .rev()
        .take(10)
        .map(|r| {
            let what = if r.projects.is_empty() { tr("설정").to_string() } else { r.projects.join(", ") };
            (format!("{} — {what}", summary_line(r)), r.ok)
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
            mamp: None,
            mamp_dbs: Vec::new(),
            mamp_vhosts: Vec::new(),
            mamp_user: "root".into(),
            mamp_password: "root".into(),
            mamp_users: true,
            mamp_show_pw: false,
            mamp_job: LanJob::default(),
            mamp_output: None,
        };
        (
            s,
            Task::batch([
                Task::done(TransferMessage::LoadDatabases),
                // DB 폴더 크기를 재므로 작업 스레드에서
                Task::perform(blocking(detect_mamp), TransferMessage::MampDetected),
            ]),
        )
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
            TransferMessage::CopyLog(s) => iced::clipboard::write(s),
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
                            .set_title(tr("백업 파일 저장 위치"))
                            .set_file_name(default_bundle_name())
                            .add_filter(tr("localman 백업"), &["gz"])
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
                            .set_title(tr("가져올 localman 백업 파일"))
                            .add_filter(tr("localman 백업"), &["gz", "tgz"])
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
                        self.sending.result = Some(Err(tr("받는 PC 주소가 올바르지 않습니다.").into()));
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
            TransferMessage::MampDetected(info) => {
                if let Some(i) = &info {
                    // DB 는 전부, 사이트는 폴더가 남아 있는 것만 기본으로 고른다
                    self.mamp_dbs = vec![true; i.databases.len()];
                    self.mamp_vhosts = i.vhosts.iter().map(|v| v.exists && vhost_to_project(v).is_some()).collect();
                }
                self.mamp = info;
                Task::none()
            }
            TransferMessage::MampToggleDb(i, on) => {
                if let Some(x) = self.mamp_dbs.get_mut(i) {
                    *x = on;
                }
                Task::none()
            }
            TransferMessage::MampToggleAllDbs(on) => {
                self.mamp_dbs.iter_mut().for_each(|x| *x = on);
                Task::none()
            }
            TransferMessage::MampToggleVhost(i, on) => {
                if let Some(x) = self.mamp_vhosts.get_mut(i) {
                    *x = on;
                }
                Task::none()
            }
            TransferMessage::MampUserChanged(v) => {
                self.mamp_user = v;
                Task::none()
            }
            TransferMessage::MampToggleShowPw => {
                self.mamp_show_pw = !self.mamp_show_pw;
                Task::none()
            }
            TransferMessage::MampToggleUsers(on) => {
                self.mamp_users = on;
                Task::none()
            }
            TransferMessage::MampPasswordChanged(v) => {
                self.mamp_password = v;
                Task::none()
            }
            TransferMessage::MampExport => {
                let Some(info) = &self.mamp else { return Task::none() };
                let opts = MampExport {
                    databases: info.databases.iter().zip(&self.mamp_dbs).filter(|(_, on)| **on).map(|(d, _)| d.name.clone()).collect(),
                    projects: info
                        .vhosts
                        .iter()
                        .zip(&self.mamp_vhosts)
                        .filter(|(_, on)| **on)
                        .filter_map(|(v, _)| vhost_to_project(v))
                        .collect(),
                    user: self.mamp_user.clone(),
                    password: self.mamp_password.clone(),
                    users: self.mamp_users,
                };
                self.mamp_job.start();
                self.mamp_output = None;
                let name = default_bundle_name().replace("localman-backup-", "localman-mamp-");
                Task::run(
                    {
                        let (tx, rx) = iced::futures::channel::mpsc::unbounded();
                        std::thread::spawn(move || {
                            // 저장 위치는 작업 스레드에서 묻는다 (GUI 를 멈추지 않게)
                            let dest = rfd::FileDialog::new()
                                .set_title(tr("MAMP 백업 파일 저장 위치"))
                                .set_file_name(&name)
                                .add_filter(tr("localman 백업"), &["gz"])
                                .save_file();
                            let r = match dest {
                                None => Err(String::new()),
                                Some(dest) => {
                                    let tx2 = tx.clone();
                                    export_mamp(&dest, &opts, move |m| {
                                        let _ = tx2.unbounded_send(LanEvent::Status(m));
                                    })
                                    .map(|mut l| {
                                        l.push(format!("__path:{}", dest.display()));
                                        l
                                    })
                                }
                            };
                            let _ = tx.unbounded_send(LanEvent::Done(r));
                        });
                        rx
                    },
                    TransferMessage::MampEvent,
                )
            }
            TransferMessage::MampEvent(e) => {
                let e = match e {
                    // 저장 창에서 취소
                    LanEvent::Done(Err(m)) if m.is_empty() => {
                        self.mamp_job = LanJob::default();
                        return Task::none();
                    }
                    LanEvent::Done(Ok(lines)) => {
                        let (paths, rest): (Vec<String>, Vec<String>) = lines.into_iter().partition(|l| l.starts_with("__path:"));
                        self.mamp_output = paths.first().map(|p| PathBuf::from(p.trim_start_matches("__path:")));
                        LanEvent::Done(Ok(rest))
                    }
                    other => other,
                };
                self.mamp_job.apply(e);
                Task::none()
            }
            TransferMessage::MampImportHere => {
                let Some(path) = self.mamp_output.clone() else { return Task::none() };
                self.opening = true;
                Task::perform(blocking(move || Some(open_bundle(&path))), TransferMessage::BundleOpened)
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
            page_header(
                tr("백업·이전"),
                tr("프로젝트 파일·설정·DB를 다른 PC(리눅스 ↔ 맥)로 옮깁니다. 같은 와이파이면 바로 보내고, 아니면 파일로 옮기세요"),
                None,
            ),
            Space::with_height(16),
            section_label(tr("같은 네트워크로 바로 이전")),
            row![card(self.send_view()), card(self.receive_view())].spacing(12),
            Space::with_height(14),
        ];
        if self.mamp.is_some() {
            col = col.push(section_label(tr("MAMP에서 옮기기"))).push(card(self.mamp_view())).push(Space::with_height(14));
        }
        col = col.push(column![
            section_label(tr("함께 옮길 데이터베이스")),
            card(self.selection_view()),
            Space::with_height(14),
            section_label(tr("파일로 옮기기")),
            row![card(self.export_view()), card(self.import_view())].spacing(12),
            Space::with_height(14),
            section_label(tr("최근 이전 기록")),
            card(self.history_view()),
        ]);
        if let Some(e) = &self.error {
            col = col.push(Space::with_height(12)).push(result_lines(std::iter::once(format!("✗ {e}"))));
        }
        col.into()
    }

    /// 보내기·파일 내보내기에 함께 쓰는 선택: DB 와 접속 정보
    fn selection_view(&self) -> Element<'_, TransferMessage> {
        let all_on = !self.dbs.is_empty() && self.dbs.iter().all(|d| d.2);
        let mut header = row![muted(tr("바로 보내기와 파일 내보내기에 모두 적용됩니다")).width(Length::Fill)]
            .spacing(12)
            .align_y(iced::Alignment::Center);
        if !self.dbs.is_empty() {
            header = header.push(check(tr("전체"), all_on).on_toggle(TransferMessage::ToggleAllDbs));
        }
        header = header.push(btn(tr("다시 읽기"), Some(Icon::Refresh), Kind::Ghost).on_press(TransferMessage::LoadDatabases));
        let mut c = column![header, Space::with_height(10)];

        if self.dbs_loading {
            c = c.push(muted(tr("DB 목록을 읽는 중…")));
        } else if self.dbs.is_empty() {
            c = c.push(muted(tr("담을 DB가 없습니다. 데이터베이스 탭에서 접속 정보를 저장하면 목록이 나옵니다.")));
        } else {
            let mut list = row![].spacing(18);
            for (i, (engine, name, on)) in self.dbs.iter().enumerate() {
                list = list.push(check(format!("{name} · {}", engine.label()), *on).on_toggle(move |v| TransferMessage::ToggleDb(i, v)));
            }
            // DB 가 많으면(예: MAMP 에서 19개) 한 줄로 늘어서 화면 밖으로 넘친다 → 줄바꿈
            c = c.push(list.wrap());
        }

        c = c.push(Space::with_height(12)).push(theme::divider()).push(Space::with_height(12)).push(
            check(tr("DB 접속 정보(비밀번호 포함)도 담기"), self.export_credentials).on_toggle(TransferMessage::ToggleExportCredentials),
        );
        if self.export_credentials {
            c = c.push(Space::with_height(6)).push(chip(tr("비밀번호가 평문으로 들어갑니다 — 파일을 공유하지 마세요"), Tone::Warning));
        }
        c.into()
    }

    fn export_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![
            card_title(Icon::Download, tr("파일로 내보내기")),
            muted(tr("설정과 고른 DB를 한 파일로 묶습니다. 프로젝트 소스 폴더는 담지 않습니다.")),
            Space::with_height(12),
        ]
        .spacing(4);
        let b = btn(if self.exporting { tr("내보내는 중…") } else { tr("백업 파일 만들기") }, Some(Icon::HardDrive), Kind::Primary);
        c = c.push(if self.exporting { b } else { b.on_press(TransferMessage::Export) });
        match &self.export_result {
            Some(Ok(s)) => c = c.push(Space::with_height(8)).push(result_lines(s.lines())),
            Some(Err(e)) => c = c.push(Space::with_height(8)).push(result_lines(std::iter::once(format!("✗ {e}")))),
            None => {}
        }
        c.into()
    }

    fn import_view(&self) -> Element<'_, TransferMessage> {
        let mut c = column![card_title(Icon::Upload, tr("파일에서 가져오기"))].spacing(4);

        let Some(b) = &self.bundle else {
            let open = btn(if self.opening { tr("여는 중…") } else { tr("백업 파일 열기") }, Some(Icon::FolderOpen), Kind::Flat);
            return c
                .push(muted(tr("다른 PC에서 만든 localman-backup-*.tar.gz 를 엽니다.")))
                .push(Space::with_height(12))
                .push(if self.opening { open } else { open.on_press(TransferMessage::PickBundle) })
                .into();
        };

        let m = &b.manifest;
        let creds = if m.includes_credentials { format!(" · {}", tr("접속 정보 포함")) } else { String::new() };
        let summary = trf("{0} ({1})에서 만든 백업 · 프로젝트 {2}개 · DB {3}개", &[
            &m.hostname,
            &os_label(&m.source_os),
            &b.projects.len(),
            &m.databases.len(),
        ]);
        c = c.push(muted(summary + &creds));

        c = c.push(Space::with_height(10)).push(text(tr("프로젝트 경로 바꾸기")).size(12).font(theme::MEDIUM).color(p().fg3)).push(
            row![
                input(tr("원래 경로 앞부분"), &self.path_from).on_input(TransferMessage::PathFromChanged),
                icon(Icon::ArrowLeftRight, 14.0, p().fg3),
                input(tr("이 PC 경로 앞부분"), &self.path_to).on_input(TransferMessage::PathToChanged),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );

        c = c.push(Space::with_height(8));
        for pl in &self.plan {
            let (mark, tone) = if pl.exists_here && !self.overwrite {
                (tr("건너뜀"), Tone::Neutral)
            } else if !pl.folder_exists {
                (tr("폴더 없음"), Tone::Warning)
            } else if pl.exists_here {
                (tr("덮어씀"), Tone::Warning)
            } else {
                (tr("추가"), Tone::Success)
            };
            let mut line = format!("{}  {}", pl.project.id, pl.project.path);
            if let Some(old) = pl.port_changed_from {
                line.push_str("  ");
                line.push_str(&trf("(포트 {0}에서 {1}(으)로)", &[&old, &pl.project.port]));
            }
            c = c.push(row![chip(mark, tone), text(line).size(12).color(p().fg2).width(Length::Fill)].spacing(8).align_y(iced::Alignment::Center));
        }

        c = c.push(Space::with_height(8)).push(check(tr("이미 있는 프로젝트·DB 덮어쓰기"), self.overwrite).on_toggle(TransferMessage::ToggleOverwrite));
        if !m.databases.is_empty() {
            let names: Vec<String> = m.databases.iter().map(|d| d.name.clone()).collect();
            c = c.push(check(trf("DB 복원: {0}", &[&names.join(", ")]), self.restore_dbs).on_toggle(TransferMessage::ToggleRestoreDbs));
        }
        if !b.credentials.is_empty() {
            c = c.push(
                check(tr("DB 접속 정보 가져오기 (이 PC에 같은 사용자가 있으면 이 PC 설정 유지)"), self.import_credentials)
                    .on_toggle(TransferMessage::ToggleImportCredentials),
            );
        }
        c = c.push(muted(tr("프로젝트를 추가하면 이 PC의 Apache 가상호스트와 /etc/hosts가 새로 만들어집니다.")));

        let go = btn(if self.importing { tr("가져오는 중…") } else { tr("가져오기") }, Some(Icon::Check), Kind::Primary);
        let close = btn(tr("닫기"), None, Kind::Ghost);
        c = c.push(Space::with_height(10)).push(if self.importing {
            row![go, close].spacing(8)
        } else {
            row![go.on_press(TransferMessage::Import), close.on_press(TransferMessage::CloseBundle)].spacing(8)
        });
        if !self.import_log.is_empty() {
            c = c.push(Space::with_height(8)).push(result_lines(self.import_log.iter().map(|s| s.as_str())));
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
            card_title(Icon::Send, tr("다른 PC로 보내기")),
            muted(tr("바뀐 파일만 보냅니다. node_modules·venv 같은 의존성 폴더는 빼니 받은 PC에서 패키지를 설치하세요.")),
            Space::with_height(10),
            label(tr("보낼 것")),
            pick_list(scopes, Some(self.scope.clone()), TransferMessage::ScopeSelected)
                .text_size(13)
                .padding([8, 12])
                .width(Length::Fill)
                .style(theme::pick_style),
            Space::with_height(6),
        ]
        .spacing(4);

        // 받는 PC
        let find = btn(if self.searching { tr("찾는 중…") } else { tr("찾기") }, Some(Icon::Search), Kind::Flat);
        let find = if !self.searching && !busy { find.on_press(TransferMessage::FindPeers) } else { find };
        let header = row![label(tr("받는 PC")), Space::with_width(Length::Fill), find].spacing(6).align_y(iced::Alignment::Center);
        // 찾은 PC 는 버튼 옆에 붙이지 않고 아래 줄에 모아, 여러 대여도 줄바꿈되게 한다
        let mut peers = row![].spacing(6);
        for (i, pe) in self.peers.iter().enumerate() {
            let selected = self.peer_addr == pe.addr.ip().to_string();
            peers = peers.push(
                theme::chip_btn(format!("{} · {}", pe.name, os_label(&pe.os)), if selected { Kind::Primary } else { Kind::Flat })
                    .on_press(TransferMessage::PickPeer(i)),
            );
        }
        c = c.push(header);
        if !self.peers.is_empty() {
            c = c.push(peers.wrap());
        }
        c = c.push(
            row![
                input(tr("IP (예: 192.168.0.12)"), &self.peer_addr).on_input(TransferMessage::PeerAddrChanged),
                container(input(tr("코드 6자리"), &self.code).on_input(TransferMessage::CodeChanged)).width(120),
            ]
            .spacing(8),
        );
        if !self.searching && self.peers.is_empty() {
            c = c.push(muted(tr("받는 PC에서 [받기 대기]를 먼저 누르세요. 찾지 못하면 IP를 직접 입력하세요.")));
        }

        let ready = !busy && self.code.len() == 6 && !self.peer_addr.trim().is_empty();
        let go = btn(if busy { tr("보내는 중…") } else { tr("보내기") }, Some(Icon::Send), Kind::Primary);
        c = c.push(Space::with_height(8)).push(if ready { go.on_press(TransferMessage::Send) } else { go });
        c.push(job_view(&self.sending)).into()
    }

    fn receive_view(&self) -> Element<'_, TransferMessage> {
        let waiting = self.recv_stop.is_some();
        let mut c = column![
            card_title(Icon::Download, tr("이 PC에서 받기")),
            muted(tr("받기 대기를 누르면 코드가 나옵니다. 보내는 PC에 그 코드를 입력하세요. 코드는 한 번만 쓸 수 있습니다.")),
            Space::with_height(10),
            check(tr("이미 있는 것도 보내는 쪽 기준으로 덮어쓰기"), self.recv_overwrite)
                .on_toggle_maybe((!waiting).then_some(TransferMessage::ToggleRecvOverwrite)),
            muted(tr("프로젝트·DB, 그리고 이 PC 쪽이 더 최신인 파일까지")),
            Space::with_height(8),
        ]
        .spacing(4);
        c = c.push(if waiting {
            btn(tr("멈추기"), Some(Icon::Square), Kind::Danger).on_press(TransferMessage::StopReceive)
        } else {
            btn(tr("받기 대기"), Some(Icon::Wifi), Kind::Primary).on_press(TransferMessage::StartReceive)
        });

        if let Some((code, name)) = &self.recv_code {
            if self.receiving.status.is_empty() {
                c = c.push(Space::with_height(10)).push(theme::inset(
                    row![
                        column![
                            muted(tr("코드")),
                            text(format!("{} {}", &code[..3], &code[3..])).size(34).font(theme::BOLD).color(p().primary_fg),
                        ]
                        .width(Length::Fill),
                        column![
                            muted(tr("이 PC")),
                            text(name.clone()).size(15).font(theme::SEMIBOLD).color(p().fg),
                            muted(trf("포트 {0}", &[&TRANSFER_PORT])),
                        ]
                        .align_x(iced::Alignment::End),
                    ]
                    .align_y(iced::Alignment::Center),
                ));
            }
        }
        c.push(job_view(&self.receiving)).into()
    }

    fn history_view(&self) -> Element<'_, TransferMessage> {
        if self.history.is_empty() {
            return muted(tr("아직 다른 PC와 주고받은 기록이 없습니다.")).into();
        }
        self.history
            .iter()
            .fold(column![].spacing(6), |c, (l, ok)| {
                let tone = if *ok { Tone::Success } else { Tone::Warning };
                c.push(row![column![Space::with_height(6), theme::dot(tone)], text(l).size(12).color(p().fg2).width(Length::Fill)].spacing(8))
            })
            .into()
    }
}

impl TransferState {
    fn mamp_view(&self) -> Element<'_, TransferMessage> {
        let Some(info) = &self.mamp else { return Space::with_height(0).into() };
        let busy = self.mamp_job.running;
        let mut c = column![
            card_title(Icon::Database, tr("MAMP 자료 백업·이전")),
            muted(tr("MAMP의 사이트와 MySQL DB를 로컬맨 백업 파일로 만듭니다. 이 PC로 바로 가져오거나 리눅스로 옮길 수 있습니다. MAMP 폴더는 읽기만 합니다.")),
            Space::with_height(10),
        ]
        .spacing(4);

        // 사이트(가상호스트)
        c = c.push(label(tr("사이트 (가상호스트)")));
        let mut any_site = false;
        for (i, v) in info.vhosts.iter().enumerate() {
            let Some(proj) = vhost_to_project(v) else { continue };
            any_site = true;
            let on = self.mamp_vhosts.get(i).copied().unwrap_or(false);
            c = c.push(
                row![
                    check(format!("{} → {}", v.server_name, proj.domain), on).on_toggle(move |x| TransferMessage::MampToggleVhost(i, x)),
                    if v.exists { chip(tr("폴더 있음"), Tone::Success) } else { chip(tr("폴더 없음"), Tone::Warning) },
                    text(v.doc_root.clone()).size(11).color(p().fg4).width(Length::Fill),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        if !any_site {
            c = c.push(muted(tr("옮길 가상호스트가 없습니다.")));
        }

        // DB
        let total: u64 = info.databases.iter().zip(&self.mamp_dbs).filter(|(_, on)| **on).map(|(d, _)| d.bytes).sum();
        let all_on = !self.mamp_dbs.is_empty() && self.mamp_dbs.iter().all(|x| *x);
        c = c.push(Space::with_height(8)).push(
            row![
                label("MySQL DB").width(Length::Fill),
                muted(trf("고른 것 {0}", &[&human_bytes(total)])),
                check(tr("전체"), all_on).on_toggle(TransferMessage::MampToggleAllDbs),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
        );
        let mut grid = column![].spacing(4);
        for chunk in info.databases.iter().enumerate().collect::<Vec<_>>().chunks(3) {
            let mut r = row![].spacing(12);
            for (i, d) in chunk {
                let i = *i;
                let on = self.mamp_dbs.get(i).copied().unwrap_or(false);
                r = r.push(
                    container(check(format!("{} · {}", d.name, human_bytes(d.bytes)), on).on_toggle(move |x| TransferMessage::MampToggleDb(i, x)))
                        .width(Length::FillPortion(1)),
                );
            }
            for _ in chunk.len()..3 {
                r = r.push(Space::with_width(Length::FillPortion(1)));
            }
            grid = grid.push(r);
        }
        c = c.push(grid);
        c = c.push(Space::with_height(4)).push(
            check(tr("DB 사용자·권한도 옮기기 (사이트가 쓰는 DB 계정 — 비밀번호 해시째)"), self.mamp_users).on_toggle(TransferMessage::MampToggleUsers),
        );

        c = c.push(Space::with_height(8)).push(
            row![
                container(column![label(tr("MAMP MySQL 사용자")), input("root", &self.mamp_user).on_input(TransferMessage::MampUserChanged)].spacing(4)).width(160),
                container(
                    column![
                        label(tr("비밀번호")),
                        theme::password_field(
                            input("root", &self.mamp_password).on_input(TransferMessage::MampPasswordChanged),
                            self.mamp_show_pw,
                            TransferMessage::MampToggleShowPw,
                        ),
                    ]
                    .spacing(4),
                )
                .width(160),
                column![
                    Space::with_height(19),
                    {
                        let b = btn(if busy { tr("만드는 중…") } else { tr("MAMP 백업 파일 만들기") }, Some(Icon::HardDrive), Kind::Primary);
                        if busy { b } else { b.on_press(TransferMessage::MampExport) }
                    },
                ],
            ]
            .spacing(10)
            .align_y(iced::Alignment::End),
        );
        c = c.push(muted(tr("MAMP MySQL이 꺼져 있으면 덤프하는 동안만 포트 없이 띄웠다가 다시 내립니다 (로컬맨 MySQL과 부딪치지 않음).")));
        c = c.push(job_view(&self.mamp_job));
        if self.mamp_output.is_some() && !busy {
            c = c.push(Space::with_height(6)).push(
                btn(tr("이 PC로 바로 가져오기"), Some(Icon::Download), Kind::Primary).on_press(TransferMessage::MampImportHere),
            );
            c = c.push(muted(tr("아래 [파일에서 가져오기]에 열립니다. 경로·덮어쓰기를 확인한 뒤 [가져오기]를 누르세요.")));
        }
        c.into()
    }
}

fn card_title<'a>(i: Icon, t: &'a str) -> Element<'a, TransferMessage> {
    row![icon(i, 15.0, p().fg2), theme::title(t)].spacing(8).align_y(iced::Alignment::Center).into()
}

fn label<'a>(t: &'a str) -> text::Text<'a, iced::Theme> {
    text(t).size(12).font(theme::MEDIUM).color(p().fg3)
}

/// "192.168.0.12" 또는 "192.168.0.12:47801"
fn parse_addr(s: &str) -> Option<SocketAddr> {
    let s = s.trim();
    s.parse::<SocketAddr>()
        .ok()
        .or_else(|| s.parse::<std::net::IpAddr>().ok().map(|ip| SocketAddr::new(ip, TRANSFER_PORT)))
}

fn job_view(job: &LanJob) -> Element<'_, TransferMessage> {
    let mut c = column![].spacing(6);
    if !job.status.is_empty() {
        c = c.push(Space::with_height(6)).push(theme::status_line(job.status.clone(), Tone::Primary));
    }
    if let Some((files, done, total)) = job.progress {
        if total > 0 {
            let done_line = trf("{0} / {1} · 파일 {2}개 완료", &[&human_bytes(done), &human_bytes(total), &files]);
            c = c.push(theme::progress(total as f32, done as f32, Tone::Primary)).push(muted(done_line));
        }
    }
    if !job.log.is_empty() {
        c = c.push(result_lines(job.log.iter().map(|s| s.as_str())));
    }
    match &job.result {
        Some(Ok(lines)) => c = c.push(Space::with_height(4)).push(result_lines(lines.iter().map(|s| s.as_str()))),
        Some(Err(e)) => c = c.push(Space::with_height(4)).push(result_lines(std::iter::once(format!("✗ {e}")))),
        None => {}
    }
    c.into()
}

fn os_label(os: &str) -> &str {
    match os {
        "linux" => tr("리눅스"),
        "macos" => tr("맥"),
        "windows" => tr("윈도우"),
        other => other,
    }
}

fn result_lines<'a, S: AsRef<str>>(lines: impl Iterator<Item = S>) -> Element<'a, TransferMessage> {
    theme::log_block(lines.map(|l| l.as_ref().to_string()).collect(), TransferMessage::CopyLog)
}
