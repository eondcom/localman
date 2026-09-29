pub mod service;
pub mod vhost;
pub mod database;
pub mod log;

pub use service::{ServiceStatus, get_service_status, install_service, toggle_service};
pub use log::{error_log_path, read_log, clear_log};
pub use database::{DbCredentials, DbEngine, list_databases, backup_database, restore_database, import_sql, create_database, drop_database, rename_database, list_users, create_user, drop_user, rename_user, change_user_password, grant_privileges, DbUser, load_db_connections, save_db_connection};
pub use vhost::{VhostProject, ProjectType, ServerStatus, list_projects, add_project, update_project, remove_project, start_server, stop_server, server_status, auto_assign_port, setup_project, deps_ready, auto_detect_start_command, auto_detect_next_command, detect_next_app_dir, join_dir, ensure_adminer_site, open_url, is_rhymix_project, rx_reset_admin_password};

/// 설정 파일을 원자적으로 쓴다: 같은 폴더의 임시 파일에 쓰고 fsync 후 rename.
///
/// `fs::write` 는 파일을 먼저 비운 뒤 쓰므로, 디스크가 꽉 찬 상태에서 실패하면
/// 원본이 0바이트로 남는다. 2026-09-30 디스크 100% 상태에서 projects.json·
/// db_credentials.json·running.json 이 전부 이렇게 날아가, 프로젝트 편집이
/// "프로젝트를 찾을 수 없습니다" 로 실패했다.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        // ENOSPC(28)
        let hint = if e.raw_os_error() == Some(28) { " — 디스크 공간 부족" } else { "" };
        return Err(format!("{} 저장 실패{hint}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[test]
    fn write_atomic_replaces_and_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("localman_atomic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert!(!dir.join("data.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomic_failure_keeps_original() {
        let dir = std::env::temp_dir().join(format!("localman_atomic_ro_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.json");
        std::fs::write(&path, b"original").unwrap();
        // 임시 파일 자리를 디렉토리로 막아 쓰기 실패를 만든다
        std::fs::create_dir(dir.join("data.json.tmp")).unwrap();
        assert!(write_atomic(&path, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
