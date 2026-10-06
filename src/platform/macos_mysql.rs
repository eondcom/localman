//! 맥: Homebrew 가 미리 빌드한 MariaDB 를 주지 않는 경우(Intel 맥 등)를 위해
//! MySQL 8.4 LTS 공식 바이너리를 앱 전용 폴더에 설치해 DB 서버로 쓴다.
//!
//! - 설치: cdn.mysql.com 의 8.4 계열 최신 패치를 찾아 <데이터 폴더>/tools/mysql 에 푼다 (sudo 불필요)
//! - 데이터: <데이터 폴더>/mysql-data, 포트 3306, 소켓 /tmp/mysql.sock (맥 mysql 클라이언트 기본값)
//! - 처음 설치할 때 root 비밀번호를 "root" 로 정한다 (DB 탭 안내와 MAMP 기본값과 같다)

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const SERIES: &str = "8.4";
pub const SOCKET: &str = "/tmp/mysql.sock";
const PORT: &str = "3306";
const ROOT_PASSWORD: &str = "root";

fn data_root() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    p
}

pub fn home() -> PathBuf {
    data_root().join("tools").join("mysql")
}

pub fn bin_dir() -> PathBuf {
    home().join("bin")
}

fn datadir() -> PathBuf {
    data_root().join("mysql-data")
}

pub fn installed() -> bool {
    bin_dir().join("mysqld").exists()
}

fn mysqld_args() -> Vec<String> {
    let d = data_root();
    vec![
        format!("--basedir={}", home().display()),
        format!("--datadir={}", datadir().display()),
        format!("--port={PORT}"),
        format!("--socket={SOCKET}"),
        format!("--pid-file={}", d.join("mysql.pid").display()),
        format!("--log-error={}", d.join("mysql.err").display()),
        "--bind-address=127.0.0.1".into(),
        // X 프로토콜(33060)은 기본으로 모든 네트워크에 열린다. 로컬맨은 쓰지 않으니 끈다
        // (같은 와이파이의 다른 기기가 접속을 시도하지 못하게).
        "--mysqlx=OFF".into(),
    ]
}

/// 서버가 응답하는지 (인증 실패여도 서버가 살아 있으면 ping 은 성공한다)
pub fn running() -> bool {
    if !installed() {
        return false;
    }
    Command::new(bin_dir().join("mysqladmin"))
        .args(["--socket", SOCKET, "-uroot", "ping"])
        .env("MYSQL_PWD", ROOT_PASSWORD)
        .stdin(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wait_until(up: bool) -> bool {
    for _ in 0..60 {
        if running() == up {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

pub fn start() -> Result<(), String> {
    if running() {
        return Ok(());
    }
    if !installed() {
        return Err("MySQL이 설치돼 있지 않습니다.".into());
    }
    let args = mysqld_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let bin = bin_dir().join("mysqld");
    super::spawn_in_new_group(&bin.to_string_lossy(), &args, &home().to_string_lossy())?;
    if wait_until(true) {
        Ok(())
    } else {
        Err(format!("MySQL이 시작되지 않았습니다. 로그: {}", data_root().join("mysql.err").display()))
    }
}

pub fn stop() -> Result<(), String> {
    if !running() {
        return Ok(());
    }
    let out = Command::new(bin_dir().join("mysqladmin"))
        .args(["--socket", SOCKET, "-uroot", "shutdown"])
        .env("MYSQL_PWD", ROOT_PASSWORD)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "MySQL 중지 실패 (root 비밀번호를 바꿨다면 데이터베이스 탭에서 중지하세요): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if wait_until(false) { Ok(()) } else { Err("MySQL이 멈추지 않았습니다.".into()) }
}

fn curl_ok(url: &str) -> bool {
    Command::new("curl")
        .args(["-fsIL", "-o", "/dev/null", url])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 8.4 LTS 계열의 최신 패치 파일을 찾는다 (높은 번호부터 있는지 본다)
fn find_latest() -> Result<(String, String), String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x86_64",
    };
    for patch in (0..=40).rev() {
        for os in ["macos15", "macos14"] {
            let ver = format!("{SERIES}.{patch}");
            let file = format!("mysql-{ver}-{os}-{arch}.tar.gz");
            let url = format!("https://cdn.mysql.com/Downloads/MySQL-{SERIES}/{file}");
            if curl_ok(&url) {
                return Ok((ver, url));
            }
        }
    }
    Err("MySQL 8.4 배포 파일을 찾지 못했습니다.".into())
}

pub fn install() -> Result<String, String> {
    let (ver, url) = find_latest()?;
    let tools = data_root().join("tools");
    fs::create_dir_all(&tools).map_err(|e| e.to_string())?;
    let archive = tools.join("mysql.tar.gz");
    // cdn.mysql.com 에서 HTTPS 로만 받는다
    let out = Command::new("curl")
        .args(["-fSL", "--retry", "2", "-o"])
        .arg(&archive)
        .arg(&url)
        .output()
        .map_err(|e| format!("curl 실행 실패: {e}"))?;
    if !out.status.success() {
        return Err(format!("MySQL 받기 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let out = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&tools).output().map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&archive);
    if !out.status.success() {
        return Err(format!("압축 풀기 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let dir_name = url.rsplit('/').next().unwrap().trim_end_matches(".tar.gz").to_string();
    let link = home();
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink(tools.join(&dir_name), &link).map_err(|e| e.to_string())?;

    // 처음이면 데이터 폴더를 만들고 root 비밀번호를 정한다
    let fresh = !datadir().join("mysql").exists();
    if fresh {
        fs::create_dir_all(datadir()).map_err(|e| e.to_string())?;
        let out = Command::new(bin_dir().join("mysqld"))
            .arg("--initialize-insecure")
            .arg(format!("--basedir={}", home().display()))
            .arg(format!("--datadir={}", datadir().display()))
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("MySQL 데이터 초기화 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    start()?;
    if fresh {
        let out = Command::new(bin_dir().join("mysql"))
            .args(["--socket", SOCKET, "-uroot", "--skip-password", "-e"])
            .arg(format!("ALTER USER 'root'@'localhost' IDENTIFIED BY '{ROOT_PASSWORD}';"))
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("root 비밀번호 설정 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    Ok(format!("MySQL {ver} LTS 설치·시작 완료 (root 비밀번호: {ROOT_PASSWORD})"))
}
