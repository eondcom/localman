use iced::{
    widget::{button, column, container, row, text, text_input, Space, scrollable},
    Color, Element, Length, Task,
};
use crate::system::{
    VhostProject, ProjectType, ServerStatus,
    list_projects, add_project, remove_project,
    start_server, stop_server, server_status, auto_assign_port,
};
use rfd;

#[derive(Debug, Clone)]
pub enum ProjectsMessage {
    OpenFilePicker,
    PathSelected(Option<String>),
    NameChanged(String),
    IdChanged(String),
    TypeSelected(ProjectType),
    StartCommandChanged(String),
    AddProject,
    RemoveProject(String),
    StartServer(String),  // project id
    StopServer(String),   // project id
    ServerStarted(String, Result<u32, String>),
    ServerStopped(String, Result<(), String>),
    #[allow(dead_code)]
    Refresh,
}

pub struct ProjectsState {
    projects: Vec<VhostProject>,
    new_name: String,
    new_id: String,
    new_path: String,
    new_type: ProjectType,
    new_start_command: String,
    error: Option<String>,
    server_message: Option<Result<String, String>>,
}

impl ProjectsState {
    pub fn new() -> Self {
        Self {
            projects: list_projects(),
            new_name: String::new(),
            new_id: String::new(),
            new_path: String::new(),
            new_type: ProjectType::Php,
            new_start_command: "python app.py".to_string(),
            error: None,
            server_message: None,
        }
    }

    pub fn update(&mut self, msg: ProjectsMessage) -> Task<ProjectsMessage> {
        match msg {
            ProjectsMessage::Refresh => {
                self.projects = list_projects();
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
                self.new_type = t;
                Task::none()
            }
            ProjectsMessage::StartCommandChanged(v) => {
                self.new_start_command = v;
                Task::none()
            }
            ProjectsMessage::OpenFilePicker => {
                Task::perform(pick_folder(), ProjectsMessage::PathSelected)
            }
            ProjectsMessage::PathSelected(path) => {
                if let Some(p) = path { self.new_path = p; }
                Task::none()
            }
            ProjectsMessage::AddProject => {
                if self.new_id.is_empty() {
                    self.error = Some("ID를 입력하세요.".to_string());
                    return Task::none();
                }
                if self.new_type == ProjectType::Php && self.new_path.is_empty() {
                    self.error = Some("PHP 프로젝트는 경로가 필요합니다.".to_string());
                    return Task::none();
                }
                if self.new_type == ProjectType::Python && self.new_path.is_empty() {
                    self.error = Some("Python 프로젝트는 경로가 필요합니다.".to_string());
                    return Task::none();
                }
                let port = if self.new_type == ProjectType::Python {
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
                };
                match add_project(project) {
                    Ok(_) => {
                        self.error = None;
                        self.new_name.clear();
                        self.new_id.clear();
                        self.new_path.clear();
                        self.new_start_command = "python app.py".to_string();
                        self.new_type = ProjectType::Php;
                        self.projects = list_projects();
                    }
                    Err(e) => self.error = Some(e),
                }
                Task::none()
            }
            ProjectsMessage::RemoveProject(id) => {
                // 실행 중이면 먼저 중지
                let _ = stop_server(&id);
                match remove_project(&id) {
                    Ok(_) => {
                        self.error = None;
                        self.projects = list_projects();
                    }
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
        }
    }

    pub fn view(&self) -> Element<'_, ProjectsMessage> {
        let is_python = self.new_type == ProjectType::Python;

        let type_row = row![
            type_btn("PHP", !is_python, ProjectsMessage::TypeSelected(ProjectType::Php)),
            Space::with_width(8),
            type_btn("Python", is_python, ProjectsMessage::TypeSelected(ProjectType::Python)),
        ];

        let mut form_col = column![
            text("새 프로젝트 추가").size(15),
            Space::with_height(12),
            type_row,
            Space::with_height(12),
            row![
                column![
                    text("프로젝트 이름").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                    Space::with_height(4),
                    text_input("My Project", &self.new_name)
                        .on_input(ProjectsMessage::NameChanged)
                        .padding(10),
                ].width(Length::FillPortion(2)),
                Space::with_width(12),
                column![
                    text("ID (도메인)").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                    Space::with_height(4),
                    text_input("id → id.localhost", &self.new_id)
                        .on_input(ProjectsMessage::IdChanged)
                        .padding(10),
                ].width(Length::FillPortion(2)),
            ],
            Space::with_height(10),
        ]
        .spacing(0);

        // 경로 (PHP/Python 공통)
        form_col = form_col.push(
            column![
                text("프로젝트 경로").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                Space::with_height(4),
                row![
                    text_input("/home/user/projects/...", &self.new_path)
                        .padding(10)
                        .width(Length::Fill),
                    Space::with_width(8),
                    button(text("탐색").size(13))
                        .on_press(ProjectsMessage::OpenFilePicker)
                        .padding([10, 16]),
                ],
            ]
        );

        if is_python {
            form_col = form_col
                .push(Space::with_height(10))
                .push(
                    column![
                        text("실행 명령어").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                        Space::with_height(4),
                        text_input("python app.py", &self.new_start_command)
                            .on_input(ProjectsMessage::StartCommandChanged)
                            .padding(10),
                        Space::with_height(4),
                        text("포트는 자동 할당됩니다 (5001번부터 순서대로)")
                            .size(11).color(Color::from_rgb(0.4, 0.6, 0.4)),
                    ]
                );
        }

        form_col = form_col.push(Space::with_height(14)).push(
            button(text("프로젝트 추가").size(14))
                .on_press(ProjectsMessage::AddProject)
                .padding([10, 24])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.45, 0.7))),
                    border: iced::Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                })
        );

        let add_form = container(form_col)
            .padding(20)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.13, 0.13, 0.16))),
                border: iced::Border { radius: 10.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
                ..Default::default()
            });

        let project_list: Element<ProjectsMessage> = if self.projects.is_empty() {
            container(
                text("등록된 프로젝트가 없습니다.").size(14).color(Color::from_rgb(0.5,0.5,0.5))
            ).padding(20).into()
        } else {
            let items: Vec<Element<ProjectsMessage>> = self.projects.iter()
                .map(|p| project_row(p))
                .collect();
            scrollable(column(items).spacing(8)).into()
        };

        let mut col = column![
            text("프로젝트").size(22),
            Space::with_height(8),
            text("가상호스트를 설정해 id.localhost 도메인으로 접속할 수 있습니다.")
                .size(13).color(Color::from_rgb(0.6,0.6,0.6)),
            Space::with_height(20),
            add_form,
            Space::with_height(20),
            project_list,
        ];

        if let Some(err) = &self.error {
            col = col.push(Space::with_height(8)).push(
                text(format!("오류: {err}")).size(13).color(Color::from_rgb(1.0, 0.4, 0.4))
            );
        }
        if let Some(msg) = &self.server_message {
            let (txt, color) = match msg {
                Ok(m) => (m.as_str(), Color::from_rgb(0.2, 0.9, 0.4)),
                Err(e) => (e.as_str(), Color::from_rgb(1.0, 0.4, 0.4)),
            };
            col = col.push(Space::with_height(8)).push(
                text(txt).size(13).color(color)
            );
        }

        col.into()
    }
}

fn type_btn(label: &str, active: bool, msg: ProjectsMessage) -> Element<'_, ProjectsMessage> {
    let bg = if active {
        Color::from_rgb(0.15, 0.35, 0.55)
    } else {
        Color::from_rgb(0.13, 0.13, 0.16)
    };
    button(text(label).size(13))
        .on_press(msg)
        .padding([8, 20])
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

fn project_row(p: &VhostProject) -> Element<'_, ProjectsMessage> {
    let id = p.id.clone();
    let id2 = p.id.clone();

    let (type_label, type_color) = match p.project_type {
        ProjectType::Php   => ("PHP",    Color::from_rgb(0.5, 0.6, 1.0)),
        ProjectType::Python => ("Python", Color::from_rgb(0.4, 0.8, 0.5)),
    };

    let status = server_status(&p.id);
    let is_running = matches!(status, ServerStatus::Running(_));

    let server_controls: Element<ProjectsMessage> = match p.project_type {
        ProjectType::Python => {
            let (btn_label, btn_color, btn_msg) = if is_running {
                ("중지", Color::from_rgb(0.7, 0.2, 0.2), ProjectsMessage::StopServer(id2))
            } else {
                ("시작", Color::from_rgb(0.1, 0.5, 0.3), ProjectsMessage::StartServer(id2))
            };
            let status_dot_color = if is_running {
                Color::from_rgb(0.2, 0.9, 0.4)
            } else {
                Color::from_rgb(0.5, 0.5, 0.5)
            };
            let pid_str = if let ServerStatus::Running(pid) = status {
                format!("PID {pid}")
            } else {
                "중지됨".to_string()
            };
            row![
                container(Space::with_width(8))
                    .width(8).height(8)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(status_dot_color)),
                        border: iced::Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                Space::with_width(6),
                text(pid_str).size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
                Space::with_width(10),
                button(text(btn_label).size(12))
                    .on_press(btn_msg)
                    .padding([6, 14])
                    .style(move |_, _| button::Style {
                        background: Some(iced::Background::Color(btn_color)),
                        border: iced::Border { radius: 5.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        ..Default::default()
                    }),
            ]
            .align_y(iced::Alignment::Center)
            .into()
        }
        ProjectType::Php => {
            let apache_running = crate::system::get_service_status("apache2")
                == crate::system::ServiceStatus::Running;
            let dot_color = if apache_running {
                Color::from_rgb(0.2, 0.9, 0.4)
            } else {
                Color::from_rgb(0.5, 0.5, 0.5)
            };
            let label = if apache_running { "Apache 실행 중" } else { "Apache 중지됨" };
            row![
                container(Space::with_width(8))
                    .width(8).height(8)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(dot_color)),
                        border: iced::Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                Space::with_width(6),
                text(label).size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
            ]
            .align_y(iced::Alignment::Center)
            .into()
        }
    };

    container(
        row![
            column![
                row![
                    text(&p.name).size(15),
                    Space::with_width(8),
                    container(text(type_label).size(11))
                        .padding([2, 8])
                        .style(move |_| container::Style {
                            background: Some(iced::Background::Color(Color::from_rgba(
                                type_color.r, type_color.g, type_color.b, 0.15,
                            ))),
                            border: iced::Border {
                                radius: 4.0.into(),
                                color: Color::from_rgba(type_color.r, type_color.g, type_color.b, 0.4),
                                width: 1.0,
                            },
                            ..Default::default()
                        }),
                ]
                .align_y(iced::Alignment::Center),
                Space::with_height(2),
                text(&p.domain).size(12).color(Color::from_rgb(0.4, 0.7, 1.0)),
                Space::with_height(2),
                text(match p.project_type {
                    ProjectType::Php    => p.path.clone(),
                    ProjectType::Python => format!(":{} · {}", p.port, p.start_command),
                }).size(11).color(Color::from_rgb(0.5,0.5,0.5)),
            ].width(Length::Fill),
            server_controls,
            Space::with_width(10),
            button(text("삭제").size(13))
                .on_press(ProjectsMessage::RemoveProject(id))
                .padding([6, 14])
                .style(|_, _| button::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.5,0.1,0.1))),
                    border: iced::Border { radius: 5.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    ..Default::default()
                }),
        ]
        .align_y(iced::Alignment::Center)
    )
    .padding(16)
    .width(Length::Fill)
    .style(|_| container::Style {
        background: Some(iced::Background::Color(Color::from_rgb(0.13,0.13,0.16))),
        border: iced::Border { radius: 8.0.into(), color: Color::from_rgb(0.2,0.2,0.25), width: 1.0 },
        ..Default::default()
    })
    .into()
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
