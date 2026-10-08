//! MAMP 이전 도구: MAMP 의 가상호스트와 MySQL DB 를 로컬맨 백업 묶음으로 만든다.
//!
//! 만든 묶음은 transfer.rs 의 백업 파일과 같은 형식이라, 이 PC 의 [파일에서 가져오기]로
//! 바로 복원하거나 리눅스로 옮겨 가져올 수 있다.
//!
//! MAMP MySQL 이 꺼져 있으면 덤프하는 동안만 띄운다. 이때 포트를 열지 않고 소켓으로만 띄워
//! 로컬맨 MySQL(3306)과 부딪치지 않게 하고, 끝나면 우리가 띄운 경우에만 내린다.
//! MAMP 폴더의 파일은 읽기만 한다.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::database::DbEngine;
use crate::i18n::{tr, trf};
use super::project::{ProjectType, VhostProject};
use super::transfer::{DbDump, FORMAT_VERSION, Manifest, engine_dir, home_dir, hostname, json, pack_dir, scratch_dir};

pub const MAMP_ROOT: &str = "/Applications/MAMP";
const SOCKET: &str = "/Applications/MAMP/tmp/mysql/mysql.sock";

#[derive(Debug, Clone)]
pub struct MampVhost {
    pub server_name: String,
    pub doc_root: String,
    pub exists: bool,
}

#[derive(Debug, Clone)]
pub struct MampDb {
    pub name: String,
    /// 데이터 폴더 크기 (MySQL 을 켜지 않고 잰다)
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct MampInfo {
    pub vhosts: Vec<MampVhost>,
    pub databases: Vec<MampDb>,
    pub data_dir: PathBuf,
}

fn bin(name: &str) -> PathBuf {
    Path::new(MAMP_ROOT).join("Library/bin").join(name)
}

/// MAMP 가 있으면 가상호스트와 DB 목록을 읽는다 (MySQL 을 켜지 않는다)
pub fn detect() -> Option<MampInfo> {
    if !Path::new(MAMP_ROOT).join("MAMP.app").exists() {
        return None;
    }
    let conf = fs::read_to_string(Path::new(MAMP_ROOT).join("conf/apache/extra/httpd-vhosts.conf")).unwrap_or_default();
    let vhosts = parse_vhosts(&conf);

    // 가장 최근 MySQL 데이터 폴더 (mysql57, mysql80 …)
    let db_root = Path::new(MAMP_ROOT).join("db");
    let data_dir = fs::read_dir(&db_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mysql")) && p.join("mysql").is_dir())
        .max()?;
    let system = ["mysql", "performance_schema", "sys", "information_schema"];
    let mut databases: Vec<MampDb> = fs::read_dir(&data_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (!system.contains(&name.as_str()) && !name.starts_with('#')).then(|| MampDb {
                bytes: super::usage::files_size(&e.path()).0,
                name: decode_db_dirname(&name),
            })
        })
        .collect();
    databases.sort_by(|a, b| a.name.cmp(&b.name));
    Some(MampInfo { vhosts, databases, data_dir })
}

/// MySQL 은 특수문자가 든 DB 이름을 폴더명에서 @002d 처럼 적는다
fn decode_db_dirname(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '@' {
            let hex: String = it.by_ref().take(4).collect();
            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                Some(ch) => out.push(ch),
                None => {
                    out.push('@');
                    out.push_str(&hex);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// httpd-vhosts.conf 에서 주석이 아닌 <VirtualHost> 의 ServerName·DocumentRoot 를 읽는다
fn parse_vhosts(conf: &str) -> Vec<MampVhost> {
    let mut out = Vec::new();
    let (mut in_block, mut name, mut root) = (false, String::new(), String::new());
    for line in conf.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if l.starts_with("<VirtualHost") {
            in_block = true;
            name.clear();
            root.clear();
        } else if l.starts_with("</VirtualHost") {
            if in_block && !name.is_empty() && !root.is_empty() {
                out.push(MampVhost { exists: Path::new(&root).is_dir(), server_name: name.clone(), doc_root: root.clone() });
            }
            in_block = false;
        } else if in_block {
            let mut parts = l.splitn(2, char::is_whitespace);
            let key = parts.next().unwrap_or("");
            let val = parts.next().unwrap_or("").trim().trim_matches('"').to_string();
            if key.eq_ignore_ascii_case("ServerName") {
                name = val;
            } else if key.eq_ignore_ascii_case("DocumentRoot") {
                root = val;
            }
        }
    }
    out
}

/// 가상호스트를 로컬맨 프로젝트로 바꾼다. 기본 "localhost" 는 사이트가 아니라 건너뛴다.
pub fn vhost_to_project(v: &MampVhost) -> Option<VhostProject> {
    let host = v.server_name.trim().to_lowercase();
    if host == "localhost" || host.is_empty() {
        return None;
    }
    let id: String = host
        .trim_end_matches(".localhost")
        .trim_end_matches(".local")
        .trim_end_matches(".test")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if id.is_empty() {
        return None;
    }
    Some(VhostProject {
        name: id.clone(),
        domain: format!("{id}.localhost"),
        id,
        path: v.doc_root.clone(),
        project_type: ProjectType::Php,
        port: 80,
        start_command: String::new(),
        app_dir: String::new(),
        db: None,
    })
}

// ── MAMP MySQL ────────────────────────────────────────────────────────

fn mamp_mysql(program: &str, user: &str, password: &str) -> Command {
    let mut c = Command::new(bin(program));
    c.args(["--socket", SOCKET, &format!("-u{user}")]).env("MYSQL_PWD", password).stdin(Stdio::null());
    c
}

fn alive() -> bool {
    Command::new(bin("mysqladmin"))
        .args(["--socket", SOCKET, "ping"])
        .stdin(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// MAMP MySQL 을 소켓 전용(포트 없음)으로 띄운다. 이미 떠 있으면 아무것도 하지 않고 false.
fn start_socket_only(data_dir: &Path) -> Result<bool, String> {
    if alive() {
        return Ok(false);
    }
    let pid = Path::new(MAMP_ROOT).join("tmp/mysql/localman-dump.pid");
    let args = [
        format!("--basedir={MAMP_ROOT}/Library"),
        format!("--datadir={}", data_dir.display()),
        format!("--socket={SOCKET}"),
        format!("--pid-file={}", pid.display()),
        "--skip-networking".to_string(),
        format!("--log-error={MAMP_ROOT}/logs/mysql_error.log"),
    ];
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::platform::spawn_in_new_group(&bin("mysqld").to_string_lossy(), &args, MAMP_ROOT)?;
    for _ in 0..60 {
        if alive() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(trf("MAMP MySQL을 띄우지 못했습니다. 로그: {0}", &[&format!("{MAMP_ROOT}/logs/mysql_error.log")]))
}

/// 우리가 띄운 MAMP MySQL 을 내린다 (비밀번호 없이 pid 로 정상 종료 신호를 보낸다)
fn stop_ours() {
    let pid = Path::new(MAMP_ROOT).join("tmp/mysql/localman-dump.pid");
    if let Ok(p) = fs::read_to_string(&pid) {
        let _ = Command::new("kill").args(["-TERM", p.trim()]).status();
        for _ in 0..60 {
            if !alive() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

pub struct MampExport {
    pub databases: Vec<String>,
    pub projects: Vec<VhostProject>,
    pub user: String,
    pub password: String,
}

/// MAMP 자료로 로컬맨 백업 묶음을 만든다. 결과 줄을 돌려준다.
pub fn export_bundle(dest: &Path, opts: &MampExport, progress: impl Fn(String)) -> Result<Vec<String>, String> {
    let info = detect().ok_or(tr("MAMP를 찾지 못했습니다."))?;
    let work = scratch_dir("mamp")?;
    let result = (|| -> Result<Vec<String>, String> {
        let mut log = Vec::new();
        let mut dumps = Vec::new();
        if !opts.databases.is_empty() {
            progress(tr("MAMP MySQL 준비 중…").into());
            let started = start_socket_only(&info.data_dir)?;
            let dump_all = (|| -> Result<(), String> {
                fs::create_dir_all(work.join("databases").join(engine_dir(DbEngine::MariaDb))).map_err(|e| e.to_string())?;
                for (i, db) in opts.databases.iter().enumerate() {
                    progress(trf("DB 덤프 {0}/{1} · {2}", &[&(i + 1), &opts.databases.len(), db]));
                    let rel = format!("databases/{}/{db}.sql", engine_dir(DbEngine::MariaDb));
                    let out = mamp_mysql("mysqldump", &opts.user, &opts.password)
                        .args(["--single-transaction", "--routines", "--triggers", "--default-character-set=utf8mb4", db])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if out.status.success() {
                        fs::write(work.join(&rel), &out.stdout).map_err(|e| e.to_string())?;
                        dumps.push(DbDump { engine: DbEngine::MariaDb, name: db.clone(), file: rel });
                    } else {
                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        if err.contains("Access denied") {
                            return Err(trf("MAMP MySQL 로그인 실패 — 사용자·비밀번호를 확인하세요 (MAMP 기본 root/root): {0}", &[&err]));
                        }
                        log.push(format!("✗ DB {db}: {err}"));
                    }
                }
                Ok(())
            })();
            if started {
                progress(tr("MAMP MySQL 내리는 중…").into());
                stop_ours();
            }
            dump_all?;
            log.insert(0, format!("✓ {}", trf("MAMP DB {0}개 덤프", &[&dumps.len()])));
        }

        let manifest = Manifest {
            format: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            source_os: std::env::consts::OS.to_string(),
            source_home: home_dir(),
            hostname: format!("{} (MAMP)", hostname()),
            databases: dumps,
            includes_credentials: false,
        };
        fs::write(work.join("manifest.json"), json(&manifest)?).map_err(|e| e.to_string())?;
        fs::write(work.join("projects.json"), json(&opts.projects)?).map_err(|e| e.to_string())?;
        progress(tr("묶는 중…").into());
        pack_dir(&work, dest)?;
        log.push(format!("✓ {}", trf("사이트 {0}개 · 백업 파일: {1}", &[&opts.projects.len(), &dest.display()])));
        Ok(log)
    })();
    let _ = fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_vhosts_only() {
        let conf = "NameVirtualHost *:80\n<VirtualHost *:80>\n    ServerName localhost\n    DocumentRoot \"/Users/me/Sites\"\n</VirtualHost>\n\
                    #<VirtualHost *:80>\n#    ServerName old.localhost\n#    DocumentRoot \"/x\"\n#</VirtualHost>\n\
                    <VirtualHost *:80>\n        ServerName hwp.localhost\n        DocumentRoot \"/Users/me/hwp/public_html\"\n    </VirtualHost>\n";
        let v = parse_vhosts(conf);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].server_name, "hwp.localhost");
        assert_eq!(v[1].doc_root, "/Users/me/hwp/public_html");
        // 기본 localhost 는 사이트로 옮기지 않는다
        assert!(vhost_to_project(&v[0]).is_none());
        let p = vhost_to_project(&v[1]).unwrap();
        assert_eq!((p.id.as_str(), p.domain.as_str()), ("hwp", "hwp.localhost"));
    }

    #[test]
    fn decodes_mysql_dirnames() {
        assert_eq!(decode_db_dirname("my@002ddb"), "my-db");
        assert_eq!(decode_db_dirname("plain_db"), "plain_db");
    }
}

