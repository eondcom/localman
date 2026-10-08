//! eondctl 연동 — eond.com(eondcms) 배포 도구를 LocalMan 의 MCP·HTTP API 도구로 부른다.
//!
//! 두 길이 있다.
//!   1. eondctl 로컬 HTTP API(127.0.0.1:47811, 토큰은 ~/.config/eondctl/config.json 의 api_token)가
//!      떠 있으면 거기에 POST /api/tools/eondctl — GUI 가 떠 있을 때(설정 api_enabled) 또는 `eondctl api`.
//!   2. 아니면 설치된 실행 파일(`~/.local/bin/eondctl` → `~/.local/share/eondctl/eondctl`)을
//!      `eondctl <action>` 으로 띄워 출력을 받는다. 항상 되는 길이라 API 가 꺼져 있어도 배포는 된다.
//!
//! 어느 길이든 결과는 요약 문자열(단계별 ✓/✗, 핵심 줄)이고 전체 로그는 eondctl 쪽 파일에 남는다.
//! 거꾸로 eondctl 은 LocalMan 의 API(start_project 등)로 로컬 서버를 켜고 끈다 — 서로 부르는 구조.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47811;

/// MCP·API 에서 받는 action 값. eondctl 쪽 ACTIONS 와 같아야 한다.
pub const ACTIONS: &[&str] = &[
    "full", "backend", "frontend", "quick", "dry-run", "status", "check", "server", "logs", "nginx", "history",
    "local-status", "local-start", "local-stop",
];

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// eondctl 설정(~/.config/eondctl/config.json) — 리눅스·맥 모두 이 위치(eondctl 이 그렇게 정했다).
fn eondctl_config() -> Value {
    std::fs::read_to_string(home().join(".config/eondctl/config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// 설치된 eondctl 실행 파일. 없으면 None.
pub fn binary() -> Option<PathBuf> {
    [home().join(".local/bin/eondctl"), home().join(".local/share/eondctl/eondctl")]
        .into_iter()
        .find(|p| p.is_file())
}

fn http(port: u16, method: &str, path: &str, token: Option<&str>, body: Option<&Value>, read_timeout: Duration) -> Result<(u16, Value), String> {
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(800))
        .map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    let _ = s.set_read_timeout(Some(read_timeout));
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Ok((code, serde_json::from_str(body).unwrap_or_else(|_| json!({ "raw": body }))))
}

/// eondctl API 가 떠 있으면 (포트, 토큰).
fn api() -> Option<(u16, String)> {
    let cfg = eondctl_config();
    let port = cfg["api_port"].as_u64().map(|p| p as u16).unwrap_or(DEFAULT_PORT);
    let token = cfg["api_token"].as_str().unwrap_or("").to_string();
    if token.is_empty() {
        return None;
    }
    matches!(http(port, "GET", "/api/health", None, None, Duration::from_secs(2)), Ok((200, _))).then_some((port, token))
}

/// ANSI 색상 코드 제거 — CLI 출력은 색이 섞여 있다.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 현재 상태 한 줄 — 도구 응답과 화면에 쓴다.
pub fn describe() -> Value {
    let bin = binary();
    let via_api = api().is_some();
    json!({
        "installed": bin.is_some(),
        "binary": bin.as_ref().map(|p| p.display().to_string()),
        "api": via_api,
        "route": if via_api { "api" } else if bin.is_some() { "cli" } else { "none" },
    })
}

/// 배포·조회 실행. 성공 여부와 요약 문자열.
pub fn run(action: &str, lines: Option<u64>) -> Result<Value, String> {
    if !ACTIONS.contains(&action) {
        return Err(format!("action must be one of: {}", ACTIONS.join(", ")));
    }
    if let Some((port, token)) = api() {
        let mut args = json!({ "action": action });
        if let Some(n) = lines {
            args["lines"] = json!(n);
        }
        // 전체 배포는 몇 분 걸린다
        let (code, v) = http(port, "POST", "/api/tools/eondctl", Some(&token), Some(&args), Duration::from_secs(1800))?;
        if code != 200 {
            return Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("eondctl API HTTP {code}")));
        }
        let ok = v["ok"].as_bool().unwrap_or(false);
        let text = v["text"].as_str().unwrap_or("").to_string();
        return if ok { Ok(json!({ "ok": true, "route": "api", "text": text })) } else { Err(text) };
    }
    let Some(bin) = binary() else {
        return Err("eondctl 이 설치되어 있지 않다 (~/.local/bin/eondctl) — eondctl 저장소에서 cargo build --release 후 설치".into());
    };
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg(action);
    if let Some(n) = lines {
        cmd.arg(n.to_string());
    }
    // stdin 을 닫아 두면 eondctl 은 tty 가 아니라고 보고 묻지 않는다(실패 시 "다시 시도?" 프롬프트 없음)
    cmd.stdin(std::process::Stdio::null());
    let out = cmd.output().map_err(|e| format!("{}: {e}", bin.display()))?;
    let text = strip_ansi(&format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    let text = text.trim().to_string();
    if out.status.success() { Ok(json!({ "ok": true, "route": "cli", "text": text })) } else { Err(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_rejects_unknown_action() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m ok"), "bold ok");
        assert!(run("rm-rf", None).is_err());
    }
}
