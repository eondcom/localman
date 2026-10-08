//! 실서버에서 가져오기: 서버 웹 경로의 파일을 프로젝트 폴더로 받고, 서버 DB 를 이 PC 의 DB 에 넣는다.
//! 서버 정보는 서버 배포와 같은 것(deploy.json)을 쓴다.
//!
//! - 로컬에만 있는 파일은 지우지 않는다 (--delete 없음)
//! - 로컬 설정 파일(라이믹스 files/config/, .env, wp-config.php …)이 이미 있으면 덮지 않는다 — 로컬 DB 접속·주소가 바뀌지 않게
//! - 서버에 rsync 가 없으면 tar 로 통째로 받는다
//! - DB 를 넣기 전에 같은 이름의 로컬 DB 를 백업해 둔다. 받은 서버 덤프도 보관한다

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::database::{DbEngine, backup_database, drop_database, import_sql, list_databases, load_db_connections};
use super::db_users::{ensure_site_user_for, site_db_account};
use super::deploy::{
    DeployTarget, apply_auth, changed_files, dest, local_db, record_as, remote_dump_to, rsync_ssh, sq, ssh, ssh_command,
    validate,
};
use super::history::{Direction, now};
use super::project::{ProjectDb, VhostProject, set_project_db};
use super::settings::data_dir;
use crate::i18n::{tr, trf};

/// 로컬에 이미 있으면 서버 것으로 덮지 않을 설정 파일 (rsync 패턴, 로컬 경로)
const LOCAL_CONFIGS: [(&str, &str); 5] = [
    ("/files/config/", "files/config"),
    ("/.env", ".env"),
    ("/wp-config.php", "wp-config.php"),
    ("/data/dbconfig.php", "data/dbconfig.php"),
    ("/config/database.php", "config/database.php"),
];

/// 지킬 로컬 설정 파일 (rsync 패턴 목록)
fn protected(p: &VhostProject) -> Vec<&'static str> {
    LOCAL_CONFIGS
        .iter()
        .filter(|(_, rel)| Path::new(&p.path).join(rel).exists())
        .map(|(pat, _)| *pat)
        .collect()
}

fn pull_rsync_args(p: &VhostProject, t: &DeployTarget, dry_run: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-rlptzv".into()];
    if dry_run {
        a.push("--dry-run".into());
    }
    for e in t.pull_excludes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        a.push(format!("--exclude={e}"));
    }
    for pat in protected(p) {
        a.push(format!("--exclude={pat}"));
    }
    a.push("-e".into());
    a.push(rsync_ssh(t));
    a.push(format!("{}:{}/", dest(t), t.remote_path.trim_end_matches('/')));
    a.push(format!("{}/", p.path.trim_end_matches('/')));
    a
}

/// 서버의 웹 경로와 rsync 가 있는지
fn remote_check(t: &DeployTarget) -> Result<bool, String> {
    let out = ssh(t, &format!("test -d {} || echo NODIR; command -v rsync >/dev/null 2>&1 || echo NORSYNC", sq(&t.remote_path)))?;
    if out.contains("NODIR") {
        return Err(trf("서버에 웹 경로가 없습니다: {0}", &[&t.remote_path]));
    }
    Ok(!out.contains("NORSYNC"))
}

fn run_rsync(p: &VhostProject, t: &DeployTarget, dry_run: bool) -> Result<Vec<String>, String> {
    let mut c = Command::new("rsync");
    apply_auth(&mut c, t)?;
    let out = c
        .args(pull_rsync_args(p, t, dry_run))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| trf("rsync 실행 실패: {0} (rsync를 설치하세요)", &[&e]))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(changed_files(&String::from_utf8_lossy(&out.stdout)))
}

/// 서버에 rsync 가 없을 때: 서버에서 tar 로 묶어 받아 푼다 (바뀐 것만 고르지 못하고 전부 받는다)
fn tar_pull(p: &VhostProject, t: &DeployTarget) -> Result<u64, String> {
    let mut excludes: Vec<String> = t
        .pull_excludes
        .iter()
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty())
        .map(|s| format!("--exclude={}", sq(&format!("./{}", s.trim_start_matches('/')))))
        .collect();
    excludes.extend(protected(p).iter().map(|pat| format!("--exclude={}", sq(&format!(".{}", pat.trim_end_matches('/'))))));
    let remote = format!("cd {} && tar -czf - {} .", sq(&t.remote_path), excludes.join(" "));
    let mut src = ssh_command(t)?
        .arg(remote)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| trf("ssh 실행 실패: {0}", &[&e]))?;
    let pipe = src.stdout.take().ok_or("ssh stdout")?;
    // GNU tar 는 -v 목록을 stdout, bsdtar 는 stderr 로 낸다 — 둘 다 센다
    let out = Command::new("tar")
        .args(["-xzvf", "-", "-C", &p.path])
        .stdin(Stdio::from(pipe))
        .output()
        .map_err(|e| e.to_string())?;
    let remote_res = src.wait_with_output().map_err(|e| e.to_string())?;
    if !remote_res.status.success() {
        return Err(String::from_utf8_lossy(&remote_res.stderr).trim().to_string());
    }
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let count = |b: &[u8]| String::from_utf8_lossy(b).lines().filter(|l| !l.trim_end().ends_with('/')).count() as u64;
    Ok(count(&out.stdout).max(count(&out.stderr)))
}

fn protected_note(keep: &[&str]) -> Option<String> {
    (!keep.is_empty()).then(|| format!("· {}", trf("로컬 설정 유지 (덮지 않음): {0}", &[&keep.join(", ")])))
}

/// 받을 파일 목록만 본다
pub fn preview_pull(p: &VhostProject, t: &DeployTarget) -> Result<Vec<String>, String> {
    validate(t)?;
    if !remote_check(t)? {
        return Ok(vec![format!("· {}", tr("서버에 rsync가 없어 미리보기를 못 합니다 — 가져오면 tar로 전부 받습니다"))]);
    }
    let files = run_rsync(p, t, true)?;
    let mut log = vec![format!("✓ {}", trf("받을 파일 {0}개 (로컬에만 있는 파일은 지우지 않음)", &[&files.len()]))];
    log.extend(protected_note(&protected(p)));
    log.extend(files.iter().take(40).map(|f| format!("· {f}")));
    if files.len() > 40 {
        log.push(format!("· {}", trf("… 외 {0}개", &[&(files.len() - 40)])));
    }
    Ok(log)
}

/// 서버의 파일을 프로젝트 폴더로 받는다
pub fn pull_files(p: &VhostProject, t: &DeployTarget) -> Result<Vec<String>, String> {
    validate(t)?;
    fs::create_dir_all(&p.path).map_err(|e| trf("프로젝트 폴더를 만들 수 없습니다: {0}", &[&e]))?;
    let has_rsync = remote_check(t)?;
    // 받기 전에 정한다 — 처음 받는 설정 파일은 서버 것을 받는다
    let keep = protected(p);
    let (count, how) = if has_rsync {
        (run_rsync(p, t, false)?.len() as u64, "rsync")
    } else {
        (tar_pull(p, t)?, "tar")
    };
    let mut log = vec![format!(
        "✓ {}",
        trf("파일 {0}개를 {1}:{2} 에서 받음 ({3})", &[&count, &t.ssh_host, &t.remote_path, &how])
    )];
    log.extend(protected_note(&keep));
    record_as(Direction::Pulled, p, t, count, vec![], true, &log);
    Ok(log)
}

/// 서버 DB 를 가져올 때 넣을 로컬 DB 이름 기본값: 사이트 설정 → 연결된 DB → 서버 DB 이름
pub fn default_local_db_name(p: &VhostProject, t: &DeployTarget) -> String {
    site_db_account(Path::new(&p.path))
        .map(|a| a.database)
        .or_else(|| local_db(p).map(|d| d.name))
        .unwrap_or_else(|| t.db_name.trim().to_string())
}

fn backups_dir() -> Result<PathBuf, String> {
    let dir = data_dir().join("pull-backups");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 서버 DB 를 이 PC 의 `local_name` DB 에 넣는다 (그 DB 는 서버 것과 똑같아진다)
pub fn pull_database(p: &VhostProject, t: &DeployTarget, local_name: &str) -> Result<Vec<String>, String> {
    if !t.has_db() {
        return Err(tr("원격 DB 정보(호스트·사용자·DB 이름)를 입력하세요.").into());
    }
    let local_name = local_name.trim();
    if local_name.is_empty() || local_name.contains(['`', '/', '\\', '\'', '"']) {
        return Err(tr("넣을 로컬 DB 이름을 확인하세요.").into());
    }
    let engine = local_db(p).map(|d| d.engine).unwrap_or(DbEngine::MariaDb);
    let creds = load_db_connections()
        .into_iter()
        .find(|c| c.engine == engine)
        .ok_or(tr("로컬 DB 접속 정보가 없습니다. 데이터베이스 탭에서 먼저 연결하세요."))?;
    let dir = backups_dir()?;
    let stamp = now();
    let mut log = Vec::new();

    // 1) 서버 DB 받기 (받은 덤프는 보관한다)
    let server_dump = dir.join(format!("{}-server-{}-{stamp}.sql", p.id, t.db_name.trim()));
    if let Err(e) = remote_dump_to(engine, t, &server_dump) {
        let _ = fs::remove_file(&server_dump);
        return Err(trf("서버 DB를 받지 못했습니다: {0}", &[&e]));
    }
    let size = fs::metadata(&server_dump).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        let _ = fs::remove_file(&server_dump);
        return Err(tr("서버 DB 덤프가 비어 있습니다.").into());
    }
    log.push(format!("✓ {}", trf("서버 DB 받음: {0}", &[&server_dump.display()])));

    // 2) 같은 이름의 로컬 DB 가 있으면 백업한 뒤 비운다 (서버 것과 똑같이 맞추려고)
    let exists = list_databases(engine, &creds.user, &creds.password).iter().any(|d| d.name == local_name);
    if exists {
        let backup = dir.join(format!("{}-local-{local_name}-{stamp}.sql", p.id));
        backup_database(engine, &creds.user, &creds.password, local_name, &backup.to_string_lossy())
            .map_err(|e| trf("로컬 DB를 백업하지 못해 멈췄습니다 (바꾸지 않음): {0}", &[&e.trim()]))?;
        log.push(format!("✓ {}", trf("로컬 DB {0} 백업: {1}", &[&local_name, &backup.display()])));
        drop_database(engine, &creds.user, &creds.password, local_name).map_err(|e| e.trim().to_string())?;
    }

    // 3) 넣기
    import_sql(engine, &creds.user, &creds.password, local_name, &server_dump.to_string_lossy(), true)
        .map_err(|e| trf("로컬 DB에 넣지 못했습니다: {0}", &[&e.trim()]))?;
    log.push(format!(
        "✓ {}",
        trf("서버 {0} → 로컬 DB {1}", &[&format!("{}@{}", t.db_user, t.db_name), &local_name])
    ));

    // 4) 사이트에 DB 를 연결하고, 사이트 설정의 DB 계정을 만든다
    if p.db.is_none() {
        let _ = set_project_db(&p.id, Some(ProjectDb { engine, name: local_name.to_string() }));
    }
    if engine == DbEngine::MariaDb {
        log.extend(ensure_site_user_for(p, &creds.user, &creds.password));
    }
    record_as(Direction::Pulled, p, t, 0, vec![local_name.to_string()], true, &log);
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectType;
    use crate::domain::test_util::project;

    #[test]
    fn pull_never_deletes_and_keeps_local_config() {
        let dir = std::env::temp_dir().join(format!("lm-pull-{}", std::process::id()));
        fs::create_dir_all(dir.join("files/config")).unwrap();
        let p = project(ProjectType::Php, &dir.to_string_lossy(), "", 80);
        let t = DeployTarget {
            ssh_host: "example.com".into(),
            ssh_port: 22,
            ssh_user: "u".into(),
            remote_path: "/var/www/site".into(),
            pull_excludes: vec![".git/".into()],
            ..Default::default()
        };
        let a = pull_rsync_args(&p, &t, false);
        fs::remove_dir_all(&dir).unwrap();
        assert!(!a.iter().any(|x| x.contains("--delete")), "로컬 파일을 지우면 안 됩니다");
        assert!(a.contains(&"--exclude=/files/config/".to_string()), "로컬 설정을 덮으면 안 됩니다");
        assert!(!a.contains(&"--exclude=/.env".to_string()));
        assert_eq!(a[a.len() - 2], "u@example.com:/var/www/site/");
        assert!(a[a.len() - 1].ends_with('/'));
    }

    #[test]
    fn quotes_remote_shell_args() {
        assert_eq!(sq("a'b"), "'a'\\''b'");
    }
}

