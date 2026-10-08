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
    DeployTarget, apply_auth, changed_files, check_db, dest, local_db, probe_shell, record_as, remote_dump_to, rsync_ssh, validate,
};
use super::ProgressFn;
use super::history::{Direction, now};
use super::lan::human_bytes;
use super::project::{ProjectDb, VhostProject, set_project_db};
use super::settings::data_dir;
use crate::i18n::{tr, trf};

/// 로컬에 이미 있으면 서버 것으로 덮지 않을 설정 파일 (rsync 패턴, 로컬 경로).
/// 폴더가 아니라 파일로 본다 — 빈 files/config 폴더만 있을 때 설정을 못 받는 일이 없게.
const LOCAL_CONFIGS: [(&str, &str); 6] = [
    ("/files/config/config.php", "files/config/config.php"),
    ("/files/config/db.config.php", "files/config/db.config.php"),
    ("/.env", ".env"),
    ("/wp-config.php", "wp-config.php"),
    ("/data/dbconfig.php", "data/dbconfig.php"),
    ("/config/database.php", "config/database.php"),
];

/// 지킬 로컬 설정 파일 (rsync 패턴 목록)
fn protected(p: &VhostProject) -> Vec<&'static str> {
    LOCAL_CONFIGS
        .iter()
        .filter(|(_, rel)| Path::new(&p.path).join(rel).is_file())
        .map(|(pat, _)| *pat)
        .collect()
}

fn pull_rsync_args(t: &DeployTarget, local: &str, keep: &[&str], dry_run: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-rlptzv".into()];
    if dry_run {
        a.push("--dry-run".into());
    }
    for e in t.pull_excludes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        a.push(format!("--exclude={e}"));
    }
    for pat in keep {
        a.push(format!("--exclude={pat}"));
    }
    a.push("-e".into());
    a.push(rsync_ssh(t));
    a.push(format!("{}:{}/", dest(t), t.remote_path.trim_end_matches('/')));
    a.push(format!("{}/", local.trim_end_matches('/')));
    a
}

fn run_rsync(p: &VhostProject, t: &DeployTarget, keep: &[&str], dry_run: bool) -> Result<Vec<String>, String> {
    let mut c = Command::new("rsync");
    apply_auth(&mut c, t)?;
    let out = c
        .args(pull_rsync_args(t, &p.path, keep, dry_run))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| trf("rsync 실행 실패: {0} (rsync를 설치하세요)", &[&e]))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(changed_files(&String::from_utf8_lossy(&out.stdout)))
}

fn protected_note(keep: &[&str]) -> Option<String> {
    (!keep.is_empty()).then(|| format!("· {}", trf("로컬 설정 유지 (덮지 않음): {0}", &[&keep.join(", ")])))
}

/// 셸·rsync 가 되면 rsync, 아니면 SFTP 로 받는다. (파일 목록, 실제 웹 경로, 방식)
/// (받은 파일, 실제 웹 경로, 방식, 서버 파일 수 — rsync 는 모름)
fn receive(
    p: &VhostProject,
    t: &DeployTarget,
    keep: &[&str],
    dry_run: bool,
    progress: &ProgressFn,
) -> Result<(Vec<String>, String, &'static str, Option<usize>), String> {
    progress(tr("서버에 접속하는 중…").to_string(), None);
    validate(t)?;
    match probe_shell(t) {
        Ok(Some(info)) if info.rsync => {
            let root = info.root.ok_or_else(|| trf("서버에 웹 경로가 없습니다: {0}", &[&t.remote_path]))?;
            let mut t = t.clone();
            t.remote_path = root;
            progress(tr("rsync로 받는 중… (바뀐 파일만)").to_string(), None);
            Ok((run_rsync(p, &t, keep, dry_run)?, t.remote_path, "rsync", None))
        }
        _ => {
            let mut excludes = t.pull_excludes.clone();
            excludes.extend(keep.iter().map(|k| k.to_string()));
            let (files, root, total) = super::sftp::pull(t, Path::new(&p.path), excludes, dry_run, progress.clone())?;
            Ok((files, root, "SFTP", Some(total)))
        }
    }
}

/// 받을 파일 목록만 본다
pub fn preview_pull(p: &VhostProject, t: &DeployTarget, progress: &ProgressFn) -> Result<Vec<String>, String> {
    let keep = protected(p);
    let (files, root, how, _) = receive(p, t, &keep, true, progress)?;
    let mut log = vec![format!("✓ {}", trf("받을 파일 {0}개 (로컬에만 있는 파일은 지우지 않음)", &[&files.len()]))];
    log.push(format!("· {}", trf("서버 경로 {0} · {1}", &[&root, &how])));
    log.extend(protected_note(&keep));
    log.extend(files.iter().take(40).map(|f| format!("· {f}")));
    if files.len() > 40 {
        log.push(format!("· {}", trf("… 외 {0}개", &[&(files.len() - 40)])));
    }
    Ok(log)
}

/// 서버의 파일을 프로젝트 폴더로 받는다
pub fn pull_files(p: &VhostProject, t: &DeployTarget, progress: &ProgressFn) -> Result<Vec<String>, String> {
    fs::create_dir_all(&p.path).map_err(|e| trf("프로젝트 폴더를 만들 수 없습니다: {0}", &[&e]))?;
    // 받기 전에 정한다 — 처음 받는 설정 파일은 서버 것을 받는다
    let keep = protected(p);
    let (files, root, how, total) = receive(p, t, &keep, false, progress)?;
    let count = files.len() as u64;
    let mut log = vec![match total {
        Some(total) => format!(
            "✓ {}",
            trf("파일 받기 완료 — 서버 파일 {0}개 중 바뀐 {1}개를 받았습니다 (나머지 {2}개는 이미 같음 · {3})", &[&total, &count, &(total - files.len()), &how])
        ),
        None => format!("✓ {}", trf("파일 {0}개를 {1}:{2} 에서 받음 ({3})", &[&count, &t.ssh_host, &root, &how])),
    }];
    log.push(format!("· {}", trf("서버 경로 {0} · {1}", &[&root, &how])));
    log.extend(protected_note(&keep));
    // 받은 파일 전체 목록 (화면에는 앞부분만, 기록 파일에는 전부 남는다)
    log.extend(files.iter().map(|f| format!("· {f}")));
    record_as(Direction::Pulled, p, t, count, vec![], true, &log);
    Ok(log)
}

/// 로컬 DB 의 표 수
fn list_tables_count(user: &str, password: &str, db: &str, engine: DbEngine) -> Option<usize> {
    let out = match engine {
        DbEngine::MariaDb => Command::new("mysql")
            .env("MYSQL_PWD", password)
            .args([&format!("-u{user}"), "-N", "-B", "-e"])
            .arg(format!("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = '{}'", db.replace('\'', "")))
            .output()
            .ok()?,
        DbEngine::PostgreSql => Command::new("psql")
            .env("PGPASSWORD", password)
            .args(["-h", "127.0.0.1", "-U", user, "-d", db, "-At", "-c", "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'"])
            .output()
            .ok()?,
    };
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// 서버 DB 를 가져올 때 넣을 로컬 DB 이름 기본값: 사이트 설정 → 연결된 DB → 서버 DB 이름
pub fn default_local_db_name(p: &VhostProject, t: &DeployTarget) -> String {
    if !t.local_db.trim().is_empty() {
        return t.local_db.trim().to_string();
    }
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
pub fn pull_database(p: &VhostProject, t: &DeployTarget, local_name: &str, progress: &ProgressFn) -> Result<Vec<String>, String> {
    check_db(t)?;
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
    // 덤프하는 동안 받은 크기를 알린다 (전체 크기는 미리 알 수 없다)
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (done, path, progress) = (done.clone(), server_dump.clone(), progress.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                progress(trf("서버 DB 받는 중… {0}", &[&human_bytes(size)]), None);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        })
    };
    let dumped = remote_dump_to(engine, t, &server_dump);
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = watcher.join();
    if let Err(e) = dumped {
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
        progress(trf("로컬 DB {0} 백업 중…", &[&local_name]), None);
        let backup = dir.join(format!("{}-local-{local_name}-{stamp}.sql", p.id));
        backup_database(engine, &creds.user, &creds.password, local_name, &backup.to_string_lossy())
            .map_err(|e| trf("로컬 DB를 백업하지 못해 멈췄습니다 (바꾸지 않음): {0}", &[&e.trim()]))?;
        log.push(format!("✓ {}", trf("로컬 DB {0} 백업: {1}", &[&local_name, &backup.display()])));
        drop_database(engine, &creds.user, &creds.password, local_name).map_err(|e| e.trim().to_string())?;
    }

    if !exists {
        log.push(format!("✓ {}", trf("로컬 DB {0} 새로 만듦", &[&local_name])));
    }
    // 3) 넣기 (없으면 만든다)
    progress(trf("로컬 DB {0}에 넣는 중… ({1})", &[&local_name, &human_bytes(size)]), None);
    import_sql(engine, &creds.user, &creds.password, local_name, &server_dump.to_string_lossy(), true)
        .map_err(|e| trf("로컬 DB에 넣지 못했습니다: {0}", &[&e.trim()]))?;
    log.push(format!(
        "✓ {}",
        trf("서버 {0} → 로컬 DB {1}", &[&format!("{}@{}", t.db_user, t.db_name), &local_name])
    ));
    // 다 들어왔는지 — 덤프의 표 수와 로컬 DB 의 표 수를 맞춰 본다
    let expected = fs::read_to_string(&server_dump).map(|s| s.matches("\nCREATE TABLE ").count()).unwrap_or(0);
    let actual = list_tables_count(&creds.user, &creds.password, local_name, engine);
    match actual {
        Some(n) if n >= expected => log.push(format!("✓ {}", trf("테이블 {0}/{1}개 확인 — 모두 들어왔습니다", &[&n, &expected]))),
        Some(n) => log.push(format!("✗ {}", trf("테이블 {0}/{1}개만 들어왔습니다 — 덤프 파일을 확인하세요", &[&n, &expected]))),
        None => {}
    }

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
        let keep = protected(&p);
        let a = pull_rsync_args(&t, &p.path, &keep, false);
        fs::remove_dir_all(&dir).unwrap();
        assert!(!a.iter().any(|x| x.contains("--delete")), "로컬 파일을 지우면 안 됩니다");
        assert!(!a.contains(&"--exclude=/files/config/config.php".to_string()), "빈 폴더만 있으면 받아야 합니다");
        assert!(!a.contains(&"--exclude=/.env".to_string()));
        assert_eq!(a[a.len() - 2], "u@example.com:/var/www/site/");
        assert!(a[a.len() - 1].ends_with('/'));
    }

}


