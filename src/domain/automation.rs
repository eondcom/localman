//! MCP·HTTP API 가 함께 쓰는 도구 목록.
//!
//! 조회와 되돌릴 수 있는 동작만 연다. DB 지우기, 프로젝트 지우기, 운영 서버로 올리기는 열지 않는다
//! (화면에서 확인을 거쳐야 하는 동작). 결과에 비밀번호는 담지 않는다.

use serde_json::{Value, json};
use std::path::Path;

use super::database::{DbEngine, backup_database, create_database, list_databases, load_db_connections, postgres_can_connect};
use super::deploy::load_target;
use super::project::{ProjectType, VhostProject, add_project, auto_assign_port, list_projects, port_is_free};
use super::pull::{default_local_db_name, pull_database, pull_files};
use super::server::{ServerStatus, server_status, start_server, stop_server};
use super::settings::data_dir;
use crate::platform::{ServiceStatus, get_service_status, toggle_service};

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema (MCP inputSchema)
    pub schema: fn() -> Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {} })
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "list_projects",
        description: "List local sites: id, name, domain (http://<id>.localhost), type, folder, port, linked DB, dev server state.",
        schema: no_args,
    },
    Tool {
        name: "service_status",
        description: "Status of Apache, the MariaDB/MySQL server and PostgreSQL (running / stopped / not_installed).",
        schema: no_args,
    },
    Tool {
        name: "start_service",
        description: "Start a local server: apache, db (MariaDB/MySQL) or postgresql.",
        schema: || json!({ "type": "object", "properties": { "service": { "type": "string", "enum": ["apache", "db", "postgresql"] } }, "required": ["service"] }),
    },
    Tool {
        name: "stop_service",
        description: "Stop a local server: apache, db (MariaDB/MySQL) or postgresql.",
        schema: || json!({ "type": "object", "properties": { "service": { "type": "string", "enum": ["apache", "db", "postgresql"] } }, "required": ["service"] }),
    },
    Tool {
        name: "add_project",
        description: "Register a local site. It is served at http://<id>.localhost (Apache vhost + hosts entry). type: php (served directly), python or nextjs (dev server proxied by Apache).",
        schema: || json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "lowercase letters, digits, - (becomes <id>.localhost)" },
                "path": { "type": "string", "description": "absolute folder path" },
                "name": { "type": "string" },
                "type": { "type": "string", "enum": ["php", "python", "nextjs"] },
                "start_command": { "type": "string", "description": "python/nextjs only; auto-detected if empty. {port} is replaced with the assigned port" },
                "port": { "type": "integer", "description": "python/nextjs only; dev server port. Auto-assigned (5001~5999, skipping ports in use) if empty" }
            },
            "required": ["id", "path"]
        }),
    },
    Tool {
        name: "start_project",
        description: "Start the dev server of a python/nextjs site.",
        schema: || json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
    },
    Tool {
        name: "stop_project",
        description: "Stop the dev server of a python/nextjs site.",
        schema: || json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
    },
    Tool {
        name: "list_databases",
        description: "List databases on the local DB server (uses the connection saved in LocalMan).",
        schema: || json!({ "type": "object", "properties": { "engine": { "type": "string", "enum": ["mariadb", "postgresql"] } } }),
    },
    Tool {
        name: "create_database",
        description: "Create an empty local database (utf8mb4).",
        schema: || json!({ "type": "object", "properties": { "name": { "type": "string" }, "engine": { "type": "string", "enum": ["mariadb", "postgresql"] } }, "required": ["name"] }),
    },
    Tool {
        name: "backup_database",
        description: "Dump a local database to an .sql file in LocalMan's data folder. Returns the file path.",
        schema: || json!({ "type": "object", "properties": { "name": { "type": "string" }, "engine": { "type": "string", "enum": ["mariadb", "postgresql"] } }, "required": ["name"] }),
    },
    Tool {
        name: "ensure_site_db_users",
        description: "For every site, create the DB account written in its config (Rhymix, XE, WordPress, .env) if missing and grant it access. Existing passwords are not changed.",
        schema: no_args,
    },
    Tool {
        name: "eondctl",
        description: "eond.com(eondcms) 배포 도구 eondctl 을 부른다. action: full(빌드→rsync→poetry·alembic→restart→헬스체크), backend(빌드 없이), frontend(빌드→rsync), quick(rsync→restart), dry-run(전송 목록만), status(무엇을 배포해야 하나), check, server, logs/nginx(lines 줄), history, local-status/local-start/local-stop(LocalMan 에 되묻는 로컬 서버). eondctl API 가 떠 있으면 API 로, 아니면 eondctl CLI 로 실행.",
        schema: || json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": super::eondctl::ACTIONS },
                "lines": { "type": "integer", "description": "logs/nginx 줄 수 (기본 60)" }
            },
            "required": ["action"]
        }),
    },
    Tool {
        name: "eondctl_status",
        description: "eondctl 이 설치되어 있는지, 로컬 API 가 떠 있는지(어느 길로 부를지).",
        schema: no_args,
    },
    Tool {
        name: "pull_from_server",
        description: "Pull a site from its saved live server (set in LocalMan > Projects > Server). files: rsync the web folder into the project folder (never deletes local files, keeps local config). db: dump the server DB and load it into a local DB (the local DB is backed up first).",
        schema: || json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "files": { "type": "boolean", "default": true },
                "db": { "type": "boolean", "default": false },
                "local_db": { "type": "string", "description": "local DB name to load into (default: from the site config)" }
            },
            "required": ["id"]
        }),
    },
];

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).ok_or_else(|| format!("missing argument: {key}"))
}

fn engine_arg(args: &Value) -> Result<DbEngine, String> {
    match str_arg(args, "engine") {
        None | Some("mariadb") | Some("mysql") => Ok(DbEngine::MariaDb),
        Some("postgresql") | Some("postgres") => Ok(DbEngine::PostgreSql),
        Some(o) => Err(format!("unknown engine: {o}")),
    }
}

fn service_id(args: &Value) -> Result<&'static str, String> {
    match required(args, "service")? {
        "apache" | "apache2" => Ok("apache2"),
        "db" | "mariadb" | "mysql" => Ok("mariadb"),
        "postgresql" | "postgres" => Ok("postgresql"),
        o => Err(format!("unknown service: {o}")),
    }
}

fn status_str(s: &ServiceStatus) -> &'static str {
    match s {
        ServiceStatus::Running => "running",
        ServiceStatus::Stopped => "stopped",
        ServiceStatus::NotInstalled => "not_installed",
        ServiceStatus::Unknown => "unknown",
    }
}

fn type_str(t: &ProjectType) -> &'static str {
    match t {
        ProjectType::Php => "php",
        ProjectType::Python => "python",
        ProjectType::NextJs => "nextjs",
    }
}

fn find_project(id: &str) -> Result<VhostProject, String> {
    list_projects().into_iter().find(|p| p.id == id).ok_or_else(|| format!("no such project: {id}"))
}

fn creds(engine: DbEngine) -> Result<(String, String), String> {
    if let Some(c) = load_db_connections().into_iter().find(|c| c.engine == engine) {
        return Ok((c.user, c.password));
    }
    // Homebrew PostgreSQL 은 기본으로 현재 OS 사용자가 비밀번호 없이 슈퍼유저다
    if engine == DbEngine::PostgreSql {
        if let Some(user) = std::env::var("USER").ok().or_else(|| std::env::var("LOGNAME").ok()).filter(|u| !u.is_empty()) {
            if postgres_can_connect(&user, "") {
                return Ok((user, String::new()));
            }
        }
    }
    Err("no saved DB connection — connect once in LocalMan's Database tab".to_string())
}

/// DB·폴더 이름에 쓸 수 있는 글자인지
fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 도구를 실행한다. 결과는 JSON (사람이 읽을 log 줄 포함).
pub fn call(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "list_projects" => Ok(Value::Array(
            list_projects()
                .iter()
                .map(|p| {
                    let server = match server_status(&p.id) {
                        ServerStatus::Running(pid) => json!({ "running": true, "pid": pid }),
                        ServerStatus::Stopped => json!({ "running": false }),
                    };
                    json!({
                        "id": p.id, "name": p.name, "domain": p.domain, "url": format!("http://{}", p.domain),
                        "type": type_str(&p.project_type), "path": p.path, "port": p.port,
                        "db": p.db.as_ref().map(|d| d.name.clone()),
                        "dev_server": if p.project_type == ProjectType::Php { Value::Null } else { server },
                    })
                })
                .collect(),
        )),
        "service_status" => Ok(json!({
            "apache": status_str(&get_service_status("apache2")),
            "db": status_str(&get_service_status("mariadb")),
            "db_label": crate::platform::db_service_label(),
            "postgresql": status_str(&get_service_status("postgresql")),
        })),
        "start_service" | "stop_service" => {
            let id = service_id(args)?;
            let start = name == "start_service";
            toggle_service(id, start)?;
            Ok(json!({ "service": id, "status": status_str(&get_service_status(id)) }))
        }
        "add_project" => {
            let id = required(args, "id")?.to_lowercase();
            if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.starts_with('-') {
                return Err("id: use lowercase letters, digits and -".into());
            }
            let path = required(args, "path")?;
            if !Path::new(path).is_absolute() || !Path::new(path).is_dir() {
                return Err(format!("path must be an existing absolute folder: {path}"));
            }
            if list_projects().iter().any(|p| p.id == id) {
                return Err(format!("project already exists: {id}"));
            }
            let project_type = match str_arg(args, "type").unwrap_or("php") {
                "php" => ProjectType::Php,
                "python" => ProjectType::Python,
                "nextjs" | "next" => ProjectType::NextJs,
                o => return Err(format!("unknown type: {o}")),
            };
            let port = match (project_type.is_proxied(), args.get("port").and_then(|v| v.as_u64())) {
                (false, _) => 80,
                (true, None) => auto_assign_port(),
                (true, Some(p)) => {
                    let p = u16::try_from(p).ok().filter(|p| *p >= 1024).ok_or("port: 1024~65535")?;
                    if let Some(o) = list_projects().iter().find(|o| o.port == p) {
                        return Err(format!("port {p} is already used by project {}", o.id));
                    }
                    if !port_is_free(p) {
                        return Err(format!("port {p} is in use by another program"));
                    }
                    p
                }
            };
            let start_command = match str_arg(args, "start_command") {
                Some(c) => c.replace("{port}", &port.to_string()),
                None if project_type.is_proxied() => super::detect::auto_detect_start_command(path, port),
                None => String::new(),
            };
            let p = VhostProject {
                id: id.clone(),
                name: str_arg(args, "name").unwrap_or(&id).to_string(),
                path: path.to_string(),
                domain: format!("{id}.localhost"),
                project_type,
                port,
                start_command,
                app_dir: String::new(),
                db: None,
            };
            add_project(p)?;
            Ok(json!({ "id": id, "url": format!("http://{id}.localhost") }))
        }
        "start_project" => {
            let p = find_project(required(args, "id")?)?;
            if !p.project_type.is_proxied() {
                return Err("php sites are served by Apache directly — start the apache service instead".into());
            }
            let pid = start_server(&p)?;
            Ok(json!({ "id": p.id, "pid": pid, "url": format!("http://{}", p.domain) }))
        }
        "stop_project" => {
            let id = required(args, "id")?;
            find_project(id)?;
            stop_server(id)?;
            Ok(json!({ "id": id, "running": false }))
        }
        "list_databases" => {
            let engine = engine_arg(args)?;
            let (u, pw) = creds(engine)?;
            Ok(json!(list_databases(engine, &u, &pw).into_iter().map(|d| d.name).collect::<Vec<_>>()))
        }
        "create_database" => {
            let engine = engine_arg(args)?;
            let db = required(args, "name")?;
            if !safe_name(db) {
                return Err("name: use letters, digits, _ and -".into());
            }
            let (u, pw) = creds(engine)?;
            create_database(engine, &u, &pw, db)?;
            Ok(json!({ "created": db }))
        }
        "backup_database" => {
            let engine = engine_arg(args)?;
            let db = required(args, "name")?;
            if !safe_name(db) {
                return Err("name: use letters, digits, _ and -".into());
            }
            let (u, pw) = creds(engine)?;
            let dir = data_dir().join("backups");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let file = dir.join(format!("{db}-{}.sql", super::history::now()));
            backup_database(engine, &u, &pw, db, &file.to_string_lossy())?;
            Ok(json!({ "file": file }))
        }
        "ensure_site_db_users" => {
            let (u, pw) = creds(DbEngine::MariaDb)?;
            Ok(json!({ "log": super::db_users::ensure_site_users(&u, &pw) }))
        }
        "pull_from_server" => {
            let p = find_project(required(args, "id")?)?;
            let t = load_target(&p.id).ok_or("no server saved for this project — set it in LocalMan > Projects > ⋯ > Server")?;
            let files = args.get("files").and_then(|v| v.as_bool()).unwrap_or(true);
            let db = args.get("db").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut log = Vec::new();
            if files {
                log.extend(pull_files(&p, &t, &super::no_progress())?);
            }
            if db {
                let local = str_arg(args, "local_db").map(str::to_string).unwrap_or_else(|| default_local_db_name(&p, &t));
                log.extend(pull_database(&p, &t, &local, &super::no_progress())?);
            }
            Ok(json!({ "log": log }))
        }
        "eondctl" => super::eondctl::run(required(args, "action")?, args.get("lines").and_then(Value::as_u64)),
        "eondctl_status" => Ok(super::eondctl::describe()),
        _ => Err(format!("unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_unique_and_schemas_are_objects() {
        let mut names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOOLS.len());
        for t in TOOLS {
            assert_eq!((t.schema)()["type"], "object", "{}", t.name);
        }
    }

    #[test]
    fn rejects_unknown_and_unsafe_input() {
        assert!(call("drop_database", &json!({})).is_err());
        assert!(call("create_database", &json!({ "name": "x; DROP" })).is_err());
        assert!(call("add_project", &json!({ "id": "../x", "path": "/tmp" })).is_err());
    }
}
