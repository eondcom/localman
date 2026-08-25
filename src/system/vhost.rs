use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::process::CommandExt; // process_group
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Running(u32), // pid
    Stopped,
}

pub fn server_status(id: &str) -> ServerStatus {
    let g = pids();
    if let Some(map) = g.as_ref() {
        if let Some(&pid) = map.get(id) {
            if process_alive(pid) {
                return ServerStatus::Running(pid);
            }
        }
    }
    ServerStatus::Stopped
}

/// 프로세스가 실제로 살아있는지 확인한다.
/// localman은 자식을 detach(mem::forget)하고 wait하지 않으므로, 종료된 자식은 좀비로 남아
/// /proc/PID 가 계속 존재한다. 그것만 보면 이미 죽은 서버가 "실행 중"으로 표시되므로
/// /proc/PID/stat 의 상태 문자가 Z(좀비)인지까지 확인한다.
pub fn process_alive(pid: u32) -> bool {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // 형식: "pid (comm) state ..." — comm에 공백·괄호가 있을 수 있어 마지막 ')' 뒤부터 읽는다
    match stat.rsplit_once(')') {
        Some((_, rest)) => rest.split_whitespace().next().map(|s| s != "Z").unwrap_or(false),
        None => false,
    }
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

/// package.json을 읽어 파싱한다.
fn read_package_json(dir: &str) -> Option<serde_json::Value> {
    let raw = fs::read_to_string(format!("{dir}/package.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// package.json의 dependencies/devDependencies에 next가 있는지
fn has_next_dep(pkg: &serde_json::Value) -> bool {
    ["dependencies", "devDependencies"]
        .iter()
        .any(|k| pkg.get(k).and_then(|d| d.get("next")).is_some())
}

/// path 하위에서 Next.js 앱이 있는 디렉토리를 찾아 path 기준 상대 경로로 돌려준다.
/// 루트 자체가 Next.js면 빈 문자열("")을 반환한다. 못 찾으면 None.
///
/// 모노레포처럼 `easyhost/app/package.json`에 Next가 있는 구조를 지원하기 위한 것으로,
/// package.json + next 의존성을 함께 확인하므로 App Router의 `app/` 디렉토리를 오탐하지 않는다.
pub fn detect_next_app_dir(path: &str) -> Option<String> {
    // 1) 루트 우선
    if let Some(pkg) = read_package_json(path) {
        if has_next_dep(&pkg) {
            return Some(String::new());
        }
    }

    // 2) 1단계 하위 디렉토리 — 흔한 이름을 먼저 보고, 나머지는 이름순
    let preferred = ["app", "web", "frontend", "client", "site", "www"];
    let mut subdirs = list_subdirs(path);
    subdirs.sort_by_key(|d| {
        (
            preferred.iter().position(|p| p == d).unwrap_or(preferred.len()),
            d.clone(),
        )
    });
    for d in &subdirs {
        if let Some(pkg) = read_package_json(&join_dir(path, d)) {
            if has_next_dep(&pkg) {
                return Some(d.clone());
            }
        }
    }

    // 3) 모노레포 관례상 apps/*, packages/* 는 한 단계 더 내려가 본다
    for parent in ["apps", "packages"] {
        let parent_path = join_dir(path, parent);
        if !Path::new(&parent_path).is_dir() {
            continue;
        }
        let mut inner = list_subdirs(&parent_path);
        inner.sort();
        for d in inner {
            let rel = format!("{parent}/{d}");
            if let Some(pkg) = read_package_json(&join_dir(path, &rel)) {
                if has_next_dep(&pkg) {
                    return Some(rel);
                }
            }
        }
    }

    None
}

/// 숨김 디렉토리와 node_modules를 제외한 1단계 하위 디렉토리 이름 목록
fn list_subdirs(path: &str) -> Vec<String> {
    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| !n.starts_with('.') && n != "node_modules" && n != "vendor")
        .collect()
}

/// Next.js dev server 실행 명령어를 만든다.
/// package.json 스크립트에 포트가 박혀 있어도 무시하도록 --port를 명시한다.
/// 이미 설치돼 있으면 로컬 바이너리를 직접 실행해 npx/PATH 의존을 없앤다.
pub fn auto_detect_next_command(dir: &str, port: u16) -> String {
    if !dir.is_empty() && Path::new(&format!("{dir}/node_modules/.bin/next")).exists() {
        format!("node_modules/.bin/next dev --port {port}")
    } else {
        format!("npx next dev --port {port}")
    }
}

/// 실행 명령의 포트 옵션(--port / -p / --port=)을 실제 할당 포트와 일치시킨다.
/// 포트 옵션이 없으면 뒤에 붙인다. vhost의 ProxyPass 포트와 어긋나 502가 나는 걸 막는다.
pub fn sync_port_in_command(cmd: &str, port: u16) -> String {
    if cmd.trim().is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = cmd.split_whitespace().map(|s| s.to_string()).collect();
    let mut found = false;
    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "--port" || parts[i] == "-p" {
            if i + 1 < parts.len() {
                parts[i + 1] = port.to_string();
                found = true;
                i += 2;
                continue;
            }
        } else if parts[i].starts_with("--port=") {
            parts[i] = format!("--port={port}");
            found = true;
        }
        i += 1;
    }
    if !found {
        parts.push("--port".to_string());
        parts.push(port.to_string());
    }
    parts.join(" ")
}

/// lockfile로 패키지 매니저를 판별한다. (pnpm / yarn / bun / npm)
fn detect_package_manager(dir: &str) -> &'static str {
    let has = |f: &str| Path::new(&format!("{dir}/{f}")).exists();
    if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun"
    } else {
        "npm"
    }
}

/// 해당 명령이 PATH에 있는지 확인
fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Next.js(Node) 프로젝트 의존성 설치 — venv의 Node 버전.
/// lockfile에 맞는 매니저를 쓰고, 매니저가 없으면 corepack, 그래도 없으면 npm으로 폴백한다.
pub fn setup_node_modules(project: &VhostProject) -> Result<String, String> {
    let dir = project.work_dir();
    if !Path::new(&format!("{dir}/package.json")).exists() {
        return Err(format!(
            "package.json이 없습니다: {dir}\n하위 디렉토리 설정을 확인하세요."
        ));
    }

    let pm = detect_package_manager(&dir);
    eprintln!("[localman] 패키지 매니저: {pm} ({dir})");

    // (프로그램, 앞에 붙는 인자들) 결정
    let (program, prefix): (&str, Vec<&str>) = if pm == "npm" || command_exists(pm) {
        (pm, vec![])
    } else if command_exists("corepack") {
        eprintln!("[localman] {pm} 없음 → corepack 경유");
        ("corepack", vec![pm])
    } else {
        eprintln!("[localman] {pm} / corepack 모두 없음 → npm 폴백");
        ("npm", vec![])
    };

    let mut args: Vec<&str> = prefix.clone();
    args.push("install");

    eprintln!("[localman] {program} {} 실행", args.join(" "));
    let r = std::process::Command::new(program)
        .args(&args)
        // corepack이 최초 실행 시 대화형 다운로드 동의를 물으면 GUI에서 멈추므로 비활성화.
        // CI=1은 설정하지 않는다 — pnpm/yarn이 그걸 보고 frozen-lockfile을 강제해서,
        // lockfile이 package.json보다 오래되면 설치가 통째로 실패한다.
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("{program} 실행 실패: {e}\nPATH에 Node.js가 있는지 확인하세요."))?;

    if !r.status.success() {
        let err = String::from_utf8_lossy(&r.stderr);
        let tail: String = err.lines().rev().take(15).collect::<Vec<_>>()
            .into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!("의존성 설치 실패 ({program}):\n{tail}"));
    }

    let via = if program == "corepack" { format!("corepack {pm}") } else { program.to_string() };
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
    if let Some(pkgs) = ignored_build_scripts(&out) {
        // 설치는 성공했지만 반쪽이다. postinstall로 실제 코드를 받아오는 패키지
        // (@heroui-pro/react 등)가 여기 걸리면 import가 조용히 깨진다.
        return Ok(format!(
            "의존성 설치 완료 ({via}) — 다만 빌드 스크립트가 차단된 패키지가 있습니다: {pkgs}\n\
             postinstall로 코드를 받아오는 패키지면 import가 실패합니다. \
             해당 폴더에서 `pnpm approve-builds` 실행 후 다시 설치하세요."
        ));
    }
    Ok(format!("의존성 설치 완료 ({via})"))
}

/// pnpm이 "빌드 스크립트를 무시했다"고 알릴 때 그 패키지 목록을 뽑는다.
/// 설치는 성공(exit 0)으로 끝나므로 이 경고를 놓치면 반쪽 설치를 모른 채 넘어간다.
fn ignored_build_scripts(output: &str) -> Option<String> {
    let plain = strip_ansi(output);
    let lower = plain.to_lowercase();
    if !lower.contains("ignored build scripts") && !lower.contains("build scripts were ignored") {
        return None;
    }
    // 경고 줄 뒤에 붙는 패키지 이름들을 모은다 (형식이 버전마다 달라 느슨하게 훑는다)
    let names: Vec<&str> = plain
        .lines()
        .skip_while(|l| !l.to_lowercase().contains("ignored build scripts")
            && !l.to_lowercase().contains("build scripts were ignored"))
        .take(4)
        .flat_map(|l| l.split(|c: char| c == ',' || c == ':'))
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| {
            !s.is_empty()
                && (s.starts_with('@') || s.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
                && !s.contains(' ')
                && s.len() < 60
        })
        .collect();
    if names.is_empty() {
        Some("(이름 확인 불가 — 설치 로그를 확인하세요)".to_string())
    } else {
        Some(names.join(", "))
    }
}

/// 터미널 색상 escape sequence 제거 (pnpm 출력엔 ANSI 코드가 섞여 있다)
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ ... <최종 바이트> 형태를 통째로 건너뛴다
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 타입에 맞는 의존성 설치 진입점
pub fn setup_project(project: &VhostProject) -> Result<String, String> {
    match project.project_type {
        ProjectType::Python => setup_venv(project),
        ProjectType::NextJs => setup_node_modules(project),
        ProjectType::Php => Ok("PHP 프로젝트는 설치할 의존성이 없습니다.".to_string()),
    }
}

/// 의존성이 이미 설치돼 있는지 (UI의 "패키지설치" 버튼 노출 판단용)
pub fn deps_ready(project: &VhostProject) -> bool {
    let dir = project.work_dir();
    match project.project_type {
        ProjectType::Python => Path::new(&format!("{dir}/venv/bin/python3")).exists(),
        ProjectType::NextJs => Path::new(&format!("{dir}/node_modules")).exists(),
        ProjectType::Php => true,
    }
}

/// venv 생성 + 패키지 설치 (동기, 시간이 걸릴 수 있음)
pub fn setup_venv(project: &VhostProject) -> Result<String, String> {
    let base = project.work_dir();
    let venv = format!("{base}/venv");
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
    let req = format!("{base}/requirements.txt");
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
    let pyproject = format!("{base}/pyproject.toml");
    if std::path::Path::new(&pyproject).exists() {
        eprintln!("[localman] pip install -e .");
        // poetry 의존성이 있으면 pip install 가능하도록 먼저 pip install poetry-core
        let _ = std::process::Command::new(&pip)
            .args(["install", "pip-tools", "setuptools", "wheel"])
            .output();
        let r = std::process::Command::new(&pip)
            .args(["install", "-e", "."])
            .current_dir(&base)
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            // poetry인 경우 pip install . 로도 시도
            let r2 = std::process::Command::new(&pip)
                .args(["install", "."])
                .current_dir(&base)
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
    let web_dir = format!("{}/web", project.work_dir());
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
    let dir = project.work_dir();
    if !std::path::Path::new(&dir).is_dir() {
        return Err(format!("디렉토리가 없습니다: {dir}"));
    }

    // 의존성이 없으면 자동 설치 (Python: venv, Next.js: node_modules)
    match project.project_type {
        ProjectType::Php => {
            return Err("PHP 프로젝트는 Apache가 직접 서빙하므로 dev server가 없습니다.".to_string());
        }
        _ if !deps_ready(project) => {
            eprintln!("[localman] 의존성 없음 → 자동 설치");
            setup_project(project)?;
        }
        _ => {}
    }

    let command = if project.start_command.is_empty() {
        match project.project_type {
            ProjectType::NextJs => auto_detect_next_command(&dir, project.port),
            _ => auto_detect_start_command(&dir, project.port),
        }
    } else {
        project.start_command.clone()
    };

    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("실행 명령어가 올바르지 않습니다.".to_string());
    }
    eprintln!("[localman] 서버 시작: {command} in {dir}");
    // npx/npm 같은 래퍼는 실제 dev server를 자식 프로세스로 띄운다. 자식을 새 프로세스
    // 그룹의 리더로 만들면(PGID = 자식 PID) 중지할 때 손자까지 한 번에 정리할 수 있다.
    // 그러지 않으면 래퍼만 죽고 dev server가 포트를 계속 물고 있어 재시작이 실패한다.
    let child = std::process::Command::new(parts[0])
        .args(&parts[1..])
        .current_dir(&dir)
        .process_group(0)
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
        // start_server가 process_group(0)으로 띄웠으므로 PGID = PID 이다. 그룹 전체를
        // 종료해 npx가 만든 실제 dev server까지 함께 정리한다. 그룹이 없으면 단일 PID로 재시도.
        //
        // `--`가 없으면 procps의 kill이 "-<pid>"를 옵션으로 오해해 아무것도 죽이지 않으면서
        // 종료코드 0을 돌려준다(조용한 실패). 옵션 종료를 반드시 명시해야 한다.
        let group = std::process::Command::new("kill")
            .args(["-TERM", "--", &format!("-{pid}")])
            .output()
            .map_err(|e| e.to_string())?;
        if !group.status.success() {
            let r = std::process::Command::new("kill")
                .arg(pid.to_string())
                .output()
                .map_err(|e| e.to_string())?;
            if !r.status.success() {
                let err = String::from_utf8_lossy(&r.stderr).to_string();
                return Err(format!("kill 실패: {err}"));
            }
        }
        let mut g = pids();
        g.as_mut().unwrap().remove(id);
        Ok(())
    } else {
        Err("실행 중인 서버가 없습니다.".to_string())
    }
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
        app_dir: String::new(),
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

/// 프록시에 필요한 Apache 모듈을 보장한다.
/// need_ws=true면 웹소켓 터널링(proxy_wstunnel)까지 켠다 — Next.js dev의 HMR에 필요.
fn ensure_proxy_module(need_ws: bool) -> Result<(), String> {
    let out = std::process::Command::new("apache2ctl")
        .args(["-M"])
        .output()
        .map_err(|e| e.to_string())?;
    let modules = String::from_utf8_lossy(&out.stdout);

    let mut needed: Vec<&str> = Vec::new();
    if !modules.contains("proxy_http_module") {
        needed.push("proxy");
        needed.push("proxy_http");
    }
    if need_ws {
        // 웹소켓 터널링 + Upgrade 헤더 판별에 필요
        if !modules.contains("proxy_wstunnel_module") {
            needed.push("proxy_wstunnel");
        }
        if !modules.contains("rewrite_module") {
            needed.push("rewrite");
        }
    }
    if needed.is_empty() {
        return Ok(());
    }

    eprintln!("[localman] Apache 모듈 활성화 중: {}", needed.join(" "));
    // a2enmod는 /usr/sbin에 위치
    let mut args = vec!["-n", "/usr/sbin/a2enmod"];
    args.extend(needed.iter());
    let r = std::process::Command::new("sudo")
        .args(&args)
        .output()
        .map_err(|e| e.to_string())?;
    if !r.status.success() {
        let err = String::from_utf8_lossy(&r.stderr).to_string();
        eprintln!("[localman] a2enmod 실패: {err}");
        return Err(format!(
            "Apache proxy 모듈 활성화 실패.\n\
             터미널에서 한 번 실행 후 재시도하세요:\n\
             sudo a2enmod {} && sudo systemctl reload apache2",
            needed.join(" ")
        ));
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
            // public/ 같은 하위 디렉토리를 DocumentRoot로 쓸 수 있게 work_dir 사용
            path = p.work_dir(),
            id = p.id,
        ),
        // Next.js dev server는 HMR에 웹소켓을 쓴다. HMR 경로는 Next 버전과 번들러
        // (webpack/turbopack)에 따라 달라지므로 경로를 고정하지 않고 Upgrade 헤더로 판별한다.
        // RewriteRule [P]가 먼저 처리되고, 웹소켓이 아닌 요청만 아래 ProxyPass로 내려간다.
        ProjectType::NextJs => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   ProxyPreserveHost On\n\
             \x20   ProxyRequests Off\n\
             \x20   RewriteEngine On\n\
             \x20   RewriteCond %{{HTTP:Upgrade}} =websocket [NC]\n\
             \x20   RewriteRule ^/?(.*) ws://127.0.0.1:{port}/$1 [P,L]\n\
             \x20   ProxyPass / http://127.0.0.1:{port}/\n\
             \x20   ProxyPassReverse / http://127.0.0.1:{port}/\n\
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            port = p.port,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 임시 디렉토리 (tempfile 의존성 없이)
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!("localman-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn path(&self) -> String {
            self.0.to_string_lossy().to_string()
        }
        /// 하위 경로에 파일을 만든다 (상위 디렉토리 자동 생성)
        fn write(&self, rel: &str, body: &str) {
            let full = self.0.join(rel);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, body).unwrap();
        }
        fn mkdir(&self, rel: &str) {
            fs::create_dir_all(self.0.join(rel)).unwrap();
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const NEXT_PKG: &str = r#"{"dependencies":{"next":"16.3.0","react":"19.2.6"}}"#;

    fn project(t: ProjectType, path: &str, app_dir: &str, port: u16) -> VhostProject {
        VhostProject {
            id: "demo".into(),
            name: "Demo".into(),
            path: path.into(),
            domain: "demo.localhost".into(),
            project_type: t,
            port,
            start_command: String::new(),
            app_dir: app_dir.into(),
        }
    }

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

    #[test]
    fn sync_port_rewrites_existing_flags() {
        assert_eq!(sync_port_in_command("npx next dev --port 3003", 5001), "npx next dev --port 5001");
        assert_eq!(sync_port_in_command("npx next dev --port=3003", 5001), "npx next dev --port=5001");
        assert_eq!(sync_port_in_command("npm run dev -- -p 3000", 5002), "npm run dev -- -p 5002");
    }

    #[test]
    fn sync_port_appends_when_missing() {
        assert_eq!(sync_port_in_command("npx next dev", 5001), "npx next dev --port 5001");
        // 빈 명령은 그대로 비워 둔다 (start_server가 자동 감지하도록)
        assert_eq!(sync_port_in_command("   ", 5001), "");
    }

    #[test]
    fn detects_next_app_in_subdirectory() {
        let t = TmpDir::new("sub");
        // easyhost 구조: 루트에는 package.json이 없고 app/ 하위에 Next 앱이 있다
        t.write("app/package.json", NEXT_PKG);
        t.write("landing/package.json", r#"{"dependencies":{"esbuild":"0.2.0"}}"#);
        assert_eq!(detect_next_app_dir(&t.path()), Some("app".to_string()));
    }

    #[test]
    fn detects_next_at_root_and_ignores_app_router_dir() {
        let t = TmpDir::new("root");
        t.write("package.json", NEXT_PKG);
        // App Router의 app/ 디렉토리는 package.json이 없으므로 오탐하면 안 된다
        t.mkdir("app");
        assert_eq!(detect_next_app_dir(&t.path()), Some(String::new()));
    }

    #[test]
    fn detects_next_in_monorepo_apps_dir() {
        let t = TmpDir::new("mono");
        t.write("apps/web/package.json", NEXT_PKG);
        assert_eq!(detect_next_app_dir(&t.path()), Some("apps/web".to_string()));
    }

    #[test]
    fn returns_none_without_next_dependency() {
        let t = TmpDir::new("none");
        t.write("package.json", r#"{"dependencies":{"vite":"5.0.0"}}"#);
        t.write("web/package.json", r#"{"dependencies":{"express":"4.0.0"}}"#);
        assert_eq!(detect_next_app_dir(&t.path()), None);
    }

    #[test]
    fn next_vhost_proxies_websocket_before_root() {
        let conf = build_vhost_conf(&project(ProjectType::NextJs, "/srv/x", "app", 5005));
        // HMR 경로를 고정하지 않고 Upgrade 헤더로 판별한다 (webpack/turbopack 모두 대응)
        assert!(conf.contains("RewriteCond %{HTTP:Upgrade} =websocket [NC]"));
        assert!(conf.contains("RewriteRule ^/?(.*) ws://127.0.0.1:5005/$1 [P,L]"));
        assert!(conf.contains("ProxyPass / http://127.0.0.1:5005/"));
        // 웹소켓 규칙이 루트 프록시보다 먼저 와야 매칭된다
        let ws = conf.find("RewriteRule").unwrap();
        let root = conf.find("ProxyPass / http").unwrap();
        assert!(ws < root, "웹소켓 규칙이 루트 프록시보다 뒤에 있습니다");
    }

    #[test]
    fn php_vhost_uses_app_dir_as_document_root() {
        let conf = build_vhost_conf(&project(ProjectType::Php, "/srv/laravel", "public", 80));
        assert!(conf.contains("DocumentRoot /srv/laravel/public"));
        // 하위 디렉토리가 없으면 경로 그대로
        let conf2 = build_vhost_conf(&project(ProjectType::Php, "/srv/plain", "", 80));
        assert!(conf2.contains("DocumentRoot /srv/plain\n"));
    }

    #[test]
    fn python_vhost_unchanged() {
        let conf = build_vhost_conf(&project(ProjectType::Python, "/srv/x", "", 5001));
        assert!(conf.contains("ProxyPass / http://127.0.0.1:5001/"));
        assert!(!conf.contains("ws://"));
    }

    #[test]
    fn next_command_prefers_local_binary_when_installed() {
        let t = TmpDir::new("bin");
        assert_eq!(auto_detect_next_command(&t.path(), 5001), "npx next dev --port 5001");
        t.write("node_modules/.bin/next", "#!/usr/bin/env node\n");
        assert_eq!(
            auto_detect_next_command(&t.path(), 5001),
            "node_modules/.bin/next dev --port 5001"
        );
    }

    #[test]
    fn deps_ready_checks_the_right_marker_per_type() {
        let t = TmpDir::new("deps");
        let path = t.path();
        assert!(deps_ready(&project(ProjectType::Php, &path, "", 80)));
        assert!(!deps_ready(&project(ProjectType::NextJs, &path, "app", 5001)));
        t.mkdir("app/node_modules");
        assert!(deps_ready(&project(ProjectType::NextJs, &path, "app", 5001)));
        assert!(!deps_ready(&project(ProjectType::Python, &path, "", 5001)));
        t.write("venv/bin/python3", "");
        assert!(deps_ready(&project(ProjectType::Python, &path, "", 5001)));
    }

    fn pgrep(args: &[&str]) -> Vec<u32> {
        let out = match std::process::Command::new("pgrep").args(args).output() {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    /// npx가 실제 dev server를 자식으로 띄우는 구조에서, 중지하면 자식까지 정리돼야 한다.
    /// (래퍼만 죽으면 dev server가 포트를 물고 있어 재시작이 실패한다)
    #[test]
    fn stop_server_terminates_whole_process_group() {
        use std::os::unix::fs::PermissionsExt;

        let t = TmpDir::new("group");
        t.write("venv/bin/python3", ""); // deps_ready 통과용 마커
        t.write("wrapper.sh", "#!/bin/sh\nsleep 91 &\nwait\n");
        let script = t.0.join("wrapper.sh");
        let mut perm = fs::metadata(&script).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&script, perm).unwrap();

        let mut p = project(ProjectType::Python, &t.path(), "", 5099);
        p.id = "group-test".into();
        p.start_command = "./wrapper.sh".into();

        let pid = start_server(&p).expect("서버 시작 실패");

        // 래퍼가 띄운 자식(실제 dev server에 해당)의 PID를 잡아둔다
        let mut child = 0u32;
        assert!(
            wait_until(|| match pgrep(&["-P", &pid.to_string()]).first() {
                Some(&c) => { child = c; true }
                None => false,
            }),
            "래퍼가 자식 프로세스를 띄우지 않아 검증할 수 없습니다"
        );
        assert_eq!(server_status(&p.id), ServerStatus::Running(pid));

        stop_server(&p.id).expect("중지 실패");

        // 래퍼뿐 아니라 자식까지 정리돼야 한다
        assert!(wait_until(|| !process_alive(child)),
            "자식 프로세스가 살아남았습니다 (그룹 종료 실패)");
        assert!(wait_until(|| !process_alive(pid)), "래퍼 프로세스가 살아남았습니다");
        assert_eq!(server_status(&p.id), ServerStatus::Stopped);

        // 실패에 대비한 정리
        let _ = std::process::Command::new("kill").arg(child.to_string()).output();
    }

    /// pnpm은 빌드 스크립트를 차단해도 설치를 성공(exit 0)으로 끝낸다.
    /// 이 경고를 놓치면 @heroui-pro/react처럼 postinstall로 코드를 받아오는 패키지가
    /// 껍데기만 설치된 채 넘어가 import가 깨진다. (실제 pnpm 11 출력으로 검증)
    #[test]
    fn detects_pnpm_ignored_build_scripts() {
        let real = "\u{1b}[41m\u{1b}[31m[\u{1b}[39m\u{1b}[49m\u{1b}[41m\u{1b}[30mERR_PNPM_IGNORED_BUILDS\u{1b}[39m\u{1b}[49m\u{1b}[41m\u{1b}[31m]\u{1b}[39m\u{1b}[49m \u{1b}[31mIgnored build scripts: esbuild@0.28.2\u{1b}[39m\n\nRun \"pnpm approve-builds\" to pick which dependencies should be allowed to run scripts.\n";
        assert_eq!(ignored_build_scripts(real).as_deref(), Some("esbuild@0.28.2"));
    }

    #[test]
    fn lists_every_ignored_package() {
        let out = "Ignored build scripts: @heroui-pro/react@1.0.0-beta.8, unrs-resolver@1.7.2.\n\
                   Run \"pnpm approve-builds\" to pick which dependencies should be allowed.\n";
        let got = ignored_build_scripts(out).unwrap();
        assert!(got.contains("@heroui-pro/react@1.0.0-beta.8"), "got: {got}");
        assert!(got.contains("unrs-resolver@1.7.2"), "got: {got}");
    }

    #[test]
    fn no_warning_on_clean_install() {
        let out = "Packages: +2\ndependencies:\n+ esbuild 0.28.2\nDone in 3.6s using pnpm v11.23.0\n";
        assert_eq!(ignored_build_scripts(out), None);
    }

    #[test]
    fn strips_ansi_escape_codes() {
        assert_eq!(strip_ansi("\u{1b}[32m✓\u{1b}[39m ok"), "✓ ok");
        assert_eq!(strip_ansi("plain"), "plain");
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
