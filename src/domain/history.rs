//! 이전(보내기·받기) 기록. 프로젝트 목록에 "마지막 이전"을 보여주는 데도 쓴다.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::settings::data_dir;

/// 기록이 끝없이 늘지 않게 최근 것만 남긴다.
const KEEP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Sent,
    Received,
    /// 실서버로 배포 (peer 는 서버 주소)
    Deployed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRecord {
    /// unix 초
    pub at: u64,
    pub direction: Direction,
    /// 상대 PC 이름
    pub peer: String,
    pub peer_os: String,
    /// 옮긴 프로젝트 id (전체 이전이면 전부)
    pub projects: Vec<String>,
    pub databases: Vec<String>,
    /// 실제로 보낸(받은) 파일 수와 바이트 — 바뀐 것만 오가므로 전체 크기와 다르다
    pub files: u64,
    pub bytes: u64,
    pub ok: bool,
    /// 실패·건너뜀 등 결과 줄
    #[serde(default)]
    pub notes: Vec<String>,
}

fn path() -> PathBuf {
    data_dir().join("transfer_history.json")
}

pub fn load_history() -> Vec<TransferRecord> {
    fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn append_history(rec: TransferRecord) {
    let mut list = load_history();
    list.push(rec);
    if list.len() > KEEP {
        let cut = list.len() - KEEP;
        list.drain(..cut);
    }
    match serde_json::to_string_pretty(&list) {
        Ok(s) => {
            if let Err(e) = fs::write(path(), s) {
                eprintln!("[localman] 이전 기록 저장 실패: {e}");
            }
        }
        Err(e) => eprintln!("[localman] 이전 기록 직렬화 실패: {e}"),
    }
}

/// 프로젝트가 마지막으로 오간 기록
pub fn last_for_project(history: &[TransferRecord], id: &str) -> Option<TransferRecord> {
    history.iter().rev().find(|r| r.projects.iter().any(|p| p == id)).cloned()
}

/// 현지 시간대 오프셋(초). `date +%z` 를 한 번만 불러 기억한다.
/// (time 크레이트의 현지 시간대 조회는 멀티스레드 프로그램에서 막혀 있다)
fn local_offset() -> time::UtcOffset {
    static OFFSET: std::sync::OnceLock<time::UtcOffset> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let z = std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        // "+0900" → 32400
        let secs = (|| {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let d = z.get(1..5)?;
            let h: i32 = d[..2].parse().ok()?;
            let m: i32 = d[2..].parse().ok()?;
            Some(sign * (h * 3600 + m * 60))
        })()
        .unwrap_or(0);
        time::UtcOffset::from_whole_seconds(secs).unwrap_or(time::UtcOffset::UTC)
    })
}

/// "10/03 14:20" 형식 (현지 시각)
pub fn format_time(at: u64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(at as i64) {
        Ok(t) => {
            let t = t.to_offset(local_offset());
            format!("{:02}/{:02} {:02}:{:02}", u8::from(t.month()), t.day(), t.hour(), t.minute())
        }
        Err(_) => at.to_string(),
    }
}

/// 프로젝트 줄에 붙일 한 줄 요약. 예: "↗ 10/03 14:20 eond-mac(으)로 보냄"
pub fn summary_line(r: &TransferRecord) -> String {
    let (arrow, verb) = match r.direction {
        Direction::Sent => ("↗", "(으)로 보냄"),
        Direction::Received => ("↙", "에서 받음"),
        Direction::Deployed => ("↑", "에 배포"),
    };
    let status = if r.ok { "" } else { " (일부 실패)" };
    format!("{arrow} {} {}{verb}{status} · 파일 {}개", format_time(r.at), r.peer, r.files)
}
