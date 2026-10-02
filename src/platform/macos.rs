//! macOS: Homebrew(httpd·mariadb·postgresql), brew services, ps.
//!
//! Apache는 Homebrew httpd를 쓴다. 80포트를 열어야 하므로 `sudo apachectl`로 띄우고,
//! 설정 파일은 Homebrew prefix 아래(사용자 소유)라 쓰기에는 sudo가 필요 없다.
//! MariaDB/PostgreSQL은 사용자 권한의 `brew services`로 돌린다.

use super::ServiceStatus;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Homebrew 설치 위치. Apple Silicon은 /opt/homebrew, Intel은 /usr/local.
fn brew_prefix() -> &'static str {
    static PREFIX: OnceLock<String> = OnceLock::new();
    PREFIX.get_or_init(|| {
        if Path::new("/opt/homebrew/bin/brew").exists() {
            "/opt/homebrew".to_string()
        } else {
            "/usr/local".to_string()
        }
    })
}

fn brew() -> Command {
    let mut cmd = Command::new(format!("{}/bin/brew", brew_prefix()));
    // GUI 작업 스레드가 brew의 자동 업데이트·대화형 질문에 붙잡히지 않게 한다.
    cmd.env("HOMEBREW_NO_AUTO_UPDATE", "1").env("NONINTERACTIVE", "1");
    cmd
}

fn brew_installed(formula: &str) -> bool {
    brew()
        .args(["list", "--versions", formula])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// 설치된 postgresql@N 중 가장 높은 버전. 없으면 설치할 기본 버전.
fn postgres_formula() -> String {
    let installed = brew()
        .args(["list", "--formula", "-1"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    installed
        .lines()
        .filter_map(|l| l.strip_prefix("postgresql@").and_then(|v| v.parse::<u32>().ok()))
        .max()
        .map(|v| format!("postgresql@{v}"))
        .unwrap_or_else(|| "postgresql@17".to_string())
}

/// UI의 서비스 이름(리눅스 패키지명 기준)을 Homebrew formula로 바꾼다.
/// 고정된 목록만 허용해 임의 formula를 설치하지 못하게 한다.
fn formula_for(service: &str) -> Option<String> {
    match service {
        "apache2" => Some("httpd".to_string()),
        "mariadb" => Some("mariadb".to_string()),
        "postgresql" => Some(postgres_formula()),
        _ => None,
    }
}

fn apachectl() -> String {
    format!("{}/bin/apachectl", brew_prefix())
}

// ── 서비스 ─────────────────────────────────────────────────────────────

fn httpd_running() -> bool {
    // /usr/sbin/httpd(시스템 기본 Apache)는 제외하고 Homebrew httpd만 본다.
    Command::new("pgrep")
        .args(["-f", "(opt|Cellar)/httpd/.*bin/httpd"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn get_service_status(name: &str) -> ServiceStatus {
    let Some(formula) = formula_for(name) else {
        return ServiceStatus::Unknown;
    };
    if name == "apache2" {
        if !Path::new(&format!("{}/bin/httpd", brew_prefix())).exists() {
            return ServiceStatus::NotInstalled;
        }
        return if httpd_running() { ServiceStatus::Running } else { ServiceStatus::Stopped };
    }
    if !brew_installed(&formula) {
        return ServiceStatus::NotInstalled;
    }
    let out = match brew().args(["services", "info", &formula, "--json"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return ServiceStatus::Unknown,
    };
    let info: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let running = info
        .get(0)
        .and_then(|s| s.get("running"))
        .and_then(|r| r.as_bool())
        .unwrap_or(false);
    if running { ServiceStatus::Running } else { ServiceStatus::Stopped }
}

pub fn install_service(service: &str) -> Result<(), String> {
    let formula = formula_for(service)
        .ok_or_else(|| format!("설치할 수 있는 서비스가 아닙니다: {service}"))?;
    let output = brew()
        .args(["install", &formula])
        .output()
        .map_err(|e| format!("brew 실행 실패: {e}\nHomebrew가 설치돼 있는지 확인하세요."))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    if service == "apache2" {
        ensure_httpd_base()?;
    }
    Ok(())
}

pub fn toggle_service(name: &str, start: bool) -> Result<(), String> {
    let action = if start { "start" } else { "stop" };
    eprintln!("[localman] 서비스 {action}: {name}");
    let formula = formula_for(name).ok_or_else(|| format!("알 수 없는 서비스: {name}"))?;

    let output = if name == "apache2" {
        if start {
            ensure_httpd_base()?;
        }
        // 80포트를 열려면 root여야 한다. scripts/macos/install-sudoers.sh 가 이 명령만 허용한다.
        Command::new("sudo").args(["-n", &apachectl(), action]).output()
    } else {
        brew().args(["services", action, &formula]).output()
    }
    .map_err(|e| e.to_string())?;

    if output.status.success() {
        eprintln!("[localman] 서비스 {action} 완료: {name}");
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        eprintln!("[localman] 서비스 {action} 실패: {err}");
        if err.contains("password is required") {
            return Err("권한이 없습니다. scripts/macos/install-sudoers.sh 를 실행하십시오.".into());
        }
        Err(err)
    }
}

// ── Apache (Homebrew httpd) ────────────────────────────────────────────

fn httpd_conf_dir() -> PathBuf {
    PathBuf::from(format!("{}/etc/httpd", brew_prefix()))
}

fn sites_dir() -> PathBuf {
    httpd_conf_dir().join("localman")
}

fn apache_log_dir() -> String {
    format!("{}/var/log/httpd", brew_prefix())
}

/// 프로젝트별 에러 로그 경로. vhost의 `ErrorLog ${APACHE_LOG_DIR}/{id}.error.log`와 일치한다.
pub fn error_log_path(id: &str) -> String {
    format!("{}/{id}.error.log", apache_log_dir())
}

const BLOCK_BEGIN: &str = "# >>> localman (자동 생성 — 이 블록은 수정하지 마세요)";
const BLOCK_END: &str = "# <<< localman";

/// Homebrew 기본 httpd.conf를 localman이 쓸 수 있게 맞춘다. 여러 번 불러도 결과가 같다.
///
/// - Listen 8080 → 80 (리눅스와 같은 http://*.localhost 주소)
/// - User/Group을 현재 사용자로: 기본값 _www 는 홈 디렉토리의 프로젝트를 읽지 못해 403이 난다
/// - proxy·proxy_http·proxy_wstunnel·rewrite 모듈 활성화
/// - `${APACHE_LOG_DIR}` 정의: vhost 설정을 리눅스와 똑같이 쓰기 위해
/// - PHP가 설치돼 있으면 mod_php 연결
/// - localman/*.conf Include
fn ensure_httpd_base() -> Result<(), String> {
    let conf_path = httpd_conf_dir().join("httpd.conf");
    let original = match fs::read_to_string(&conf_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("Apache(httpd)가 설치돼 있지 않습니다. 서비스 탭에서 Apache2를 설치한 뒤 다시 시도하세요.".into());
        }
        Err(e) => return Err(format!("httpd.conf를 읽을 수 없습니다 ({}): {e}", conf_path.display())),
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());

    let modules = ["proxy_module", "proxy_http_module", "proxy_wstunnel_module", "rewrite_module"];
    let mut lines: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in original.lines() {
        if line == BLOCK_BEGIN {
            in_block = true;
            continue;
        }
        if in_block {
            if line == BLOCK_END {
                in_block = false;
            }
            continue;
        }
        let t = line.trim_start();
        let replaced = if t.starts_with("Listen ") {
            "Listen 80".to_string()
        } else if t.starts_with("User ") {
            format!("User {user}")
        } else if t.starts_with("Group ") {
            "Group staff".to_string()
        } else if let Some(rest) = t.strip_prefix("#LoadModule ") {
            if modules.iter().any(|m| rest.starts_with(&format!("{m} "))) {
                format!("LoadModule {rest}")
            } else {
                line.to_string()
            }
        } else {
            line.to_string()
        };
        lines.push(replaced);
    }

    let prefix = brew_prefix();
    let mut block = vec![
        BLOCK_BEGIN.to_string(),
        format!("Define APACHE_LOG_DIR {}", apache_log_dir()),
        "ServerName localhost".to_string(),
    ];
    let libphp = format!("{prefix}/opt/php/lib/httpd/modules/libphp.so");
    if Path::new(&libphp).exists() {
        block.push(format!("LoadModule php_module {libphp}"));
        block.push("<FilesMatch \\.php$>\n    SetHandler application/x-httpd-php\n</FilesMatch>".to_string());
        block.push("DirectoryIndex index.php index.html".to_string());
    }
    block.push(format!("IncludeOptional {}/*.conf", sites_dir().display()));
    block.push(BLOCK_END.to_string());

    let updated = format!("{}\n\n{}\n", lines.join("\n").trim_end(), block.join("\n"));
    if updated != original {
        fs::write(&conf_path, updated).map_err(|e| format!("httpd.conf 쓰기 실패: {e}"))?;
        eprintln!("[localman] httpd.conf 갱신: {}", conf_path.display());
    }
    fs::create_dir_all(sites_dir()).map_err(|e| format!("vhost 디렉토리 생성 실패: {e}"))?;
    fs::create_dir_all(apache_log_dir()).ok();
    Ok(())
}

fn reload_httpd() {
    if !httpd_running() {
        return;
    }
    let r = Command::new("sudo").args(["-n", &apachectl(), "graceful"]).output();
    if let Ok(o) = r {
        if !o.status.success() {
            eprintln!("[localman] httpd reload 실패: {}", String::from_utf8_lossy(&o.stderr));
        }
    }
}

/// 맥에서는 ensure_httpd_base가 필요한 모듈을 한꺼번에 켠다.
pub fn ensure_proxy_module(_need_ws: bool) -> Result<(), String> {
    ensure_httpd_base()
}

pub fn write_site(id: &str, conf: &str) -> Result<(), String> {
    ensure_httpd_base()?;
    let path = sites_dir().join(format!("{id}.conf"));
    eprintln!("[localman] vhost 파일 작성: {}", path.display());
    fs::write(&path, conf).map_err(|e| format!("vhost 파일 쓰기 실패: {e}"))?;
    reload_httpd();
    Ok(())
}

pub fn remove_site(id: &str) -> Result<(), String> {
    let path = sites_dir().join(format!("{id}.conf"));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    reload_httpd();
    Ok(())
}

// ── 프로세스 (ps) ──────────────────────────────────────────────────────

/// `ps` 로 (pgid, starttime) 을 읽는다.
///
/// 맥에는 /proc 가 없다. 시작 시각(lstart)은 초 단위 문자열이라 그대로 해시해
/// PID 재사용을 가려내는 값으로 쓴다. running.json 에 저장되므로 빌드마다
/// 값이 달라지지 않는 FNV-1a 를 쓴다.
pub fn proc_stat_fields(pid: u32) -> Option<(u32, u64)> {
    let out = Command::new("ps")
        .args(["-o", "pgid=,lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim();
    let (pgid, lstart) = s.split_once(char::is_whitespace)?;
    let pgid = pgid.parse().ok()?;
    let hash = lstart
        .trim()
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    Some((pgid, hash))
}

/// 프로세스가 존재하고 좀비가 아닌지 확인한다.
pub fn process_alive(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|o| {
            let stat = String::from_utf8_lossy(&o.stdout);
            let stat = stat.trim();
            !stat.is_empty() && !stat.starts_with('Z')
        })
        .unwrap_or(false)
}

// ── 데스크톱 ──────────────────────────────────────────────────────────

/// Finder·Dock 으로 실행한 앱은 PATH 가 `/usr/bin:/bin:/usr/sbin:/sbin` 뿐이라
/// brew·psql·npm·node 를 찾지 못한다. 로그인 셸의 PATH 를 가져오고,
/// keg-only 라 링크되지 않는 클라이언트(postgresql@N, mysql-client) 경로를 덧붙인다.
pub fn init_env() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut paths: Vec<String> = Command::new(&shell)
        // -i: nvm 등은 .zshrc 에서 PATH 를 잡으므로 대화형 설정까지 읽는다
        .args(["-ilc", "printf '__LM__%s__LM__' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let start = s.find("__LM__")? + 6;
            let end = s[start..].find("__LM__")? + start;
            Some(s[start..end].split(':').map(String::from).collect())
        })
        .unwrap_or_else(|| {
            std::env::var("PATH").unwrap_or_default().split(':').map(String::from).collect()
        });

    let prefix = brew_prefix();
    let mut extra = vec![format!("{prefix}/bin"), format!("{prefix}/sbin")];
    extra.push(format!("{prefix}/opt/{}/bin", postgres_formula()));
    extra.push(format!("{prefix}/opt/mysql-client/bin"));
    for p in extra {
        if Path::new(&p).is_dir() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    // SAFETY: main 시작 직후, 다른 스레드가 생기기 전에 한 번만 호출된다.
    unsafe { std::env::set_var("PATH", paths.join(":")) };
}

/// 기본 브라우저로 URL을 연다.
pub fn open_url(url: &str) {
    let _ = Command::new("open").arg(url).spawn();
}

/// 중복 실행 방지: 잠금 파일에 flock. 프로세스가 끝나면 커널이 자동으로 푼다.
/// (맥에는 리눅스의 abstract unix socket 이 없다)
pub fn acquire_single_instance() -> bool {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;

    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    path.push("localman");
    let _ = fs::create_dir_all(&path);
    path.push("localman.lock");
    let Ok(file) = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) else {
        return true;
    };
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        // drop되면 잠금이 풀리므로 프로세스 수명 동안 유지
        std::mem::forget(file);
        true
    } else {
        false
    }
}


