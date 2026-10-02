//! 사이트별 사용 용량: 파일(그중 의존성 폴더 몫), DB, 합계.

use std::fs;
use std::path::Path;

use super::database::{DbEngine, database_size, load_db_connections};
use super::lan::EXCLUDED_DIRS;
use super::project::{ProjectDb, VhostProject, join_dir};

#[derive(Debug, Clone, Default)]
pub struct SiteUsage {
    /// 프로젝트 폴더 전체 (바이트)
    pub files: u64,
    /// 그중 node_modules·venv 등 다시 만들 수 있는 폴더
    pub deps: u64,
    /// 연결된(또는 감지한) DB
    pub db: Option<ProjectDb>,
    /// DB 크기. DB 가 없거나 접속 정보가 없어 못 읽으면 None
    pub db_bytes: Option<u64>,
}

impl SiteUsage {
    pub fn total(&self) -> u64 {
        self.files + self.db_bytes.unwrap_or(0)
    }
}

/// 사이트 폴더 크기. 심볼릭 링크는 따라가지 않는다(같은 파일을 두 번 세거나 폴더 밖으로 나가지 않게).
pub fn files_size(root: &Path) -> (u64, u64) {
    fn walk(dir: &Path, in_deps: bool, total: &mut u64, deps: &mut u64) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.is_dir() {
                let name = e.file_name();
                let is_dep = in_deps || EXCLUDED_DIRS.contains(&name.to_string_lossy().as_ref());
                walk(&e.path(), is_dep, total, deps);
            } else if m.is_file() {
                *total += m.len();
                if in_deps {
                    *deps += m.len();
                }
            }
        }
    }
    let (mut total, mut deps) = (0, 0);
    walk(root, false, &mut total, &mut deps);
    (total, deps)
}

/// 설정 파일에서 사이트가 쓰는 DB 를 찾는다.
/// 라이믹스 files/config/config.php, 라라벨 등의 .env (DB_CONNECTION·DB_DATABASE, DATABASE_URL).
/// 비밀번호는 읽지 않는다.
pub fn detect_db(p: &VhostProject) -> Option<ProjectDb> {
    let roots = [p.work_dir(), p.path.clone()];
    for root in &roots {
        if let Ok(php) = fs::read_to_string(join_dir(root, "files/config/config.php")) {
            if let Some(db) = parse_rhymix_config(&php) {
                return Some(db);
            }
        }
        if let Ok(env) = fs::read_to_string(join_dir(root, ".env")) {
            if let Some(db) = parse_dotenv(&env) {
                return Some(db);
            }
        }
    }
    None
}

/// 'master' => array( ... 'type' => 'mysql', ... 'database' => 'rx', ... )
fn parse_rhymix_config(php: &str) -> Option<ProjectDb> {
    let master = &php[php.find("'master'")?..];
    let value = |key: &str| -> Option<String> {
        let at = master.find(&format!("'{key}'"))?;
        let rest = &master[at + key.len() + 2..];
        let rest = &rest[rest.find("=>")? + 2..];
        let start = rest.find('\'')? + 1;
        let end = start + rest[start..].find('\'')?;
        Some(rest[start..end].to_string())
    };
    let name = value("database").filter(|s| !s.is_empty())?;
    let engine = match value("type").as_deref() {
        Some(t) if t.contains("pgsql") || t.contains("postgres") => DbEngine::PostgreSql,
        _ => DbEngine::MariaDb,
    };
    Some(ProjectDb { engine, name })
}

fn parse_dotenv(env: &str) -> Option<ProjectDb> {
    let get = |key: &str| {
        env.lines().find_map(|l| {
            let l = l.trim();
            let v = l.strip_prefix(key)?.trim_start().strip_prefix('=')?;
            Some(v.trim().trim_matches(['"', '\'']).to_string())
        })
    };
    if let Some(name) = get("DB_DATABASE").filter(|s| !s.is_empty()) {
        let engine = match get("DB_CONNECTION").as_deref() {
            Some("pgsql" | "postgres" | "postgresql") => DbEngine::PostgreSql,
            _ => DbEngine::MariaDb,
        };
        return Some(ProjectDb { engine, name });
    }
    // DATABASE_URL=postgres://user:pw@host:5432/name?sslmode=...
    let url = get("DATABASE_URL")?;
    let (scheme, rest) = url.split_once("://")?;
    let engine = if scheme.starts_with("postgres") { DbEngine::PostgreSql } else if scheme.starts_with("mysql") || scheme.starts_with("mariadb") { DbEngine::MariaDb } else { return None };
    let name = rest.rsplit_once('/')?.1.split('?').next()?.to_string();
    (!name.is_empty()).then_some(ProjectDb { engine, name })
}

/// 연결해 둔 DB, 없으면 자동 감지한 DB
pub fn effective_db(p: &VhostProject) -> Option<ProjectDb> {
    p.db.clone().or_else(|| detect_db(p))
}

/// 사이트 하나의 용량을 잰다 (폴더를 훑으므로 오래 걸릴 수 있다 — 작업 스레드에서 부를 것).
pub fn site_usage(p: &VhostProject) -> SiteUsage {
    let (files, deps) = files_size(Path::new(&p.path));
    let db = effective_db(p);
    let db_bytes = db.as_ref().and_then(|d| {
        let c = load_db_connections().into_iter().find(|c| c.engine == d.engine)?;
        database_size(d.engine, &c.user, &c.password, &d.name)
    });
    SiteUsage { files, deps, db, db_bytes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::TmpDir;

    #[test]
    fn reads_rhymix_db_name_without_password() {
        let php = "<?php\nreturn array(\n'db' => array(\n'master' => array(\n'type' => 'mysql',\n'host' => 'localhost',\n'user' => 'u',\n'pass' => 'secret',\n'database' => 'rx',\n'prefix' => 'rx_',\n),\n),\n);";
        assert_eq!(parse_rhymix_config(php), Some(ProjectDb { engine: DbEngine::MariaDb, name: "rx".into() }));
    }

    #[test]
    fn reads_dotenv_variants() {
        assert_eq!(
            parse_dotenv("APP_NAME=x\nDB_CONNECTION=pgsql\nDB_DATABASE=\"shop\"\n"),
            Some(ProjectDb { engine: DbEngine::PostgreSql, name: "shop".into() })
        );
        assert_eq!(
            parse_dotenv("DATABASE_URL=mysql://u:p@127.0.0.1:3306/blog?charset=utf8mb4\n"),
            Some(ProjectDb { engine: DbEngine::MariaDb, name: "blog".into() })
        );
        assert_eq!(parse_dotenv("DATABASE_URL=sqlite:///tmp/x.db\n"), None);
        assert_eq!(parse_dotenv("FOO=1\n"), None);
    }

    #[test]
    fn counts_files_and_dependency_share() {
        let t = TmpDir::new("usage");
        t.write("a.txt", "12345");
        t.write("node_modules/x/i.js", "1234567890");
        t.write("web/node_modules/y.js", "123");
        let (total, deps) = files_size(&t.0);
        assert_eq!(total, 18);
        assert_eq!(deps, 13);
    }
}
