use iced::{
    widget::{column, container, row, scrollable, text, Space},
    Background, Element, Length, Task,
};
use super::theme::{
    self, Icon, Kind, Tone, btn, card, chip, icon, icon_btn, input, muted, p, page_header, result_line,
    segmented, status,
};
use crate::domain::settings::load_settings;
use crate::platform::open_url;
use crate::domain::{
    VhostProject, ProjectType, ServerStatus,
    list_projects, add_project, update_project, remove_project,
    start_server, stop_server, server_status, auto_assign_port,
    setup_project, deps_ready, auto_detect_start_command, auto_detect_next_command,
    detect_next_app_dir, join_dir,
    is_rhymix_project, rx_reset_admin_password,
};
use crate::platform::{error_log_path, read_log, clear_log};
use crate::domain::history::{last_for_project, load_history, summary_line};
use rfd;

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

/// 라이믹스 관리자 비번 변경 모달 상태
struct RxPasswordModal {
    project_id: String,
    project_name: String,
    user_id: String,
    new_password: String,
    running: bool,
    message: Option<Result<String, String>>,
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
    // 라이믹스 관리자 비번 변경
    OpenRxPasswordModal(String),
    RxUserIdChanged(String),
    RxNewPasswordChanged(String),
    RxResetPassword,
    RxResetPasswordDone(Result<String, String>),
    CloseRxPasswordModal,
    // 새 프로젝트 폼 열기/닫기
    ToggleAddForm,
    /// 도메인을 브라우저로 연다
    OpenSite(String),
    // 더보기(⋯) 메뉴
    ToggleMenu(String),
    /// 다른 PC로 보내기 — App 이 백업·이전 탭으로 넘겨 처리한다
    SendToPc(String),
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
    rx_password_modal: Option<RxPasswordModal>,
    // 메시지
    error: Option<String>,
    server_message: Option<Result<String, String>>,
    // 새 프로젝트 폼이 펼쳐져 있는지
    adding: bool,
    // 더보기 메뉴가 열린 project id
    menu_open: Option<String>,
    // project id → (마지막 이전 한 줄, 최근 이전 기록 줄들). 그릴 때마다 파일을 읽지 않게 미리 만든다
    transfers: std::collections::HashMap<String, (String, Vec<String>)>,
}

fn load_transfers(projects: &[VhostProject]) -> std::collections::HashMap<String, (String, Vec<String>)> {
    let history = load_history();
    projects
        .iter()
        .filter_map(|p| {
            let last = last_for_project(&history, &p.id)?;
            let recent: Vec<String> = history
                .iter()
                .rev()
                .filter(|r| r.projects.iter().any(|x| x == &p.id))
                .take(5)
                .map(summary_line)
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
            adding: false,
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
            rx_password_modal: None,
            error: None,
            server_message: None,
        }
    }

    pub fn update(&mut self, msg: ProjectsMessage) -> Task<ProjectsMessage> {
        match msg {
            ProjectsMessage::Refresh => {
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
                    self.error = Some("ID를 입력하세요.".to_string());
                    return Task::none();
                }
                if self.new_path.is_empty() {
                    self.error = Some("경로를 입력하세요.".to_string());
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
                    Err(e) => self.error = Some(e),
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
                    Err(e) => self.error = Some(e),
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
                self.server_message = Some(result.map(|pid| format!("{id} 시작됨 (PID {pid})")));
                Task::none()
            }
            ProjectsMessage::ServerStopped(id, result) => {
                self.server_message = Some(result.map(|_| format!("{id} 중지됨")));
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
                        self.editing_id = None;
                        self.error = None;
                        self.projects = list_projects();
                    }
                    Err(e) => self.error = Some(e),
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
                    self.server_message = Some(Ok(format!("{id}: 패키지 설치 중...")));
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
                    Err(e) => self.server_message = Some(Err(format!("{id}: {e}"))),
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
                    self.server_message = Some(Ok("로그를 클립보드에 복사했습니다.".to_string()));
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
                        self.server_message = Some(Ok("로그를 비웠습니다.".to_string()));
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
            ProjectsMessage::OpenRxPasswordModal(id) => {
                let name = self.projects.iter().find(|p| p.id == id)
                    .map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
                self.rx_password_modal = Some(RxPasswordModal {
                    project_id: id,
                    project_name: name,
                    user_id: "admin".to_string(),
                    new_password: String::new(),
                    running: false,
                    message: None,
                });
                Task::none()
            }
            ProjectsMessage::RxUserIdChanged(v) => {
                if let Some(m) = self.rx_password_modal.as_mut() { m.user_id = v; }
                Task::none()
            }
            ProjectsMessage::RxNewPasswordChanged(v) => {
                if let Some(m) = self.rx_password_modal.as_mut() { m.new_password = v; }
                Task::none()
            }
            ProjectsMessage::RxResetPassword => {
                let Some(m) = self.rx_password_modal.as_mut() else { return Task::none(); };
                let user_id = m.user_id.trim().to_string();
                let new_password = m.new_password.clone();
                if user_id.is_empty() {
                    m.message = Some(Err("관리자 ID를 입력하세요.".to_string()));
                    return Task::none();
                }
                if new_password.is_empty() {
                    m.message = Some(Err("새 비밀번호를 입력하세요.".to_string()));
                    return Task::none();
                }
                let Some(project) = self.projects.iter().find(|p| p.id == m.project_id).cloned() else {
                    return Task::none();
                };
                m.running = true;
                m.message = None;
                Task::perform(
                    async move { rx_reset_admin_password(&project, &user_id, &new_password) },
                    ProjectsMessage::RxResetPasswordDone,
                )
            }
            ProjectsMessage::RxResetPasswordDone(result) => {
                if let Some(m) = self.rx_password_modal.as_mut() {
                    m.running = false;
                    m.message = Some(result);
                }
                Task::none()
            }
            ProjectsMessage::CloseRxPasswordModal => {
                self.rx_password_modal = None;
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, ProjectsMessage> {
        let add_btn = if self.adding {
            btn("닫기", Some(Icon::X), Kind::Surface).on_press(ProjectsMessage::ToggleAddForm)
        } else {
            btn("새 프로젝트", Some(Icon::Plus), Kind::Primary).on_press(ProjectsMessage::ToggleAddForm)
        };
        let mut col = column![
            page_header("프로젝트", "*.localhost 도메인으로 여는 가상호스트를 관리합니다", Some(add_btn.into())),
            Space::with_height(20),
        ]
        .spacing(0);

        if self.adding {
            col = col.push(self.add_form()).push(Space::with_height(16));
        }
        if let Some(m) = &self.rx_password_modal {
            col = col.push(rx_password_panel(m)).push(Space::with_height(16));
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
                    text("등록된 프로젝트가 없습니다").size(14).font(theme::MEDIUM).color(p().fg2),
                    muted("오른쪽 위 [새 프로젝트]로 폴더를 등록하면 id.localhost 로 열립니다"),
                ]
                .spacing(6)
                .align_x(iced::Alignment::Center)
                .width(Length::Fill),
            ));
        } else {
            let items: Vec<Element<ProjectsMessage>> = self
                .projects
                .iter()
                .map(|p| {
                    if editing_id == Some(p.id.as_str()) {
                        project_row_editing(p, &self.edit_name, &self.edit_path, &self.edit_start_command, &self.edit_app_dir, &self.edit_type)
                    } else {
                        let is_setting_up = self.setting_up.contains(&p.id);
                        let transfer = self.transfers.get(&p.id);
                        let menu = (self.menu_open.as_deref() == Some(p.id.as_str()))
                            .then(|| transfer.map(|t| t.1.as_slice()).unwrap_or(&[]));
                        project_row_view_with_state(p, editing_id.is_some(), is_setting_up, transfer.map(|t| t.0.as_str()), menu)
                    }
                })
                .collect();
            col = col.push(column(items).spacing(10));
        }
        col.into()
    }

    fn add_form(&self) -> Element<'_, ProjectsMessage> {
        let needs_server = self.new_type.is_proxied();
        let mut form = column![
            row![
                text("새 프로젝트").size(15).font(theme::SEMIBOLD).color(p().fg),
                Space::with_width(Length::Fill),
                type_picker(&self.new_type, ProjectsMessage::TypeSelected),
            ]
            .align_y(iced::Alignment::Center),
            Space::with_height(14),
            row![
                field("프로젝트 이름", input("My Project", &self.new_name).on_input(ProjectsMessage::NameChanged).into(), None),
                field("ID (도메인)", input("id → id.localhost", &self.new_id).on_input(ProjectsMessage::IdChanged).into(), None),
            ]
            .spacing(12),
            Space::with_height(12),
            field(
                "프로젝트 경로",
                row![
                    input("/home/user/projects/...", &self.new_path)
                        .on_input(|v| ProjectsMessage::PathSelected(Some(v)))
                        .width(Length::Fill),
                    btn("찾아보기", Some(Icon::FolderOpen), Kind::Flat).on_press(ProjectsMessage::OpenFilePicker),
                ]
                .spacing(8)
                .into(),
                None,
            ),
            Space::with_height(12),
            field(
                "하위 디렉토리 (선택)",
                input("예: app — 비우면 프로젝트 경로를 그대로 사용", &self.new_app_dir)
                    .on_input(ProjectsMessage::AppDirChanged)
                    .into(),
                Some(if self.new_type == ProjectType::Php {
                    "PHP: DocumentRoot로 사용됩니다 (예: public)"
                } else {
                    "앱이 하위 폴더에 있을 때 (예: easyhost/app). 폴더를 고르면 자동으로 찾습니다"
                }),
            ),
        ]
        .spacing(0);

        if needs_server {
            let placeholder = if self.new_type == ProjectType::NextJs { "npx next dev --port 5001" } else { "python3 app.py" };
            form = form.push(Space::with_height(12)).push(field(
                "실행 명령어",
                input(placeholder, &self.new_start_command).on_input(ProjectsMessage::StartCommandChanged).into(),
                Some(if self.new_type == ProjectType::NextJs {
                    "포트는 5001번부터 자동으로 정해지고 --port 값도 거기에 맞춰집니다"
                } else {
                    "포트는 5001번부터 자동으로 정해집니다"
                }),
            ));
        }

        form = form.push(Space::with_height(16)).push(
            row![
                btn("추가", Some(Icon::Plus), Kind::Primary).on_press(ProjectsMessage::AddProject),
                btn("취소", None, Kind::Ghost).on_press(ProjectsMessage::ToggleAddForm),
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
            btn("복사", Some(Icon::Copy), Kind::Ghost).on_press(ProjectsMessage::CopyText(copy_text)),
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
    menu: Option<&'a [String]>,
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
        text(if p_.project_type == ProjectType::Php {
            p_.work_dir()
        } else if p_.app_dir.is_empty() {
            format!(":{} · {}", p_.port, p_.start_command)
        } else {
            format!(":{} · {}/ · {}", p_.port, p_.app_dir, p_.start_command)
        })
        .size(12)
        .color(c.fg4),
    ]
    .spacing(4)
    .width(Length::Fill);
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
            actions = actions.push(status("패키지 설치 중…", Tone::Warning));
        } else {
            let st = server_status(&p_.id);
            let running = matches!(st, ServerStatus::Running(_));
            let label = match st {
                ServerStatus::Running(pid) => format!("실행 중 · PID {pid}"),
                ServerStatus::Stopped => "중지됨".to_string(),
            };
            actions = actions.push(status(label, if running { Tone::Success } else { Tone::Neutral }));
            if !deps_ready(p_) {
                actions = actions.push(btn("패키지 설치", Some(Icon::Package), Kind::Flat).on_press(ProjectsMessage::SetupDeps(id.clone())));
            }
            actions = actions.push(if running {
                btn("중지", Some(Icon::Square), Kind::Danger).on_press(ProjectsMessage::StopServer(id.clone()))
            } else {
                btn("시작", Some(Icon::Play), Kind::Success).on_press(ProjectsMessage::StartServer(id.clone()))
            });
        }
    } else {
        let apache_running = crate::platform::get_service_status("apache2") == crate::platform::ServiceStatus::Running;
        actions = actions.push(status(
            if apache_running { "Apache 실행 중" } else { "Apache 중지됨" },
            if apache_running { Tone::Success } else { Tone::Neutral },
        ));
    }
    if is_rhymix_project(p_) {
        actions = actions.push(btn("관리자 비번", Some(Icon::Key), Kind::Flat).on_press(ProjectsMessage::OpenRxPasswordModal(id.clone())));
    }
    actions = actions
        .push(btn("로그", Some(Icon::FileText), Kind::Flat).on_press(ProjectsMessage::ViewLog(id.clone())))
        .push({
            let b = btn("수정", Some(Icon::Pencil), Kind::Flat);
            if any_editing { b } else { b.on_press(ProjectsMessage::EditProject(id.clone())) }
        })
        .push(btn("삭제", Some(Icon::Trash), Kind::Danger).on_press(ProjectsMessage::RemoveProject(id.clone())))
        .push(icon_btn(Icon::Ellipsis, menu.is_some()).on_press(ProjectsMessage::ToggleMenu(id.clone())));

    let main = row![info, actions].spacing(16).align_y(iced::Alignment::Center);
    card(with_menu(main.into(), menu, id))
}

/// 더보기 메뉴가 열려 있으면 카드 아래에 동작과 최근 이전 기록을 붙인다.
fn with_menu<'a>(main: Element<'a, ProjectsMessage>, menu: Option<&'a [String]>, id: String) -> Element<'a, ProjectsMessage> {
    let Some(recent) = menu else {
        return main;
    };
    let mut history = column![text("이전 기록").size(12).font(theme::SEMIBOLD).color(p().fg3)].spacing(4);
    if recent.is_empty() {
        history = history.push(muted("아직 다른 PC와 주고받은 적이 없습니다."));
    }
    for l in recent {
        let tone = if l.contains("일부 실패") { Tone::Warning } else { Tone::Success };
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
            btn("다른 PC로 보내기", Some(Icon::Send), Kind::Primary).on_press(ProjectsMessage::SendToPc(id)),
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
) -> Element<'a, ProjectsMessage> {
    let needs_server = edit_type.is_proxied();
    let type_changed = *edit_type != p_.project_type;

    let mut form = column![
        row![
            text(&p_.domain).size(15).font(theme::SEMIBOLD).color(p().fg),
            chip("편집 중", Tone::Warning),
            Space::with_width(Length::Fill),
            type_picker(edit_type, ProjectsMessage::EditTypeSelected),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
        Space::with_height(14),
        field("이름", input("프로젝트 이름", edit_name).on_input(ProjectsMessage::EditNameChanged).into(), None),
        Space::with_height(12),
        field(
            "경로",
            row![
                input("/home/...", edit_path).on_input(ProjectsMessage::EditPathChanged).width(Length::Fill),
                btn("찾아보기", Some(Icon::FolderOpen), Kind::Flat).on_press(ProjectsMessage::EditPickFolder(p_.id.clone())),
            ]
            .spacing(8)
            .into(),
            None,
        ),
        Space::with_height(12),
        field(
            "하위 디렉토리 (선택)",
            input("예: app — 비우면 경로를 그대로 사용", edit_app_dir).on_input(ProjectsMessage::EditAppDirChanged).into(),
            None,
        ),
    ]
    .spacing(0);

    if type_changed {
        form = form.push(Space::with_height(10)).push(chip(
            if needs_server {
                "저장하면 새 포트를 받고 vhost가 프록시 형태로 다시 만들어집니다"
            } else {
                "저장하면 포트가 80으로 바뀌고 vhost가 DocumentRoot 형태로 다시 만들어집니다"
            },
            Tone::Warning,
        ));
    }
    if needs_server {
        let placeholder = if *edit_type == ProjectType::NextJs { "npx next dev --port 5001" } else { "python3 run_server.py 5001" };
        form = form.push(Space::with_height(12)).push(field(
            "실행 명령어",
            input(placeholder, edit_start_cmd).on_input(ProjectsMessage::EditStartCommandChanged).into(),
            None,
        ));
    }
    form = form.push(Space::with_height(16)).push(
        row![
            btn("저장", Some(Icon::Check), Kind::Primary).on_press(ProjectsMessage::SaveEdit(p_.id.clone())),
            btn("취소", None, Kind::Ghost).on_press(ProjectsMessage::CancelEdit),
        ]
        .spacing(8),
    );
    card(form)
}

// 라이믹스 관리자 비번 변경 패널
fn rx_password_panel(m: &RxPasswordModal) -> Element<'_, ProjectsMessage> {
    let header = row![
        icon(Icon::Key, 15.0, p().fg2),
        text(format!("라이믹스 관리자 비번 변경 · {}", m.project_name)).size(15).font(theme::SEMIBOLD).color(p().fg),
        Space::with_width(Length::Fill),
        btn("닫기", Some(Icon::X), Kind::Ghost).on_press(ProjectsMessage::CloseRxPasswordModal),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let change = btn(if m.running { "변경 중…" } else { "변경" }, Some(Icon::Check), Kind::Primary);
    let form = row![
        container(field("관리자 ID", input("admin", &m.user_id).on_input(ProjectsMessage::RxUserIdChanged).into(), None)).width(180),
        field(
            "새 비밀번호",
            input("새 비밀번호", &m.new_password).on_input(ProjectsMessage::RxNewPasswordChanged).secure(true).into(),
            None,
        ),
        column![Space::with_height(20), if m.running { change } else { change.on_press(ProjectsMessage::RxResetPassword) }],
    ]
    .spacing(12);

    let mut body = column![
        header,
        Space::with_height(6),
        muted("rx-cli(rx member reset-password)로 라이믹스 코어의 비밀번호 변경 로직을 그대로 부릅니다. 사이트 구분 없는 전역 회원 계정이니 ID를 정확히 입력하세요."),
        Space::with_height(14),
        form,
    ];
    if let Some(msg) = &m.message {
        let line = match msg {
            Ok(m) => format!("✓ {m}"),
            Err(e) => format!("✗ {e}"),
        };
        body = body.push(Space::with_height(12)).push(result_line(line));
    }
    card(body)
}

// 에러 로그 패널
fn log_panel(lv: &LogView) -> Element<'_, ProjectsMessage> {
    let header = row![
        column![
            row![icon(Icon::FileText, 15.0, p().fg2), text(format!("에러 로그 · {}", lv.name)).size(15).font(theme::SEMIBOLD).color(p().fg)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            muted(error_log_path(&lv.id)),
        ]
        .spacing(4)
        .width(Length::Fill),
        btn("복사", Some(Icon::Copy), Kind::Flat).on_press(ProjectsMessage::CopyLog),
        btn("새로고침", Some(Icon::Refresh), Kind::Flat).on_press(ProjectsMessage::ViewLog(lv.id.clone())),
        btn("비우기", Some(Icon::Trash), Kind::Danger).on_press(ProjectsMessage::ClearLog(lv.id.clone())),
        btn("닫기", Some(Icon::X), Kind::Ghost).on_press(ProjectsMessage::CloseLog),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let body: Element<ProjectsMessage> = if let Some(err) = &lv.error {
        text(err).size(12).color(p().warning_fg).into()
    } else if lv.content.trim().is_empty() {
        // 한글 안내는 기본 글꼴로 (고정폭 글꼴에는 한글이 없다)
        muted("(로그가 비어 있습니다)").into()
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
        .set_title("프로젝트 경로 선택")
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
