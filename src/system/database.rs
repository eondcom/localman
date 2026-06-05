use std::process::Command;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DbInfo {
    pub name: String,
}

pub fn list_databases(user: &str, password: &str) -> Vec<DbInfo> {
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

pub fn create_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
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

pub fn drop_database(user: &str, password: &str, db_name: &str) -> Result<(), String> {
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

pub fn backup_database(user: &str, password: &str, db_name: &str, output_path: &str) -> Result<(), String> {
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

pub fn restore_database(user: &str, password: &str, db_name: &str, sql_path: &str) -> Result<(), String> {
    if !Path::new(sql_path).exists() {
        return Err("SQL 파일을 찾을 수 없습니다.".to_string());
    }

    let sql = std::fs::read_to_string(sql_path).map_err(|e| e.to_string())?;

    let mut child = Command::new("mysql")
        .args([
            &format!("-u{user}"),
            &format!("-p{password}"),
            db_name,
        ])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    use std::io::Write;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(sql.as_bytes()).map_err(|e| e.to_string())?;
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("복원 실패".to_string())
    }
}
