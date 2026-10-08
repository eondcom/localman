//! DB 사용자 옮기기·만들기 (MariaDB·MySQL).
//!
//! - MAMP 등 다른 서버의 사용자를 비밀번호 해시째 옮긴다 (`dump_users` → 백업 묶음 db_users.json → `apply_users`).
//!   비밀번호 원문은 모르므로 mysql_native_password 해시를 그대로 쓴다.
//! - 사이트 설정 파일(라이믹스·XE·워드프레스·.env)에 적힌 계정으로 사용자를 만든다 (`ensure_site_users`).
//!   이미 있는 사용자의 비밀번호는 바꾸지 않는다 — 다른 사이트가 같은 사용자를 쓸 수 있다.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use super::project::list_projects;
use crate::i18n::{tr, trf};

/// 백업 묶음 안의 사용자 목록 파일
pub const USERS_FILE: &str = "db_users.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigratedUser {
    pub user: String,
    pub host: String,
    /// 인증 방식 (mysql_native_password 만 옮길 수 있다)
    pub plugin: String,
    /// 비밀번호 해시 (*로 시작하는 41자)
    pub auth: String,
    /// SHOW GRANTS 결과 그대로
    pub grants: Vec<String>,
}

/// 서버가 쓰는 계정 — 옮기거나 만들지 않는다
fn is_system_user(user: &str) -> bool {
    user.is_empty()
        || user == "root"
        || user.starts_with("mysql.")
        || user == "mariadb.sys"
        || user == "debian-sys-maint"
        || user == "PUBLIC"
}

/// SQL 문자열 따옴표 안에 넣을 수 있게 바꾼다
fn sql_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn account(user: &str, host: &str) -> String {
    format!("'{}'@'{}'", sql_str(user), sql_str(host))
}

/// mysql 클라이언트에 SQL 을 표준입력으로 넘겨 실행한다 (비밀번호는 명령줄에 남기지 않는다).
/// 결과는 -N -B (머리줄 없음·탭 구분) 형식의 표준출력.
fn run_sql(mut cmd: Command, sql: &str) -> Result<String, String> {
    let mut child = cmd
        .args(["-N", "-B"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| trf("mysql 실행 실패: {0}", &[&e]))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(sql.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 이 PC 의 MariaDB·MySQL 에 관리자로 접속하는 명령
fn local_mysql(admin: &str, admin_pw: &str) -> Command {
    let mut c = Command::new("mysql");
    c.arg(format!("-u{admin}")).env("MYSQL_PWD", admin_pw);
    c
}

/// 사용자와 권한을 읽는다. `mysql` 은 접속 옵션이 붙은 mysql 클라이언트 명령을 만든다.
pub fn dump_users(mysql: impl Fn() -> Command) -> Result<Vec<MigratedUser>, String> {
    let rows = run_sql(mysql(), "SELECT user, host, plugin, authentication_string FROM mysql.user;")?;
    let mut users = Vec::new();
    for line in rows.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let [user, host, plugin, auth] = cols[..] else { continue };
        if is_system_user(user) {
            continue;
        }
        let grants = run_sql(mysql(), &format!("SHOW GRANTS FOR {};", account(user, host)))
            .map(|g| g.lines().map(str::to_string).collect())
            .unwrap_or_default();
        users.push(MigratedUser {
            user: user.to_string(),
            host: host.to_string(),
            plugin: plugin.to_string(),
            auth: if auth == "NULL" { String::new() } else { auth.to_string() },
            grants,
        });
    }
    Ok(users)
}

fn existing_accounts(admin: &str, admin_pw: &str) -> Result<Vec<(String, String)>, String> {
    let rows = run_sql(local_mysql(admin, admin_pw), "SELECT user, host FROM mysql.user;")?;
    Ok(rows
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(u, h)| (u.to_string(), h.to_string()))
        .collect())
}

/// MySQL 8.4 는 mysql_native_password 를 기본으로 끈다. 해시째 옮기려면 켜져 있어야 한다.
fn native_password_ready(admin: &str, admin_pw: &str) -> bool {
    run_sql(
        local_mysql(admin, admin_pw),
        "SELECT PLUGIN_STATUS FROM information_schema.PLUGINS WHERE PLUGIN_NAME = 'mysql_native_password';",
    )
    .map(|s| s.trim() == "ACTIVE")
    .unwrap_or(false)
}

/// 다른 서버에서 읽은 사용자를 이 PC 에 만든다. 이미 있는 사용자는 비밀번호를 건드리지 않고 권한만 더한다.
pub fn apply_users(users: &[MigratedUser], admin: &str, admin_pw: &str) -> Vec<String> {
    let mut log = Vec::new();
    if users.is_empty() {
        return log;
    }
    let needs_native = users.iter().any(|u| u.plugin == "mysql_native_password" && !u.auth.is_empty());
    if needs_native && !native_password_ready(admin, admin_pw) {
        // 앱이 관리하는 MySQL 이면 옵션을 켜고 다시 띄운다
        if let Err(e) = crate::platform::enable_native_password() {
            log.push(format!("✗ {e}"));
        }
    }
    let existing = match existing_accounts(admin, admin_pw) {
        Ok(v) => v,
        Err(e) => {
            log.push(format!("✗ {}", trf("DB 사용자 목록을 읽지 못했습니다: {0}", &[&e])));
            return log;
        }
    };
    for u in users {
        let who = format!("{}@{}", u.user, u.host);
        let exists = existing.iter().any(|(eu, eh)| *eu == u.user && *eh == u.host);
        if !exists {
            let create = if u.auth.is_empty() {
                format!("CREATE USER {};", account(&u.user, &u.host))
            } else if u.plugin == "mysql_native_password" {
                format!(
                    "CREATE USER {} IDENTIFIED WITH mysql_native_password AS '{}';",
                    account(&u.user, &u.host),
                    sql_str(&u.auth)
                )
            } else {
                log.push(format!(
                    "✗ {}",
                    trf("DB 사용자 {0}: 인증 방식 {1}은 옮길 수 없어 건너뜀 — 사용자 탭에서 새로 만드세요", &[&who, &u.plugin])
                ));
                continue;
            };
            if let Err(e) = run_sql(local_mysql(admin, admin_pw), &create) {
                log.push(format!("✗ {}", trf("DB 사용자 {0}: {1}", &[&who, &e])));
                continue;
            }
        }
        let mut failed = Vec::new();
        for g in &u.grants {
            // 접속만 허용하는 USAGE 와 프록시 권한은 옮길 필요가 없다
            if g.starts_with("GRANT USAGE ON *.*") || g.starts_with("GRANT PROXY") {
                continue;
            }
            if let Err(e) = run_sql(local_mysql(admin, admin_pw), &format!("{g};")) {
                failed.push(e);
            }
        }
        let msg = if exists {
            trf("DB 사용자 {0}: 이미 있어 권한만 더함 (비밀번호 유지)", &[&who])
        } else {
            trf("DB 사용자 {0} 옮김", &[&who])
        };
        if failed.is_empty() {
            log.push(format!("✓ {msg}"));
        } else {
            log.push(format!("✗ {msg} — {}", trf("권한 {0}개 실패: {1}", &[&failed.len(), &failed[0]])));
        }
    }
    log
}

// ── 사이트 설정 파일의 DB 계정 ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct SiteDbAccount {
    pub user: String,
    pub pass: String,
    pub database: String,
    pub host: String,
}

/// `'key' => 'value'` 의 value (PHP 작은따옴표 문자열). from 이후에서 처음 나오는 것.
fn php_array_value(text: &str, key: &str) -> Option<String> {
    let needle = format!("'{key}'");
    let mut rest = text;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix("=>") else { continue };
        return php_quoted(after.trim_start());
    }
    None
}

/// 따옴표로 시작하는 PHP 문자열을 읽는다 ('…' 또는 "…")
fn php_quoted(s: &str) -> Option<String> {
    let q = s.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let mut out = String::new();
    let mut chars = s[1..].chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) if n == q || n == '\\' => out.push(n),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => break,
            }
        } else if c == q {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// define('DB_USER', 'x') 의 값
fn php_define(text: &str, name: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let needle = format!("{q}{name}{q}");
        if let Some(i) = text.find(&needle) {
            let after = text[i + needle.len()..].trim_start().strip_prefix(',')?.trim_start();
            return php_quoted(after);
        }
    }
    None
}

fn dotenv_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
        Some(v.trim_matches(|c| c == '"' || c == '\'').to_string())
    })
}

/// 프로젝트 폴더의 설정 파일에서 DB 계정을 찾는다 (라이믹스 → XE → 워드프레스 → .env)
pub fn site_db_account(dir: &Path) -> Option<SiteDbAccount> {
    let read = |rel: &str| fs::read_to_string(dir.join(rel)).ok();
    if let Some(t) = read("files/config/config.php") {
        // 라이믹스: 'db' => ['master' => ['host' => …, 'user' => …, 'pass' => …, 'database' => …]]
        let master = t.find("'master'").map(|i| &t[i..]).unwrap_or(&t);
        return Some(SiteDbAccount {
            user: php_array_value(master, "user")?,
            pass: php_array_value(master, "pass").unwrap_or_default(),
            database: php_array_value(master, "database")?,
            host: php_array_value(master, "host").unwrap_or_default(),
        });
    }
    if let Some(t) = read("files/config/db.config.php") {
        // XE: $db_info->master_db = array('db_hostname' => …, 'db_userid' => …, …)
        return Some(SiteDbAccount {
            user: php_array_value(&t, "db_userid")?,
            pass: php_array_value(&t, "db_password").unwrap_or_default(),
            database: php_array_value(&t, "db_database")?,
            host: php_array_value(&t, "db_hostname").unwrap_or_default(),
        });
    }
    if let Some(t) = read("wp-config.php") {
        return Some(SiteDbAccount {
            user: php_define(&t, "DB_USER")?,
            pass: php_define(&t, "DB_PASSWORD").unwrap_or_default(),
            database: php_define(&t, "DB_NAME")?,
            host: php_define(&t, "DB_HOST").unwrap_or_default(),
        });
    }
    if let Some(t) = read(".env") {
        let conn = dotenv_value(&t, "DB_CONNECTION").unwrap_or_default();
        if conn == "mysql" || conn == "mariadb" {
            return Some(SiteDbAccount {
                user: dotenv_value(&t, "DB_USERNAME")?,
                pass: dotenv_value(&t, "DB_PASSWORD").unwrap_or_default(),
                database: dotenv_value(&t, "DB_DATABASE")?,
                host: dotenv_value(&t, "DB_HOST").unwrap_or_default(),
            });
        }
    }
    None
}

/// 이 PC 의 DB 서버를 가리키는 호스트인지 (포트가 붙어 있어도 된다)
fn is_local_host(host: &str) -> bool {
    let h = host.split(':').next().unwrap_or("").trim();
    h.is_empty() || h == "localhost" || h == "127.0.0.1" || h.starts_with('/')
}

/// 그 계정으로 실제 로그인되는지
fn can_login(user: &str, pass: &str) -> bool {
    run_sql(local_mysql(user, pass), "SELECT 1;").is_ok()
}

/// 등록된 사이트마다 설정 파일의 DB 계정이 이 PC 에 있는지 보고, 없으면 만든다.
pub fn ensure_site_users(admin: &str, admin_pw: &str) -> Vec<String> {
    let mut log = Vec::new();
    let existing = match existing_accounts(admin, admin_pw) {
        Ok(v) => v,
        Err(e) => return vec![format!("✗ {}", trf("DB 사용자 목록을 읽지 못했습니다: {0}", &[&e]))],
    };
    let mut done: Vec<(String, String)> = Vec::new();
    for p in list_projects() {
        let Some(a) = site_db_account(Path::new(&p.path)) else { continue };
        let site = &p.domain;
        if !is_local_host(&a.host) {
            log.push(format!("· {}", trf("{0}: 다른 서버의 DB({1})를 써서 건너뜀", &[site, &a.host])));
            continue;
        }
        if is_system_user(&a.user) {
            log.push(format!("· {}", trf("{0}: {1} 계정을 써서 건너뜀", &[site, &a.user])));
            continue;
        }
        // TCP(127.0.0.1)로 붙는 사이트는 'user'@'127.0.0.1' 로 들어온다
        let mut hosts = vec!["localhost"];
        if a.host.starts_with("127.0.0.1") {
            hosts.push("127.0.0.1");
        }
        for host in hosts {
            let key = (a.user.clone(), host.to_string());
            let exists = existing.contains(&key) || done.contains(&key);
            let who = format!("{}@{host}", a.user);
            if !exists {
                let create = format!("CREATE USER {} IDENTIFIED BY '{}';", account(&a.user, host), sql_str(&a.pass));
                if let Err(e) = run_sql(local_mysql(admin, admin_pw), &create) {
                    log.push(format!("✗ {}", trf("{0}: 사용자 {1} 만들기 실패: {2}", &[site, &who, &e])));
                    continue;
                }
            }
            let grant = format!("GRANT ALL PRIVILEGES ON `{}`.* TO {};", a.database.replace('`', ""), account(&a.user, host));
            if let Err(e) = run_sql(local_mysql(admin, admin_pw), &grant) {
                log.push(format!("✗ {}", trf("{0}: {1} 권한 주기 실패: {2}", &[site, &who, &e])));
                continue;
            }
            done.push(key);
            if exists {
                if host == "localhost" && !can_login(&a.user, &a.pass) {
                    log.push(format!(
                        "✗ {}",
                        trf("{0}: 사용자 {1}가 이미 있지만 비밀번호가 사이트 설정과 다릅니다 — 사용자 탭에서 바꾸세요", &[site, &who])
                    ));
                } else {
                    log.push(format!("✓ {}", trf("{0}: 사용자 {1} 있음 · {2} 권한 확인", &[site, &who, &a.database])));
                }
            } else {
                log.push(format!("✓ {}", trf("{0}: 사용자 {1} 만듦 · {2} 권한", &[site, &who, &a.database])));
            }
        }
    }
    if log.is_empty() {
        log.push(format!("· {}", tr("DB 계정이 적힌 사이트 설정 파일을 찾지 못했습니다")));
    }
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rhymix_config() {
        let dir = std::env::temp_dir().join(format!("lm-dbusers-{}", std::process::id()));
        fs::create_dir_all(dir.join("files/config")).unwrap();
        fs::write(
            dir.join("files/config/config.php"),
            "<?php\nreturn array(\n\t'db' => array(\n\t\t'master' => array(\n\t\t\t'type' => 'mysql',\n\t\t\t'host' => 'localhost',\n\t\t\t'port' => '3306',\n\t\t\t'user' => 'site',\n\t\t\t'pass' => 'p\\'w',\n\t\t\t'database' => 'sitedb',\n\t\t),\n\t),\n\t'ftp' => array('user' => 'ftpuser'),\n);\n",
        )
        .unwrap();
        let a = site_db_account(&dir).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(a, SiteDbAccount { user: "site".into(), pass: "p'w".into(), database: "sitedb".into(), host: "localhost".into() });
    }

    #[test]
    fn reads_wordpress_and_dotenv() {
        let wp = "define( 'DB_NAME', 'wp' );\ndefine('DB_USER', \"wpu\");\ndefine( 'DB_PASSWORD', 'x' );\ndefine( 'DB_HOST', '127.0.0.1:3306' );";
        assert_eq!(php_define(wp, "DB_USER").as_deref(), Some("wpu"));
        assert_eq!(php_define(wp, "DB_HOST").as_deref(), Some("127.0.0.1:3306"));
        assert!(is_local_host("127.0.0.1:3306"));
        assert!(!is_local_host("db.example.com"));
        let env = "DB_CONNECTION=mysql\nDB_DATABASE=\"lara\"\nDB_USERNAME=lu\n";
        assert_eq!(dotenv_value(env, "DB_DATABASE").as_deref(), Some("lara"));
        assert_eq!(dotenv_value(env, "DB_USERNAME").as_deref(), Some("lu"));
    }

    #[test]
    fn quotes_accounts() {
        assert_eq!(account("a'b", "localhost"), "'a\\'b'@'localhost'");
        assert!(is_system_user("mysql.sys") && is_system_user("root") && !is_system_user("eond"));
    }
}

