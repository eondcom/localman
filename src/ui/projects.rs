use iced::{
    widget::{column, container, row, scrollable, text, Space},
    Background, Element, Length, Task,
};
use super::theme::{
    self, Icon, Kind, Tone, btn, card, chip, icon, icon_btn, input, muted, p, page_header, result_line,
    segmented, status,
};
use crate::domain::settings::load_settings;
use crate::domain::DbEngine;
use crate::domain::deploy::{DeployTarget, default_target, load_target, merge_targets, preview_files, push_database, save_target, test_connection, upload_files};
use crate::domain::ProgressFn;
use crate::domain::history::{RunLog, list_run_logs, read_run_log, run_log_time, save_run_log};
use crate::domain::site_config::{self, SiteKind};
use crate::domain::pull::{default_local_db_name, preview_pull, pull_database, pull_files};
use crate::domain::lan::human_bytes;
use crate::domain::project::{ProjectDb, set_project_db};
use crate::domain::usage::{SiteUsage, detect_db, site_usage};
use std::collections::HashMap;
use crate::platform::open_url;
use crate::domain::{
    VhostProject, ProjectType, ServerStatus,
    list_projects, add_project, update_project, remove_project,
    start_server, stop_server, server_status, auto_assign_port,
    setup_project, deps_ready, auto_detect_start_command, auto_detect_next_command,
    detect_next_app_dir, join_dir,
};
use crate::platform::{error_log_path, read_log, clear_log};
use crate::domain::history::{last_for_project, load_history, summary_line};
use rfd;
use crate::i18n::{tr, trf};

/// 타입에 맞는 실행 명령어를 자동 감지한다.
fn detect_command_for(t: &ProjectType, path: &str, app_dir: &str, port: u16) -> String {
    let dir = join_dir(path, app_dir);
    match t {
        ProjectType::NextJs => auto_detect_next_command(&dir, port),
        ProjectType::Python => auto_detect_start_command(&dir, port),
        ProjectType::Php => String::new(),
    }
}

/// 프로젝트별 에러 로그 뷰 상태
struct LogView {
    id: String,
    name: String,
    content: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SiteToolMode {
    /// 관리자 비밀번호 바꾸기
    Password,
    /// 설정 파일의 DB 정보를 로컬로
    LocalDb,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SiteField {
    UserId,
    NewPassword,
    DbName,
    DbUser,
    DbPassword,
}

/// 라이믹스·XE·워드프레스 손보기 패널
struct SiteTool {
    project_id: String,
    project_name: String,
    kind: SiteKind,
    mode: SiteToolMode,
    admins: Vec<(String, String)>,
    user_id: String,
    new_password: String,
    db_name: String,
    db_user: String,
    db_password: String,
    url: bool,
    has_backup: bool,
    running: bool,
    log: Vec<String>,
    /// 서버(운영) DB 를 손볼지 — 서버 정보가 저장돼 있을 때만
    server: Option<DeployTarget>,
    on_server: bool,
    /// 서버 비밀번호 바꾸기 확인 중
    confirm_server: bool,
    show_pw: bool,
}

#[derive(Debug, Clone)]
pub enum ProjectsMessage {
    // 신규 추가 폼
    OpenFilePicker,
    PathSelected(Option<String>),
    NameChanged(String),
    IdChanged(String),
    TypeSelected(ProjectType),
    StartCommandChanged(String),
    AppDirChanged(String),
    AddProject,
    // 목록 아이템
    RemoveProject(String),
    StartServer(String),
    StopServer(String),
    ServerStarted(String, Result<u32, String>),
    ServerStopped(String, Result<(), String>),
    // 인라인 편집
    EditProject(String),
    EditNameChanged(String),
    EditPathChanged(String),
    EditStartCommandChanged(String),
    EditAppDirChanged(String),
    EditTypeSelected(ProjectType),
    EditPickFolder(String),
    EditFolderSelected(String, Option<String>), // (id, path)
    SaveEdit(String),
    CancelEdit,
    // 의존성 자동 설치 (Python: venv, Next.js: node_modules)
    SetupDeps(String),
    DepsSetupDone(String, Result<String, String>),
    // 에러 로그
    ViewLog(String),
    LogLoaded(String, Result<String, String>),
    CopyLog,
    CopyText(String),
    ClearLog(String),
    LogCleared(String, Result<(), String>),
    CloseLog,
    #[allow(dead_code)]
    Refresh,
    // 사이트(라이믹스·XE·워드프레스) 손보기
    OpenSiteTool(String, SiteToolMode),
    SiteAdminsLoaded(Result<Vec<(String, String)>, String>),
    SiteField(SiteField, String),
    SiteUrlToggled(bool),
    SiteSetPassword,
    /// 이 PC(false) / 서버(true)
    SiteTarget(bool),
    SiteServerConfirm,
    SiteServerCancel,
    SiteToggleShowPw,
    DeployToggleShowPw,
    SiteLocalize,
    SiteRestore,
    SiteToolDone(Result<Vec<String>, String>),
    CloseSiteTool,
    // 새 프로젝트 폼 열기/닫기
    ToggleAddForm,
    // 도메인·이름 검색
    SearchChanged(String),
    // 사이트별 용량
    ComputeUsage,
    UsageComputed(Vec<(String, SiteUsage)>),
    // 수정 화면의 DB 지정 (비우면 자동 감지)
    EditDbNameChanged(String),
    EditDbEngineSelected(DbEngine),
    // 서버 배포
    OpenDeploy(String),
    CloseDeploy,
    DeployField(DeployField, String),
    DeploySave,
    DeployTest,
    DeployPreview,
    DeployUpload,
    DeployDbAsk,
    DeployDbCancel,
    DeployDbConfirm,
    DeployDone(Result<Vec<String>, String>),
    /// 진행 상황 (설명, 한 양/전체 양)
    DeployProgress(String, Option<(u64, u64)>),
    /// 지난 작업 로그를 패널에 띄운다
    ViewRunLog(std::path::PathBuf),
    /// 기록 파일을 기본 앱으로 연다
    OpenRunLog(std::path::PathBuf),
    /// 연결 시험 결과와 서버에서 찾은 웹 경로
    DeployTested(Vec<String>, Option<String>),
    DeployDbViaSsh(bool),
    PullPreview,
    PullFiles,
    /// 서버에서 가져오기 확인 (true: 파일까지 모두)
    PullDbAsk(bool),
    PullConfirm,
    /// 도메인을 브라우저로 연다
    OpenSite(String),
    /// 프로젝트 폴더를 Finder·파일 관리자로 연다
    OpenFolder(String),
    // 더보기(⋯) 메뉴
    ToggleMenu(String),
    /// 다른 PC로 보내기 — App 이 백업·이전 탭으로 넘겨 처리한다
    SendToPc(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeployField {
    Host,
    Port,
    User,
    Key,
    Path,
    Excludes,
    DbHost,
    DbPort,
    DbUser,
    DbPassword,
    DbName,
    SshPassword,
    PullExcludes,
    LocalDb,
}

/// 서버 배포 패널의 입력값과 진행 상태
struct DeployForm {
    id: String,
    host: String,
    port: String,
    user: String,
    key: String,
    path: String,
    excludes: String,
    db_host: String,
    db_port: String,
    db_user: String,
    db_password: String,
    db_name: String,
    ssh_password: String,
    db_via_ssh: bool,
    pull_excludes: String,
    /// 서버 DB 를 넣을 로컬 DB 이름
    local_db: String,
    busy: Option<&'static str>,
    /// 진행 중인 작업의 지금 상황
    progress: Option<(String, Option<(u64, u64)>)>,
    /// 로그를 보일 구역 (0: 위, 1: 가져오기, 2: 올리기)
    log_at: u8,
    /// 비밀번호 칸 글자 보기
    show_pw: bool,
    /// 넣을 로컬 DB 가 이미 있는지 (가져오기 확인할 때 확인)
    local_db_exists: Option<bool>,
    /// 지난 작업 기록
    runs: Vec<RunLog>,
    /// 지금 보이는 로그의 기록 파일
    log_file: Option<std::path::PathBuf>,
    log: Vec<String>,
    /// DB 덮어쓰기 확인 중
    confirm_db: bool,
    /// 서버에서 가져오기 확인 중 (Some(true): 파일까지 모두)
    confirm_pull: Option<bool>,
    /// 창을 연(또는 마지막으로 저장한) 때의 값 — 저장할 때 3-way 병합의 기준
    loaded: DeployTarget,
}

impl DeployForm {
    fn from_target(id: &str, t: &DeployTarget, local_db: String) -> Self {
        Self {
            id: id.to_string(),
            host: t.ssh_host.clone(),
            port: t.ssh_port.to_string(),
            user: t.ssh_user.clone(),
            key: t.ssh_key.clone(),
            path: t.remote_path.clone(),
            excludes: t.excludes.join(", "),
            db_host: t.db_host.clone(),
            db_port: if t.db_port == 0 { String::new() } else { t.db_port.to_string() },
            db_user: t.db_user.clone(),
            db_password: t.db_password.clone(),
            db_name: t.db_name.clone(),
            ssh_password: t.ssh_password.clone(),
            db_via_ssh: t.db_via_ssh,
            pull_excludes: t.pull_excludes.join(", "),
            local_db,
            busy: None,
            progress: None,
            log_at: 0,
            show_pw: false,
            local_db_exists: None,
            runs: list_run_logs(id),
            log_file: None,
            log: Vec::new(),
            confirm_db: false,
            confirm_pull: None,
            loaded: t.clone(),
        }
    }

    /// 칸들을 이 값으로 채운다 (진행 상태·로그는 그대로)
    fn fill(&mut self, t: &DeployTarget) {
        let fresh = Self::from_target(&self.id, t, t.local_db.clone());
        self.host = fresh.host;
        self.port = fresh.port;
        self.user = fresh.user;
        self.key = fresh.key;
        self.path = fresh.path;
        self.excludes = fresh.excludes;
        self.db_host = fresh.db_host;
        self.db_port = fresh.db_port;
        self.db_user = fresh.db_user;
        self.db_password = fresh.db_password;
        self.db_name = fresh.db_name;
        self.ssh_password = fresh.ssh_password;
        self.db_via_ssh = fresh.db_via_ssh;
        self.pull_excludes = fresh.pull_excludes;
        if !t.local_db.is_empty() {
            self.local_db = t.local_db.clone();
        }
    }

    /// 입력값을 저장한다. 창을 연 뒤 다른 곳에서 바뀐 칸은, 여기서 손대지 않았다면 그 값을 따른다.
    fn save(&mut self) -> Result<DeployTarget, String> {
        let ours = self.to_target()?;
        let merged = match load_target(&self.id) {
            Some(theirs) => merge_targets(&self.loaded, &ours, &theirs),
            None => ours,
        };
        save_target(&self.id, &merged)?;
        self.fill(&merged);
        self.loaded = merged.clone();
        Ok(merged)
    }

    fn to_target(&self) -> Result<DeployTarget, String> {
        let port = |s: &str, d: u16| -> Result<u16, String> {
            if s.trim().is_empty() { Ok(d) } else { s.trim().parse().map_err(|_| trf("포트가 숫자가 아닙니다: {0}", &[&s])) }
        };
        Ok(DeployTarget {
            ssh_host: self.host.trim().to_string(),
            ssh_port: port(&self.port, 22)?,
            ssh_user: self.user.trim().to_string(),
            ssh_key: self.key.trim().to_string(),
            remote_path: self.path.trim().to_string(),
            excludes: self.excludes.split(',').map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect(),
            db_host: self.db_host.trim().to_string(),
            db_port: port(&self.db_port, 3306)?,
            db_user: self.db_user.trim().to_string(),
            db_password: self.db_password.clone(),
            db_name: self.db_name.trim().to_string(),
            ssh_password: self.ssh_password.clone(),
            db_via_ssh: self.db_via_ssh,
            pull_excludes: self.pull_excludes.split(',').map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect(),
            local_db: self.local_db.trim().to_string(),
        })
    }
}

pub struct ProjectsState {
    projects: Vec<VhostProject>,
    // 신규 추가 폼
    new_name: String,
    new_id: String,
    new_path: String,
    new_type: ProjectType,
    new_start_command: String,
    new_app_dir: String,
    // 인라인 편집 상태
    editing_id: Option<String>,
    edit_name: String,
    edit_path: String,
    edit_start_command: String,
    edit_app_dir: String,
    edit_type: ProjectType,
    // 의존성 설치 중인 project id 목록
    setting_up: std::collections::HashSet<String>,
    // 에러 로그 뷰 (열려 있으면 Some)
    log_view: Option<LogView>,
    // 라이믹스 관리자 비번 변경 모달 (열려 있으면 Some)
    site_tool: Option<SiteTool>,
    // 메시지
    error: Option<String>,
    server_message: Option<Result<String, String>>,
    // App 이 꺼내 토스트로 띄울 알림
    toasts: Vec<Result<String, String>>,
    // 새 프로젝트 폼이 펼쳐져 있는지
    adding: bool,
    search: String,
    usage: HashMap<String, SiteUsage>,
    usage_loading: bool,
    edit_db_name: String,
    edit_db_engine: DbEngine,
    deploy: Option<DeployForm>,
    // 더보기 메뉴가 열린 project id
    menu_open: Option<String>,
    // project id → (마지막 이전 한 줄, 최근 이전 기록 줄들). 그릴 때마다 파일을 읽지 않게 미리 만든다
    transfers: std::collections::HashMap<String, (String, Vec<(bool, String)>)>,
}

fn load_transfers(projects: &[VhostProject]) -> std::collections::HashMap<String, (String, Vec<(bool, String)>)> {
    let history = load_history();
    projects
        .iter()
        .filter_map(|p| {
            let last = last_for_project(&history, &p.id)?;
            let recent: Vec<(bool, String)> = history
                .iter()
                .rev()
                .filter(|r| r.projects.iter().any(|x| x == &p.id))
                .take(5)
                .map(|r| (r.ok, summary_line(r)))
                .collect();
            Some((p.id.clone(), (summary_line(&last), recent)))
        })
        .collect()
}

impl ProjectsState {
    pub fn new() -> Self {
        let projects = list_projects();
        Self {
            transfers: load_transfers(&projects),
            menu_open: None,
            toasts: Vec::new(),
            adding: false,
            search: String::new(),
            usage: HashMap::new(),
            usage_loading: false,
            edit_db_name: String::new(),
            edit_db_engine: DbEngine::MariaDb,
            deploy: None,
            projects,
            new_name: String::new(),
            new_id: String::new(),
            new_path: String::new(),
            new_type: ProjectType::Php,
            new_start_command: "python app.py".to_string(),
            new_app_dir: String::new(),
            editing_id: None,
            edit_name: String::new(),
            edit_path: String::new(),
            edit_start_command: String::new(),
            edit_app_dir: String::new(),
            edit_type: ProjectType::Php,
            setting_up: std::collections::HashSet::new(),
            log_view: None,
            site_tool: None,
            error: None,
            server_message: None,
        }
    }

    pub fn take_toasts(&mut self) -> Vec<Result<String, String>> {
        std::mem::take(&mut self.toasts)
    }

    /// 인라인 오류 표시와 함께 토스트도 띄운다.
    /// 오류 문구는 목록 위쪽에 찍혀, 목록이 길면 편집 중인 행에서 보이지 않는다.
    fn set_error(&mut self, e: String) {
        self.toasts.push(Err(e.clone()));
        self.error = Some(e);
    }

    pub fn update(&mut self, msg: ProjectsMessage) -> Task<ProjectsMessage> {
        match msg {
            ProjectsMessage::Refresh => {
                self.projects = list_projects();
                self.transfers = load_transfers(&self.projects);
                Task::none()
            }
            ProjectsMessage::SearchChanged(v) => {
                self.search = v;
                Task::none()
            }
            ProjectsMessage::ComputeUsage => {
                if self.usage_loading {
                    return Task::none();
                }
                self.usage_loading = true;
                let projects = self.projects.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            projects.iter().map(|p| (p.id.clone(), site_usage(p))).collect::<Vec<_>>()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    ProjectsMessage::UsageComputed,
                )
            }
            ProjectsMessage::UsageComputed(list) => {
                self.usage_loading = false;
                self.usage = list.into_iter().collect();
                Task::none()
            }
            ProjectsMessage::EditDbNameChanged(v) => {
                self.edit_db_name = v;
                Task::none()
            }
            ProjectsMessage::EditDbEngineSelected(e) => {
                self.edit_db_engine = e;
                Task::none()
            }
            ProjectsMessage::OpenDeploy(id) => {
                if let Some(p) = self.projects.iter().find(|p| p.id == id) {
                    let t = load_target(&id).unwrap_or_else(|| default_target(p));
                    let local = default_local_db_name(p, &t);
                    self.deploy = Some(DeployForm::from_target(&id, &t, local));
                }
                self.menu_open = None;
                Task::none()
            }
            ProjectsMessage::CloseDeploy => {
                self.deploy = None;
                Task::none()
            }
            ProjectsMessage::DeployField(f, v) => {
                if let Some(d) = self.deploy.as_mut() {
                    let slot = match f {
                        DeployField::Host => &mut d.host,
                        DeployField::Port => &mut d.port,
                        DeployField::User => &mut d.user,
                        DeployField::Key => &mut d.key,
                        DeployField::Path => &mut d.path,
                        DeployField::Excludes => &mut d.excludes,
                        DeployField::DbHost => &mut d.db_host,
                        DeployField::DbPort => &mut d.db_port,
                        DeployField::DbUser => &mut d.db_user,
                        DeployField::DbPassword => &mut d.db_password,
                        DeployField::DbName => &mut d.db_name,
                        DeployField::SshPassword => &mut d.ssh_password,
                        DeployField::PullExcludes => &mut d.pull_excludes,
                        DeployField::LocalDb => &mut d.local_db,
                    };
                    *slot = v;
                    d.confirm_db = false;
                    d.confirm_pull = None;
                }
                Task::none()
            }
            ProjectsMessage::DeploySave => {
                if let Some(d) = self.deploy.as_mut() {
                    d.log_at = 0;
                    d.log = match d.save() {
                        Ok(_) => vec![format!("✓ {}", tr("서버 정보를 저장했습니다"))],
                        Err(e) => vec![format!("✗ {e}")],
                    };
                }
                Task::none()
            }
            ProjectsMessage::DeployDbViaSsh(on) => {
                if let Some(d) = self.deploy.as_mut() {
                    d.db_via_ssh = on;
                }
                Task::none()
            }
            ProjectsMessage::PullDbAsk(all) => {
                if let Some(d) = self.deploy.as_mut() {
                    d.confirm_db = false;
                    // 시작하기 전에 빠진 값을 알린다 (파일을 다 받고 나서야 실패하지 않게)
                    let missing = if d.db_name.trim().is_empty() || d.db_user.trim().is_empty() {
                        Some(tr("서버 DB 정보(DB 이름·사용자)를 입력하세요."))
                    } else if d.local_db.trim().is_empty() {
                        Some(tr("넣을 로컬 DB 이름을 입력하세요 (없으면 새로 만듭니다)."))
                    } else {
                        None
                    };
                    d.log_at = 1;
                    match missing {
                        Some(m) => {
                            d.confirm_pull = None;
                            d.log = vec![format!("✗ {m}")];
                        }
                        None => {
                            d.log.clear();
                            let name = d.local_db.trim().to_string();
                            d.local_db_exists = crate::domain::load_db_connections()
                                .into_iter()
                                .find(|c| c.engine == crate::domain::DbEngine::MariaDb)
                                .map(|c| crate::domain::list_databases(c.engine, &c.user, &c.password).iter().any(|x| x.name == name));
                            d.confirm_pull = Some(all);
                        }
                    }
                }
                Task::none()
            }
            ProjectsMessage::DeployTest
            | ProjectsMessage::DeployPreview
            | ProjectsMessage::DeployUpload
            | ProjectsMessage::DeployDbConfirm
            | ProjectsMessage::PullPreview
            | ProjectsMessage::PullFiles
            | ProjectsMessage::PullConfirm => {
                let Some(d) = self.deploy.as_mut() else { return Task::none() };
                let Some(p) = self.projects.iter().find(|p| p.id == d.id).cloned() else { return Task::none() };
                // 실행할 때마다 입력값을 저장해 둔다 (다음에 다시 쓰도록) — 다른 곳에서 바뀐 값은 살린다
                let t = match d.save() {
                    Ok(t) => t,
                    Err(e) => {
                        d.log = vec![format!("✗ {e}")];
                        return Task::none();
                    }
                };
                d.log_at = match msg {
                    ProjectsMessage::PullPreview | ProjectsMessage::PullFiles | ProjectsMessage::PullConfirm => 1,
                    ProjectsMessage::DeployPreview | ProjectsMessage::DeployUpload | ProjectsMessage::DeployDbConfirm => 2,
                    _ => 0,
                };
                let pull_all = d.confirm_pull == Some(true);
                let local_db = d.local_db.trim().to_string();
                d.confirm_db = false;
                d.confirm_pull = None;
                d.log.clear();
                type Job = Box<dyn FnOnce(&ProgressFn) -> Result<Vec<String>, String> + Send>;
                let (label, job): (&'static str, Job) = match msg {
                    ProjectsMessage::DeployTest => {
                        d.busy = Some(tr("연결 시험 중…"));
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || test_connection(&p, &t))
                                    .await
                                    .unwrap_or_else(|e| (vec![format!("✗ {e}")], None))
                            },
                            |(log, found)| ProjectsMessage::DeployTested(log, found),
                        );
                    }
                    ProjectsMessage::DeployPreview => (tr("올라갈 파일을 보는 중…"), Box::new(move |pr| {
                        preview_files(&p, &t, pr).map(|files| {
                            let mut log = vec![format!("✓ {}", trf("올라갈 파일 {0}개 (서버에만 있는 파일은 지우지 않음)", &[&files.len()]))];
                            log.extend(files.iter().take(40).map(|f| format!("· {f}")));
                            if files.len() > 40 {
                                log.push(format!("· {}", trf("… 외 {0}개", &[&(files.len() - 40)])));
                            }
                            log
                        })
                    })),
                    ProjectsMessage::DeployUpload => (tr("파일 올리는 중…"), Box::new(move |pr| upload_files(&p, &t, pr))),
                    ProjectsMessage::PullPreview => (tr("받을 파일을 보는 중…"), Box::new(move |pr| preview_pull(&p, &t, pr))),
                    ProjectsMessage::PullFiles => (tr("서버에서 파일 받는 중…"), Box::new(move |pr| pull_files(&p, &t, pr))),
                    ProjectsMessage::PullConfirm => (
                        if pull_all { tr("서버에서 파일·DB 받는 중…") } else { tr("서버 DB 받는 중…") },
                        Box::new(move |pr| {
                            let mut log = Vec::new();
                            if pull_all {
                                log.extend(pull_files(&p, &t, pr)?);
                            }
                            // 파일을 받은 뒤라야 사이트 설정(DB 계정)을 읽을 수 있다.
                            // DB 가 실패해도 받은 파일 기록은 남긴다
                            match pull_database(&p, &t, &local_db, pr) {
                                Ok(l) => log.extend(l),
                                Err(e) => log.push(format!("✗ {e}")),
                            }
                            Ok(log)
                        }),
                    ),
                    _ => (tr("원격 DB 백업 후 넣는 중…"), Box::new(move |_| push_database(&p, &t))),
                };
                d.busy = Some(label);
                d.progress = None;
                // 작업 스레드가 진행 상황을 보내고, 끝나면 결과를 보낸다
                let (tx, rx) = iced::futures::channel::mpsc::unbounded();
                std::thread::spawn(move || {
                    let ptx = tx.clone();
                    let pr: ProgressFn = std::sync::Arc::new(move |s, r| {
                        let _ = ptx.unbounded_send(ProjectsMessage::DeployProgress(s, r));
                    });
                    let _ = tx.unbounded_send(ProjectsMessage::DeployDone(job(&pr)));
                });
                Task::run(rx, |m| m)
            }
            ProjectsMessage::DeployDbAsk => {
                if let Some(d) = self.deploy.as_mut() {
                    d.confirm_db = true;
                    d.confirm_pull = None;
                    d.log_at = 2;
                }
                Task::none()
            }
            ProjectsMessage::DeployDbCancel => {
                if let Some(d) = self.deploy.as_mut() {
                    d.confirm_db = false;
                    d.confirm_pull = None;
                }
                Task::none()
            }
            ProjectsMessage::DeployTested(mut log, found) => {
                if let Some(d) = self.deploy.as_mut() {
                    d.busy = None;
                    // 찾은 웹 경로로 바꿔 저장해 둔다
                    if let Some(root) = found {
                        d.path = root;
                        if d.save().is_ok() {
                            log.push(format!("✓ {}", trf("웹 경로를 {0}(으)로 바꿔 저장했습니다", &[&d.path])));
                        }
                    }
                    d.log = log;
                }
                Task::none()
            }
            ProjectsMessage::ViewRunLog(path) => {
                if let Some(d) = self.deploy.as_mut() {
                    d.log = read_run_log(&path);
                    d.log_file = Some(path);
                    d.progress = None;
                    d.log_at = 3;
                }
                Task::none()
            }
            ProjectsMessage::OpenRunLog(path) => {
                open_url(&path.to_string_lossy());
                Task::none()
            }
            ProjectsMessage::DeployProgress(s, r) => {
                if let Some(d) = self.deploy.as_mut() {
                    if d.busy.is_some() {
                        d.progress = Some((s, r));
                    }
                }
                Task::none()
            }
            ProjectsMessage::DeployDone(r) => {
                if let Some(d) = self.deploy.as_mut() {
                    let title = d.busy.unwrap_or("").trim_end_matches('…').to_string();
                    d.busy = None;
                    d.log = match r {
                        Ok(l) => l,
                        Err(e) => vec![format!("✗ {e}")],
                    };
                    let ok = !d.log.iter().any(|l| l.starts_with('✗'));
                    // 끝났음을 분명히 — 성공이면 막대를 100% 로 남긴다
                    d.progress = ok.then(|| (tr("완료").to_string(), Some((1, 1))));
                    d.log_file = save_run_log(&d.id, &title, &d.log);
                    d.runs = list_run_logs(&d.id);
                }
                // 가져오기로 사이트에 DB 가 연결됐을 수 있다
                self.projects = list_projects();
                self.transfers = load_transfers(&self.projects);
                Task::none()
            }
            ProjectsMessage::ToggleAddForm => {
                self.adding = !self.adding;
                Task::none()
            }
            ProjectsMessage::OpenSite(domain) => {
                let scheme = if load_settings().https { "https" } else { "http" };
                open_url(&format!("{scheme}://{domain}"));
                Task::none()
            }
            ProjectsMessage::OpenFolder(path) => {
                if std::path::Path::new(&path).is_dir() {
                    open_url(&path);
                } else {
                    self.toasts.push(Err(trf("폴더가 없습니다: {0}", &[&path])));
                }
                Task::none()
            }
            ProjectsMessage::ToggleMenu(id) => {
                self.menu_open = if self.menu_open.as_deref() == Some(id.as_str()) { None } else { Some(id) };
                Task::none()
            }
            ProjectsMessage::SendToPc(_) => {
                // App 이 탭을 바꾸고 백업·이전 탭에 프로젝트를 넘긴다
                self.menu_open = None;
                Task::none()
            }
            ProjectsMessage::NameChanged(v) => {
                if self.new_id.is_empty() || self.new_id == slugify(&self.new_name) {
                    self.new_id = slugify(&v);
                }
                self.new_name = v;
                Task::none()
            }
            ProjectsMessage::IdChanged(v) => { self.new_id = v; Task::none() }
            ProjectsMessage::TypeSelected(t) => {
                if self.new_type != t {
                    self.new_type = t.clone();
                    let port = auto_assign_port();
                    self.new_start_command = if self.new_path.is_empty() {
                        match t {
                            ProjectType::NextJs => auto_detect_next_command("", port),
                            ProjectType::Python => "python app.py".to_string(),
                            ProjectType::Php => String::new(),
                        }
                    } else {
                        detect_command_for(&t, &self.new_path, &self.new_app_dir, port)
                    };
                }
                Task::none()
            }
            ProjectsMessage::StartCommandChanged(v) => { self.new_start_command = v; Task::none() }
            ProjectsMessage::AppDirChanged(v) => { self.new_app_dir = v; Task::none() }
            ProjectsMessage::OpenFilePicker => {
                Task::perform(pick_folder(), ProjectsMessage::PathSelected)
            }
            ProjectsMessage::PathSelected(path) => {
                if let Some(p) = path {
                    // 루트 또는 하위에 Next.js 앱이 있으면 타입·하위 디렉토리를 자동으로 맞춘다
                    // (아직 타입을 고르지 않은 기본 상태이거나 이미 Next.js를 고른 경우만)
                    if self.new_type == ProjectType::Php || self.new_type == ProjectType::NextJs {
                        if let Some(sub) = detect_next_app_dir(&p) {
                            self.new_type = ProjectType::NextJs;
                            self.new_app_dir = sub;
                        }
                    }
                    // 경로 선택 시 start_command 자동 감지
                    if self.new_type != ProjectType::Php {
                        let port = auto_assign_port();
                        self.new_start_command =
                            detect_command_for(&self.new_type, &p, &self.new_app_dir, port);
                    }
                    self.new_path = p;
                }
                Task::none()
            }
            ProjectsMessage::AddProject => {
                if self.new_id.is_empty() {
                    self.error = Some(tr("ID를 입력하세요.").to_string());
                    return Task::none();
                }
                if self.new_path.is_empty() {
                    self.error = Some(tr("경로를 입력하세요.").to_string());
                    return Task::none();
                }
                let port = if self.new_type.is_proxied() {
                    auto_assign_port()
                } else {
                    80
                };
                let project = VhostProject {
                    id: self.new_id.clone(),
                    name: self.new_name.clone(),
                    path: self.new_path.clone(),
                    domain: format!("{}.localhost", self.new_id),
                    project_type: self.new_type.clone(),
                    port,
                    start_command: self.new_start_command.clone(),
                    app_dir: self.new_app_dir.clone(),
                    db: None,
                };
                let added_id = project.id.clone();
                let needs_deps = project.project_type.is_proxied();
                match add_project(project) {
                    Ok(_) => {
                        self.error = None;
                        self.new_name.clear();
                        self.new_id.clear();
                        self.new_path.clear();
                        self.new_app_dir.clear();
                        self.new_start_command = "python app.py".to_string();
                        self.new_type = ProjectType::Php;
                        self.adding = false;
                        self.projects = list_projects();
                        // Python/Next.js면 의존성 자동 설치 시작
                        if needs_deps {
                            return Task::done(ProjectsMessage::SetupDeps(added_id));
                        }
                    }
                    Err(e) => self.set_error(e),
                }
                Task::none()
            }
            ProjectsMessage::RemoveProject(id) => {
                let _ = stop_server(&id);
                if self.editing_id.as_deref() == Some(&id) {
                    self.editing_id = None;
                }
                match remove_project(&id) {
                    Ok(_) => { self.error = None; self.projects = list_projects(); }
                    Err(e) => self.set_error(e),
                }
                Task::none()
            }
            ProjectsMessage::StartServer(id) => {
                let project = self.projects.iter().find(|p| p.id == id).cloned();
                if let Some(p) = project {
                    let pid = p.id.clone();
                    Task::perform(
                        async move { start_server(&p) },
                        move |r| ProjectsMessage::ServerStarted(pid.clone(), r),
                    )
                } else {
                    Task::none()
                }
            }
            ProjectsMessage::StopServer(id) => {
                let sid = id.clone();
                Task::perform(
                    async move { stop_server(&sid) },
                    move |r| ProjectsMessage::ServerStopped(id.clone(), r),
                )
            }
            ProjectsMessage::ServerStarted(id, result) => {
                if let Err(e) = &result {
                    self.toasts.push(Err(trf("{0} 시작 실패: {1}", &[&id, e])));
                }
                self.server_message = Some(result.map(|pid| trf("{0} 시작됨 (PID {1})", &[&id, &pid])));
                Task::none()
            }
            ProjectsMessage::ServerStopped(id, result) => {
                if let Err(e) = &result {
                    self.toasts.push(Err(trf("{0} 중지 실패: {1}", &[&id, e])));
                }
                self.server_message = Some(result.map(|_| trf("{0} 중지됨", &[&id])));
                Task::none()
            }
            // 인라인 편집
            ProjectsMessage::EditProject(id) => {
                if let Some(p) = self.projects.iter().find(|p| p.id == id) {
                    self.edit_name = p.name.clone();
                    self.edit_path = p.path.clone();
                    self.edit_start_command = p.start_command.clone();
                    self.edit_app_dir = p.app_dir.clone();
                    self.edit_type = p.project_type.clone();
                    match &p.db {
                        Some(db) => {
                            self.edit_db_name = db.name.clone();
                            self.edit_db_engine = db.engine;
                        }
                        None => {
                            self.edit_db_name.clear();
                            self.edit_db_engine = detect_db(p).map(|d| d.engine).unwrap_or(DbEngine::MariaDb);
                        }
                    }
                    self.editing_id = Some(id);
                    self.error = None;
                }
                Task::none()
            }
            ProjectsMessage::EditNameChanged(v) => { self.edit_name = v; Task::none() }
            ProjectsMessage::EditPathChanged(v) => { self.edit_path = v; Task::none() }
            ProjectsMessage::EditStartCommandChanged(v) => { self.edit_start_command = v; Task::none() }
            ProjectsMessage::EditAppDirChanged(v) => { self.edit_app_dir = v; Task::none() }
            ProjectsMessage::EditTypeSelected(t) => {
                // 타입이 바뀌었는데 명령어가 비었거나 이전 타입의 기본값이면 자동 감지로 교체
                if t != self.edit_type && t != ProjectType::Php {
                    let stale = self.edit_start_command.trim().is_empty()
                        || self.edit_start_command == "python app.py"
                        || self.edit_type == ProjectType::Php;
                    if stale {
                        // 정확한 포트는 저장 시 update_project 가 할당하므로, 자동 감지엔 임시값 사용
                        self.edit_start_command =
                            detect_command_for(&t, &self.edit_path, &self.edit_app_dir, 5001);
                    }
                }
                self.edit_type = t;
                Task::none()
            }
            ProjectsMessage::EditPickFolder(id) => {
                Task::perform(pick_folder(), move |p| ProjectsMessage::EditFolderSelected(id.clone(), p))
            }
            ProjectsMessage::EditFolderSelected(_, path) => {
                if let Some(p) = path {
                    // 신규 폼과 동일하게 Next.js 앱 위치를 자동 감지
                    if self.edit_type == ProjectType::Php || self.edit_type == ProjectType::NextJs {
                        if let Some(sub) = detect_next_app_dir(&p) {
                            self.edit_type = ProjectType::NextJs;
                            self.edit_app_dir = sub;
                        }
                    }
                    self.edit_path = p;
                }
                Task::none()
            }
            ProjectsMessage::SaveEdit(id) => {
                match update_project(
                    &id,
                    self.edit_name.clone(),
                    self.edit_path.clone(),
                    self.edit_start_command.clone(),
                    self.edit_type.clone(),
                    self.edit_app_dir.clone(),
                ) {
                    Ok(_) => {
                        let db = (!self.edit_db_name.trim().is_empty())
                            .then(|| ProjectDb { engine: self.edit_db_engine, name: self.edit_db_name.trim().to_string() });
                        self.editing_id = None;
                        self.error = None;
                        self.projects = list_projects();
                        if let Err(e) = set_project_db(&id, db) {
                            self.set_error(trf("{0} DB 연결 저장 실패: {1}", &[&id, &e]));
                        } else {
                            // vhost 는 바로 반영되지만, 이미 떠 있는 dev 서버는 옛 설정(명령어·포트)으로 돈다.
                            let msg = if matches!(server_status(&id), ServerStatus::Running(_)) {
                                trf("{0} 저장됨 — 실행 중인 서버는 중지 후 다시 시작해야 반영됩니다", &[&id])
                            } else {
                                trf("{0} 저장·반영 완료", &[&id])
                            };
                            self.toasts.push(Ok(msg));
                        }
                    }
                    Err(e) => self.set_error(trf("{0} 편집 저장 실패: {1}", &[&id, &e])),
                }
                Task::none()
            }
            ProjectsMessage::CancelEdit => {
                self.editing_id = None;
                self.error = None;
                Task::none()
            }
            ProjectsMessage::SetupDeps(id) => {
                let project = self.projects.iter().find(|p| p.id == id).cloned();
                if let Some(p) = project {
                    self.setting_up.insert(id.clone());
                    self.server_message = Some(Ok(trf("{0}: 패키지 설치 중...", &[&id])));
                    Task::perform(
                        async move { setup_project(&p) },
                        move |r| ProjectsMessage::DepsSetupDone(id.clone(), r),
                    )
                } else {
                    Task::none()
                }
            }
            ProjectsMessage::DepsSetupDone(id, result) => {
                self.setting_up.remove(&id);
                self.projects = list_projects();
                match result {
                    Ok(msg) => self.server_message = Some(Ok(format!("{id}: {msg}"))),
                    Err(e) => {
                        self.toasts.push(Err(format!("{id}: {e}")));
                        self.server_message = Some(Err(format!("{id}: {e}")));
                    }
                }
                Task::none()
            }
            ProjectsMessage::ViewLog(id) => {
                let name = self.projects.iter().find(|p| p.id == id)
                    .map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
                // 로그 패널을 즉시 열고 "불러오는 중" 표시
                self.log_view = Some(LogView {
                    id: id.clone(),
                    name,
                    content: String::new(),
                    error: None,
                });
                let path = error_log_path(&id);
                Task::perform(
                    async move { read_log(&path, 500) },
                    move |r| ProjectsMessage::LogLoaded(id.clone(), r),
                )
            }
            ProjectsMessage::LogLoaded(id, result) => {
                if let Some(lv) = self.log_view.as_mut() {
                    if lv.id == id {
                        match result {
                            Ok(content) => {
                                // 빈 내용은 그대로 두고, 표시는 log_panel에서 기본 폰트로 처리
                                lv.content = content;
                                lv.error = None;
                            }
                            Err(e) => {
                                lv.content = String::new();
                                lv.error = Some(e);
                            }
                        }
                    }
                }
                Task::none()
            }
            ProjectsMessage::CopyLog => {
                if let Some(lv) = self.log_view.as_ref() {
                    let content = lv.content.clone();
                    self.server_message = Some(Ok(tr("로그를 클립보드에 복사했습니다.").to_string()));
                    return iced::clipboard::write(content);
                }
                Task::none()
            }
            ProjectsMessage::ClearLog(id) => {
                let path = error_log_path(&id);
                Task::perform(
                    async move { clear_log(&path) },
                    move |r| ProjectsMessage::LogCleared(id.clone(), r),
                )
            }
            ProjectsMessage::LogCleared(id, result) => {
                match result {
                    Ok(_) => {
                        self.server_message = Some(Ok(tr("로그를 비웠습니다.").to_string()));
                        // 비운 뒤 다시 로드해 빈 상태 반영
                        return Task::done(ProjectsMessage::ViewLog(id));
                    }
                    Err(e) => {
                        self.server_message = Some(Err(e));
                    }
                }
                Task::none()
            }
            ProjectsMessage::CopyText(s) => {
                return iced::clipboard::write(s);
            }
            ProjectsMessage::CloseLog => {
                self.log_view = None;
                Task::none()
            }
            ProjectsMessage::OpenSiteTool(id, mode) => {
                self.menu_open = None;
                let Some(p) = self.projects.iter().find(|p| p.id == id).cloned() else { return Task::none() };
                let Some(kind) = site_config::detect(&p) else { return Task::none() };
                let acc = site_config::current_account(&p);
                let has_backup = std::path::Path::new(&p.path)
                    .join(format!("{}.{}", kind.config_file(), site_config::SERVER_BACKUP_EXT))
                    .exists();
                self.site_tool = Some(SiteTool {
                    project_id: id,
                    project_name: p.name.clone(),
                    kind,
                    mode,
                    admins: Vec::new(),
                    user_id: String::new(),
                    new_password: String::new(),
                    db_name: p.db.as_ref().map(|d| d.name.clone()).or_else(|| acc.as_ref().map(|a| a.database.clone())).unwrap_or_default(),
                    db_user: acc.as_ref().map(|a| a.user.clone()).unwrap_or_default(),
                    db_password: acc.map(|a| a.pass).unwrap_or_default(),
                    url: true,
                    has_backup,
                    running: false,
                    log: Vec::new(),
                    server: load_target(&p.id).filter(|t| t.has_db()),
                    on_server: false,
                    confirm_server: false,
                    show_pw: false,
                });
                if mode == SiteToolMode::Password {
                    return Task::done(ProjectsMessage::SiteTarget(false));
                }
                Task::none()
            }
            ProjectsMessage::SiteToggleShowPw => {
                if let Some(m) = self.site_tool.as_mut() {
                    m.show_pw = !m.show_pw;
                }
                Task::none()
            }
            ProjectsMessage::DeployToggleShowPw => {
                if let Some(d) = self.deploy.as_mut() {
                    d.show_pw = !d.show_pw;
                }
                Task::none()
            }
            ProjectsMessage::SiteServerCancel => {
                if let Some(m) = self.site_tool.as_mut() {
                    m.confirm_server = false;
                }
                Task::none()
            }
            ProjectsMessage::SiteTarget(server) => {
                let Some(m) = self.site_tool.as_mut() else { return Task::none() };
                let Some(p) = self.projects.iter().find(|p| p.id == m.project_id).cloned() else { return Task::none() };
                m.on_server = server && m.server.is_some();
                m.confirm_server = false;
                m.admins.clear();
                m.user_id.clear();
                m.log.clear();
                let t = if m.on_server { m.server.clone() } else { None };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let target = match &t {
                                Some(t) => site_config::Target::Server(t),
                                None => site_config::Target::Local,
                            };
                            site_config::list_admins(&p, target)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    ProjectsMessage::SiteAdminsLoaded,
                )
            }
            ProjectsMessage::SiteAdminsLoaded(r) => {
                if let Some(m) = self.site_tool.as_mut() {
                    match r {
                        Ok(list) => {
                            if m.user_id.is_empty() {
                                m.user_id = list.first().map(|(id, _)| id.clone()).unwrap_or_default();
                            }
                            m.admins = list;
                        }
                        Err(e) => m.log = vec![format!("✗ {e}")],
                    }
                }
                Task::none()
            }
            ProjectsMessage::SiteField(f, v) => {
                if let Some(m) = self.site_tool.as_mut() {
                    *match f {
                        SiteField::UserId => &mut m.user_id,
                        SiteField::NewPassword => &mut m.new_password,
                        SiteField::DbName => &mut m.db_name,
                        SiteField::DbUser => &mut m.db_user,
                        SiteField::DbPassword => &mut m.db_password,
                    } = v;
                }
                Task::none()
            }
            ProjectsMessage::SiteUrlToggled(on) => {
                if let Some(m) = self.site_tool.as_mut() {
                    m.url = on;
                }
                Task::none()
            }
            ProjectsMessage::SiteSetPassword if self.site_tool.as_ref().is_some_and(|m| m.on_server && !m.confirm_server) => {
                // 운영 서버 — 한 번 더 확인받는다
                if let Some(m) = self.site_tool.as_mut() {
                    m.confirm_server = !m.user_id.trim().is_empty() && !m.new_password.is_empty();
                    if !m.confirm_server {
                        m.log = vec![format!("✗ {}", tr("관리자 ID와 새 비밀번호를 입력하세요."))];
                    }
                }
                Task::none()
            }
            ProjectsMessage::SiteSetPassword
            | ProjectsMessage::SiteServerConfirm
            | ProjectsMessage::SiteLocalize
            | ProjectsMessage::SiteRestore => {
                let Some(m) = self.site_tool.as_mut() else { return Task::none() };
                m.confirm_server = false;
                let Some(p) = self.projects.iter().find(|p| p.id == m.project_id).cloned() else { return Task::none() };
                m.running = true;
                m.log.clear();
                let job: Box<dyn FnOnce() -> Result<Vec<String>, String> + Send> = match msg {
                    ProjectsMessage::SiteSetPassword | ProjectsMessage::SiteServerConfirm => {
                        let (login, pw) = (m.user_id.clone(), m.new_password.clone());
                        m.new_password.clear();
                        let server = if m.on_server { m.server.clone() } else { None };
                        Box::new(move || {
                            let target = match &server {
                                Some(t) => site_config::Target::Server(t),
                                None => site_config::Target::Local,
                            };
                            site_config::set_admin_password(&p, target, &login, &pw).map(|l| vec![format!("✓ {l}")])
                        })
                    }
                    ProjectsMessage::SiteLocalize => {
                        let s = site_config::LocalDbSettings {
                            database: m.db_name.clone(),
                            user: m.db_user.clone(),
                            password: m.db_password.clone(),
                            url: m.url,
                        };
                        Box::new(move || site_config::localize(&p, &s))
                    }
                    _ => Box::new(move || site_config::restore_server_config(&p).map(|l| vec![format!("✓ {l}")])),
                };
                Task::perform(
                    async move { tokio::task::spawn_blocking(job).await.unwrap_or_else(|e| Err(e.to_string())) },
                    ProjectsMessage::SiteToolDone,
                )
            }
            ProjectsMessage::SiteToolDone(r) => {
                if let Some(m) = self.site_tool.as_mut() {
                    m.running = false;
                    m.log = match r {
                        Ok(l) => l,
                        Err(e) => vec![format!("✗ {e}")],
                    };
                    if let Some(p) = self.projects.iter().find(|p| p.id == m.project_id) {
                        m.has_backup = std::path::Path::new(&p.path)
                            .join(format!("{}.{}", m.kind.config_file(), site_config::SERVER_BACKUP_EXT))
                            .exists();
                    }
                }
                Task::none()
            }
            ProjectsMessage::CloseSiteTool => {
                self.site_tool = None;
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, ProjectsMessage> {
        let add_btn = if self.adding {
            btn(tr("닫기"), Some(Icon::X), Kind::Surface).on_press(ProjectsMessage::ToggleAddForm)
        } else {
            btn(tr("새 프로젝트"), Some(Icon::Plus), Kind::Primary).on_press(ProjectsMessage::ToggleAddForm)
        };
        let mut col = column![
            page_header(tr("프로젝트"), tr("*.localhost 도메인으로 여는 가상호스트를 관리합니다"), Some(add_btn.into())),
            Space::with_height(20),
        ]
        .spacing(0);

        if self.adding {
            col = col.push(self.add_form()).push(Space::with_height(16));
        }
        if let Some(m) = &self.site_tool {
            col = col.push(site_tool_panel(m)).push(Space::with_height(16));
        }
        if let Some(lv) = &self.log_view {
            col = col.push(log_panel(lv)).push(Space::with_height(16));
        }
        if let Some(err) = &self.error {
            col = col.push(message_card(format!("✗ {err}"))).push(Space::with_height(12));
        }
        if let Some(msg) = &self.server_message {
            let line = match msg {
                Ok(m) => format!("✓ {m}"),
                Err(e) => format!("✗ {e}"),
            };
            col = col.push(message_card(line)).push(Space::with_height(12));
        }

        let editing_id = self.editing_id.as_deref();
        if self.projects.is_empty() {
            col = col.push(card(
                column![
                    icon(Icon::Folder, 22.0, p().fg4),
                    text(tr("등록된 프로젝트가 없습니다")).size(14).font(theme::MEDIUM).color(p().fg2),
                    muted(tr("오른쪽 위 [새 프로젝트]로 폴더를 등록하면 id.localhost 로 열립니다")),
                ]
                .spacing(6)
                .align_x(iced::Alignment::Center)
                .width(Length::Fill),
            ));
            return col.into();
        }

        col = col.push(self.overview()).push(Space::with_height(12));

        let q = self.search.trim().to_lowercase();
        let shown: Vec<&VhostProject> = self
            .projects
            .iter()
            .filter(|p| q.is_empty() || [&p.domain, &p.name, &p.id].iter().any(|s| s.to_lowercase().contains(&q)))
            .collect();
        if shown.is_empty() {
            col = col.push(card(muted(trf("'{0}'에 맞는 사이트가 없습니다", &[&self.search.trim()]))));
        }
        let mut list = column![].spacing(10);
        for p_ in shown {
            if editing_id == Some(p_.id.as_str()) {
                list = list.push(project_row_editing(
                    p_,
                    &self.edit_name,
                    &self.edit_path,
                    &self.edit_start_command,
                    &self.edit_app_dir,
                    &self.edit_type,
                    &self.edit_db_name,
                    self.edit_db_engine,
                ));
                continue;
            }
            let transfer = self.transfers.get(&p_.id);
            let menu = (self.menu_open.as_deref() == Some(p_.id.as_str())).then(|| transfer.map(|t| t.1.as_slice()).unwrap_or(&[]));
            list = list.push(project_row_view_with_state(
                p_,
                editing_id.is_some(),
                self.setting_up.contains(&p_.id),
                transfer.map(|t| t.0.as_str()),
                menu,
                self.usage.get(&p_.id),
            ));
            if let Some(d) = self.deploy.as_ref().filter(|d| d.id == p_.id) {
                list = list.push(deploy_panel(d));
            }
        }
        col = col.push(list);
        col.into()
    }

    /// 검색창 + 전체 용량 요약
    fn overview(&self) -> Element<'_, ProjectsMessage> {
        let (files, deps, dbs) = self.usage.values().fold((0u64, 0u64, 0u64), |(f, d, b), u| {
            (f + u.files, d + u.deps, b + u.db_bytes.unwrap_or(0))
        });
        let stat = |label: &'static str, value: String| -> Element<'_, ProjectsMessage> {
            column![muted(label), text(value).size(20).font(theme::BOLD).color(p().fg)].spacing(2).into()
        };
        let summary: Element<ProjectsMessage> = if self.usage.is_empty() {
            muted(if self.usage_loading { tr("용량을 재는 중…") } else { tr("[용량 계산]을 누르면 사이트별 파일·DB 크기를 잽니다") }).into()
        } else {
            row![
                stat(tr("사이트"), trf("{0}개", &[&self.projects.len()])),
                stat(tr("파일"), human_bytes(files)),
                stat(tr("그중 의존성"), human_bytes(deps)),
                stat("DB", human_bytes(dbs)),
                stat(tr("합계"), human_bytes(files + dbs)),
            ]
            .spacing(28)
            .into()
        };
        let calc = btn(if self.usage_loading { tr("재는 중…") } else { tr("용량 계산") }, Some(Icon::HardDrive), Kind::Flat);
        card(
            column![
                row![
                    input(tr("도메인·이름으로 찾기"), &self.search).on_input(ProjectsMessage::SearchChanged).width(Length::Fill),
                    if self.usage_loading { calc } else { calc.on_press(ProjectsMessage::ComputeUsage) },
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
                Space::with_height(14),
                summary,
            ],
        )
    }

    fn add_form(&self) -> Element<'_, ProjectsMessage> {
        let needs_server = self.new_type.is_proxied();
        let mut form = column![
            row![
                text(tr("새 프로젝트")).size(15).font(theme::SEMIBOLD).color(p().fg),
                Space::with_width(Length::Fill),
                type_picker(&self.new_type, ProjectsMessage::TypeSelected),
            ]
            .align_y(iced::Alignment::Center),
            Space::with_height(14),
            row![
                field(tr("프로젝트 이름"), input("My Project", &self.new_name).on_input(ProjectsMessage::NameChanged).into(), None),
                field(tr("ID (도메인)"), input("id → id.localhost", &self.new_id).on_input(ProjectsMessage::IdChanged).into(), None),
            ]
            .spacing(12),
            Space::with_height(12),
            field(
                tr("프로젝트 경로"),
                row![
                    input("/home/user/projects/...", &self.new_path)
                        .on_input(|v| ProjectsMessage::PathSelected(Some(v)))
                        .width(Length::Fill),
                    btn(tr("찾아보기"), Some(Icon::FolderOpen), Kind::Flat).on_press(ProjectsMessage::OpenFilePicker),
                ]
                .spacing(8)
                .into(),
                None,
            ),
            Space::with_height(12),
            field(
                tr("하위 디렉토리 (선택)"),
                input(tr("예: app — 비우면 프로젝트 경로를 그대로 사용"), &self.new_app_dir)
                    .on_input(ProjectsMessage::AppDirChanged)
                    .into(),
                Some(if self.new_type == ProjectType::Php {
                    tr("PHP: DocumentRoot로 사용됩니다 (예: public)")
                } else {
                    tr("앱이 하위 폴더에 있을 때 (예: easyhost/app). 폴더를 고르면 자동으로 찾습니다")
                }),
            ),
        ]
        .spacing(0);

        if needs_server {
            let placeholder = if self.new_type == ProjectType::NextJs { "npx next dev --port 5001" } else { "python3 app.py" };
            form = form.push(Space::with_height(12)).push(field(
                tr("실행 명령어"),
                input(placeholder, &self.new_start_command).on_input(ProjectsMessage::StartCommandChanged).into(),
                Some(if self.new_type == ProjectType::NextJs {
                    tr("포트는 5001번부터 자동으로 정해지고 --port 값도 거기에 맞춰집니다")
                } else {
                    tr("포트는 5001번부터 자동으로 정해집니다")
                }),
            ));
        }

        form = form.push(Space::with_height(16)).push(
            row![
                btn(tr("추가"), Some(Icon::Plus), Kind::Primary).on_press(ProjectsMessage::AddProject),
                btn(tr("취소"), None, Kind::Ghost).on_press(ProjectsMessage::ToggleAddForm),
            ]
            .spacing(8),
        );
        card(form)
    }
}

/// 라벨 + 입력 + 도움말
fn field<'a>(label: &'a str, control: Element<'a, ProjectsMessage>, help: Option<&'a str>) -> Element<'a, ProjectsMessage> {
    let mut c = column![text(label).size(12).font(theme::MEDIUM).color(p().fg3), control].spacing(6);
    if let Some(h) = help {
        c = c.push(text(h).size(11).color(p().fg4));
    }
    c.width(Length::Fill).into()
}

fn type_picker<'a>(selected: &ProjectType, on: impl Fn(ProjectType) -> ProjectsMessage + 'a) -> Element<'a, ProjectsMessage> {
    segmented(
        &[(ProjectType::Php, "PHP"), (ProjectType::Python, "Python"), (ProjectType::NextJs, "Next.js")],
        selected,
        on,
    )
}

fn type_chip<'a>(t: &ProjectType) -> Element<'a, ProjectsMessage> {
    match t {
        ProjectType::Php => chip("PHP", Tone::Primary),
        ProjectType::Python => chip("Python", Tone::Success),
        ProjectType::NextJs => chip("Next.js", Tone::Neutral),
    }
}

fn message_card(line: String) -> Element<'static, ProjectsMessage> {
    let copy_text = line.trim_start_matches(['✓', '✗', ' ']).to_string();
    card(
        row![
            container(result_line(line)).width(Length::Fill),
            btn(tr("복사"), Some(Icon::Copy), Kind::Ghost).on_press(ProjectsMessage::CopyText(copy_text)),
        ]
        .align_y(iced::Alignment::Center),
    )
}

/// last_transfer: 마지막 이전 한 줄. menu: 더보기가 열려 있으면 Some(최근 이전 기록)
fn project_row_view_with_state<'a>(
    p_: &'a VhostProject,
    any_editing: bool,
    setting_up: bool,
    last_transfer: Option<&'a str>,
    menu: Option<&'a [(bool, String)]>,
    usage: Option<&'a SiteUsage>,
) -> Element<'a, ProjectsMessage> {
    let c = p();
    let id = p_.id.clone();

    // 왼쪽: 이름·타입, 도메인(누르면 열림), 경로·명령, 마지막 이전
    let mut info = column![
        row![text(&p_.name).size(15).font(theme::SEMIBOLD).color(c.fg), type_chip(&p_.project_type)]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        iced::widget::button(
            row![icon(Icon::Globe, 12.0, c.primary_fg), text(&p_.domain).size(13).color(c.primary_fg)]
                .spacing(5)
                .align_y(iced::Alignment::Center),
        )
        .padding(0)
        .on_press(ProjectsMessage::OpenSite(p_.domain.clone()))
        .style(|_, _| iced::widget::button::Style::default()),
        // 경로 — 누르면 Finder·파일 관리자로 연다
        iced::widget::button(
            row![icon(Icon::FolderOpen, 12.0, c.fg3), text(&p_.path).size(12).color(c.fg3)]
                .spacing(5)
                .align_y(iced::Alignment::Center),
        )
        .padding(0)
        .on_press(ProjectsMessage::OpenFolder(p_.path.clone()))
        .style(|_, _| iced::widget::button::Style::default()),
    ]
    .spacing(4)
    .width(Length::Fill);
    if p_.project_type != ProjectType::Php {
        info = info.push(
            text(if p_.app_dir.is_empty() {
                format!(":{} · {}", p_.port, p_.start_command)
            } else {
                format!(":{} · {}/ · {}", p_.port, p_.app_dir, p_.start_command)
            })
            .size(12)
            .color(c.fg4),
        );
    }
    if let Some(u) = usage {
        let db = match (&u.db, u.db_bytes) {
            (Some(d), Some(b)) => format!(" · DB {} {}", d.name, human_bytes(b)),
            (Some(d), None) => format!(" · {}", trf("DB {0} (크기 못 읽음)", &[&d.name])),
            (None, _) => format!(" · {}", tr("DB 없음")),
        };
        info = info.push(
            row![
                icon(Icon::HardDrive, 11.0, c.fg3),
                text(trf("파일 {0} (의존성 {1}){2} · 합계 {3}",
                    &[&human_bytes(u.files), &human_bytes(u.deps), &db, &human_bytes(u.total())],
                ))
                .size(11)
                .color(c.fg3),
            ]
            .spacing(5)
            .align_y(iced::Alignment::Center),
        );
    }
    if let Some(t) = last_transfer {
        info = info.push(
            row![icon(Icon::History, 11.0, c.fg3), text(t).size(11).color(c.fg3)]
                .spacing(5)
                .align_y(iced::Alignment::Center),
        );
    }

    // 오른쪽: 상태와 동작
    let mut actions = row![].spacing(6).align_y(iced::Alignment::Center);
    if p_.project_type.is_proxied() {
        if setting_up {
            actions = actions.push(status(tr("패키지 설치 중…"), Tone::Warning));
        } else {
            let st = server_status(&p_.id);
            let running = matches!(st, ServerStatus::Running(_));
            let label = match st {
                ServerStatus::Running(pid) => trf("실행 중 · PID {0}", &[&pid]),
                ServerStatus::Stopped => tr("중지됨").to_string(),
            };
            actions = actions.push(status(label, if running { Tone::Success } else { Tone::Neutral }));
            if !deps_ready(p_) {
                actions = actions.push(btn(tr("패키지 설치"), Some(Icon::Package), Kind::Flat).on_press(ProjectsMessage::SetupDeps(id.clone())));
            }
            actions = actions.push(if running {
                btn(tr("중지"), Some(Icon::Square), Kind::Danger).on_press(ProjectsMessage::StopServer(id.clone()))
            } else {
                btn(tr("시작"), Some(Icon::Play), Kind::Success).on_press(ProjectsMessage::StartServer(id.clone()))
            });
        }
    } else {
        let apache_running = super::services::apache_running();
        actions = actions.push(status(
            if apache_running { tr("Apache 실행 중") } else { tr("Apache 중지됨") },
            if apache_running { Tone::Success } else { Tone::Neutral },
        ));
    }
    actions = actions
        .push(btn(tr("로그"), Some(Icon::FileText), Kind::Flat).on_press(ProjectsMessage::ViewLog(id.clone())))
        .push({
            let b = btn(tr("수정"), Some(Icon::Pencil), Kind::Flat);
            if any_editing { b } else { b.on_press(ProjectsMessage::EditProject(id.clone())) }
        })
        .push(btn(tr("삭제"), Some(Icon::Trash), Kind::Danger).on_press(ProjectsMessage::RemoveProject(id.clone())))
        .push(icon_btn(Icon::Ellipsis, menu.is_some()).on_press(ProjectsMessage::ToggleMenu(id.clone())));

    let main = row![info, actions].spacing(16).align_y(iced::Alignment::Center);
    card(with_menu(main.into(), menu, id, p_.path.clone(), site_config::detect(p_)))
}

/// 더보기 메뉴가 열려 있으면 카드 아래에 동작과 최근 이전 기록을 붙인다.
fn with_menu<'a>(
    main: Element<'a, ProjectsMessage>,
    menu: Option<&'a [(bool, String)]>,
    id: String,
    path: String,
    site: Option<SiteKind>,
) -> Element<'a, ProjectsMessage> {
    let Some(recent) = menu else {
        return main;
    };
    let mut history = column![text(tr("이전 기록")).size(12).font(theme::SEMIBOLD).color(p().fg3)].spacing(4);
    if recent.is_empty() {
        history = history.push(muted(tr("아직 다른 PC와 주고받은 적이 없습니다.")));
    }
    // 성공 여부는 표시 문구(번역될 수 있음)가 아니라 기록의 ok 값으로 판단한다
    for (ok, l) in recent {
        let tone = if *ok { Tone::Success } else { Tone::Warning };
        history = history.push(
            row![theme::dot(tone), text(l).size(12).color(p().fg2)].spacing(8).align_y(iced::Alignment::Center),
        );
    }
    column![
        main,
        Space::with_height(14),
        theme::divider(),
        Space::with_height(14),
        row![
            column![
                btn(tr("다른 PC로 보내기"), Some(Icon::Send), Kind::Primary).on_press(ProjectsMessage::SendToPc(id.clone())),
                btn(tr("서버 배포·가져오기"), Some(Icon::Upload), Kind::Flat).on_press(ProjectsMessage::OpenDeploy(id.clone())),
                btn(tr("폴더 열기"), Some(Icon::FolderOpen), Kind::Flat).on_press(ProjectsMessage::OpenFolder(path)),
            ]
            .push_maybe(site.map(|_| {
                btn(tr("관리자 비밀번호"), Some(Icon::Key), Kind::Flat).on_press(ProjectsMessage::OpenSiteTool(id.clone(), SiteToolMode::Password))
            }))
            .push_maybe(site.map(|_| {
                btn(tr("로컬 DB로 설정 바꾸기"), Some(Icon::Database), Kind::Flat).on_press(ProjectsMessage::OpenSiteTool(id.clone(), SiteToolMode::LocalDb))
            }))
            .spacing(6),
            Space::with_width(24),
            history.width(Length::Fill),
        ]
        .align_y(iced::Alignment::Start),
    ]
    .into()
}

// 편집 모드 카드
fn project_row_editing<'a>(
    p_: &'a VhostProject,
    edit_name: &'a str,
    edit_path: &'a str,
    edit_start_cmd: &'a str,
    edit_app_dir: &'a str,
    edit_type: &'a ProjectType,
    edit_db_name: &'a str,
    edit_db_engine: DbEngine,
) -> Element<'a, ProjectsMessage> {
    let needs_server = edit_type.is_proxied();
    let detected = detect_db(p_).map(|d| d.name);
    let type_changed = *edit_type != p_.project_type;

    let mut form = column![
        row![
            text(&p_.domain).size(15).font(theme::SEMIBOLD).color(p().fg),
            chip(tr("편집 중"), Tone::Warning),
            Space::with_width(Length::Fill),
            type_picker(edit_type, ProjectsMessage::EditTypeSelected),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        Space::with_height(14),
        field(tr("이름"), input(tr("프로젝트 이름"), edit_name).on_input(ProjectsMessage::EditNameChanged).into(), None),
        Space::with_height(12),
        field(
            tr("경로"),
            row![
                input("/home/...", edit_path).on_input(ProjectsMessage::EditPathChanged).width(Length::Fill),
                btn(tr("찾아보기"), Some(Icon::FolderOpen), Kind::Flat).on_press(ProjectsMessage::EditPickFolder(p_.id.clone())),
            ]
            .spacing(8)
            .into(),
            None,
        ),
        Space::with_height(12),
        field(
            tr("하위 디렉토리 (선택)"),
            input(tr("예: app — 비우면 경로를 그대로 사용"), edit_app_dir).on_input(ProjectsMessage::EditAppDirChanged).into(),
            None,
        ),
        Space::with_height(12),
        field(
            tr("DB (용량 표시·서버 배포에 씀)"),
            row![
                segmented(
                    &[(DbEngine::MariaDb, "MariaDB·MySQL"), (DbEngine::PostgreSql, "PostgreSQL")],
                    &edit_db_engine,
                    ProjectsMessage::EditDbEngineSelected,
                ),
                input(
                    &match &detected {
                        Some(n) => trf("비우면 자동 감지 ({0})", &[n]),
                        None => tr("DB 이름 — 비우면 설정 파일에서 자동 감지").to_string(),
                    },
                    edit_db_name,
                )
                .on_input(ProjectsMessage::EditDbNameChanged)
                .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into(),
            None,
        ),
    ]
    .spacing(0);

    if type_changed {
        form = form.push(Space::with_height(10)).push(chip(
            if needs_server {
                tr("저장하면 새 포트를 받고 vhost가 프록시 형태로 다시 만들어집니다")
            } else {
                tr("저장하면 포트가 80으로 바뀌고 vhost가 DocumentRoot 형태로 다시 만들어집니다")
            },
            Tone::Warning,
        ));
    }
    if needs_server {
        let placeholder = if *edit_type == ProjectType::NextJs { "npx next dev --port 5001" } else { "python3 run_server.py 5001" };
        form = form.push(Space::with_height(12)).push(field(
            tr("실행 명령어"),
            input(placeholder, edit_start_cmd).on_input(ProjectsMessage::EditStartCommandChanged).into(),
            None,
        ));
    }
    form = form.push(Space::with_height(16)).push(
        row![
            btn(tr("저장"), Some(Icon::Check), Kind::Primary).on_press(ProjectsMessage::SaveEdit(p_.id.clone())),
            btn(tr("취소"), None, Kind::Ghost).on_press(ProjectsMessage::CancelEdit),
        ]
        .spacing(8),
    );
    card(form)
}

/// 서버 배포 패널 (프로젝트 바로 아래에 열린다)
fn deploy_panel(d: &DeployForm) -> Element<'_, ProjectsMessage> {
    let inp = |ph: &str, value: &str, which: DeployField| input(ph, value).on_input(move |v| ProjectsMessage::DeployField(which, v));

    let busy = d.busy.is_some();
    let action = |label: &'static str, ic: Icon, kind: Kind, msg: ProjectsMessage| {
        let b = btn(label, Some(ic), kind);
        if busy { b } else { b.on_press(msg) }
    };

    let server = column![
        theme::section_label(tr("서버 (SSH·SFTP)")),
        row![
            field(tr("주소"), inp("example.com", &d.host, DeployField::Host).into(), None),
            container(field(tr("포트"), inp("22", &d.port, DeployField::Port).into(), None)).width(90),
            container(field(tr("사용자"), inp("deploy", &d.user, DeployField::User).into(), None)).width(160),
        ]
        .spacing(10),
        Space::with_height(8),
        row![
            field(tr("웹 경로"), inp("/var/www/site", &d.path, DeployField::Path).into(), None),
            field(
                tr("비밀번호 (선택)"),
                theme::password_field(inp(tr("비우면 SSH 키로 로그인"), &d.ssh_password, DeployField::SshPassword), d.show_pw, ProjectsMessage::DeployToggleShowPw),
                None,
            ),
            field(tr("SSH 키 (선택)"), inp(tr("비우면 기본 키 (~/.ssh)"), &d.key, DeployField::Key).into(), None),
        ]
        .spacing(10),
    ];

    let db = column![
        row![
            theme::section_label(tr("서버 DB")),
            Space::with_width(Length::Fill),
            theme::check(tr("SSH로 서버에 들어가서 접속 (호스팅 DB가 외부 접속을 막을 때)"), d.db_via_ssh)
                .on_toggle(ProjectsMessage::DeployDbViaSsh),
        ]
        .align_y(iced::Alignment::Center),
        row![
            field(
                tr("호스트"),
                inp(if d.db_via_ssh { "localhost" } else { "db.example.com" }, &d.db_host, DeployField::DbHost).into(),
                None,
            ),
            container(field(tr("포트"), inp("3306", &d.db_port, DeployField::DbPort).into(), None)).width(90),
            field(tr("DB 이름"), inp("site", &d.db_name, DeployField::DbName).into(), None),
        ]
        .spacing(10),
        Space::with_height(8),
        row![
            field(tr("사용자"), inp("site_user", &d.db_user, DeployField::DbUser).into(), None),
            field(tr("비밀번호"), theme::password_field(inp("", &d.db_password, DeployField::DbPassword), d.show_pw, ProjectsMessage::DeployToggleShowPw), None),
        ]
        .spacing(10),
    ];

    let common = row![
        action(tr("저장"), Icon::Check, Kind::Flat, ProjectsMessage::DeploySave),
        action(tr("연결 시험"), Icon::Zap, Kind::Flat, ProjectsMessage::DeployTest),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    // 서버에서 가져오기 (↓)
    let pull = theme::inset(
        column![
            row![icon(Icon::Download, 14.0, p().fg2), text(tr("서버에서 가져오기")).size(14).font(theme::SEMIBOLD).color(p().fg)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            muted(tr("서버 웹 경로의 파일을 이 프로젝트 폴더로 받고, 서버 DB를 이 PC의 DB에 넣습니다. 로컬에만 있는 파일은 지우지 않고, 로컬 설정 파일(files/config, .env, wp-config.php)이 있으면 덮지 않습니다.")),
            Space::with_height(4),
            row![
                field(
                    tr("받지 않을 것 (쉼표로 구분)"),
                    inp(".git/, files/cache/", &d.pull_excludes, DeployField::PullExcludes).into(),
                    None,
                ),
                container(field(tr("넣을 로컬 DB"), inp("site", &d.local_db, DeployField::LocalDb).into(), None)).width(180),
            ]
            .spacing(10),
            Space::with_height(4),
            row![
                action(tr("받을 파일 보기"), Icon::Search, Kind::Flat, ProjectsMessage::PullPreview),
                action(tr("파일 가져오기"), Icon::Download, Kind::Flat, ProjectsMessage::PullFiles),
                action(tr("DB 가져오기"), Icon::Database, Kind::Flat, ProjectsMessage::PullDbAsk(false)),
                action(tr("모두 가져오기"), Icon::Download, Kind::Primary, ProjectsMessage::PullDbAsk(true)),
            ]
            .spacing(6),
        ]
        .push_maybe(d.confirm_pull.map(|all| pull_confirm(d, all)))
        .push_maybe(if d.log_at == 1 { status_view(d) } else { None })
        .spacing(6),
    );

    // 서버로 올리기 (↑)
    let push = theme::inset(
        column![
            row![icon(Icon::Upload, 14.0, p().fg2), text(tr("서버로 올리기")).size(14).font(theme::SEMIBOLD).color(p().fg)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            field(
                tr("올리지 않을 것 (쉼표로 구분)"),
                inp(".git/, node_modules/, .env", &d.excludes, DeployField::Excludes).into(),
                Some(tr("서버에만 있는 파일은 지우지 않습니다. 서버의 .env·설정 파일이 덮이지 않게 여기에 넣으세요")),
            ),
            Space::with_height(4),
            row![
                action(tr("미리보기"), Icon::Search, Kind::Flat, ProjectsMessage::DeployPreview),
                action(tr("파일 올리기"), Icon::Upload, Kind::Primary, ProjectsMessage::DeployUpload),
                action(tr("DB 올리기"), Icon::Database, Kind::Danger, ProjectsMessage::DeployDbAsk),
            ]
            .spacing(6),
        ]
        .push_maybe(if d.log_at == 2 { status_view(d) } else { None })
        .spacing(6),
    );

    let mut body = column![
        row![
            icon(Icon::ArrowLeftRight, 15.0, p().fg2),
            text(trf("서버 연결 · {0}", &[&d.id])).size(15).font(theme::SEMIBOLD).color(p().fg),
            Space::with_width(Length::Fill),
            btn(tr("닫기"), Some(Icon::X), Kind::Ghost).on_press(ProjectsMessage::CloseDeploy),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        Space::with_height(8),
        server,
        Space::with_height(12),
        db,
        Space::with_height(12),
        common,
    ];
    if d.log_at == 0 {
        body = body.push_maybe(status_view(d));
    }
    body = body.push(column![
        Space::with_height(12),
        pull,
        Space::with_height(8),
        push,
    ]);
    // 지난 작업 기록
    if !d.runs.is_empty() {
        let mut list = column![theme::section_label(tr("최근 기록"))].spacing(4);
        for r in d.runs.iter().take(8) {
            list = list.push(
                row![
                    theme::dot(if r.ok { Tone::Success } else { Tone::Danger }),
                    text(format!("{} · {}", run_log_time(r.at), r.title)).size(12).color(p().fg2).width(Length::Fill),
                    btn(tr("보기"), None, Kind::Ghost).on_press(ProjectsMessage::ViewRunLog(r.path.clone())),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        body = body.push(Space::with_height(12)).push(list);
        if d.log_at == 3 {
            body = body.push_maybe(status_view(d));
        }
    }

    if d.confirm_db {
        body = body.push(Space::with_height(12)).push(theme::inset(
            column![
                row![icon(Icon::Database, 14.0, p().danger_fg), text(tr("운영 DB를 덮어씁니다")).size(14).font(theme::SEMIBOLD).color(p().danger_fg)]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                muted(trf("{0}@{1} 의 '{2}' 내용을 이 PC의 DB로 바꿉니다. 넣기 직전에 원격 DB를 이 PC에 백업해 두지만, 그 사이 서버에 쌓인 데이터는 사라집니다.",
                    &[&d.db_user, &d.db_host, &d.db_name],
                )),
                Space::with_height(6),
                row![
                    btn(tr("덮어쓰기"), Some(Icon::Database), Kind::Danger).on_press(ProjectsMessage::DeployDbConfirm),
                    btn(tr("취소"), None, Kind::Ghost).on_press(ProjectsMessage::DeployDbCancel),
                ]
                .spacing(6),
            ]
            .spacing(4),
        ));
    }
    card(body)
}

/// 진행 상황(막대)과 결과 로그 — 마지막으로 누른 버튼이 있는 구역 안에 보여 준다
fn status_view(d: &DeployForm) -> Option<Element<'_, ProjectsMessage>> {
    let mut c = column![].spacing(6);
    let mut any = false;
    let current = match (d.busy, &d.progress) {
        (_, Some((l, r))) => Some((l.clone(), *r)),
        (Some(busy), None) => Some((busy.to_string(), None)),
        (None, None) => None,
    };
    if let Some((label, ratio)) = current {
        any = true;
        match ratio {
            Some((done, total)) => {
                let pct = (done as f64 / total.max(1) as f64 * 100.0).min(100.0);
                c = c.push(theme::progress(total as f32, done as f32, Tone::Primary)).push(muted(format!("{pct:.0}% · {label}")));
            }
            None => c = c.push(status(label, Tone::Primary)),
        }
    }
    if !d.log.is_empty() {
        any = true;
        // 줄이 많으면(받은 파일 목록) 앞부분만 보이고, 전체는 기록 파일에
        const SHOW: usize = 60;
        let mut shown: Vec<String> = d.log.iter().take(SHOW).cloned().collect();
        if d.log.len() > SHOW {
            shown.push(format!("· {}", trf("… {0}줄 더 — 전체는 기록 파일에", &[&(d.log.len() - SHOW)])));
        }
        c = c.push(theme::log_block_full(shown, d.log.join("\n"), ProjectsMessage::CopyText));
        if let Some(f) = &d.log_file {
            c = c.push(btn(tr("기록 파일 열기"), Some(Icon::FileText), Kind::Ghost).on_press(ProjectsMessage::OpenRunLog(f.clone())));
        }
    }
    any.then(|| column![Space::with_height(6), c].into())
}

/// "서버에서 가져오기" 확인 — 가져오기 버튼 바로 아래에 띄운다
fn pull_confirm(d: &DeployForm, all: bool) -> Element<'_, ProjectsMessage> {
    let what = if all { tr("파일과 DB를 서버 것으로 받습니다") } else { tr("DB를 서버 것으로 받습니다") };
    column![
        Space::with_height(6),
        row![icon(Icon::Download, 14.0, p().primary_fg), text(what).size(14).font(theme::SEMIBOLD).color(p().fg)]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        muted(match d.local_db_exists {
            Some(false) => trf("이 PC에 DB '{0}'가 없습니다 — 새로 만든 뒤 서버 '{1}'를 받아 넣습니다.", &[&d.local_db, &d.db_name]),
            _ => trf("이 PC의 DB '{0}'를 서버 '{1}'와 똑같이 바꿉니다. 바꾸기 전에 로컬 DB를 백업해 둡니다.", &[&d.local_db, &d.db_name]),
        }),
        row![
            btn(
                if d.local_db_exists == Some(false) { tr("DB 만들고 가져오기") } else { tr("가져오기") },
                Some(Icon::Download),
                Kind::Primary,
            )
            .on_press(ProjectsMessage::PullConfirm),
            btn(tr("취소"), None, Kind::Ghost).on_press(ProjectsMessage::DeployDbCancel),
        ]
        .spacing(6),
    ]
    .spacing(6)
    .into()
}

// 사이트 손보기 패널 — 관리자 비밀번호 / 로컬 DB 로 설정 바꾸기
fn site_tool_panel(m: &SiteTool) -> Element<'_, ProjectsMessage> {
    let inp = |ph: &str, value: &str, f: SiteField| input(ph, value).on_input(move |v| ProjectsMessage::SiteField(f, v));
    let title = match m.mode {
        SiteToolMode::Password => trf("{0} 관리자 비밀번호 · {1}", &[&m.kind.label(), &m.project_name]),
        SiteToolMode::LocalDb => trf("로컬 DB로 설정 바꾸기 · {0}", &[&m.project_name]),
    };
    let header = row![
        icon(if m.mode == SiteToolMode::Password { Icon::Key } else { Icon::Database }, 15.0, p().fg2),
        text(title).size(15).font(theme::SEMIBOLD).color(p().fg),
        Space::with_width(Length::Fill),
        btn(tr("닫기"), Some(Icon::X), Kind::Ghost).on_press(ProjectsMessage::CloseSiteTool),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    let run = |label: &'static str, ic: Icon, kind: Kind, msg: ProjectsMessage| {
        let b = btn(if m.running { tr("처리 중…") } else { label }, Some(ic), kind);
        if m.running { b } else { b.on_press(msg) }
    };

    let mut body = column![header, Space::with_height(6)];
    match m.mode {
        SiteToolMode::Password => {
            if m.server.is_some() {
                body = body.push(
                    row![
                        text(tr("대상")).size(12).color(p().fg3),
                        theme::segmented(&[(false, tr("이 PC")), (true, tr("서버 (운영)"))], &m.on_server, ProjectsMessage::SiteTarget),
                    ]
                    .spacing(10)
                    .align_y(iced::Alignment::Center),
                );
                body = body.push(Space::with_height(6));
            }
            body = body.push(muted(if m.on_server {
                tr("서버 연결에 저장된 DB 정보로 운영 사이트의 관리자 비밀번호를 바꿉니다. 바꾸기 전에 한 번 더 확인합니다.")
            } else {
                tr("사이트 DB의 관리자 계정 비밀번호를 바로 바꿉니다 (라이믹스·XE는 bcrypt, 워드프레스는 다음 로그인 때 자동으로 강한 방식으로 바뀝니다).")
            }));
            if !m.admins.is_empty() {
                let list = m.admins.iter().map(|(id, mail)| format!("{id} ({mail})")).collect::<Vec<_>>().join(", ");
                body = body.push(Space::with_height(4)).push(muted(trf("관리자: {0}", &[&list])));
            }
            body = body.push(Space::with_height(12)).push(
                row![
                    container(field(tr("관리자 ID"), inp("admin", &m.user_id, SiteField::UserId).into(), None)).width(200),
                    field(tr("새 비밀번호"), theme::password_field(inp(tr("새 비밀번호"), &m.new_password, SiteField::NewPassword), m.show_pw, ProjectsMessage::SiteToggleShowPw), None),
                    column![
                        Space::with_height(20),
                        run(tr("변경"), Icon::Check, if m.on_server { Kind::Danger } else { Kind::Primary }, ProjectsMessage::SiteSetPassword)
                    ],
                ]
                .spacing(12),
            );
            if m.confirm_server {
                let host = m.server.as_ref().map(|t| t.ssh_host.clone()).unwrap_or_default();
                body = body.push(Space::with_height(10)).push(theme::inset(
                    column![
                        row![icon(Icon::Key, 14.0, p().danger_fg), text(tr("운영 사이트의 관리자 비밀번호를 바꿉니다")).size(14).font(theme::SEMIBOLD).color(p().danger_fg)]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                        muted(trf("{0}의 '{1}' 계정 비밀번호가 바로 바뀝니다. 다른 관리자에게 알려 주세요.", &[&host, &m.user_id])),
                        row![
                            btn(tr("서버 비밀번호 바꾸기"), Some(Icon::Check), Kind::Danger).on_press(ProjectsMessage::SiteServerConfirm),
                            btn(tr("취소"), None, Kind::Ghost).on_press(ProjectsMessage::SiteServerCancel),
                        ]
                        .spacing(6),
                    ]
                    .spacing(6),
                ));
            }
        }
        SiteToolMode::LocalDb => {
            body = body.push(muted(trf(
                "{0}의 DB 접속을 이 PC(localhost) 것으로 바꾸고, 그 계정을 이 PC의 DB에 만듭니다. 처음 바꿀 때 서버 원본을 .localman-server로 보관하고, 서버로 올릴 때는 이 설정 파일을 뺍니다.",
                &[&m.kind.config_file()],
            )));
            body = body.push(Space::with_height(12)).push(
                row![
                    field(tr("DB 이름"), inp("site", &m.db_name, SiteField::DbName).into(), None),
                    field(tr("사용자"), inp("site_user", &m.db_user, SiteField::DbUser).into(), None),
                    field(tr("비밀번호"), theme::password_field(inp("", &m.db_password, SiteField::DbPassword), m.show_pw, ProjectsMessage::SiteToggleShowPw), None),
                ]
                .spacing(10),
            );
            body = body.push(Space::with_height(8)).push(
                theme::check(tr("사이트 주소도 로컬 도메인으로 (설정 파일·DB)"), m.url).on_toggle(ProjectsMessage::SiteUrlToggled),
            );
            let mut actions = row![run(tr("로컬로 바꾸기"), Icon::Check, Kind::Primary, ProjectsMessage::SiteLocalize)].spacing(6);
            if m.has_backup {
                actions = actions.push(run(tr("서버 원본으로 되돌리기"), Icon::Refresh, Kind::Flat, ProjectsMessage::SiteRestore));
            }
            body = body.push(Space::with_height(10)).push(actions);
        }
    }
    if !m.log.is_empty() {
        body = body.push(Space::with_height(12)).push(theme::log_block(m.log.clone(), ProjectsMessage::CopyText));
    }
    card(body)
}

// 에러 로그 패널
fn log_panel(lv: &LogView) -> Element<'_, ProjectsMessage> {
    let header = row![
        column![
            row![icon(Icon::FileText, 15.0, p().fg2), text(trf("에러 로그 · {0}", &[&lv.name])).size(15).font(theme::SEMIBOLD).color(p().fg)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            muted(error_log_path(&lv.id)),
        ]
        .spacing(4)
        .width(Length::Fill),
        btn(tr("복사"), Some(Icon::Copy), Kind::Flat).on_press(ProjectsMessage::CopyLog),
        btn(tr("새로고침"), Some(Icon::Refresh), Kind::Flat).on_press(ProjectsMessage::ViewLog(lv.id.clone())),
        btn(tr("비우기"), Some(Icon::Trash), Kind::Danger).on_press(ProjectsMessage::ClearLog(lv.id.clone())),
        btn(tr("닫기"), Some(Icon::X), Kind::Ghost).on_press(ProjectsMessage::CloseLog),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let body: Element<ProjectsMessage> = if let Some(err) = &lv.error {
        text(err).size(12).color(p().warning_fg).into()
    } else if lv.content.trim().is_empty() {
        // 한글 안내는 기본 글꼴로 (고정폭 글꼴에는 한글이 없다)
        muted(tr("(로그가 비어 있습니다)")).into()
    } else {
        scrollable(text(&lv.content).size(12).font(iced::Font::MONOSPACE).color(p().fg2))
            .height(Length::Fixed(320.0))
            .width(Length::Fill)
            .into()
    };

    card(column![
        header,
        Space::with_height(12),
        container(body).padding(12).width(Length::Fill).style(|_| container::Style {
            background: Some(Background::Color(p().c2)),
            border: iced::Border { radius: theme::R_ROW.into(), ..Default::default() },
            ..Default::default()
        }),
    ])
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

async fn pick_folder() -> Option<String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title(tr("프로젝트 경로 선택"))
        .pick_folder()
        .await;
    match result {
        Some(h) => {
            let path = h.path().to_string_lossy().to_string();
            eprintln!("[localman] 폴더 선택: {path}");
            Some(path)
        }
        None => None,
    }
}
