use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Mutex;
use std::os::unix::process::CommandExt;

/// 실행 중인 dev server 한 건.
///
/// 예전에는 PID 하나만 메모리 HashMap 에 들고 있었다. 그 탓에
/// (1) localman 이 죽으면 목록이 통째로 증발해 서버들이 관리 불가능한 고아가 되고
/// (2) 다시 켜도 그 사실을 몰라 같은 서버를 또 띄웠으며
/// (3) `kill <pid>` 로는 npm→vite, uvicorn→chromedriver 같은 손자 프로세스가 살아남았다.
///
/// 실제로 2026-07-28 에 이 세 가지가 겹쳐, 6일간 방치된 freethread 크롤러가
/// Chrome 임시 프로필 5,074개(약 220G)를 쌓아 디스크를 100% 채웠다.
/// 그래서 pgid 를 함께 기록해 그룹째 죽이고, 파일로 남겨 재시작 후에도 회수한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunningProc {
    pid: u32,
    /// 프로세스 그룹 ID. `process_group(0)` 으로 띄우므로 최초에는 pid 와 같다.
    pgid: u32,
    /// `/proc/<pid>/stat` 의 starttime(22번째 필드).
    /// PID 는 재사용되므로 이것까지 맞아야 같은 프로세스로 인정한다.
    /// 이 검증이 없으면 재사용된 PID 를 죽여 엉뚱한 프로세스를 날릴 수 있다.
    starttime: u64,
}

// id → RunningProc. 디스크(running.json)가 원본이고 이 맵은 캐시다.
static RUNNING_PIDS: Mutex<Option<HashMap<String, RunningProc>>> = Mutex::new(None);

fn running_state_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    let _ = fs::create_dir_all(&p);
    p.push("running.json");
    p
}

fn load_running() -> HashMap<String, RunningProc> {
    let Ok(s) = fs::read_to_string(running_state_path()) else {
        return HashMap::new();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

fn save_running(map: &HashMap<String, RunningProc>) {
    let path = running_state_path();
    match serde_json::to_string_pretty(map) {
        Ok(s) => {
            if let Err(e) = fs::write(&path, s) {
                eprintln!("[localman] 실행 상태 저장 실패 {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[localman] 실행 상태 직렬화 실패: {e}"),
    }
}

fn pids() -> std::sync::MutexGuard<'static, Option<HashMap<String, RunningProc>>> {
    let mut g = RUNNING_PIDS.lock().unwrap();
    if g.is_none() {
        // 첫 접근 때 디스크에서 읽어온다. 이전 localman 세션이 띄워둔 서버를
        // 여기서 회수하므로, 재시작해도 고아가 생기지 않는다.
        *g = Some(load_running());
    }
    g
}

/// `/proc/<pid>/stat` 에서 (pgrp, starttime) 을 읽는다.
///
/// comm 필드는 괄호로 감싸여 있고 그 안에 공백·괄호가 들어갈 수 있어
/// 앞에서부터 자르면 깨진다. 마지막 ')' 뒤부터 파싱한다.
fn proc_stat_fields(pid: u32) -> Option<(u32, u64)> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = s.rfind(')')?;
    let rest = s.get(close + 2..)?; // ") S ..." 에서 state 부터
    let f: Vec<&str> = rest.split_whitespace().collect();
    // stat 필드 번호(1-based): 3=state … 5=pgrp … 22=starttime
    // rest 는 3번 필드부터이므로 index = 번호 - 3
    let pgrp = f.get(2)?.parse().ok()?;
    let starttime = f.get(19)?.parse().ok()?;
    Some((pgrp, starttime))
}

/// 기록된 프로세스가 "그때 그 프로세스" 그대로 살아 있는지.
fn is_alive(rec: &RunningProc) -> bool {
    match proc_stat_fields(rec.pid) {
        Some((_, starttime)) => starttime == rec.starttime,
        None => false,
    }
}

/// 프로세스 그룹 전체에 시그널을 보낸다 (`kill -SIG -- -PGID`).
///
/// 단일 PID 로는 `npm run dev` 가 띄운 vite, uvicorn 이 띄운 chromedriver 처럼
/// 손자 프로세스가 남는다. 그룹째 보내야 실제로 정리된다.
fn signal_group(pgid: u32, sig: &str) -> Result<(), String> {
    let out = std::process::Command::new("kill")
        .arg(format!("-{sig}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // 이미 다 죽어서 그룹이 없는 경우는 성공으로 친다.
    if err.contains("No such process") {
        Ok(())
    } else {
        Err(err)
    }
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
    let mut g = pids();
    let map = g.as_mut().unwrap();
    if let Some(rec) = map.get(id).cloned() {
        // PID 존재만으로는 부족하다. PID 는 재사용되므로 starttime 까지 맞아야
        // "그때 띄운 그 서버"다.
        if is_alive(&rec) {
            return ServerStatus::Running(rec.pid);
        }
        // 죽은 기록은 남겨두지 않는다.
        map.remove(id);
        save_running(map);
    }
    ServerStatus::Stopped
}

/// 경로를 보고 entry point 파일과 명령어를 자동 감지
pub fn auto_detect_start_command(path: &str, port: u16) -> String {
    let candidates = [
        ("run_server.py",  format!("venv/bin/python3 run_server.py {port}")),
        ("manage.py",      format!("venv/bin/python3 manage.py runserver 0.0.0.0:{port}")),
        ("app.py",         "venv/bin/python3 app.py".to_string()),
        ("main.py",        "venv/bin/python3 main.py".to_string()),
        ("server.py",      "venv/bin/python3 server.py".to_string()),
        ("wsgi.py",        format!("venv/bin/python3 -m uvicorn wsgi:app --host 0.0.0.0 --port {port}")),
        ("asgi.py",        format!("venv/bin/python3 -m uvicorn asgi:app --host 0.0.0.0 --port {port}")),
    ];
    for (file, cmd) in &candidates {
        if std::path::Path::new(&format!("{path}/{file}")).exists() {
            return cmd.clone();
        }
    }
    format!("venv/bin/python3 app.py")
}

/// venv 생성 + 패키지 설치 (동기, 시간이 걸릴 수 있음)
pub fn setup_venv(project: &VhostProject) -> Result<String, String> {
    let venv = format!("{}/venv", project.path);
    let venv_python = format!("{venv}/bin/python3");
    let pip = format!("{venv}/bin/pip");

    // venv 생성
    if !std::path::Path::new(&venv_python).exists() {
        eprintln!("[localman] venv 생성 중: {venv}");
        let r = std::process::Command::new("python3")
            .args(["-m", "venv", &venv])
            .output()
            .map_err(|e| format!("venv 생성 실패: {e}"))?;
        if !r.status.success() {
            return Err(format!("venv 생성 실패:\n{}", String::from_utf8_lossy(&r.stderr)));
        }
        eprintln!("[localman] venv 생성 완료");
    } else {
        eprintln!("[localman] venv 이미 존재");
    }

    // pip 업그레이드
    let _ = std::process::Command::new(&pip)
        .args(["install", "--upgrade", "pip"])
        .output();

    // requirements.txt 우선
    let req = format!("{}/requirements.txt", project.path);
    if std::path::Path::new(&req).exists() {
        eprintln!("[localman] pip install -r requirements.txt");
        let r = std::process::Command::new(&pip)
            .args(["install", "-r", &req])
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            return Err(format!("패키지 설치 실패:\n{}", String::from_utf8_lossy(&r.stderr)));
        }
        build_frontend_if_needed(project)?;
        return Ok(format!("패키지 설치 완료 (requirements.txt)"));
    }

    // pyproject.toml (poetry/pip editable)
    let pyproject = format!("{}/pyproject.toml", project.path);
    if std::path::Path::new(&pyproject).exists() {
        eprintln!("[localman] pip install -e .");
        // poetry 의존성이 있으면 pip install 가능하도록 먼저 pip install poetry-core
        let _ = std::process::Command::new(&pip)
            .args(["install", "pip-tools", "setuptools", "wheel"])
            .output();
        let r = std::process::Command::new(&pip)
            .args(["install", "-e", "."])
            .current_dir(&project.path)
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            // poetry인 경우 pip install . 로도 시도
            let r2 = std::process::Command::new(&pip)
                .args(["install", "."])
                .current_dir(&project.path)
                .output()
                .map_err(|e| e.to_string())?;
            if !r2.status.success() {
                return Err(format!("패키지 설치 실패:\n{}", String::from_utf8_lossy(&r2.stderr)));
            }
        }
        build_frontend_if_needed(project)?;
        return Ok("패키지 설치 완료 (pyproject.toml)".to_string());
    }

    // web/ 디렉토리가 있으면 프론트엔드 빌드
    build_frontend_if_needed(project)?;

    Ok("venv 생성 완료 (설치할 패키지 파일 없음)".to_string())
}

/// web/ 디렉토리에 package.json이 있으면 npm install + npm run build
pub fn build_frontend_if_needed(project: &VhostProject) -> Result<(), String> {
    let web_dir = format!("{}/web", project.path);
    let pkg_json = format!("{web_dir}/package.json");
    if !std::path::Path::new(&pkg_json).exists() {
        return Ok(());
    }
    eprintln!("[localman] 프론트엔드 빌드 시작: {web_dir}");

    // npm install
    let install = std::process::Command::new("npm")
        .args(["install", "--legacy-peer-deps"])
        .current_dir(&web_dir)
        .output()
        .map_err(|e| format!("npm install 실패: {e}"))?;
    if !install.status.success() {
        return Err(format!("npm install 실패:\n{}", String::from_utf8_lossy(&install.stderr)));
    }

    // npm run build
    let build = std::process::Command::new("npm")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .output()
        .map_err(|e| format!("npm run build 실패: {e}"))?;
    if !build.status.success() {
        return Err(format!("npm run build 실패:\n{}", String::from_utf8_lossy(&build.stderr)));
    }

    eprintln!("[localman] 프론트엔드 빌드 완료");
    Ok(())
}

pub fn start_server(project: &VhostProject) -> Result<u32, String> {
    // 이미 떠 있으면 또 띄우지 않는다.
    // 예전에는 localman 이 재시작되면 기존 서버를 기억하지 못해 같은 프로젝트를
    // 중복 실행했다(실제로 redholics viewer 가 7/28·7/30 두 벌 떠 있었다).
    // 이제 running.json 에서 회수하므로 여기서 걸러진다.
    if let ServerStatus::Running(pid) = server_status(&project.id) {
        eprintln!("[localman] 이미 실행 중 PID={pid} — 중복 실행하지 않음");
        return Ok(pid);
    }

    // venv가 없으면 자동 설치
    let venv_python = format!("{}/venv/bin/python3", project.path);
    if !std::path::Path::new(&venv_python).exists() {
        eprintln!("[localman] venv 없음 → 자동 설치");
        setup_venv(project)?;
    }

    let command = if project.start_command.is_empty() {
        auto_detect_start_command(&project.path, project.port)
    } else {
        project.start_command.clone()
    };

    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("실행 명령어가 올바르지 않습니다.".to_string());
    }
    eprintln!("[localman] 서버 시작: {command} in {}", project.path);
    let child = std::process::Command::new(parts[0])
        .args(&parts[1..])
        .current_dir(&project.path)
        // 자식을 새 프로세스 그룹의 리더로 만든다(pgid == 자식 pid).
        // 그래야 나중에 `kill -- -pgid` 한 방으로 그 서버가 파생시킨
        // vite·chromedriver·Chrome 까지 통째로 정리할 수 있다.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("실행 실패: {e}"))?;
    let pid = child.id();
    // child를 drop해도 프로세스는 계속 실행됨 (detach)
    std::mem::forget(child);

    // starttime 을 즉시 읽어 기록한다. 이후 이 값이 PID 재사용을 걸러낸다.
    // 못 읽으면(이미 즉사한 경우 등) 0 으로 둔다 — is_alive 가 false 가 되어
    // "죽은 것"으로 취급되므로 안전한 쪽으로 기운다.
    let (pgid, starttime) = proc_stat_fields(pid).unwrap_or((pid, 0));
    let rec = RunningProc { pid, pgid, starttime };

    let mut g = pids();
    let map = g.as_mut().unwrap();
    map.insert(project.id.clone(), rec);
    // 메모리에만 두면 localman 이 죽는 순간 사라진다. 반드시 디스크에 남긴다.
    save_running(map);

    eprintln!("[localman] 서버 시작됨 PID={pid} PGID={pgid}");
    Ok(pid)
}

/// 서버를 프로세스 그룹째 정지한다.
///
/// SIGTERM 으로 정상 종료를 먼저 시도하고, 유예 시간이 지나도 살아 있으면
/// SIGKILL 로 확실히 끝낸다. 단일 PID 가 아니라 그룹에 보내므로
/// `npm run dev` → vite → esbuild, uvicorn → chromedriver → Chrome 처럼
/// 손자까지 함께 정리된다.
pub fn stop_server(id: &str) -> Result<(), String> {
    let rec = {
        let g = pids();
        g.as_ref().and_then(|m| m.get(id).cloned())
    };
    let Some(rec) = rec else {
        return Err("실행 중인 서버가 없습니다.".to_string());
    };

    // 기록만 남고 실제로는 이미 죽은 경우. 기록만 지우고 정상 처리한다.
    if !is_alive(&rec) {
        let mut g = pids();
        let map = g.as_mut().unwrap();
        map.remove(id);
        save_running(map);
        return Ok(());
    }

    eprintln!("[localman] 서버 중지 PID={} PGID={}", rec.pid, rec.pgid);
    signal_group(rec.pgid, "TERM").map_err(|e| format!("kill 실패: {e}"))?;

    // 정상 종료를 최대 5초 기다린다.
    let mut terminated = false;
    for _ in 0..50 {
        if !is_alive(&rec) {
            terminated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if !terminated {
        eprintln!("[localman] SIGTERM 무응답 → SIGKILL PGID={}", rec.pgid);
        signal_group(rec.pgid, "KILL").map_err(|e| format!("kill -9 실패: {e}"))?;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let mut g = pids();
    let map = g.as_mut().unwrap();
    map.remove(id);
    save_running(map);
    Ok(())
}

/// pma.localhost 에 Adminer(단일 PHP 파일 DB 관리도구)를 설치하고 URL을 반환한다.
/// 이미 설치돼 있으면 vhost/hosts만 보장하고 그대로 반환한다(멱등).
pub fn ensure_adminer_site() -> Result<String, String> {
    // 1) pma 디렉토리 + adminer 파일 준비
    let mut dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.push("localman");
    dir.push("pma");
    fs::create_dir_all(&dir).map_err(|e| format!("pma 디렉토리 생성 실패: {e}"))?;

    let index = dir.join("index.php");
    if !index.exists() {
        download_adminer(&index)?;
    }

    // 2) pma.localhost vhost (PHP, DocumentRoot = pma 디렉토리)
    let pma = VhostProject {
        id: "pma".to_string(),
        name: "Adminer".to_string(),
        path: dir.to_string_lossy().to_string(),
        domain: "pma.localhost".to_string(),
        project_type: ProjectType::Php,
        port: 80,
        start_command: String::new(),
    };
    write_vhost(&pma)?;
    update_hosts(&pma.domain, true)?;

    Ok(format!("http://{}", pma.domain))
}

/// adminer.org에서 최신 Adminer를 내려받아 dest에 저장한다(curl 우선, 실패 시 wget).
fn download_adminer(dest: &Path) -> Result<(), String> {
    let url = "https://www.adminer.org/latest.php";
    let dest_str = dest.to_str().ok_or_else(|| "경로 변환 실패".to_string())?;

    let curl = std::process::Command::new("curl")
        .args(["-fsSL", url, "-o", dest_str])
        .output();
    if let Ok(o) = curl {
        if o.status.success() && dest.exists() {
            return Ok(());
        }
    }
    let wget = std::process::Command::new("wget")
        .args(["-q", url, "-O", dest_str])
        .output();
    if let Ok(o) = wget {
        if o.status.success() && dest.exists() {
            return Ok(());
        }
    }
    Err("Adminer 다운로드 실패 (curl/wget·인터넷 연결 확인)".to_string())
}

/// 기본 브라우저로 URL을 연다.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
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
    // vhost/hosts 먼저 성공한 뒤 projects.json 저장 (실패 시 반쪽 상태 방지)
    if project.project_type == ProjectType::Python {
        ensure_proxy_module()?;
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
    if type_changed {
        p.project_type = project_type.clone();
        p.port = match project_type {
            ProjectType::Python => {
                let mut candidate: u16 = 5001;
                while used_ports.contains(&candidate) { candidate += 1; }
                candidate
            }
            ProjectType::Php => 80,
        };
    }
    let updated = p.clone();
    save_projects(&list)?;
    write_vhost(&updated)?;
    eprintln!("[localman] 프로젝트 업데이트: {id} (type_changed={type_changed})");
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
    // a2enmod는 /usr/sbin에 위치
    let r = std::process::Command::new("sudo")
        .args(["-n", "/usr/sbin/a2enmod", "proxy", "proxy_http"])
        .output()
        .map_err(|e| e.to_string())?;
    if !r.status.success() {
        let err = String::from_utf8_lossy(&r.stderr).to_string();
        eprintln!("[localman] a2enmod 실패: {err}");
        return Err(
            "Apache proxy 모듈 활성화 실패.\n\
             터미널에서 한 번 실행 후 재시도하세요:\n\
             sudo a2enmod proxy proxy_http && sudo systemctl reload apache2".to_string()
        );
    }
    let reload = std::process::Command::new("sudo")
        .args(["-n", "systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    if !reload.status.success() {
        return Err(format!("Apache reload 실패: {}", String::from_utf8_lossy(&reload.stderr)));
    }
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
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            path = p.path,
            id = p.id,
        ),
        ProjectType::Python => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   ProxyPreserveHost On\n\
             \x20   ProxyPass / http://127.0.0.1:{port}/\n\
             \x20   ProxyPassReverse / http://127.0.0.1:{port}/\n\
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            port = p.port,
            id = p.id,
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
