//! 데비안/우분투 계열 리눅스: systemd, apt, a2enmod, /etc/apache2, /proc.

use super::ServiceStatus;
use std::fs;
use std::process::Command;

// ── 서비스 (systemd / apt) ─────────────────────────────────────────────

/// `is-active`의 종료 코드만으로는 미설치와 다른 오류를 구별할 수 없으므로 유닛 파일을 확인한다.
fn unit_exists(name: &str) -> bool {
    let unit_name = format!("{name}.service");

    Command::new("systemctl")
        .args(["cat", &unit_name])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn get_service_status(name: &str) -> ServiceStatus {
    let output = Command::new("systemctl")
        .args(["is-active", name])
        .output();

    match output {
        Ok(out) => {
            let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if status == "active" {
                ServiceStatus::Running
            } else if !unit_exists(name) {
                // 종료 코드 4는 미설치와 다른 실패에도 같으므로, 유닛 파일 부재만 미설치로 표시한다.
                ServiceStatus::NotInstalled
            } else {
                ServiceStatus::Stopped
            }
        }
        Err(_) => ServiceStatus::Unknown,
    }
}

/// 서비스별 패키지를 고정해 UI나 외부 입력으로 임의 패키지를 root 권한으로 설치하지 못하게 한다.
pub fn install_package_for(service: &str) -> Option<&'static str> {
    match service {
        "postgresql" => Some("postgresql"),
        "mariadb" => Some("mariadb-server"),
        "apache2" => Some("apache2"),
        _ => None,
    }
}

pub fn install_service(service: &str) -> Result<(), String> {
    let package = install_package_for(service)
        .ok_or_else(|| format!("설치할 수 있는 서비스가 아닙니다: {service}"))?;

    let output = Command::new("sudo")
        .args(["-n", "/usr/bin/apt-get", "install", "-y", package])
        // apt의 대화형 질문이 GUI 작업 스레드를 무한히 붙잡지 않게 한다.
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let error = String::from_utf8_lossy(&output.stderr);
    // `sudo -n`의 권한 오류를 그대로 노출하면 사용자가 sudoers 재설치 방법을 알기 어렵다.
    if error.contains("password is required") || error.contains("a password is required") {
        return Err("설치 권한이 없습니다. install-sudoers.sh 를 다시 실행하십시오.".into());
    }

    Err(error.to_string())
}

pub fn toggle_service(name: &str, start: bool) -> Result<(), String> {
    let action = if start { "start" } else { "stop" };
    eprintln!("[localman] 서비스 {action}: {name}");
    let output = Command::new("sudo")
        .args(["-n", "systemctl", action, name])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        eprintln!("[localman] 서비스 {action} 완료: {name}");
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        eprintln!("[localman] 서비스 {action} 실패: {err}");
        Err(err)
    }
}

// ── Apache (sites-available / a2enmod) ─────────────────────────────────

/// Apache 로그 디렉토리. ${APACHE_LOG_DIR}는 데비안/우분투에서 이 경로로 해석된다.
const APACHE_LOG_DIR: &str = "/var/log/apache2";

/// 프로젝트별 에러 로그 경로. vhost의 `ErrorLog ${APACHE_LOG_DIR}/{id}.error.log`와 일치한다.
pub fn error_log_path(id: &str) -> String {
    format!("{APACHE_LOG_DIR}/{id}.error.log")
}

/// 프록시에 필요한 Apache 모듈을 보장한다.
/// need_ws=true면 웹소켓 터널링(proxy_wstunnel)까지 켠다 — Next.js dev의 HMR에 필요.
pub fn ensure_proxy_module(need_ws: bool) -> Result<(), String> {
    let out = Command::new("apache2ctl")
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
    let r = Command::new("sudo")
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
    let reload = Command::new("sudo")
        .args(["-n", "systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    if !reload.status.success() {
        return Err(format!("Apache reload 실패: {}", String::from_utf8_lossy(&reload.stderr)));
    }
    Ok(())
}

/// vhost 설정을 sites-available에 쓰고 활성화한 뒤 Apache를 reload한다.
pub fn write_site(id: &str, conf: &str) -> Result<(), String> {
    let conf_path = format!("/etc/apache2/sites-available/{id}.conf");
    let enable_path = format!("/etc/apache2/sites-enabled/{id}.conf");
    eprintln!("[localman] vhost 파일 작성: {conf_path}");

    let tmp_path = format!("/tmp/localman_vhost_{id}.conf");
    fs::write(&tmp_path, conf).map_err(|e| format!("임시 파일 쓰기 실패: {e}"))?;
    let cp_out = Command::new("sudo")
        .args(["cp", &tmp_path, &conf_path])
        .output()
        .map_err(|e| e.to_string())?;
    if !cp_out.status.success() {
        let err = String::from_utf8_lossy(&cp_out.stderr).to_string();
        eprintln!("[localman] vhost cp 실패: {err}");
        return Err(format!("vhost 파일 쓰기 실패: {err}"));
    }
    Command::new("sudo")
        .args(["ln", "-sf", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    Command::new("sudo")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_site(id: &str) -> Result<(), String> {
    let conf_path = format!("/etc/apache2/sites-available/{id}.conf");
    let enable_path = format!("/etc/apache2/sites-enabled/{id}.conf");
    Command::new("sudo")
        .args(["rm", "-f", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    Command::new("sudo")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── 프로세스 (/proc) ───────────────────────────────────────────────────

/// `/proc/<pid>/stat` 에서 (pgrp, starttime) 을 읽는다.
///
/// comm 필드는 괄호로 감싸여 있고 그 안에 공백·괄호가 들어갈 수 있어
/// 앞에서부터 자르면 깨진다. 마지막 ')' 뒤부터 파싱한다.
pub fn proc_stat_fields(pid: u32) -> Option<(u32, u64)> {
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

/// 프로세스가 존재하고 좀비가 아닌지 확인한다.
///
/// 회수 스레드가 wait 하기 전 잠깐 동안은 종료된 자식이 좀비로 남아
/// /proc/PID 가 계속 존재한다. 그것만 보면 이미 죽은 서버가 "실행 중"으로 표시된다.
/// (같은 프로세스인지까지 확인하려면 starttime을 함께 보는 `is_alive`를 쓸 것)
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

// ── 데스크톱 ──────────────────────────────────────────────────────────

/// GUI로 실행해도 셸과 같은 환경을 쓰도록 맞춘다. 리눅스 데스크톱은 이미 그렇다.
pub fn init_env() {}

/// 기본 브라우저로 URL을 연다.
pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// 중복 실행 방지: abstract unix socket을 잠금으로 사용 (프로세스 종료 시 커널이 자동 해제)
pub fn acquire_single_instance() -> bool {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let Ok(addr) = SocketAddr::from_abstract_name(b"localman.single-instance") else {
        return true;
    };
    match UnixListener::bind_addr(&addr) {
        Ok(listener) => {
            // drop되면 잠금이 풀리므로 프로세스 수명 동안 유지
            std::mem::forget(listener);
            true
        }
        Err(_) => false,
    }
}
