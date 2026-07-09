use std::process::Command;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DbEngine {
    MariaDb,
    PostgreSql,
}

impl Default for DbEngine {
    fn default() -> Self {
        DbEngine::MariaDb
    }
}

impl DbEngine {
    pub fn label(self) -> &'static str {
        match self {
            DbEngine::MariaDb => "MariaDB",
            DbEngine::PostgreSql => "PostgreSQL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DbInfo {
    pub name: String,
}

/// 저장되는 DB 접속 자격증명 (로컬 개발용 — 평문 저장)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbCredentials {
    #[serde(default)]
    pub engine: DbEngine,
    pub user: String,
    pub password: String,
}

fn credentials_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    let _ = std::fs::create_dir_all(&p);
    p.push("db_credentials.json");
    p
}

/// 저장된 DB 접속 목록을 불러온다. 예전 단일 객체 형식도 호환한다.
pub fn load_db_connections() -> Vec<DbCredentials> {
    let Ok(data) = std::fs::read_to_string(credentials_path()) else {
        return vec![];
    };
    if let Ok(list) = serde_json::from_str::<Vec<DbCredentials>>(&data) {
        return list;
    }
    serde_json::from_str::<DbCredentials>(&data).map(|c| vec![c]).unwrap_or_default()
}

/// DB 접속 정보를 목록에 저장한다. 같은 엔진/사용자는 최신 값으로 갱신한다.
pub fn save_db_connection(engine: DbEngine, user: &str, password: &str) -> Result<Vec<DbCredentials>, String> {
    let mut list = load_db_connections();
    let creds = DbCredentials { engine, user: user.to_string(), password: password.to_string() };
    list.retain(|c| !(c.engine == engine && c.user == user));
    list.insert(0, creds);
    if list.len() > 12 {
        list.truncate(12);
    }
    let data = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    std::fs::write(credentials_path(), data).map_err(|e| e.to_string())?;
    Ok(list)
}

#[derive(Debug, Clone)]
pub struct DbUser {
    pub username: String,
    pub host: String,
}

pub fn list_databases(engine: DbEngine, user: &str, password: &str) -> Vec<DbInfo> {
    match engine {
        DbEngine::MariaDb => list_mariadb_databases(user, password),
        DbEngine::PostgreSql => list_postgres_databases(user, password),
    }
}

fn list_mariadb_databases(user: &str, password: &str) -> Vec<DbInfo> {
    let output = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "-e",
            "SHOW DATABASES;",
            "--skip-column-names",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| !matches!(*l, "information_schema" | "performance_schema" | "mysql" | "sys"))
                .map(|l| DbInfo { name: l.to_string() })
                .collect()
        }
        _ => vec![],
    }
}

fn list_postgres_databases(user: &str, password: &str) -> Vec<DbInfo> {
    let output = postgres_command(user, password, "postgres")
        .args([
            "-At",
            "-c",
            "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname;",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| !matches!(*l, "postgres"))
                .map(|l| DbInfo { name: l.to_string() })
                .collect()
        }
        _ => vec![],
    }
}

pub fn create_database(engine: DbEngine, user: &str, password: &str, db_name: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => create_mariadb_database(user, password, db_name),
        DbEngine::PostgreSql => create_postgres_database(user, password, db_name),
    }
}

fn create_mariadb_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let output = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "-e",
            &format!("CREATE DATABASE `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"),
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn create_postgres_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let sql = format!("CREATE DATABASE {} ENCODING 'UTF8';", pg_ident(db_name));
    run_postgres_sql(user, password, "postgres", &sql)
}

pub fn drop_database(engine: DbEngine, user: &str, password: &str, db_name: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => drop_mariadb_database(user, password, db_name),
        DbEngine::PostgreSql => drop_postgres_database(user, password, db_name),
    }
}

pub fn rename_database(engine: DbEngine, user: &str, password: &str, old_name: &str, new_name: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => Err("MariaDB/MySQL은 안전한 DB 이름 변경을 직접 지원하지 않습니다. 새 DB 생성 후 덤프/복원으로 이관하세요.".to_string()),
        DbEngine::PostgreSql => {
            let sql = format!("ALTER DATABASE {} RENAME TO {};", pg_ident(old_name), pg_ident(new_name));
            run_postgres_sql(user, password, "postgres", &sql)
        }
    }
}

fn drop_mariadb_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let output = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "-e",
            &format!("DROP DATABASE `{db_name}`;"),
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn drop_postgres_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let sql = format!("DROP DATABASE {};", pg_ident(db_name));
    run_postgres_sql(user, password, "postgres", &sql)
}

pub fn backup_database(engine: DbEngine, user: &str, password: &str, db_name: &str, output_path: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => backup_mariadb_database(user, password, db_name, output_path),
        DbEngine::PostgreSql => backup_postgres_database(user, password, db_name, output_path),
    }
}

fn backup_mariadb_database(user: &str, password: &str, db_name: &str, output_path: &str) -> Result<(), String> {
    let output = Command::new("mysqldump")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "--single-transaction",
            db_name,
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        std::fs::write(output_path, &output.stdout).map_err(|e| e.to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn backup_postgres_database(user: &str, password: &str, db_name: &str, output_path: &str) -> Result<(), String> {
    let output = postgres_dump_command(user, password, db_name)
        .args(["--no-owner", "--no-privileges"])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        std::fs::write(output_path, &output.stdout).map_err(|e| e.to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

pub fn list_users(engine: DbEngine, user: &str, password: &str) -> Vec<DbUser> {
    match engine {
        DbEngine::MariaDb => list_mariadb_users(user, password),
        DbEngine::PostgreSql => list_postgres_users(user, password),
    }
}

fn list_mariadb_users(user: &str, password: &str) -> Vec<DbUser> {
    let output = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "-e",
            "SELECT User, Host FROM mysql.user WHERE User != '' ORDER BY User, Host;",
            "--skip-column-names",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| {
                    let parts: Vec<&str> = l.splitn(2, '\t').collect();
                    if parts.len() == 2 {
                        Some(DbUser {
                            username: parts[0].to_string(),
                            host: parts[1].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => vec![],
    }
}

fn list_postgres_users(user: &str, password: &str) -> Vec<DbUser> {
    let output = postgres_command(user, password, "postgres")
        .args([
            "-At",
            "-c",
            "SELECT rolname FROM pg_roles WHERE rolcanlogin = true ORDER BY rolname;",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| DbUser { username: l.to_string(), host: "local".to_string() })
            .collect(),
        _ => vec![],
    }
}

pub fn create_user(engine: DbEngine, admin: &str, admin_pw: &str, new_user: &str, new_pw: &str, host: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => create_mariadb_user(admin, admin_pw, new_user, new_pw, host),
        DbEngine::PostgreSql => create_postgres_user(admin, admin_pw, new_user, new_pw),
    }
}

fn create_mariadb_user(admin: &str, admin_pw: &str, new_user: &str, new_pw: &str, host: &str) -> Result<(), String> {
    let sql = format!(
        "CREATE USER '{new_user}'@'{host}' IDENTIFIED BY '{new_pw}'; FLUSH PRIVILEGES;"
    );
    let output = Command::new("mysql")
        .args([&format!("-u{admin}"), &format!("-p{admin_pw}"), "-e", &sql])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn create_postgres_user(admin: &str, admin_pw: &str, new_user: &str, new_pw: &str) -> Result<(), String> {
    let sql = format!("CREATE USER {} WITH PASSWORD '{}';", pg_ident(new_user), pg_literal(new_pw));
    run_postgres_sql(admin, admin_pw, "postgres", &sql)
}

pub fn drop_user(engine: DbEngine, admin: &str, admin_pw: &str, target_user: &str, host: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => drop_mariadb_user(admin, admin_pw, target_user, host),
        DbEngine::PostgreSql => drop_postgres_user(admin, admin_pw, target_user),
    }
}

pub fn rename_user(engine: DbEngine, admin: &str, admin_pw: &str, target_user: &str, host: &str, new_user: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => {
            let sql = format!("RENAME USER '{}'@'{}' TO '{}'@'{}'; FLUSH PRIVILEGES;", target_user, host, new_user, host);
            run_mariadb_sql(admin, admin_pw, &sql)
        }
        DbEngine::PostgreSql => {
            let sql = format!("ALTER USER {} RENAME TO {};", pg_ident(target_user), pg_ident(new_user));
            run_postgres_sql(admin, admin_pw, "postgres", &sql)
        }
    }
}

pub fn change_user_password(engine: DbEngine, admin: &str, admin_pw: &str, target_user: &str, host: &str, new_pw: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => {
            let sql = format!("ALTER USER '{}'@'{}' IDENTIFIED BY '{}'; FLUSH PRIVILEGES;", target_user, host, new_pw.replace('\'', "''"));
            run_mariadb_sql(admin, admin_pw, &sql)
        }
        DbEngine::PostgreSql => {
            let sql = format!("ALTER USER {} WITH PASSWORD '{}';", pg_ident(target_user), pg_literal(new_pw));
            run_postgres_sql(admin, admin_pw, "postgres", &sql)
        }
    }
}

fn drop_mariadb_user(admin: &str, admin_pw: &str, target_user: &str, host: &str) -> Result<(), String> {
    let sql = format!("DROP USER '{target_user}'@'{host}'; FLUSH PRIVILEGES;");
    let output = Command::new("mysql")
        .args([&format!("-u{admin}"), &format!("-p{admin_pw}"), "-e", &sql])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn drop_postgres_user(admin: &str, admin_pw: &str, target_user: &str) -> Result<(), String> {
    let sql = format!("DROP USER {};", pg_ident(target_user));
    run_postgres_sql(admin, admin_pw, "postgres", &sql)
}

pub fn grant_privileges(engine: DbEngine, admin: &str, admin_pw: &str, target_user: &str, host: &str, db_name: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => grant_mariadb_privileges(admin, admin_pw, target_user, host, db_name),
        DbEngine::PostgreSql => grant_postgres_privileges(admin, admin_pw, target_user, db_name),
    }
}

fn grant_mariadb_privileges(admin: &str, admin_pw: &str, target_user: &str, host: &str, db_name: &str) -> Result<(), String> {
    let sql = format!(
        "GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{target_user}'@'{host}'; FLUSH PRIVILEGES;"
    );
    let output = Command::new("mysql")
        .args([&format!("-u{admin}"), &format!("-p{admin_pw}"), "-e", &sql])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn grant_postgres_privileges(admin: &str, admin_pw: &str, target_user: &str, db_name: &str) -> Result<(), String> {
    let user_ident = pg_ident(target_user);
    let sql = format!(
        "GRANT ALL PRIVILEGES ON DATABASE {} TO {};",
        pg_ident(db_name),
        user_ident,
    );
    run_postgres_sql(admin, admin_pw, "postgres", &sql)?;

    let schema_sql = format!(
        "GRANT USAGE, CREATE ON SCHEMA public TO {0}; GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO {0}; GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO {0}; ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO {0}; ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON SEQUENCES TO {0};",
        user_ident,
    );
    run_postgres_sql(admin, admin_pw, db_name, &schema_sql)
}

pub fn restore_database(engine: DbEngine, user: &str, password: &str, db_name: &str, sql_path: &str) -> Result<(), String> {
    // 기존 DB로 복원. 파일을 stdin으로 스트리밍하고 mysql의 실제 에러를 stderr로 잡는다.
    // (예전 구현은 Rust가 stdin에 write_all 하다가 mysql이 먼저 죽으면 "broken pipe"만 보였다.)
    import_sql(engine, user, password, db_name, sql_path, false)
}

/// SQL 덤프 파일을 가져와 데이터베이스로 복원한다.
///
/// - `create_db`가 true이면 대상 DB를 `IF NOT EXISTS`로 먼저 생성한다.
/// - 파일은 통째로 메모리에 올리지 않고 mysql의 stdin으로 스트리밍한다(대용량/바이너리 안전).
/// - 확장자가 `.gz`이면 `gunzip -c`로 압축을 풀며 파이프한다.
pub fn import_sql(
    engine: DbEngine,
    user: &str,
    password: &str,
    db_name: &str,
    sql_path: &str,
    create_db: bool,
) -> Result<(), String> {
    use std::process::Stdio;

    if db_name.trim().is_empty() {
        return Err("대상 데이터베이스 이름이 비어 있습니다.".to_string());
    }
    let path = Path::new(sql_path);
    if !path.exists() {
        return Err("SQL 파일을 찾을 수 없습니다.".to_string());
    }

    // 1) 필요 시 대상 DB 생성
    if create_db {
        create_database_if_not_exists(engine, user, password, db_name)?;
    }

    // 2) 입력 파일 열기
    let file = std::fs::File::open(path).map_err(|e| format!("파일 열기 실패: {e}"))?;
    let is_gzip = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gz"))
        .unwrap_or(false);

    // 파이프라인: 파일 [→ gunzip] → sed(collation 호환 치환) → mysql
    //
    // 3) (gzip이면) 압축 해제 단계
    let mut gunzip_child: Option<std::process::Child> = None;
    let sed_stdin: Stdio = if is_gzip {
        let mut gunzip = Command::new("gunzip")
            .arg("-c")
            .stdin(Stdio::from(file))
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("gunzip 실행 실패(설치 여부 확인): {e}"))?;
        let out = gunzip
            .stdout
            .take()
            .ok_or_else(|| "gunzip 출력 파이프 생성 실패".to_string())?;
        gunzip_child = Some(gunzip);
        Stdio::from(out)
    } else {
        Stdio::from(file)
    };

    // 4) sed: 덤프 정리
    //    (a) Mac Adminer가 내보낼 때 SQL 앞에 끼어든 PHP 경고/HTML 노이즈 제거
    //        (예: "<br /><b>Warning</b>: PHP Request Startup: Input variables exceeded 1000...")
    //        → 줄 맨 앞(^)에서 시작하는 것만 지우므로 INSERT 데이터 안의 HTML은 보존된다.
    //    (b) MySQL 8.x collation/charset을 MariaDB 호환으로 치환 ("Unknown collation" 방지)
    let sed_script = concat!(
        r"/^<br ?\/>[[:space:]]*$/d", "\n",
        "/^<b>Warning/d\n",
        "/^<b>Notice/d\n",
        "/^<b>Deprecated/d\n",
        "/^<b>Fatal error/d\n",
        "/^<b>Parse error/d\n",
        "/^Warning: /d\n",
        "/^Notice: /d\n",
        "s/utf8mb4_0900_ai_ci/utf8mb4_unicode_ci/g\n",
        "s/utf8mb4_0900_as_cs/utf8mb4_unicode_ci/g\n",
        "s/utf8mb4_0900_bin/utf8mb4_bin/g",
    );
    let mut sed_child = Command::new("sed")
        .arg("-E")
        .arg(sed_script)
        .stdin(sed_stdin)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("sed 실행 실패: {e}"))?;
    let sed_stdout = sed_child
        .stdout
        .take()
        .ok_or_else(|| "sed 출력 파이프 생성 실패".to_string())?;

    // 5) DB 클라이언트 실행 (sed 출력을 stdin으로)
    let db_child = match engine {
        DbEngine::MariaDb => Command::new("mysql")
            .args([
                &format!("-u{user}"),
                &format!("-p{password}"),
                "--default-character-set=utf8mb4",
                db_name,
            ])
            .stdin(Stdio::from(sed_stdout))
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("mysql 실행 실패: {e}"))?,
        DbEngine::PostgreSql => {
            let mut cmd = postgres_command(user, password, db_name);
            cmd.args(["-v", "ON_ERROR_STOP=1"])
                .stdin(Stdio::from(sed_stdout))
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("psql 실행 실패: {e}"))?
        }
    };

    // 6) 완료 대기 및 에러 수집
    let db_out = db_child
        .wait_with_output()
        .map_err(|e| e.to_string())?;
    let _ = sed_child.wait();
    if let Some(mut gz) = gunzip_child {
        let _ = gz.wait();
    }

    if db_out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&db_out.stderr).trim().to_string();
        if err.is_empty() {
            Err("가져오기 실패 (원인 불명)".to_string())
        } else {
            Err(err)
        }
    }
}

fn create_database_if_not_exists(engine: DbEngine, user: &str, password: &str, db_name: &str) -> Result<(), String> {
    match engine {
        DbEngine::MariaDb => create_mariadb_database_if_not_exists(user, password, db_name),
        DbEngine::PostgreSql => create_postgres_database_if_not_exists(user, password, db_name),
    }
}

fn create_mariadb_database_if_not_exists(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let output = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            "-e",
            &format!(
                "CREATE DATABASE IF NOT EXISTS `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
            ),
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn run_mariadb_sql(user: &str, password: &str, sql: &str) -> Result<(), String> {
    let output = Command::new("mysql")
        .args([&format!("-u{user}"), &format!("-p{password}"), "-e", sql])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn create_postgres_database_if_not_exists(user: &str, password: &str, db_name: &str) -> Result<(), String> {
    let check_sql = format!("SELECT 1 FROM pg_database WHERE datname = '{}';", pg_literal(db_name));
    let output = postgres_command(user, password, "postgres")
        .args(["-At", "-c", &check_sql])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    if String::from_utf8_lossy(&output.stdout).trim() == "1" {
        return Ok(());
    }
    create_postgres_database(user, password, db_name)
}

fn run_postgres_sql(user: &str, password: &str, db_name: &str, sql: &str) -> Result<(), String> {
    let output = postgres_command(user, password, db_name)
        .args(["-v", "ON_ERROR_STOP=1", "-c", sql])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn postgres_command(user: &str, password: &str, db_name: &str) -> Command {
    let mut cmd = Command::new("psql");
    cmd.args(["-h", "127.0.0.1", "-U", user, "-d", db_name]);
    if !password.is_empty() {
        cmd.env("PGPASSWORD", password);
    }
    cmd
}

fn postgres_dump_command(user: &str, password: &str, db_name: &str) -> Command {
    let mut cmd = Command::new("pg_dump");
    cmd.args(["-h", "127.0.0.1", "-U", user, "-d", db_name]);
    if !password.is_empty() {
        cmd.env("PGPASSWORD", password);
    }
    cmd
}

fn pg_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn pg_literal(value: &str) -> String {
    value.replace('\'', "''")
}
