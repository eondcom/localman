use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    NotInstalled,
    Unknown,
}

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
