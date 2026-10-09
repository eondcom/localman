//! 프로젝트 예약 작업(launchd). 규칙: `<프로젝트>/ops/*.plist` 가 그 프로젝트의 작업이다.
//!
//! - 켜기 = `~/Library/LaunchAgents/<Label>.plist` 로 복사 + `launchctl bootstrap`.
//!   끄기 = `bootout` + 복사본 삭제. 지금 실행 = `launchctl kickstart -k`(켜져 있을 때만).
//! - 원본은 프로젝트에 남고 LocalMan 은 복사본만 다룬다. 원본을 고치면 끄고 다시 켜면 반영된다.
//! - projects.json 은 바꾸지 않는다. 맥 전용(리눅스는 plutil·launchctl 이 없어 목록이 비어 나온다).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Job {
    pub label: String,
    pub source: PathBuf,
    /// LaunchAgents 에 올라가 있음
    pub enabled: bool,
    /// 지금 프로세스가 떠 있음
    pub running: bool,
    pub last_exit: Option<i32>,
    /// 일정: "10:20 · 14:20 · 18:20", "Mon 08:00", "every 300s", "keepalive"(상주), "manual"(수동).
    /// 언어와 무관한 값 — 화면은 schedule_label 로 바꿔 보인다.
    pub schedule: String,
    pub log: Option<String>,
}

fn agents_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Library/LaunchAgents")
}

fn domain() -> String {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    format!("gui/{}", unsafe { getuid() })
}

fn plist_get(path: &Path, key: &str, fmt: &str) -> Option<String> {
    let o = Command::new("plutil").args(["-extract", key, fmt, "-o", "-"]).arg(path).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Label 은 파일 이름·launchctl 인자로 쓰이므로 안전한 글자만 허용한다
fn safe_label(l: &str) -> bool {
    !l.is_empty() && l.len() <= 128 && l.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// StartCalendarInterval(사전 하나 또는 배열)을 "HH:MM · HH:MM" 으로
fn calendar_text(v: &serde_json::Value) -> String {
    let items = v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]);
    items
        .iter()
        .map(|i| {
            let hm = format!("{:02}:{:02}", i["Hour"].as_u64().unwrap_or(0), i["Minute"].as_u64().unwrap_or(0));
            match i["Weekday"].as_u64() {
                Some(w) => format!("{} {hm}", ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][w.min(7) as usize]),
                None => hm,
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn schedule_of(path: &Path) -> String {
    if plist_get(path, "KeepAlive", "raw").as_deref() == Some("true") {
        return "keepalive".into();
    }
    if let Some(json) = plist_get(path, "StartCalendarInterval", "json") {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        return calendar_text(&v);
    }
    if let Some(s) = plist_get(path, "StartInterval", "raw") {
        return format!("every {s}s");
    }
    "manual".into()
}

/// `launchctl print` 출력에서 마지막 종료 코드. "(never exited)" 면 None.
fn parse_last_exit(t: &str) -> Option<i32> {
    // "last exit code = 0" 또는 "last exit code = 78: EX_CONFIG"
    t.lines().find_map(|l| {
        let v = l.trim().strip_prefix("last exit code = ")?;
        let n: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        n.parse().ok()
    })
}

pub fn list_jobs(project_path: &str) -> Vec<Job> {
    let Ok(rd) = std::fs::read_dir(Path::new(project_path).join("ops")) else { return vec![] };
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("plist") {
            continue;
        }
        let Some(label) = plist_get(&p, "Label", "raw").filter(|l| safe_label(l)) else { continue };
        let printed = Command::new("launchctl").args(["print", &format!("{}/{label}", domain())]).output().ok();
        let text = printed.as_ref().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string());
        out.push(Job {
            enabled: text.is_some() && agents_dir().join(format!("{label}.plist")).exists(),
            running: text.as_deref().is_some_and(|t| t.contains("state = running")),
            last_exit: text.as_deref().and_then(parse_last_exit),
            schedule: schedule_of(&p),
            log: plist_get(&p, "StandardOutPath", "raw").or_else(|| plist_get(&p, "StandardErrorPath", "raw")),
            label,
            source: p,
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

fn find(project_path: &str, label: &str) -> Result<Job, String> {
    list_jobs(project_path).into_iter().find(|j| j.label == label).ok_or_else(|| format!("no such job: {label}"))
}

pub fn set_job(project_path: &str, label: &str, enabled: bool) -> Result<(), String> {
    let job = find(project_path, label)?;
    let dst = agents_dir().join(format!("{label}.plist"));
    // 켜든 끄든 먼저 내린다 (이미 올라가 있으면 plist 변경을 반영하려고)
    let _ = Command::new("launchctl").args(["bootout", &format!("{}/{label}", domain())]).output();
    if !enabled {
        let _ = std::fs::remove_file(&dst);
        return Ok(());
    }
    std::fs::create_dir_all(agents_dir()).map_err(|e| e.to_string())?;
    std::fs::copy(&job.source, &dst).map_err(|e| e.to_string())?;
    let o = Command::new("launchctl").args(["bootstrap", &domain()]).arg(&dst).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(())
}

pub fn run_job(project_path: &str, label: &str) -> Result<(), String> {
    let job = find(project_path, label)?;
    if !job.enabled {
        return Err("job is off — turn it on first".into());
    }
    let o = Command::new("launchctl")
        .args(["kickstart", "-k", &format!("{}/{label}", domain())])
        .output()
        .map_err(|e| e.to_string())?;
    o.status.success().then_some(()).ok_or_else(|| String::from_utf8_lossy(&o.stderr).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_must_be_safe() {
        assert!(safe_label("com.eond.yasul.crawl"));
        assert!(!safe_label("a/b"));
        assert!(!safe_label("x; rm"));
        assert!(!safe_label(""));
    }

    #[test]
    fn calendar_one_or_many() {
        assert_eq!(calendar_text(&json!({"Hour": 9, "Minute": 5})), "09:05");
        assert_eq!(calendar_text(&json!([{"Hour": 10, "Minute": 20}, {"Hour": 14, "Minute": 20}])), "10:20 · 14:20");
        assert_eq!(calendar_text(&json!({"Weekday": 1, "Hour": 8, "Minute": 0})), "Mon 08:00");
    }

    #[test]
    fn last_exit_code() {
        assert_eq!(parse_last_exit("\tstate = not running\n\tlast exit code = 78: EX_CONFIG\n"), Some(78));
        assert_eq!(parse_last_exit("\tlast exit code = (never exited)\n"), None);
    }
}
