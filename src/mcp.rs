//! MCP 서버 (stdio). `LocalMan --mcp` 로 실행한다.
//!
//! Claude Code: `claude mcp add -s user localman -- /Applications/LocalMan.app/Contents/MacOS/LocalMan --mcp`
//! 표준입출력으로 한 줄에 JSON-RPC 메시지 하나씩 주고받는다. 도구는 domain::automation 과 같다.

use serde_json::{Value, json};
use std::io::{BufRead, Write};

use crate::domain::automation::{TOOLS, call};

const DEFAULT_PROTOCOL: &str = "2025-06-18";

/// 프로토콜용 출력 — 표준출력을 따로 떼어 두고, 1번(stdout)은 stderr 로 돌린다.
/// 도구가 띄우는 자식 프로세스가 stdout 에 무엇을 쓰더라도 JSON-RPC 흐름을 깨지 않게 한다.
#[cfg(unix)]
fn protocol_out() -> std::fs::File {
    use std::os::fd::FromRawFd;
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        fn dup2(src: i32, dst: i32) -> i32;
    }
    // close-on-exec 로 떼어 둔다 — 그냥 dup 하면 도구가 띄운 서버가 이 채널을 물려받아,
    // MCP 가 끝나도 파이프가 안 닫혀 부른 쪽이 멈춘다.
    #[cfg(target_os = "macos")]
    const F_DUPFD_CLOEXEC: i32 = 67;
    #[cfg(not(target_os = "macos"))]
    const F_DUPFD_CLOEXEC: i32 = 1030;
    unsafe {
        let fd = fcntl(1, F_DUPFD_CLOEXEC, 3);
        dup2(2, 1);
        std::fs::File::from_raw_fd(fd)
    }
}

fn reply(out: &mut impl Write, id: &Value, result: Result<Value, (i64, String)>) {
    let msg = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    };
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn handle(method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "localman", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "LocalMan manages local web development: sites at http://<id>.localhost served by Apache, the local MariaDB/MySQL/PostgreSQL servers, and pulling sites from their live server. Destructive actions (deleting DBs or projects, pushing to live servers) are not available here."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": TOOLS.iter().map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": (t.schema)() })).collect::<Vec<_>>()
        })),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).ok_or((-32602, "missing tool name".to_string()))?;
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // 도구 실패는 프로토콜 오류가 아니라 결과의 isError 로 알린다 (AI 가 읽고 고칠 수 있게)
            Ok(match call(name, &args) {
                Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

pub fn run() {
    #[cfg(unix)]
    let mut out = protocol_out();
    #[cfg(not(unix))]
    let mut out = std::io::stdout();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                reply(&mut out, &Value::Null, Err((-32700, format!("parse error: {e}"))));
                continue;
            }
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // id 가 없으면 알림 — 답하지 않는다
        let Some(id) = msg.get("id") else { continue };
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        reply(&mut out, id, handle(method, &params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_tools_and_answers_unknown_methods() {
        let r = handle("tools/list", &json!({})).unwrap();
        assert!(r["tools"].as_array().unwrap().iter().any(|t| t["name"] == "list_projects"));
        assert_eq!(handle("nope", &json!({})).unwrap_err().0, -32601);
        let init = handle("initialize", &json!({ "protocolVersion": "2024-11-05" })).unwrap();
        assert_eq!(init["protocolVersion"], "2024-11-05");
        let bad = handle("tools/call", &json!({ "name": "drop_everything" })).unwrap();
        assert_eq!(bad["isError"], true);
    }
}
