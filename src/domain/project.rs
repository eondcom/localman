use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::apache::{remove_vhost, write_vhost};
use super::detect::sync_port_in_command;
use crate::platform::{ensure_proxy_module, update_hosts};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProjectType {
    Php,
    Python,
    NextJs,
}

impl Default for ProjectType {
    fn default() -> Self {
        ProjectType::Php
    }
}

impl ProjectType {
    /// dev server를 띄우고 Apache가 프록시하는 타입인지 (PHP는 DocumentRoot 직접 서빙)
    pub fn is_proxied(&self) -> bool {
        matches!(self, ProjectType::Python | ProjectType::NextJs)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VhostProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub domain: String,
    #[serde(default)]
    pub project_type: ProjectType,
    // PHP: 80 고정, Python/Next.js: dev server 포트
    pub port: u16,
    // Python/Next.js 전용: 실행 명령어 (예: "python app.py", "npx next dev --port 3003")
    #[serde(default)]
    pub start_command: String,
    /// 실행/DocumentRoot 기준이 되는 하위 디렉토리 (path 기준 상대 경로).
    /// 비어 있으면 path 자체를 사용한다. 예: 모노레포의 "app", Laravel의 "public"
    #[serde(default)]
    pub app_dir: String,
}

impl VhostProject {
    /// 실제 작업 디렉토리 — app_dir이 있으면 path/app_dir, 없으면 path
    pub fn work_dir(&self) -> String {
        join_dir(&self.path, &self.app_dir)
    }
}

/// base와 상대 하위 경로를 합친다. sub가 비었으면 base 그대로.
pub fn join_dir(base: &str, sub: &str) -> String {
    let sub = sub.trim().trim_matches('/');
    if sub.is_empty() {
        base.trim_end_matches('/').to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), sub)
    }
}

fn data_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    fs::create_dir_all(&p).ok();
    p.push("projects.json");
    p
}

pub fn list_projects() -> Vec<VhostProject> {
    let path = data_path();
    let data = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn auto_assign_port() -> u16 {
    let used: std::collections::HashSet<u16> = list_projects()
        .iter()
        .map(|p| p.port)
        .collect();
    (5001u16..6000).find(|p| !used.contains(p)).unwrap_or(5001)
}

pub fn add_project(mut project: VhostProject) -> Result<(), String> {
    eprintln!("[localman] 프로젝트 추가: {} ({:?})", project.id, project.project_type);
    // 실행 명령의 포트를 vhost가 프록시할 포트와 맞춘다
    if project.project_type == ProjectType::NextJs {
        project.start_command = sync_port_in_command(&project.start_command, project.port);
    }
    // vhost/hosts 먼저 성공한 뒤 projects.json 저장 (실패 시 반쪽 상태 방지)
    if project.project_type.is_proxied() {
        // Next.js는 HMR(웹소켓)까지 프록시해야 하므로 wstunnel 모듈도 필요하다
        ensure_proxy_module(project.project_type == ProjectType::NextJs)?;
    }
    write_vhost(&project)?;
    update_hosts(&project.domain, true)?;
    let mut list = list_projects();
    // 중복 방지
    list.retain(|p| p.id != project.id);
    list.push(project.clone());
    save_projects(&list)?;
    eprintln!("[localman] 프로젝트 추가 완료: {}", project.domain);
    Ok(())
}

pub fn update_project(
    id: &str,
    name: String,
    path: String,
    start_command: String,
    project_type: ProjectType,
    app_dir: String,
) -> Result<(), String> {
    let mut list = list_projects();
    // 충돌 방지를 위해 mutable borrow 전에 다른 프로젝트 포트 수집
    let used_ports: Vec<u16> = list.iter().filter(|x| x.id != id).map(|x| x.port).collect();
    let p = list.iter_mut().find(|p| p.id == id)
        .ok_or_else(|| "프로젝트를 찾을 수 없습니다.".to_string())?;
    let type_changed = p.project_type != project_type;
    p.name = name;
    p.path = path;
    p.start_command = start_command;
    p.app_dir = app_dir;
    // 타입이 바뀌었거나, 프록시 타입인데 포트가 80(PHP 잔재)이면 포트를 다시 잡는다
    let needs_port = type_changed || (project_type.is_proxied() && p.port == 80);
    if needs_port {
        p.port = if project_type.is_proxied() {
            let mut candidate: u16 = 5001;
            while used_ports.contains(&candidate) { candidate += 1; }
            candidate
        } else {
            80
        };
    }
    p.project_type = project_type.clone();
    if project_type == ProjectType::NextJs {
        p.start_command = sync_port_in_command(&p.start_command, p.port);
    }
    let updated = p.clone();
    save_projects(&list)?;
    if updated.project_type.is_proxied() {
        ensure_proxy_module(updated.project_type == ProjectType::NextJs)?;
    }
    write_vhost(&updated)?;
    eprintln!("[localman] 프로젝트 업데이트: {id} (type_changed={type_changed}, port={})", updated.port);
    Ok(())
}

pub fn remove_project(id: &str) -> Result<(), String> {
    let mut list = list_projects();
    if let Some(p) = list.iter().find(|p| p.id == id).cloned() {
        update_hosts(&p.domain, false)?;
        remove_vhost(&p)?;
    }
    list.retain(|p| p.id != id);
    save_projects(&list)
}

fn save_projects(list: &[VhostProject]) -> Result<(), String> {
    let data = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(data_path(), data).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::project;

    #[test]
    fn join_dir_handles_empty_and_slashes() {
        assert_eq!(join_dir("/srv/app", ""), "/srv/app");
        assert_eq!(join_dir("/srv/app/", ""), "/srv/app");
        assert_eq!(join_dir("/srv/app", "web"), "/srv/app/web");
        assert_eq!(join_dir("/srv/app/", "/web/"), "/srv/app/web");
        assert_eq!(join_dir("/srv/app", "apps/web"), "/srv/app/apps/web");
    }

    #[test]
    fn work_dir_falls_back_to_path() {
        assert_eq!(project(ProjectType::NextJs, "/srv/x", "", 5001).work_dir(), "/srv/x");
        assert_eq!(project(ProjectType::NextJs, "/srv/x", "app", 5001).work_dir(), "/srv/x/app");
    }

    /// 기존 projects.json(= app_dir 필드가 없는 형식)이 그대로 읽혀야 한다
    #[test]
    fn deserializes_legacy_project_json_without_app_dir() {
        let raw = r#"[{"id":"kpop","name":"kpop","path":"/home/dell/dev/kpop",
            "domain":"kpop.localhost","project_type":"Python","port":5004,
            "start_command":"venv/bin/python3 app.py"}]"#;
        let list: Vec<VhostProject> = serde_json::from_str(raw).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].app_dir, "");
        assert_eq!(list[0].work_dir(), "/home/dell/dev/kpop");
        assert_eq!(list[0].project_type, ProjectType::Python);
    }
}
