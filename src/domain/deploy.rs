//! 실서버 배포: SSH 서버에 rsync 로 파일을 올리고, 원격 DB 에 직접 접속해 DB 를 넣는다.
//!
//! - 서버에만 있는 파일은 지우지 않는다 (rsync 에 --delete 를 쓰지 않는다)
//! - SSH 는 키 로그인만 쓴다 (BatchMode). 비밀번호를 묻는 창이 GUI 를 붙잡지 않게 한다
//! - DB 올리기는 운영 DB 를 덮어쓰므로, 넣기 직전에 원격 DB 를 이 PC 에 백업해 둔다
//! - 서버·DB 접속 정보는 deploy.json(권한 600)에 프로젝트별로 저장한다. 백업 묶음에는 담지 않는다

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::database::{DbEngine, load_db_connections};
use super::history::{Direction, TransferRecord, append_history, now};
use super::project::{ProjectDb, VhostProject};
use super::settings::data_dir;
use super::usage::{detect_db, effective_db};
use crate::i18n::{tr, trf};

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeployTarget {
    pub ssh_host: String,
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    pub ssh_user: String,
    /// 개인 키 경로. 비우면 ssh 기본 키·에이전트를 쓴다
    #[serde(default)]
    pub ssh_key: String,
    /// 서버의 웹 경로 (예: /var/www/site)
    pub remote_path: String,
    /// 올리지 않을 경로 (rsync --exclude 패턴)
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub db_host: String,
    #[serde(default)]
    pub db_port: u16,
    #[serde(default)]
    pub db_user: String,
    #[serde(default)]
    pub db_password: String,
    #[serde(default)]
    pub db_name: String,
}

impl DeployTarget {
    pub fn has_db(&self) -> bool {
        !self.db_host.trim().is_empty() && !self.db_name.trim().is_empty() && !self.db_user.trim().is_empty()
    }
}

/// 처음 설정할 때 채워 줄 기본값
pub fn default_target(p: &VhostProject) -> DeployTarget {
    let mut excludes: Vec<String> = [
        ".git/", "node_modules/", "venv/", ".venv/", "__pycache__/", ".next/", ".DS_Store", ".env", "*.localman-part",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // 라이믹스: 서버 쪽 설정·캐시를 로컬 것으로 덮어쓰면 사이트가 깨진다
    if Path::new(&super::project::join_dir(&p.work_dir(), "common/autoload.php")).exists() {
        excludes.extend(["files/config/", "files/cache/", "files/env/", "files/supercache/"].iter().map(|s| s.to_string()));
    }
    let db = effective_db(p);
    DeployTarget {
        ssh_port: 22,
        excludes,
        db_port: match db.as_ref().map(|d| d.engine) {
            Some(DbEngine::PostgreSql) => 5432,
            _ => 3306,
        },
        db_name: db.map(|d| d.name).unwrap_or_default(),
        ..Default::default()
    }
}

fn store_path() -> PathBuf {
    data_dir().join("deploy.json")
}

fn load_all() -> HashMap<String, DeployTarget> {
    fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn load_target(id: &str) -> Option<DeployTarget> {
    load_all().remove(id)
}

pub fn save_target(id: &str, t: &DeployTarget) -> Result<(), String> {
    validate(t)?;
    let mut all = load_all();
    all.insert(id.to_string(), t.clone());
    let path = store_path();
    super::write_atomic(&path, serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn validate(t: &DeployTarget) -> Result<(), String> {
    let bad = |s: &str| s.chars().any(|c| c.is_whitespace() || "'\"`$;&|<>\\".contains(c));
    if t.ssh_host.trim().is_empty() || t.ssh_user.trim().is_empty() {
        return Err(tr("서버 주소와 SSH 사용자를 입력하세요.").into());
    }
    if bad(&t.ssh_host) || bad(&t.ssh_user) {
        return Err(tr("서버 주소·사용자에 공백이나 특수문자를 쓸 수 없습니다.").into());
    }
    if !t.remote_path.starts_with('/') {
        return Err(tr("웹 경로는 /로 시작하는 절대 경로여야 합니다 (예: /var/www/site).").into());
    }
    // 원격 셸과 rsync 가 경로를 다르게 해석하지 않도록 공백·따옴표 등은 막는다
    if bad(&t.remote_path) {
        return Err(tr("웹 경로에 공백이나 따옴표 같은 특수문자를 쓸 수 없습니다.").into());
    }
    if t.remote_path.trim_end_matches('/').is_empty() {
        return Err(tr("웹 경로로 / 를 쓸 수 없습니다.").into());
    }
    Ok(())
}

// ── SSH ────────────────────────────────────────────────────────────────

fn ssh_opts(t: &DeployTarget) -> Vec<String> {
    let mut v = vec![
        "-p".into(),
        t.ssh_port.to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    if !t.ssh_key.trim().is_empty() {
        v.push("-i".into());
        v.push(t.ssh_key.trim().to_string());
    }
    v
}

fn dest(t: &DeployTarget) -> String {
    format!("{}@{}", t.ssh_user.trim(), t.ssh_host.trim())
}

fn ssh(t: &DeployTarget, remote_cmd: &str) -> Result<String, String> {
    let out = Command::new("ssh")
        .args(ssh_opts(t))
        .arg(dest(t))
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| trf("ssh 실행 실패: {0}", &[&e]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.contains("Permission denied") {
            Err(format!(
                "{}\n{err}",
                trf(
                    "SSH 키로 로그인하지 못했습니다. 터미널에서 `ssh-copy-id -p {0} {1}` 로 키를 등록하세요.",
                    &[&t.ssh_port, &dest(t)],
                )
            ))
        } else {
            Err(err)
        }
    }
}

// ── 원격 DB ────────────────────────────────────────────────────────────

fn remote_db_cmd(engine: DbEngine, t: &DeployTarget, program: &str) -> Command {
    let mut c = Command::new(program);
    match engine {
        DbEngine::MariaDb => {
            // 비밀번호는 명령줄(ps 로 보임) 대신 환경 변수로 넘긴다
            c.env("MYSQL_PWD", &t.db_password)
                .args(["-h", t.db_host.trim(), "-P", &t.db_port.to_string(), "-u", t.db_user.trim()])
                .arg("--connect-timeout=10");
        }
        DbEngine::PostgreSql => {
            c.env("PGPASSWORD", &t.db_password)
                .env("PGCONNECT_TIMEOUT", "10")
                .args(["-h", t.db_host.trim(), "-p", &t.db_port.to_string(), "-U", t.db_user.trim()]);
        }
    }
    c
}

fn run(mut c: Command) -> Result<String, String> {
    let out = c.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 이 사이트의 로컬 DB (엔진 판단용). 연결·감지 모두 안 되면 MariaDB 로 본다.
fn local_db(p: &VhostProject) -> Option<ProjectDb> {
    p.db.clone().or_else(|| detect_db(p))
}

// ── 동작 ───────────────────────────────────────────────────────────────

/// SSH 접속, 웹 경로, 서버의 rsync, 원격 DB 접속을 차례로 확인한다.
pub fn test_connection(p: &VhostProject, t: &DeployTarget) -> Vec<String> {
    let mut log = Vec::new();
    if let Err(e) = validate(t) {
        return vec![format!("✗ {e}")];
    }
    match ssh(t, &format!("test -d {} && echo DIR || echo NODIR; command -v rsync >/dev/null && echo RSYNC || echo NORSYNC", t.remote_path)) {
        Ok(out) => {
            log.push(format!("✓ {}", trf("SSH 접속: {0}", &[&dest(t)])));
            log.push(if out.contains("NODIR") {
                format!("✗ {}", trf("웹 경로가 없습니다: {0} (처음 올릴 때 만들어집니다)", &[&t.remote_path]))
            } else {
                format!("✓ {}", trf("웹 경로: {0}", &[&t.remote_path]))
            });
            log.push(if out.contains("NORSYNC") {
                format!("✗ {}", tr("서버에 rsync가 없습니다 — 서버에서 rsync를 설치하세요"))
            } else {
                format!("✓ {}", tr("서버 rsync 있음"))
            });
        }
        Err(e) => log.push(format!("✗ {}", trf("SSH 접속 실패: {0}", &[&e]))),
    }
    if t.has_db() {
        let engine = local_db(p).map(|d| d.engine).unwrap_or(DbEngine::MariaDb);
        let mut c = match engine {
            DbEngine::MariaDb => remote_db_cmd(engine, t, "mysql"),
            DbEngine::PostgreSql => remote_db_cmd(engine, t, "psql"),
        };
        match engine {
            DbEngine::MariaDb => c.args(["-e", "SELECT 1", t.db_name.trim()]),
            DbEngine::PostgreSql => c.args(["-d", t.db_name.trim(), "-c", "SELECT 1"]),
        };
        match run(c) {
            Ok(_) => log.push(format!("✓ {}", trf("원격 DB 접속: {0}", &[&format!("{}@{}/{}", t.db_user, t.db_host, t.db_name)]))),
            Err(e) => log.push(format!("✗ {}", trf("원격 DB 접속 실패: {0}", &[&e]))),
        }
    } else {
        log.push(format!("· {}", tr("원격 DB 정보가 없어 DB 확인은 건너뜀")));
    }
    log
}

fn rsync_args(p: &VhostProject, t: &DeployTarget, dry_run: bool) -> Vec<String> {
    let ssh_cmd = std::iter::once("ssh".to_string())
        .chain(ssh_opts(t).into_iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a }))
        .collect::<Vec<_>>()
        .join(" ");
    // -rlptz: 권한·시각을 맞추되 소유자·그룹은 서버 것을 그대로 둔다. --delete 는 쓰지 않는다.
    let mut a: Vec<String> = vec!["-rlptzv".into()];
    if dry_run {
        a.push("--dry-run".into());
    }
    for e in &t.excludes {
        if !e.trim().is_empty() {
            a.push(format!("--exclude={}", e.trim()));
        }
    }
    a.push("-e".into());
    a.push(ssh_cmd);
    a.push(format!("{}/", p.path.trim_end_matches('/')));
    a.push(format!("{}:{}/", dest(t), t.remote_path.trim_end_matches('/')));
    a
}

/// rsync -v 출력에서 파일 이름 줄만 고른다 (GNU rsync·openrsync 모두)
fn changed_files(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.ends_with('/')
                && !l.starts_with("sending incremental")
                && !l.starts_with("Transfer starting")
                && !l.starts_with("sent ")
                && !l.starts_with("total size")
                && !l.starts_with("created directory")
                && !l.contains("(DRY RUN)")
        })
        .map(String::from)
        .collect()
}

fn rsync(p: &VhostProject, t: &DeployTarget, dry_run: bool) -> Result<Vec<String>, String> {
    validate(t)?;
    if !Path::new(&p.path).is_dir() {
        return Err(trf("프로젝트 폴더가 없습니다: {0}", &[&p.path]));
    }
    if !dry_run {
        // 처음 올릴 때 웹 경로가 없으면 만든다
        ssh(t, &format!("mkdir -p {}", t.remote_path))?;
    }
    let out = Command::new("rsync")
        .args(rsync_args(p, t, dry_run))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| trf("rsync 실행 실패: {0} (rsync를 설치하세요)", &[&e]))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(changed_files(&String::from_utf8_lossy(&out.stdout)))
}

/// 올라갈 파일 목록만 본다 (실제로 보내지 않음)
pub fn preview_files(p: &VhostProject, t: &DeployTarget) -> Result<Vec<String>, String> {
    rsync(p, t, true)
}

/// 바뀐 파일을 서버에 올린다. 서버에만 있는 파일은 그대로 둔다.
pub fn upload_files(p: &VhostProject, t: &DeployTarget) -> Result<Vec<String>, String> {
    let files = rsync(p, t, false)?;
    let mut log = vec![format!("✓ {}", trf("파일 {0}개를 {1}:{2} 에 올림", &[&files.len(), &t.ssh_host, &t.remote_path]))];
    log.extend(files.iter().take(30).map(|f| format!("· {f}")));
    if files.len() > 30 {
        log.push(format!("· {}", trf("… 외 {0}개", &[&(files.len() - 30)])));
    }
    record(p, t, files.len() as u64, vec![], true, &log);
    Ok(log)
}

/// 로컬 DB 를 원격 DB 에 넣는다. 넣기 전에 원격 DB 를 이 PC 에 백업한다.
pub fn push_database(p: &VhostProject, t: &DeployTarget) -> Result<Vec<String>, String> {
    if !t.has_db() {
        return Err(tr("원격 DB 정보(호스트·사용자·DB 이름)를 입력하세요.").into());
    }
    let local = local_db(p).ok_or(tr("이 사이트에 연결된 로컬 DB가 없습니다. 수정에서 DB를 지정하세요."))?;
    let creds = load_db_connections()
        .into_iter()
        .find(|c| c.engine == local.engine)
        .ok_or(tr("로컬 DB 접속 정보가 없습니다. 데이터베이스 탭에서 먼저 연결하세요."))?;
    let mut log = Vec::new();
    let stamp = now();
    let dir = data_dir().join("deploy-backups");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // 1) 원격 DB 백업 — 되돌릴 수 있게
    let backup = dir.join(format!("{}-{}-{stamp}.sql", p.id, t.db_name.trim()));
    let mut dump = match local.engine {
        DbEngine::MariaDb => {
            let mut c = remote_db_cmd(local.engine, t, "mysqldump");
            c.args(["--single-transaction", t.db_name.trim()]);
            c
        }
        DbEngine::PostgreSql => {
            let mut c = remote_db_cmd(local.engine, t, "pg_dump");
            c.args(["--no-owner", "--no-privileges", "-d", t.db_name.trim()]);
            c
        }
    };
    let out = dump.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(trf(
            "원격 DB를 백업하지 못해 중단했습니다 (덮어쓰지 않음): {0}",
            &[&String::from_utf8_lossy(&out.stderr).trim()],
        ));
    }
    fs::write(&backup, &out.stdout).map_err(|e| e.to_string())?;
    log.push(format!("✓ {}", trf("원격 DB 백업: {0}", &[&backup.display()])));

    // 2) 로컬 DB 덤프 — 같은 이름의 표를 지우고 다시 만드는 형태로
    let local_dump = std::env::temp_dir().join(format!("localman-deploy-{}-{stamp}.sql", p.id));
    let ld = match local.engine {
        DbEngine::MariaDb => Command::new("mysqldump")
            .env("MYSQL_PWD", &creds.password)
            .args(["-u", &creds.user, "--single-transaction", "--add-drop-table", &local.name])
            .output(),
        DbEngine::PostgreSql => Command::new("pg_dump")
            .env("PGPASSWORD", &creds.password)
            .args(["-h", "127.0.0.1", "-U", &creds.user, "--clean", "--if-exists", "--no-owner", "--no-privileges", "-d", &local.name])
            .output(),
    }
    .map_err(|e| e.to_string())?;
    if !ld.status.success() {
        return Err(trf("로컬 DB 덤프 실패: {0}", &[&String::from_utf8_lossy(&ld.stderr).trim()]));
    }
    fs::write(&local_dump, &ld.stdout).map_err(|e| e.to_string())?;

    // 3) 원격에 넣기
    let file = fs::File::open(&local_dump).map_err(|e| e.to_string())?;
    let mut import = match local.engine {
        DbEngine::MariaDb => {
            let mut c = remote_db_cmd(local.engine, t, "mysql");
            c.args(["--default-character-set=utf8mb4", t.db_name.trim()]);
            c
        }
        DbEngine::PostgreSql => {
            let mut c = remote_db_cmd(local.engine, t, "psql");
            c.args(["-v", "ON_ERROR_STOP=1", "-d", t.db_name.trim()]);
            c
        }
    };
    let res = import.stdin(Stdio::from(file)).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    let _ = fs::remove_file(&local_dump);
    let res = res.map_err(|e| e.to_string())?;
    if !res.status.success() {
        return Err(format!(
            "{}\n{}",
            trf("원격 DB에 넣지 못했습니다: {0}", &[&String::from_utf8_lossy(&res.stderr).trim()]),
            trf("넣기 전 백업: {0}", &[&backup.display()])
        ));
    }
    log.push(format!(
        "✓ {}",
        trf("로컬 DB {0} → 원격 {1}", &[&local.name, &format!("{}@{}/{}", t.db_user, t.db_host, t.db_name)])
    ));
    record(p, t, 0, vec![t.db_name.clone()], true, &log);
    Ok(log)
}

fn record(p: &VhostProject, t: &DeployTarget, files: u64, dbs: Vec<String>, ok: bool, notes: &[String]) {
    append_history(TransferRecord {
        at: now(),
        direction: Direction::Deployed,
        peer: t.ssh_host.clone(),
        peer_os: "server".into(),
        projects: vec![p.id.clone()],
        databases: dbs,
        files,
        bytes: 0,
        ok,
        notes: notes.to_vec(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectType;
    use crate::domain::test_util::project;

    fn target() -> DeployTarget {
        DeployTarget {
            ssh_host: "example.com".into(),
            ssh_port: 2222,
            ssh_user: "deploy".into(),
            ssh_key: "/home/me/.ssh/id ed25519".into(),
            remote_path: "/var/www/site".into(),
            excludes: vec![".git/".into(), ".env".into()],
            ..Default::default()
        }
    }

    #[test]
    fn rsync_never_deletes_and_keeps_paths_safe() {
        let p = project(ProjectType::Php, "/srv/site", "", 80);
        let a = rsync_args(&p, &target(), true);
        assert!(!a.iter().any(|x| x.contains("--delete")), "서버 파일을 지우면 안 됩니다");
        assert!(a.contains(&"--dry-run".to_string()));
        assert!(a.contains(&"--exclude=.env".to_string()));
        assert_eq!(a[a.len() - 2], "/srv/site/");
        assert_eq!(a[a.len() - 1], "deploy@example.com:/var/www/site/");
        // 공백 있는 키 경로는 ssh 명령 안에서 따옴표로 감싼다
        assert!(a.iter().any(|x| x.starts_with("ssh -p 2222") && x.contains("'/home/me/.ssh/id ed25519'")));
    }

    #[test]
    fn rejects_dangerous_remote_paths() {
        let mut t = target();
        t.remote_path = "/var/www/my site".into();
        assert!(validate(&t).is_err());
        t.remote_path = "/".into();
        assert!(validate(&t).is_err());
        t.remote_path = "var/www".into();
        assert!(validate(&t).is_err());
        t.remote_path = "/var/www/x;rm -rf ~".into();
        assert!(validate(&t).is_err());
        t.remote_path = "/var/www/site".into();
        assert!(validate(&t).is_ok());
    }

    #[test]
    fn parses_changed_files_from_both_rsync_flavors() {
        let gnu = "sending incremental file list\nindex.php\nsrc/\nsrc/a.php\n\nsent 1 bytes  received 2 bytes\ntotal size is 3  speedup is 1.00 (DRY RUN)\n";
        assert_eq!(changed_files(gnu), vec!["index.php", "src/a.php"]);
        let open = "Transfer starting: 2 files\nx.txt\n\nsent 110 bytes  received 26 bytes\ntotal size is 2  speedup is 0.01\n";
        assert_eq!(changed_files(open), vec!["x.txt"]);
    }
}

