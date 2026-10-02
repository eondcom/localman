//! 외부 도구 연동: Adminer(pma.localhost), 라이믹스 rx-cli.

use std::fs;
use std::path::{Path, PathBuf};

use super::apache::write_vhost;
use super::project::{ProjectType, VhostProject, join_dir};
use super::setup::strip_ansi;
use crate::platform::update_hosts;

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
        db: None,
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

/// 프로젝트 작업 디렉토리가 라이믹스(Rhymix) 설치본인지 판별한다.
/// `common/autoload.php` 존재 여부로 판단한다 — Rhymix 코어의 부트스트랩 진입점이라
/// 제로보드4/그누보드 등 다른 PHP CMS와 헷갈릴 일이 없다.
pub fn is_rhymix_project(project: &VhostProject) -> bool {
    project.project_type == ProjectType::Php
        && Path::new(&join_dir(&project.work_dir(), "common/autoload.php")).exists()
}

/// PATH에 등록된 `rx`(rx-cli, rxdashboard 모듈 제공) 실행 파일 경로를 찾는다.
fn find_rx_cli() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("rx"))
        .find(|p| p.is_file())
}

/// rx-cli(`rx member reset-password`)로 라이믹스 관리자 비밀번호를 바꾼다.
///
/// rx-cli는 라이믹스 코어의 `MemberController::updateMemberPassword()`를 그대로
/// 호출하므로, 해시 알고리즘·강도 검사 등을 이 쪽에서 다시 구현할 필요가 없다.
/// member 테이블은 사이트 구분 없는 전역 테이블이라 user_id만으로 대상이 정해진다.
pub fn rx_reset_admin_password(project: &VhostProject, user_id: &str, new_password: &str) -> Result<String, String> {
    let user_id = user_id.trim();
    if user_id.is_empty() {
        return Err("관리자 ID를 입력하세요.".to_string());
    }
    if new_password.is_empty() {
        return Err("새 비밀번호를 입력하세요.".to_string());
    }
    let rx_bin = find_rx_cli().ok_or_else(|| {
        "rx-cli(rx 명령)를 찾을 수 없습니다. rxdashboard 모듈의 bin/rx를 PATH에 심볼릭 링크로 등록하세요.".to_string()
    })?;

    let output = std::process::Command::new(&rx_bin)
        .args(["member", "reset-password", user_id, new_password])
        .current_dir(project.work_dir())
        .output()
        .map_err(|e| format!("rx-cli 실행 실패: {e}"))?;

    let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout)).trim().to_string();
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr)).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { "비밀번호를 변경했습니다.".to_string() } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}