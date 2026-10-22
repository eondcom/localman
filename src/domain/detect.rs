use std::fs;
use std::path::Path;

use super::project::join_dir;

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
pub(crate) fn detect_package_manager(dir: &str) -> &'static str {
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
/// 프로젝트 폴더에 깔린 PHP CMS 종류 (프로젝트 목록 분류용)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cms {
    WordPress,
    Gnuboard,
    Zeroboard,
    /// 라이믹스와 XE 는 한 갈래로 묶는다
    RhymixXe,
}

impl Cms {
    pub const ALL: [Cms; 4] = [Cms::WordPress, Cms::Gnuboard, Cms::Zeroboard, Cms::RhymixXe];

    pub fn label(self) -> &'static str {
        match self {
            Cms::WordPress => "WordPress",
            Cms::Gnuboard => "Gnuboard",
            Cms::Zeroboard => "Zeroboard",
            Cms::RhymixXe => "Rhymix/XE",
        }
    }
}

/// 폴더의 표식 파일로 CMS 를 알아낸다. 파일 몇 개의 존재만 보므로 가볍다.
/// 사이트 폴더 바로 아래와, DocumentRoot 로 흔히 쓰는 public_html·www·html 하위를 본다.
pub fn detect_cms(dirs: &[&str]) -> Option<Cms> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for d in dirs.iter().filter(|d| !d.is_empty()) {
        let base = Path::new(d);
        roots.push(base.to_path_buf());
        for sub in ["public_html", "www", "html"] {
            roots.push(base.join(sub));
        }
    }
    roots.iter().find_map(|r| cms_at(r))
}

fn cms_at(root: &Path) -> Option<Cms> {
    let has = |f: &str| root.join(f).exists();
    if has("wp-config.php") || has("wp-load.php") || has("wp-includes/version.php") {
        Some(Cms::WordPress)
    } else if has("common/autoload.php") || has("files/config/db.config.php") || has("config/config.inc.php") {
        Some(Cms::RhymixXe)
    } else if has("bbs/board.php") && (has("lib/common.lib.php") || has("data/dbconfig.php") || has("dbconfig.php")) {
        // 그누보드 4·5 — bbs/ 와 공용 함수 파일
        Some(Cms::Gnuboard)
    } else if has("zboard.php") {
        // 제로보드 4 (제로보드XE 는 XE 표식으로 위에서 잡힌다)
        Some(Cms::Zeroboard)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::TmpDir;

    const NEXT_PKG: &str = r#"{"dependencies":{"next":"16.3.0","react":"19.2.6"}}"#;

    #[test]
    fn detects_cms_kinds() {
        let t = TmpDir::new("cms-wp");
        t.write("wp-config.php", "<?php");
        assert_eq!(detect_cms(&[&t.path()]), Some(Cms::WordPress));

        let t = TmpDir::new("cms-rx");
        t.write("common/autoload.php", "<?php");
        assert_eq!(detect_cms(&[&t.path()]), Some(Cms::RhymixXe));

        let t = TmpDir::new("cms-g5");
        t.write("bbs/board.php", "<?php");
        t.write("lib/common.lib.php", "<?php");
        assert_eq!(detect_cms(&[&t.path()]), Some(Cms::Gnuboard));

        // 제로보드는 public_html 아래에 있어도 잡는다
        let t = TmpDir::new("cms-zb");
        t.write("public_html/zboard.php", "<?php");
        assert_eq!(detect_cms(&["", &t.path()]), Some(Cms::Zeroboard));

        let t = TmpDir::new("cms-none");
        t.write("index.php", "<?php");
        assert_eq!(detect_cms(&[&t.path()]), None);
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
    fn next_command_prefers_local_binary_when_installed() {
        let t = TmpDir::new("bin");
        assert_eq!(auto_detect_next_command(&t.path(), 5001), "npx next dev --port 5001");
        t.write("node_modules/.bin/next", "#!/usr/bin/env node\n");
        assert_eq!(
            auto_detect_next_command(&t.path(), 5001),
            "node_modules/.bin/next dev --port 5001"
        );
    }
}
