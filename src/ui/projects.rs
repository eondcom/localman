use iced::{
    widget::{button, column, container, row, text, text_input, Space, scrollable},
    Color, Element, Length, Task,
};
use crate::system::{VhostProject, ProjectType, list_projects, add_project, remove_project};
use rfd;

#[derive(Debug, Clone)]
pub enum ProjectsMessage {
    OpenFilePicker,
    PathSelected(Option<String>),
    NameChanged(String),
    IdChanged(String),
    TypeSelected(ProjectType),
    PortChanged(String),
    AddProject,
    RemoveProject(String),
    #[allow(dead_code)]
    Refresh,
}

pub struct ProjectsState {
    projects: Vec<VhostProject>,
    new_name: String,
    new_id: String,
    new_path: String,
    new_type: ProjectType,
    new_port: String,
    error: Option<String>,
}

impl ProjectsState {
    pub fn new() -> Self {
        Self {
            projects: list_projects(),
            new_name: String::new(),
            new_id: String::new(),
            new_path: String::new(),
            new_type: ProjectType::Php,
            new_port: "8000".to_string(),
            error: None,
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
            ProjectsMessage::IdChanged(v) => {
                self.new_id = v;
                Task::none()
            }
            ProjectsMessage::TypeSelected(t) => {
                self.new_type = t;
                Task::none()
            }
            ProjectsMessage::PortChanged(v) => {
                if v.chars().all(|c| c.is_ascii_digit()) {
                    self.new_port = v;
                }
                Task::none()
            }
            ProjectsMessage::OpenFilePicker => {
                Task::perform(pick_folder(), ProjectsMessage::PathSelected)
            }
            ProjectsMessage::PathSelected(path) => {
                if let Some(p) = path {
                    self.new_path = p;
                }
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
                let port: u16 = self.new_port.parse().unwrap_or(8000);
                let project = VhostProject {
                    id: self.new_id.clone(),
                    name: self.new_name.clone(),
                    path: self.new_path.clone(),
                    domain: format!("{}.localhost", self.new_id),
                    project_type: self.new_type.clone(),
                    port,
                };
                match add_project(project) {
                    Ok(_) => {
                        self.error = None;
                        self.new_name.clear();
                        self.new_id.clear();
                        self.new_path.clear();
                        self.new_port = "8000".to_string();
                        self.new_type = ProjectType::Php;
                        self.projects = list_projects();
                    }
                    Err(e) => self.error = Some(e),
                }
                Task::none()
            }
            ProjectsMessage::RemoveProject(id) => {
                match remove_project(&id) {
                    Ok(_) => {
                        self.error = None;
                        self.projects = list_projects();
                    }
                    Err(e) => self.error = Some(e),
                }
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

        let path_row: Element<ProjectsMessage> = if !is_python {
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
            ].into()
        } else {
            column![
                text("Dev server 포트").size(12).color(Color::from_rgb(0.6,0.6,0.6)),
                Space::with_height(4),
                row![
                    text_input("8000", &self.new_port)
                        .on_input(ProjectsMessage::PortChanged)
                        .padding(10)
                        .width(120),
                    Space::with_width(12),
                    text("python app.py 또는 python manage.py runserver 0.0.0.0:포트 로 실행 후 등록")
                        .size(12)
                        .color(Color::from_rgb(0.5, 0.7, 0.5)),
                ]
                .align_y(iced::Alignment::Center),
            ].into()
        };

        let add_form = container(
            column![
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
                        text_input("myproject → myproject.localhost", &self.new_id)
                            .on_input(ProjectsMessage::IdChanged)
                            .padding(10),
                    ].width(Length::FillPortion(2)),
                ],
                Space::with_height(10),
                path_row,
                Space::with_height(14),
                button(text("프로젝트 추가").size(14))
                    .on_press(ProjectsMessage::AddProject)
                    .padding([10, 24])
                    .style(|_, _| button::Style {
                        background: Some(iced::Background::Color(Color::from_rgb(0.1, 0.45, 0.7))),
                        border: iced::Border { radius: 6.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        ..Default::default()
                    }),
            ]
            .spacing(0)
        )
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
            let items: Vec<Element<ProjectsMessage>> = self.projects.iter().map(|p| {
                project_row(p)
            }).collect();
            scrollable(column(items).spacing(8)).into()
        };

        let mut col = column![
            text("프로젝트").size(22),
            Space::with_height(8),
            text("가상호스트를 설정해 id.localhost 도메인으로 접속할 수 있습니다.").size(13).color(Color::from_rgb(0.6,0.6,0.6)),
            Space::with_height(20),
            add_form,
            Space::with_height(20),
            project_list,
        ];

        if let Some(err) = &self.error {
            col = col.push(Space::with_height(10)).push(
                text(format!("오류: {err}")).size(13).color(Color::from_rgb(1.0, 0.4, 0.4))
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
    let (type_label, type_color) = match p.project_type {
        ProjectType::Php => ("PHP", Color::from_rgb(0.5, 0.6, 1.0)),
        ProjectType::Python => ("Python", Color::from_rgb(0.4, 0.8, 0.5)),
    };
    let sub_info = match p.project_type {
        ProjectType::Php => p.path.clone(),
        ProjectType::Python => format!("→ 127.0.0.1:{}", p.port),
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
                text(sub_info).size(11).color(Color::from_rgb(0.5,0.5,0.5)),
            ].width(Length::Fill),
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
        Some(handle) => {
            let path = handle.path().to_string_lossy().to_string();
            eprintln!("[localman] 폴더 선택: {path}");
            Some(path)
        }
        None => {
            eprintln!("[localman] 폴더 선택 취소");
            None
        }
    }
}
