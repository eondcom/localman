//! 다른 PC(리눅스 ↔ 맥)로 옮기기 위한 백업 묶음 내보내기·가져오기.
//!
//! 묶음 형식 (.tar.gz):
//!   manifest.json                  형식 버전, 만든 OS·홈 경로·시각, 담긴 항목
//!   projects.json                  프로젝트 설정 (localman 데이터 그대로)
//!   db_credentials.json            (선택) 저장된 DB 접속 정보 — 비밀번호가 평문이다
//!   databases/<engine>/<name>.sql  (선택) DB 덤프
//!
//! 프로젝트 소스 폴더는 담지 않는다(git 등으로 따로 옮긴다). 가져올 때 경로 앞부분을
//! 새 PC에 맞게 바꾼다. 예: /home/dell/dev/x → /Users/eond/dev/x
//! 압축은 OS 명령이 아니라 tar·flate2 크레이트로 해서 어느 OS에서나 같은 파일이 나온다.

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::database::{
    DbCredentials, DbEngine, backup_database, import_sql, list_databases, load_db_connections,
    save_db_connection,
};
use super::detect::sync_port_in_command;
use super::project::{ProjectType, VhostProject, add_project, list_projects};

/// 묶음 형식 버전. 읽는 쪽보다 높은 버전은 거부한다.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    /// 만든 시각 (unix 초)
    pub created_at: u64,
    /// "linux" | "macos" ...
    pub source_os: String,
    /// 만든 PC의 홈 디렉토리. 가져올 때 경로 바꾸기의 기본값이 된다.
    pub source_home: String,
    pub hostname: String,
    pub databases: Vec<DbDump>,
    pub includes_credentials: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbDump {
    pub engine: DbEngine,
    pub name: String,
    /// 묶음 안의 상대 경로
    pub file: String,
}

fn engine_dir(engine: DbEngine) -> &'static str {
    match engine {
        DbEngine::MariaDb => "mariadb",
        DbEngine::PostgreSql => "postgresql",
    }
}

/// 엔진별로 저장된 접속 정보 중 첫 번째(가장 최근 저장한 것).
fn connection_for(engine: DbEngine) -> Option<DbCredentials> {
    load_db_connections().into_iter().find(|c| c.engine == engine)
}

fn home_dir() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn scratch_dir(tag: &str) -> Result<PathBuf, String> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("localman-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    Ok(dir)
}

/// 기본 파일 이름: localman-backup-<호스트>-<YYYYMMDD-HHMM>.tar.gz
pub fn default_bundle_name() -> String {
    let stamp = std::process::Command::new("date")
        .arg("+%Y%m%d-%H%M")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
        });
    let host: String = hostname()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    format!("localman-backup-{host}-{stamp}.tar.gz")
}

// ── 내보내기 ──────────────────────────────────────────────────────────

pub struct ExportOptions {
    /// 덤프해서 담을 DB 목록
    pub databases: Vec<(DbEngine, String)>,
    /// 저장된 DB 접속 정보(평문 비밀번호)를 담을지
    pub include_credentials: bool,
}

/// 백업 묶음을 dest에 만든다. DB 하나가 실패해도 나머지는 계속 담고, 결과 요약을 돌려준다.
pub fn export_bundle(dest: &Path, opts: &ExportOptions) -> Result<String, String> {
    let work = scratch_dir("export")?;
    let result = export_into(&work, dest, opts);
    let _ = fs::remove_dir_all(&work);
    result
}

fn export_into(work: &Path, dest: &Path, opts: &ExportOptions) -> Result<String, String> {
    let projects = list_projects();
    let mut notes: Vec<String> = Vec::new();

    // DB 덤프
    let mut dumps: Vec<DbDump> = Vec::new();
    for (engine, name) in &opts.databases {
        let Some(c) = connection_for(*engine) else {
            notes.push(format!("✗ {} {name}: 저장된 접속 정보가 없어 건너뜀", engine.label()));
            continue;
        };
        let rel = format!("databases/{}/{name}.sql", engine_dir(*engine));
        let full = work.join(&rel);
        fs::create_dir_all(full.parent().unwrap()).map_err(|e| e.to_string())?;
        match backup_database(*engine, &c.user, &c.password, name, &full.to_string_lossy()) {
            Ok(()) => dumps.push(DbDump { engine: *engine, name: name.clone(), file: rel }),
            Err(e) => notes.push(format!("✗ {} {name}: 덤프 실패 — {}", engine.label(), e.trim())),
        }
    }

    let manifest = Manifest {
        format: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        source_os: std::env::consts::OS.to_string(),
        source_home: home_dir(),
        hostname: hostname(),
        databases: dumps.clone(),
        includes_credentials: opts.include_credentials,
    };

    fs::write(work.join("manifest.json"), json(&manifest)?).map_err(|e| e.to_string())?;
    fs::write(work.join("projects.json"), json(&projects)?).map_err(|e| e.to_string())?;
    if opts.include_credentials {
        fs::write(work.join("db_credentials.json"), json(&load_db_connections())?)
            .map_err(|e| e.to_string())?;
    }

    // tar.gz로 묶는다. 실패하면 반쯤 쓰인 파일을 남기지 않는다.
    let write = || -> Result<(), String> {
        let file = fs::File::create(dest).map_err(|e| format!("파일 만들기 실패: {e}"))?;
        let mut tar = tar::Builder::new(GzEncoder::new(file, Compression::default()));
        tar.append_dir_all(".", work).map_err(|e| format!("압축 실패: {e}"))?;
        tar.into_inner()
            .and_then(|gz| gz.finish())
            .map_err(|e| format!("압축 마무리 실패: {e}"))?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(dest);
        return Err(e);
    }

    let mut summary = vec![format!(
        "✓ 내보내기 완료: 프로젝트 {}개, DB {}개{}",
        projects.len(),
        dumps.len(),
        if opts.include_credentials { ", DB 접속 정보 포함" } else { "" }
    )];
    summary.extend(notes);
    Ok(summary.join("\n"))
}

fn json<T: Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

// ── 가져오기 ──────────────────────────────────────────────────────────

/// 풀어 둔 백업 묶음. 가져오기를 마치거나 화면을 떠나면 `close`로 임시 폴더를 지운다.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub projects: Vec<VhostProject>,
    pub credentials: Vec<DbCredentials>,
}

impl Bundle {
    pub fn close(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn open_bundle(path: &Path) -> Result<Bundle, String> {
    let file = fs::File::open(path).map_err(|e| format!("파일 열기 실패: {e}"))?;
    let dir = scratch_dir("import")?;
    // tar 크레이트의 unpack은 ".." 이나 절대 경로로 폴더 밖에 쓰는 항목을 거부한다.
    if let Err(e) = tar::Archive::new(GzDecoder::new(file)).unpack(&dir) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("localman 백업 파일이 아니거나 손상됐습니다: {e}"));
    }
    let read = |name: &str| fs::read_to_string(dir.join(name));

    let parsed = (|| -> Result<Bundle, String> {
        let manifest: Manifest = serde_json::from_str(
            &read("manifest.json").map_err(|_| "manifest.json이 없습니다. localman 백업 파일이 아닙니다.")?,
        )
        .map_err(|e| format!("manifest.json 형식 오류: {e}"))?;
        if manifest.format > FORMAT_VERSION {
            return Err(format!(
                "더 새 버전의 localman(형식 {})에서 만든 파일입니다. localman을 업데이트하세요.",
                manifest.format
            ));
        }
        let projects = serde_json::from_str(&read("projects.json").unwrap_or_else(|_| "[]".into()))
            .map_err(|e| format!("projects.json 형식 오류: {e}"))?;
        let credentials = match read("db_credentials.json") {
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("db_credentials.json 형식 오류: {e}"))?,
            Err(_) => Vec::new(),
        };
        Ok(Bundle { dir: dir.clone(), manifest, projects, credentials })
    })();
    if parsed.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    parsed
}

/// 경로 앞부분 from을 to로 바꾼다. 경로 구분자 경계에서만 바꾼다
/// (/home/dell 이 /home/dellbackup 을 건드리지 않게).
pub fn remap_path(path: &str, from: &str, to: &str) -> String {
    let from = from.trim_end_matches('/');
    let to = to.trim_end_matches('/');
    if from.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(from) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{to}{rest}"),
        _ => path.to_string(),
    }
}

/// 명령어 안의 포트 번호 토큰을 바꾼다 ("run_server.py 5001", "0.0.0.0:5001", "--port 5001").
fn replace_port_tokens(cmd: &str, old: u16, new: u16) -> String {
    let (old_s, new_s) = (old.to_string(), new.to_string());
    cmd.split(' ')
        .map(|tok| {
            if tok == old_s {
                new_s.clone()
            } else if let Some(head) = tok.strip_suffix(&format!(":{old_s}")) {
                format!("{head}:{new_s}")
            } else if let Some(head) = tok.strip_suffix(&format!("={old_s}")) {
                format!("{head}={new_s}")
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct ImportOptions {
    pub path_from: String,
    pub path_to: String,
    /// 같은 id의 프로젝트·같은 이름의 DB가 이미 있으면 덮어쓸지 (아니면 건너뜀)
    pub overwrite: bool,
    pub restore_databases: bool,
    pub import_credentials: bool,
}

/// 가져오기 계획에서 프로젝트 한 건이 어떻게 될지 (미리보기와 실제 가져오기가 같은 규칙을 쓴다)
#[derive(Debug, Clone)]
pub struct PlannedProject {
    pub project: VhostProject,
    pub exists_here: bool,
    pub folder_exists: bool,
    pub port_changed_from: Option<u16>,
}

pub fn plan_projects(bundle: &Bundle, opts: &ImportOptions) -> Vec<PlannedProject> {
    let current = list_projects();
    let current_ids: HashSet<String> = current.iter().map(|p| p.id.clone()).collect();
    // 덮어쓰지 않고 남을 기존 프로젝트의 포트는 피해야 한다
    let mut used: HashSet<u16> = current
        .iter()
        .filter(|p| !(opts.overwrite && bundle.projects.iter().any(|b| b.id == p.id)))
        .map(|p| p.port)
        .collect();

    bundle
        .projects
        .iter()
        .map(|src| {
            let mut p = src.clone();
            p.path = remap_path(&p.path, &opts.path_from, &opts.path_to);
            let exists_here = current_ids.contains(&p.id);
            let mut port_changed_from = None;
            let will_add = opts.overwrite || !exists_here;
            if will_add && p.project_type.is_proxied() {
                if used.contains(&p.port) {
                    let old = p.port;
                    let new = (5001u16..6000).find(|c| !used.contains(c)).unwrap_or(old);
                    p.port = new;
                    p.start_command = match p.project_type {
                        ProjectType::NextJs => sync_port_in_command(&p.start_command, new),
                        _ => replace_port_tokens(&p.start_command, old, new),
                    };
                    port_changed_from = Some(old);
                }
                used.insert(p.port);
            }
            PlannedProject {
                folder_exists: Path::new(&p.path).is_dir(),
                project: p,
                exists_here,
                port_changed_from,
            }
        })
        .collect()
}

/// 다른 OS에서 만든 venv는 python 실행 파일 링크가 깨져 있어 쓸 수 없다.
fn venv_is_foreign(p: &VhostProject) -> bool {
    let py = PathBuf::from(p.work_dir()).join("venv/bin/python3");
    py.symlink_metadata().is_ok() && !py.exists()
}

/// 가져오기를 실행한다. 항목 하나가 실패해도 나머지는 계속하고, 줄 단위 결과를 돌려준다.
pub fn import_bundle(bundle: &Bundle, opts: &ImportOptions) -> Vec<String> {
    let mut log: Vec<String> = Vec::new();

    // 1) DB 접속 정보 — DB 복원에 쓰이므로 먼저. 이 PC에 이미 있는 (엔진, 사용자)는
    //    건드리지 않는다: 같은 사용자라도 PC마다 비밀번호가 다를 수 있다.
    if opts.import_credentials && !bundle.credentials.is_empty() {
        let existing = load_db_connections();
        for c in &bundle.credentials {
            if existing.iter().any(|e| e.engine == c.engine && e.user == c.user) {
                log.push(format!("· DB 접속 {} {}: 이 PC 설정 유지", c.engine.label(), c.user));
                continue;
            }
            match save_db_connection(c.engine, &c.user, &c.password) {
                Ok(_) => log.push(format!("✓ DB 접속 {} {} 추가", c.engine.label(), c.user)),
                Err(e) => log.push(format!("✗ DB 접속 {} {}: {e}", c.engine.label(), c.user)),
            }
        }
    }

    // 2) DB 복원
    if opts.restore_databases {
        for d in &bundle.manifest.databases {
            let label = format!("{} {}", d.engine.label(), d.name);
            let Some(c) = connection_for(d.engine) else {
                log.push(format!("✗ DB {label}: 이 PC에 {} 접속 정보가 없습니다 (데이터베이스 탭에서 먼저 연결)", d.engine.label()));
                continue;
            };
            let exists = list_databases(d.engine, &c.user, &c.password).iter().any(|x| x.name == d.name);
            if exists && !opts.overwrite {
                log.push(format!("· DB {label}: 이미 있어 건너뜀"));
                continue;
            }
            let file = bundle.dir.join(&d.file);
            match import_sql(d.engine, &c.user, &c.password, &d.name, &file.to_string_lossy(), true) {
                Ok(()) => log.push(format!("✓ DB {label} 복원{}", if exists { " (덮어씀)" } else { "" })),
                Err(e) => log.push(format!("✗ DB {label}: {}", e.trim())),
            }
        }
    }

    // 3) 프로젝트 — vhost·hosts까지 이 PC 방식으로 새로 만든다
    for plan in plan_projects(bundle, opts) {
        let p = plan.project;
        if plan.exists_here && !opts.overwrite {
            log.push(format!("· 프로젝트 {}: 이미 있어 건너뜀", p.id));
            continue;
        }
        let mut notes: Vec<String> = Vec::new();
        if let Some(old) = plan.port_changed_from {
            notes.push(format!("포트 {old}→{} (충돌)", p.port));
        }
        if !plan.folder_exists {
            notes.push(format!("폴더 없음: {} — 소스를 옮긴 뒤 사용", p.path));
        } else if p.project_type == ProjectType::Python && venv_is_foreign(&p) {
            notes.push("venv가 다른 OS용 — venv 폴더를 지우고 패키지 설치를 다시 하세요".to_string());
        }
        let id = p.id.clone();
        match add_project(p) {
            Ok(()) => {
                let extra = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
                log.push(format!("✓ 프로젝트 {id}{extra}"));
            }
            Err(e) => log.push(format!("✗ 프로젝트 {id}: {e}")),
        }
    }

    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::project;

    #[test]
    fn remap_only_on_path_boundary() {
        assert_eq!(remap_path("/home/dell/dev/x", "/home/dell", "/Users/eond"), "/Users/eond/dev/x");
        assert_eq!(remap_path("/home/dell", "/home/dell/", "/Users/eond/"), "/Users/eond");
        assert_eq!(remap_path("/home/dellbackup/x", "/home/dell", "/Users/eond"), "/home/dellbackup/x");
        assert_eq!(remap_path("/srv/x", "/home/dell", "/Users/eond"), "/srv/x");
        assert_eq!(remap_path("/srv/x", "", "/Users/eond"), "/srv/x");
    }

    #[test]
    fn replaces_port_in_python_commands() {
        assert_eq!(replace_port_tokens("venv/bin/python3 run_server.py 5001", 5001, 5003),
            "venv/bin/python3 run_server.py 5003");
        assert_eq!(replace_port_tokens("venv/bin/python3 manage.py runserver 0.0.0.0:5001", 5001, 5003),
            "venv/bin/python3 manage.py runserver 0.0.0.0:5003");
        assert_eq!(replace_port_tokens("uvicorn a:app --port=5001", 5001, 5003), "uvicorn a:app --port=5003");
        // 포트와 무관한 숫자는 건드리지 않는다
        assert_eq!(replace_port_tokens("python app.py 15001", 5001, 5003), "python app.py 15001");
    }

    /// 리눅스에서 만든 묶음을 그대로 풀어 읽을 수 있어야 한다 (manifest·프로젝트·DB 목록 왕복).
    #[test]
    fn bundle_round_trip() {
        let work = scratch_dir("t-src").unwrap();
        let manifest = Manifest {
            format: FORMAT_VERSION,
            app_version: "0.1.0".into(),
            created_at: 1,
            source_os: "linux".into(),
            source_home: "/home/dell".into(),
            hostname: "dell".into(),
            databases: vec![DbDump { engine: DbEngine::MariaDb, name: "rx".into(), file: "databases/mariadb/rx.sql".into() }],
            includes_credentials: false,
        };
        let projects = vec![project(ProjectType::Php, "/home/dell/dev/rx", "", 80)];
        fs::write(work.join("manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();
        fs::write(work.join("projects.json"), serde_json::to_string(&projects).unwrap()).unwrap();
        fs::create_dir_all(work.join("databases/mariadb")).unwrap();
        fs::write(work.join("databases/mariadb/rx.sql"), "CREATE TABLE t (id int);").unwrap();

        let out = work.with_extension("tar.gz");
        {
            let f = fs::File::create(&out).unwrap();
            let mut tar = tar::Builder::new(GzEncoder::new(f, Compression::default()));
            tar.append_dir_all(".", &work).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }

        let b = open_bundle(&out).unwrap();
        assert_eq!(b.manifest.source_os, "linux");
        assert_eq!(b.projects.len(), 1);
        assert_eq!(b.projects[0].path, "/home/dell/dev/rx");
        assert!(b.dir.join("databases/mariadb/rx.sql").is_file());
        b.close();
        assert!(!b.dir.exists());
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn rejects_non_bundle_files() {
        let f = std::env::temp_dir().join(format!("localman-notbundle-{}.tar.gz", std::process::id()));
        fs::write(&f, b"not a tarball").unwrap();
        assert!(open_bundle(&f).is_err());
        let _ = fs::remove_file(&f);
    }

    #[test]
    fn rejects_newer_format() {
        let work = scratch_dir("t-new").unwrap();
        fs::write(work.join("manifest.json"), format!(
            r#"{{"format":{},"app_version":"9","created_at":0,"source_os":"linux","source_home":"/","hostname":"h","databases":[],"includes_credentials":false}}"#,
            FORMAT_VERSION + 1)).unwrap();
        let out = work.with_extension("tar.gz");
        {
            let f = fs::File::create(&out).unwrap();
            let mut tar = tar::Builder::new(GzEncoder::new(f, Compression::default()));
            tar.append_dir_all(".", &work).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let err = open_bundle(&out).unwrap_err();
        assert!(err.contains("새 버전"), "{err}");
        let _ = fs::remove_dir_all(&work);
        let _ = fs::remove_file(&out);
    }
}

