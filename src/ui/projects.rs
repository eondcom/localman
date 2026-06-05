use iced::{
    widget::{button, column, container, row, text, text_input, Space, scrollable},
    Color, Element, Length, Task,
};
use crate::system::{VhostProject, list_projects, add_project, remove_project};

#[derive(Debug, Clone)]
pub enum ProjectsMessage {
    OpenFilePicker,
    PathSelected(Option<String>),
    NameChanged(String),
    IdChanged(String),
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
    error: Option<String>,
}

impl ProjectsState {
    pub fn new() -> Self {
        Self {
            projects: list_projects(),
            new_name: String::new(),
            new_id: String::new(),
            new_path: String::new(),
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
                if self.new_id.is_empty() || self.new_path.is_empty() {
                    self.error = Some("ID와 경로를 입력하세요.".to_string());
                    return Task::none();
                }
                let project = VhostProject {
                    id: self.new_id.clone(),
                    name: self.new_name.clone(),
                    path: self.new_path.clone(),
                    domain: format!("{}.localhost", self.new_id),
                    port: 80,
                };
                match add_project(project) {
                    Ok(_) => {
                        self.error = None;
                        self.new_name.clear();
                        self.new_id.clear();
                        self.new_path.clear();
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
        let add_form = container(
            column![
                text("새 프로젝트 추가").size(15),
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
                row![
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
                    ].width(Length::Fill),
                ],
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

fn project_row(p: &VhostProject) -> Element<'_, ProjectsMessage> {
    let id = p.id.clone();
    container(
        row![
            column![
                text(&p.name).size(15),
                Space::with_height(2),
                text(&p.domain).size(12).color(Color::from_rgb(0.4, 0.7, 1.0)),
                Space::with_height(2),
                text(&p.path).size(11).color(Color::from_rgb(0.5,0.5,0.5)),
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
    let result = std::process::Command::new("zenity")
        .args(["--file-selection", "--directory", "--title=프로젝트 경로 선택"])
        .output();

    match result {
        Ok(out) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => None,
    }
}
