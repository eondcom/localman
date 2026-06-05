use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Unknown,
}

pub fn get_service_status(name: &str) -> ServiceStatus {
    let output = Command::new("systemctl")
        .args(["is-active", name])
        .output();

    match output {
        Ok(out) => {
            let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
            match status.as_str() {
                "active" => ServiceStatus::Running,
                _ => ServiceStatus::Stopped,
            }
        }
        Err(_) => ServiceStatus::Unknown,
    }
}

pub fn toggle_service(name: &str, start: bool) -> Result<(), String> {
    let action = if start { "start" } else { "stop" };
    eprintln!("[localman] 서비스 {action}: {name}");
    let output = Command::new("pkexec")
        .args(["systemctl", action, name])
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
