//! CMS 사이트 손보기 — 라이믹스·XE·워드프레스.
//!
//! - 관리자 비밀번호 바꾸기: 사이트 DB 를 직접 고친다 (라이믹스·XE 는 bcrypt `$2y$`, 워드프레스는 MD5 —
//!   워드프레스는 모든 버전이 MD5 를 받아 다음 로그인 때 자기 방식으로 다시 저장한다)
//! - 로컬 DB 로 설정 바꾸기: 설정 파일의 DB 접속(호스트·이름·사용자·비밀번호)과 사이트 주소를 이 PC 것으로 바꾼다.
//!   처음 바꿀 때 원본을 `<파일>.localman-server` 로 남기고, 서버로 올릴 때 그 파일은 자동으로 뺀다

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::database::{DbEngine, load_db_connections};
use super::db_users::{SiteDbAccount, ensure_site_user_for, site_db_account};
use super::project::VhostProject;
use super::settings::load_settings;
use crate::i18n::{tr, trf};

/// 원본(서버) 설정 백업 꼬리표
pub const SERVER_BACKUP_EXT: &str = "localman-server";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    Rhymix,
    Xe,
    WordPress,
}

impl SiteKind {
    pub fn label(self) -> &'static str {
        match self {
            SiteKind::Rhymix => "Rhymix",
            SiteKind::Xe => "XE",
            SiteKind::WordPress => "WordPress",
        }
    }

    /// DB 접속이 적힌 설정 파일 (사이트 폴더 기준)
    pub fn config_file(self) -> &'static str {
        match self {
            SiteKind::Rhymix => "files/config/config.php",
            SiteKind::Xe => "files/config/db.config.php",
            SiteKind::WordPress => "wp-config.php",
        }
    }
}

pub fn detect(p: &VhostProject) -> Option<SiteKind> {
    let root = Path::new(&p.path);
    if root.join("wp-config.php").exists() || root.join("wp-load.php").exists() {
        Some(SiteKind::WordPress)
    } else if root.join("common/autoload.php").exists() {
        Some(SiteKind::Rhymix)
    } else if root.join("files/config/db.config.php").exists() || root.join("config/config.inc.php").exists() {
        Some(SiteKind::Xe)
    } else {
        None
    }
}

/// 서버로 올릴 때 빼야 할 설정 파일 — 로컬용으로 바꾼(원본 백업이 있는) 것과 그 백업 자체
pub fn localized_excludes(p: &VhostProject) -> Vec<String> {
    let root = Path::new(&p.path);
    let mut v = vec![format!("*.{SERVER_BACKUP_EXT}")];
    for kind in [SiteKind::Rhymix, SiteKind::Xe, SiteKind::WordPress] {
        let f = kind.config_file();
        if root.join(format!("{f}.{SERVER_BACKUP_EXT}")).exists() {
            v.push(format!("/{f}"));
        }
    }
    v
}

// ── DB 실행 ─────────────────────────────────────────────────────────────

fn sql_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn ident(s: &str) -> String {
    s.replace('`', "")
}

/// 이 PC 의 DB 에 관리자로 SQL 을 돌린다 (표준입력으로 — 비밀번호는 명령줄에 남기지 않는다)
fn run_sql(db: &str, sql: &str) -> Result<String, String> {
    use std::io::Write;
    let c = load_db_connections()
        .into_iter()
        .find(|c| c.engine == DbEngine::MariaDb)
        .ok_or(tr("로컬 DB 접속 정보가 없습니다. 데이터베이스 탭에서 먼저 연결하세요."))?;
    let mut child = Command::new("mysql")
        .args([&format!("-u{}", c.user), "-N", "-B", "--default-character-set=utf8mb4", db])
        .env("MYSQL_PWD", &c.password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| trf("mysql 실행 실패: {0}", &[&e]))?;
    child.stdin.take().ok_or("stdin")?.write_all(sql.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 사이트의 DB 이름과 표 접두어
fn db_and_prefix(p: &VhostProject, kind: SiteKind) -> Result<(String, String), String> {
    let root = Path::new(&p.path);
    let acc = site_db_account(root).ok_or(tr("사이트 설정 파일에서 DB 정보를 찾지 못했습니다."))?;
    let text = fs::read_to_string(root.join(kind.config_file())).unwrap_or_default();
    let prefix = match kind {
        SiteKind::Rhymix => php_value_after(&text, "'master'", "prefix").unwrap_or_else(|| "rx_".into()),
        SiteKind::Xe => php_value_after(&text, "", "db_table_prefix").unwrap_or_else(|| "xe".into()),
        SiteKind::WordPress => wp_table_prefix(&text).unwrap_or_else(|| "wp_".into()),
    };
    // XE 는 접두어를 'xe' 처럼 _ 없이 적기도 한다
    let prefix = if kind != SiteKind::WordPress && !prefix.ends_with('_') { format!("{prefix}_") } else { prefix };
    Ok((acc.database, ident(&prefix)))
}

// ── 관리자 ──────────────────────────────────────────────────────────────

/// 관리자 계정 (아이디, 이메일)
pub fn list_admins(p: &VhostProject) -> Result<Vec<(String, String)>, String> {
    let kind = detect(p).ok_or(tr("라이믹스·XE·워드프레스 사이트가 아닙니다."))?;
    let (db, pre) = db_and_prefix(p, kind)?;
    let sql = match kind {
        SiteKind::Rhymix | SiteKind::Xe => format!("SELECT user_id, email_address FROM `{pre}member` WHERE is_admin = 'Y' ORDER BY member_srl;"),
        SiteKind::WordPress => format!(
            "SELECT u.user_login, u.user_email FROM `{pre}users` u JOIN `{pre}usermeta` m ON m.user_id = u.ID \
             AND m.meta_key = '{pre}capabilities' WHERE m.meta_value LIKE '%administrator%' ORDER BY u.ID;"
        ),
    };
    Ok(run_sql(&db, &sql)?
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string())))
        .collect())
}

/// 관리자 비밀번호를 바꾼다
pub fn set_admin_password(p: &VhostProject, login: &str, password: &str) -> Result<String, String> {
    let login = login.trim();
    if login.is_empty() {
        return Err(tr("관리자 ID를 입력하세요.").into());
    }
    if password.is_empty() {
        return Err(tr("새 비밀번호를 입력하세요.").into());
    }
    let kind = detect(p).ok_or(tr("라이믹스·XE·워드프레스 사이트가 아닙니다."))?;
    let (db, pre) = db_and_prefix(p, kind)?;
    let sql = match kind {
        SiteKind::Rhymix | SiteKind::Xe => {
            let hash = bcrypt::hash_with_result(password, 10).map_err(|e| e.to_string())?.format_for_version(bcrypt::Version::TwoY);
            format!(
                "UPDATE `{pre}member` SET password = '{}' WHERE user_id = '{}'; SELECT ROW_COUNT();",
                sql_str(&hash),
                sql_str(login)
            )
        }
        SiteKind::WordPress => format!(
            "UPDATE `{pre}users` SET user_pass = MD5('{}') WHERE user_login = '{}'; SELECT ROW_COUNT();",
            sql_str(password),
            sql_str(login)
        ),
    };
    let changed = run_sql(&db, &sql)?.trim().parse::<i64>().unwrap_or(0);
    if changed < 1 {
        // 같은 비밀번호여도 해시가 매번 달라 0 이 나오는 일은 없다 — 0 이면 그 아이디가 없다
        return Err(trf("{0} 계정을 찾지 못했습니다.", &[&login]));
    }
    Ok(trf("{0} {1} 관리자 비밀번호를 바꿨습니다.", &[&kind.label(), &login]))
}

// ── 설정 파일 고치기 ─────────────────────────────────────────────────────

/// `'key' => 값` 의 값 범위 (따옴표 문자열·숫자·NULL)
fn php_value_span(text: &str, from: usize, key: &str) -> Option<(usize, usize)> {
    let needle = format!("'{key}'");
    let mut pos = from;
    while let Some(i) = text[pos..].find(&needle) {
        let after = pos + i + needle.len();
        let rest = &text[after..];
        let trimmed = rest.trim_start();
        if let Some(r) = trimmed.strip_prefix("=>") {
            let vstart = text.len() - r.trim_start().len();
            let v = &text[vstart..];
            let len = if v.starts_with('\'') || v.starts_with('"') {
                let q = v.as_bytes()[0];
                let mut i = 1;
                let b = v.as_bytes();
                while i < b.len() {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == q {
                        break;
                    }
                    i += 1;
                }
                i + 1
            } else {
                v.find([',', ')', '\n']).unwrap_or(v.len())
            };
            return Some((vstart, vstart + len.min(v.len())));
        }
        pos = after;
    }
    None
}

fn php_value_after(text: &str, marker: &str, key: &str) -> Option<String> {
    let from = if marker.is_empty() { 0 } else { text.find(marker)? };
    let (s, e) = php_value_span(text, from, key)?;
    let v = text[s..e].trim();
    Some(v.trim_matches(|c| c == '\'' || c == '"').to_string())
}

fn php_quote(v: &str) -> String {
    format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// marker 이후 처음 나오는 'key' => … 를 바꾼다. 없으면 false.
fn set_php_value(text: &mut String, marker: &str, key: &str, value: &str) -> bool {
    let from = if marker.is_empty() { Some(0) } else { text.find(marker) };
    let Some(from) = from else { return false };
    match php_value_span(text, from, key) {
        Some((s, e)) => {
            text.replace_range(s..e, value);
            true
        }
        None => false,
    }
}

/// define('NAME', '값') 의 값을 바꾼다
fn set_define(text: &mut String, name: &str, value: &str) -> bool {
    for q in ['\'', '"'] {
        let needle = format!("{q}{name}{q}");
        let Some(i) = text.find(&needle) else { continue };
        let after = i + needle.len();
        let Some(comma) = text[after..].find(',') else { continue };
        let vstart = after + comma + 1;
        let v = &text[vstart..];
        let lead = v.len() - v.trim_start().len();
        let vs = vstart + lead;
        let b = text.as_bytes();
        if vs >= b.len() || (b[vs] != b'\'' && b[vs] != b'"') {
            continue;
        }
        let qch = b[vs];
        let mut j = vs + 1;
        while j < b.len() {
            if b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == qch {
                break;
            }
            j += 1;
        }
        text.replace_range(vs..=j.min(b.len() - 1), &php_quote(value));
        return true;
    }
    false
}

fn wp_table_prefix(text: &str) -> Option<String> {
    let i = text.find("$table_prefix")?;
    let rest = text[i..].split_once('=')?.1.trim_start();
    let q = rest.chars().next()?;
    let inner = &rest[1..];
    Some(inner[..inner.find(q)?].to_string())
}

/// 로컬 사이트 주소 (HTTPS 를 켰으면 https)
pub fn local_url(p: &VhostProject) -> String {
    let scheme = if load_settings().https { "https" } else { "http" };
    format!("{scheme}://{}/", p.domain)
}

/// 바꿀 값
#[derive(Debug, Clone)]
pub struct LocalDbSettings {
    pub database: String,
    pub user: String,
    pub password: String,
    /// 사이트 주소도 로컬 도메인으로
    pub url: bool,
}

/// 지금 설정 파일에 적힌 값 (화면의 기본값)
pub fn current_account(p: &VhostProject) -> Option<SiteDbAccount> {
    site_db_account(Path::new(&p.path))
}

fn backup_once(file: &Path) -> Result<Option<PathBuf>, String> {
    let backup = PathBuf::from(format!("{}.{SERVER_BACKUP_EXT}", file.display()));
    if backup.exists() {
        return Ok(None);
    }
    fs::copy(file, &backup).map_err(|e| e.to_string())?;
    Ok(Some(backup))
}

/// 설정 파일의 DB 접속·사이트 주소를 이 PC 것으로 바꾸고, 그 DB 계정을 만든다
pub fn localize(p: &VhostProject, s: &LocalDbSettings) -> Result<Vec<String>, String> {
    let kind = detect(p).ok_or(tr("라이믹스·XE·워드프레스 사이트가 아닙니다."))?;
    let db = s.database.trim();
    let user = s.user.trim();
    if db.is_empty() || user.is_empty() {
        return Err(tr("DB 이름과 사용자를 입력하세요.").into());
    }
    let file = Path::new(&p.path).join(kind.config_file());
    let mut text = fs::read_to_string(&file).map_err(|e| trf("{0} 읽기 실패: {1}", &[&file.display(), &e]))?;
    let url = local_url(p);
    let https = load_settings().https;
    let mut log = Vec::new();

    let ok = match kind {
        SiteKind::Rhymix => {
            let m = "'master'";
            let mut ok = set_php_value(&mut text, m, "host", "'localhost'")
                && set_php_value(&mut text, m, "user", &php_quote(user))
                && set_php_value(&mut text, m, "pass", &php_quote(&s.password))
                && set_php_value(&mut text, m, "database", &php_quote(db));
            set_php_value(&mut text, m, "port", "'3306'");
            if s.url && ok {
                ok = set_php_value(&mut text, "'url'", "default", &php_quote(&url));
                set_php_value(&mut text, "'url'", "ssl", if https { "'always'" } else { "'none'" });
                set_php_value(&mut text, "'url'", "http_port", "NULL");
                set_php_value(&mut text, "'url'", "https_port", "NULL");
            }
            ok
        }
        SiteKind::Xe => {
            let mut ok = set_php_value(&mut text, "", "db_hostname", "'localhost'")
                && set_php_value(&mut text, "", "db_userid", &php_quote(user))
                && set_php_value(&mut text, "", "db_password", &php_quote(&s.password))
                && set_php_value(&mut text, "", "db_database", &php_quote(db));
            set_php_value(&mut text, "", "db_port", "'3306'");
            if s.url && ok {
                // $db_info->default_url = '…'; 형식
                if let Some(i) = text.find("default_url") {
                    if let Some(eq) = text[i..].find('=') {
                        let vs = i + eq + 1;
                        if let Some(semi) = text[vs..].find(';') {
                            text.replace_range(vs..vs + semi, &format!(" {}", php_quote(&url)));
                        }
                    }
                }
                ok = true;
            }
            ok
        }
        SiteKind::WordPress => {
            set_define(&mut text, "DB_HOST", "localhost")
                && set_define(&mut text, "DB_USER", user)
                && set_define(&mut text, "DB_PASSWORD", &s.password)
                && set_define(&mut text, "DB_NAME", db)
        }
    };
    if !ok {
        return Err(trf("{0}에서 DB 설정 자리를 찾지 못했습니다 — 파일을 직접 고쳐 주세요.", &[&kind.config_file()]));
    }
    if let Some(b) = backup_once(&file)? {
        log.push(format!("✓ {}", trf("서버 원본 보관: {0} (서버로 올릴 때는 이 설정 파일을 뺍니다)", &[&b.display()])));
    }
    super::write_atomic(&file, text.as_bytes())?;
    log.push(format!("✓ {}", trf("{0}: DB {1} · 사용자 {2} · localhost", &[&kind.config_file(), &db, &user])));

    // DB 안의 사이트 주소
    if s.url {
        let (_, pre) = db_and_prefix(p, kind)?;
        let domain = p.domain.clone();
        let sql = match kind {
            SiteKind::WordPress => Some(format!(
                "UPDATE `{pre}options` SET option_value = '{}' WHERE option_name IN ('siteurl', 'home');",
                sql_str(url.trim_end_matches('/'))
            )),
            SiteKind::Rhymix => Some(format!(
                "UPDATE `{pre}domains` SET domain = '{}', http_port = NULL, https_port = NULL, security = '{}' WHERE is_default_domain = 'Y';",
                sql_str(&domain),
                if https { "always" } else { "none" }
            )),
            SiteKind::Xe => None,
        };
        if let Some(sql) = sql {
            match run_sql(db, &sql) {
                Ok(_) => log.push(format!("✓ {}", trf("사이트 주소 → {0}", &[&url]))),
                Err(e) => log.push(format!("✗ {}", trf("DB의 사이트 주소를 바꾸지 못했습니다: {0}", &[&e]))),
            }
        } else {
            log.push(format!("✓ {}", trf("사이트 주소 → {0}", &[&url])));
        }
        if kind == SiteKind::WordPress {
            log.push(format!("· {}", tr("글 본문·첨부 경로의 옛 주소는 그대로입니다 (필요하면 WP-CLI search-replace)")));
        }
    }

    // 설정에 적은 계정을 이 PC 의 DB 에 만든다
    if let Some(c) = load_db_connections().into_iter().find(|c| c.engine == DbEngine::MariaDb) {
        log.extend(ensure_site_user_for(p, &c.user, &c.password));
    }
    Ok(log)
}

/// 보관해 둔 서버 원본으로 되돌린다
pub fn restore_server_config(p: &VhostProject) -> Result<String, String> {
    let kind = detect(p).ok_or(tr("라이믹스·XE·워드프레스 사이트가 아닙니다."))?;
    let file = Path::new(&p.path).join(kind.config_file());
    let backup = PathBuf::from(format!("{}.{SERVER_BACKUP_EXT}", file.display()));
    if !backup.exists() {
        return Err(tr("보관해 둔 서버 원본이 없습니다.").into());
    }
    fs::rename(&backup, &file).map_err(|e| e.to_string())?;
    Ok(trf("{0}을(를) 서버 원본으로 되돌렸습니다.", &[&kind.config_file()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_rhymix_config_in_place() {
        let mut t = String::from(
            "<?php\nreturn array(\n\t'db' => array(\n\t\t'master' => array(\n\t\t\t'type' => 'mysql',\n\t\t\t'host' => 'db.server',\n\t\t\t'port' => '3307',\n\t\t\t'user' => 'srv',\n\t\t\t'pass' => 'p\\'w',\n\t\t\t'database' => 'srvdb',\n\t\t\t'prefix' => 'rx_',\n\t\t),\n\t),\n\t'ftp' => array('user' => 'ftp'),\n\t'url' => array(\n\t\t'default' => 'https://example.com/',\n\t\t'http_port' => 8080,\n\t\t'ssl' => 'always',\n\t),\n);\n",
        );
        assert!(set_php_value(&mut t, "'master'", "host", "'localhost'"));
        assert!(set_php_value(&mut t, "'master'", "pass", &php_quote("new'pw")));
        assert!(set_php_value(&mut t, "'url'", "default", "'http://x.localhost/'"));
        assert!(set_php_value(&mut t, "'url'", "http_port", "NULL"));
        assert!(t.contains("'host' => 'localhost'"));
        assert!(t.contains("'pass' => 'new\\'pw'"));
        assert!(t.contains("'ftp' => array('user' => 'ftp')"), "다른 구역은 건드리지 않는다");
        assert!(t.contains("'default' => 'http://x.localhost/'"));
        assert!(t.contains("'http_port' => NULL,"));
        assert_eq!(php_value_after(&t, "'master'", "prefix").as_deref(), Some("rx_"));
    }

    #[test]
    fn edits_wp_config_defines() {
        let mut t = String::from("define( 'DB_NAME', 'live' );\ndefine('DB_PASSWORD', \"a\\\"b\");\n$table_prefix = 'wpx_';\n");
        assert!(set_define(&mut t, "DB_NAME", "local"));
        assert!(set_define(&mut t, "DB_PASSWORD", "x'y"));
        assert!(t.contains("define( 'DB_NAME', 'local' );"));
        assert!(t.contains("define('DB_PASSWORD', 'x\\'y');"));
        assert_eq!(wp_table_prefix(&t).as_deref(), Some("wpx_"));
    }
}

