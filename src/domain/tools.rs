//! 개발 도구 확인·설치: Node.js(LTS), Python 3, PHP, rsync.
//!
//! Node.js 는 OS 패키지가 오래된 경우가 많아(데비안 apt 등) 공식 배포본을 쓴다.
//! nodejs.org 의 배포 목록에서 지금의 LTS 를 골라 받고, 공식 SHA-256 으로 검증한 뒤
//! 앱 전용 폴더(<데이터 폴더>/tools/node)에 푼다. sudo 가 필요 없고 맥·리눅스가 같다.
//! 나머지는 OS 패키지 관리자(Homebrew·apt)로 설치한다.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::settings::data_dir;
use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Node,
    Python,
    Php,
    Rsync,
}

pub const TOOLS: [Tool; 4] = [Tool::Node, Tool::Python, Tool::Php, Tool::Rsync];

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Node => "Node.js",
            Tool::Python => "Python 3",
            Tool::Php => "PHP",
            Tool::Rsync => "rsync",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Tool::Node => "Next.js·프론트 빌드용. 지금의 LTS를 앱 전용 폴더에 설치 (sudo 불필요)",
            Tool::Python => "Python 프로젝트·venv",
            Tool::Php => "PHP 프로젝트 (Apache 연결 포함)",
            Tool::Rsync => "서버 배포",
        }
    }

    /// 플랫폼 설치에 넘기는 이름
    pub fn key(self) -> &'static str {
        match self {
            Tool::Node => "node",
            Tool::Python => "python",
            Tool::Php => "php",
            Tool::Rsync => "rsync",
        }
    }

    fn probe(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Tool::Node => ("node", &["--version"]),
            Tool::Python => ("python3", &["--version"]),
            Tool::Php => ("php", &["-v"]),
            Tool::Rsync => ("rsync", &["--version"]),
        }
    }
}

/// 설치돼 있으면 버전 문자열 (첫 줄에서 숫자 부분)
pub fn installed_version(t: Tool) -> Option<String> {
    let (prog, args) = t.probe();
    let out = Command::new(prog).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let first = text.lines().next().unwrap_or("").trim();
    // "v24.21.0" / "Python 3.12.5" / "PHP 8.4.1 (cli) ..." / "rsync  version 3.4.4 ..." / "openrsync: protocol version 29"
    let ver = first
        .split_whitespace()
        .find(|w| w.trim_start_matches('v').chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|w| w.trim_start_matches('v').trim_end_matches(',').to_string())
        .unwrap_or_else(|| first.to_string());
    Some(ver)
}

pub fn install(t: Tool) -> Result<String, String> {
    match t {
        Tool::Node => install_node_lts(),
        _ => platform::install_tool(t.key()),
    }
}

// ── Node.js LTS ────────────────────────────────────────────────────────

pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

/// 앱이 설치한 Node 의 bin 폴더 (PATH 앞에 붙인다)
pub fn node_bin_dir() -> PathBuf {
    tools_dir().join("node").join("bin")
}

fn curl(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", url])
        .output()
        .map_err(|e| format!("curl 실행 실패: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("{url} 받기 실패: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// nodejs.org 배포 목록에서 가장 최근 LTS (목록은 최신순)
pub fn latest_lts() -> Result<(String, String), String> {
    let index: serde_json::Value =
        serde_json::from_slice(&curl("https://nodejs.org/dist/index.json")?).map_err(|e| format!("배포 목록 형식 오류: {e}"))?;
    pick_lts(&index).ok_or_else(|| "LTS 버전을 찾지 못했습니다.".into())
}

/// (버전, LTS 이름) — lts 가 false 가 아닌 첫 항목
fn pick_lts(index: &serde_json::Value) -> Option<(String, String)> {
    index.as_array()?.iter().find_map(|r| {
        let name = r.get("lts")?.as_str()?;
        Some((r.get("version")?.as_str()?.to_string(), name.to_string()))
    })
}

/// 설치된 Node 의 지원 상태 (nodejs/Release 의 schedule.json 기준)
#[derive(Debug, Clone, PartialEq)]
pub enum NodeSupport {
    /// 활발히 지원되는 LTS
    ActiveLts(String),
    /// 보안 수정만 받는 LTS — (이름, 지원 종료일)
    MaintenanceLts(String, String),
    /// LTS 가 아닌 판 (홀수 판, LTS 되기 전)
    Current,
    /// 지원이 끝난 판
    Eol,
    /// 일정에 없음 (알 수 없음)
    Unknown,
}

/// Node 공식 릴리스 일정
pub fn node_schedule() -> Result<serde_json::Value, String> {
    serde_json::from_slice(&curl("https://raw.githubusercontent.com/nodejs/Release/main/schedule.json")?)
        .map_err(|e| format!("릴리스 일정 형식 오류: {e}"))
}

pub fn today() -> String {
    let d = time::OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// 날짜는 YYYY-MM-DD 문자열이라 사전순 비교가 곧 날짜 비교다.
pub fn node_support(schedule: &serde_json::Value, version: &str, today: &str) -> NodeSupport {
    let Some(major) = version.trim_start_matches('v').split('.').next() else { return NodeSupport::Unknown };
    let Some(e) = schedule.get(format!("v{major}")) else { return NodeSupport::Unknown };
    let get = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let (lts, maint, end) = (get("lts"), get("maintenance"), get("end"));
    let name = get("codename").to_string();
    if !end.is_empty() && end <= today {
        NodeSupport::Eol
    } else if lts.is_empty() || lts > today {
        NodeSupport::Current
    } else if !maint.is_empty() && maint <= today {
        NodeSupport::MaintenanceLts(name, end.to_string())
    } else {
        NodeSupport::ActiveLts(name)
    }
}

fn node_platform() -> Result<&'static str, String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => return Err(format!("{other}용 Node 자동 설치는 아직 지원하지 않습니다.")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(format!("{other} CPU용 Node 자동 설치는 아직 지원하지 않습니다.")),
    };
    Ok(match (os, arch) {
        ("darwin", "x64") => "darwin-x64",
        ("darwin", _) => "darwin-arm64",
        ("linux", "x64") => "linux-x64",
        _ => "linux-arm64",
    })
}

fn install_node_lts() -> Result<String, String> {
    let (version, codename) = latest_lts()?;
    let plat = node_platform()?;
    let file = format!("node-{version}-{plat}.tar.gz");
    let base = format!("https://nodejs.org/dist/{version}");

    // 공식 체크섬과 맞는지 확인한 뒤에만 푼다
    let sums = String::from_utf8_lossy(&curl(&format!("{base}/SHASUMS256.txt"))?).to_string();
    let expected = sums
        .lines()
        .find_map(|l| l.strip_suffix(&format!("  {file}")).map(str::to_string))
        .ok_or_else(|| format!("{file} 의 체크섬을 찾지 못했습니다."))?;
    let data = curl(&format!("{base}/{file}"))?;
    let actual = format!("{:x}", Sha256::digest(&data));
    if actual != expected {
        return Err(format!("내려받은 파일의 체크섬이 맞지 않아 설치를 멈췄습니다 ({file})."));
    }

    let dir = tools_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join(&file);
    fs::write(&archive, &data).map_err(|e| e.to_string())?;
    let out = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&dir).output().map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&archive);
    if !out.status.success() {
        return Err(format!("압축 풀기 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }

    // tools/node → node-vXX-플랫폼 (다음 LTS 로 바꿀 때 링크만 갈아 끼운다)
    let unpacked = dir.join(file.trim_end_matches(".tar.gz"));
    let link = dir.join("node");
    let _ = fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&unpacked, &link).map_err(|e| e.to_string())?;
    prepend_path(&node_bin_dir());
    Ok(format!("Node.js {version} ({codename} LTS) 설치: {}", unpacked.display()))
}

/// 앱이 설치한 도구를 PATH 앞에 둔다 (시작할 때와 설치 직후)
pub fn prepend_path(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    let cur = std::env::var("PATH").unwrap_or_default();
    let d = dir.to_string_lossy().to_string();
    if cur.split(':').any(|p| p == d) {
        return;
    }
    // SAFETY: 시작 직후 또는 설치 작업 한 곳에서만 바꾼다. 다른 스레드가 PATH 를 동시에 바꾸지 않는다.
    unsafe { std::env::set_var("PATH", format!("{d}:{cur}")) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_node_support_by_schedule() {
        let sch = serde_json::json!({
            "v12": {"lts": "2019-10-21", "maintenance": "2020-11-30", "end": "2022-04-30", "codename": "Erbium"},
            "v22": {"lts": "2024-10-29", "maintenance": "2025-10-21", "end": "2027-04-30", "codename": "Jod"},
            "v24": {"lts": "2025-10-28", "maintenance": "2026-10-20", "end": "2028-04-30", "codename": "Krypton"},
            "v25": {"maintenance": "2026-04-01", "end": "2026-06-01"},
            "v26": {"lts": "2026-10-28", "maintenance": "2027-10-20", "end": "2029-04-30", "codename": ""}
        });
        let t = "2026-10-03";
        assert_eq!(node_support(&sch, "12.14.1", t), NodeSupport::Eol);
        assert_eq!(node_support(&sch, "22.15.0", t), NodeSupport::MaintenanceLts("Jod".into(), "2027-04-30".into()));
        assert_eq!(node_support(&sch, "v24.21.0", t), NodeSupport::ActiveLts("Krypton".into()));
        assert_eq!(node_support(&sch, "25.1.0", t), NodeSupport::Eol);
        assert_eq!(node_support(&sch, "26.0.0", t), NodeSupport::Current);
        assert_eq!(node_support(&sch, "99.0.0", t), NodeSupport::Unknown);
    }

    #[test]
    fn picks_newest_lts_not_current() {
        let index = serde_json::json!([
            {"version": "v26.1.0", "lts": false},
            {"version": "v24.21.0", "lts": "Krypton"},
            {"version": "v22.20.0", "lts": "Jod"}
        ]);
        assert_eq!(pick_lts(&index), Some(("v24.21.0".into(), "Krypton".into())));
    }
}

