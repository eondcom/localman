use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::Mutex;

// 실행 중인 Python dev server PID 목록 (id → pid)
static RUNNING_PIDS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn pids() -> std::sync::MutexGuard<'static, Option<HashMap<String, u32>>> {
    let mut g = RUNNING_PIDS.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProjectType {
    Php,
    Python,
}

impl Default for ProjectType {
    fn default() -> Self {
        ProjectType::Php
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
    // PHP: 80 고정, Python: dev server 포트
    pub port: u16,
    // Python 전용: 실행 명령어 (예: "python app.py")
    #[serde(default)]
    pub start_command: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Running(u32), // pid
    Stopped,
}

pub fn server_status(id: &str) -> ServerStatus {
    let g = pids();
    if let Some(map) = g.as_ref() {
        if let Some(&pid) = map.get(id) {
            // /proc/PID 존재 여부로 살아있는지 확인
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return ServerStatus::Running(pid);
            }
        }
    }
    ServerStatus::Stopped
}

pub fn start_server(project: &VhostProject) -> Result<u32, String> {
    if project.start_command.is_empty() {
        return Err("실행 명령어가 비어 있습니다.".to_string());
    }
    let parts: Vec<&str> = project.start_command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("실행 명령어가 올바르지 않습니다.".to_string());
    }
    eprintln!("[localman] 서버 시작: {} in {}", project.start_command, project.path);
    let child = std::process::Command::new(parts[0])
        .args(&parts[1..])
        .current_dir(&project.path)
        .spawn()
        .map_err(|e| format!("실행 실패: {e}"))?;
    let pid = child.id();
    // child를 drop해도 프로세스는 계속 실행됨 (detach)
    std::mem::forget(child);
    let mut g = pids();
    g.as_mut().unwrap().insert(project.id.clone(), pid);
    eprintln!("[localman] 서버 시작됨 PID={pid}");
    Ok(pid)
}

pub fn stop_server(id: &str) -> Result<(), String> {
    let pid = {
        let g = pids();
        g.as_ref().and_then(|m| m.get(id).copied())
    };
    if let Some(pid) = pid {
        eprintln!("[localman] 서버 중지 PID={pid}");
        let r = std::process::Command::new("kill")
            .arg(pid.to_string())
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            let err = String::from_utf8_lossy(&r.stderr).to_string();
            return Err(format!("kill 실패: {err}"));
        }
        let mut g = pids();
        g.as_mut().unwrap().remove(id);
        Ok(())
    } else {
        Err("실행 중인 서버가 없습니다.".to_string())
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

pub fn add_project(project: VhostProject) -> Result<(), String> {
    eprintln!("[localman] 프로젝트 추가: {} ({:?})", project.id, project.project_type);
    let mut list = list_projects();
    list.push(project.clone());
    save_projects(&list)?;
    if project.project_type == ProjectType::Python {
        ensure_proxy_module()?;
    }
    write_vhost(&project)?;
    update_hosts(&project.domain, true)?;
    eprintln!("[localman] 프로젝트 추가 완료: {}", project.domain);
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

fn ensure_proxy_module() -> Result<(), String> {
    let out = std::process::Command::new("apache2ctl")
        .args(["-M"])
        .output()
        .map_err(|e| e.to_string())?;
    let modules = String::from_utf8_lossy(&out.stdout);
    if modules.contains("proxy_http_module") {
        return Ok(());
    }
    eprintln!("[localman] proxy_http 모듈 활성화 중...");
    let r = std::process::Command::new("sudo")
        .args(["a2enmod", "proxy", "proxy_http"])
        .output()
        .map_err(|e| e.to_string())?;
    if !r.status.success() {
        return Err(format!("a2enmod 실패: {}", String::from_utf8_lossy(&r.stderr)));
    }
    std::process::Command::new("sudo")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn build_vhost_conf(p: &VhostProject) -> String {
    match p.project_type {
        ProjectType::Php => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   DocumentRoot {path}\n\
             \x20   <Directory {path}>\n\
             \x20       AllowOverride All\n\
             \x20       Require all granted\n\
             \x20   </Directory>\n\
             </VirtualHost>\n",
            domain = p.domain,
            path = p.path,
        ),
        ProjectType::Python => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   ProxyPreserveHost On\n\
             \x20   ProxyPass / http://127.0.0.1:{port}/\n\
             \x20   ProxyPassReverse / http://127.0.0.1:{port}/\n\
             </VirtualHost>\n",
            domain = p.domain,
            port = p.port,
        ),
    }
}

fn write_vhost(p: &VhostProject) -> Result<(), String> {
    let conf = build_vhost_conf(p);
    let conf_path = format!("/etc/apache2/sites-available/{}.conf", p.id);
    let enable_path = format!("/etc/apache2/sites-enabled/{}.conf", p.id);
    eprintln!("[localman] vhost 파일 작성: {conf_path}");

    let tmp_path = format!("/tmp/localman_vhost_{}.conf", p.id);
    fs::write(&tmp_path, &conf).map_err(|e| format!("임시 파일 쓰기 실패: {e}"))?;
    let cp_out = std::process::Command::new("sudo")
        .args(["cp", &tmp_path, &conf_path])
        .output()
        .map_err(|e| e.to_string())?;
    if !cp_out.status.success() {
        let err = String::from_utf8_lossy(&cp_out.stderr).to_string();
        eprintln!("[localman] vhost cp 실패: {err}");
        return Err(format!("vhost 파일 쓰기 실패: {err}"));
    }
    std::process::Command::new("sudo")
        .args(["ln", "-sf", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    std::process::Command::new("sudo")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn remove_vhost(p: &VhostProject) -> Result<(), String> {
    let conf_path = format!("/etc/apache2/sites-available/{}.conf", p.id);
    let enable_path = format!("/etc/apache2/sites-enabled/{}.conf", p.id);
    std::process::Command::new("sudo")
        .args(["rm", "-f", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    std::process::Command::new("sudo")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn update_hosts(domain: &str, add: bool) -> Result<(), String> {
    let hosts = fs::read_to_string("/etc/hosts").map_err(|e| e.to_string())?;
    let entry = format!("127.0.0.1\t{domain}");

    let new_hosts = if add {
        if hosts.contains(domain) {
            return Ok(());
        }
        format!("{hosts}\n{entry}\n")
    } else {
        hosts
            .lines()
            .filter(|l| !l.contains(domain))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };

    let tmp = "/tmp/localman_hosts";
    fs::write(tmp, new_hosts).map_err(|e| e.to_string())?;
    std::process::Command::new("sudo")
        .args(["cp", tmp, "/etc/hosts"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}
