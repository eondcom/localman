//! 로컬 HTTP API — 설정에서 켜면 앱이 떠 있는 동안 127.0.0.1 에서만 듣는다.
//!
//!   GET  /api/health              → {"ok":true,"version":"…"}  (토큰 없이)
//!   GET  /api/tools               → 도구 목록 (이름·설명·입력 스키마)
//!   POST /api/tools/<이름>         → 본문 JSON 을 인자로 도구 실행
//!
//! 모든 요청(health 제외)에 `Authorization: Bearer <토큰>` 이 필요하다.
//! Host 가 127.0.0.1/localhost 가 아니면 거절한다 (DNS 리바인딩으로 브라우저가 대신 부르는 것을 막는다).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::domain::automation::{TOOLS, call};

pub const DEFAULT_PORT: u16 = 47810;
const MAX_BODY: usize = 1 << 20;

static STOP: AtomicBool = AtomicBool::new(false);
static RUNNING: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// 32자 무작위 토큰
pub fn new_token() -> String {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 듣기 시작한다. 이미 듣고 있으면 멈추고 새 토큰으로 다시 시작한다.
pub fn start(port: u16, token: String) -> Result<(), String> {
    stop();
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    STOP.store(false, Ordering::SeqCst);
    let handle = std::thread::spawn(move || {
        while !STOP.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let token = token.clone();
                    std::thread::spawn(move || {
                        let _ = serve(stream, port, &token);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(150)),
                Err(_) => std::thread::sleep(Duration::from_millis(150)),
            }
        }
    });
    *RUNNING.lock().unwrap() = Some(handle);
    Ok(())
}

pub fn stop() {
    STOP.store(true, Ordering::SeqCst);
    if let Some(h) = RUNNING.lock().unwrap().take() {
        let _ = h.join();
    }
}

/// 길이가 같을 때 걸리는 시간이 내용과 상관없게 비교한다
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_request(stream: &TcpStream) -> Result<Request, String> {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err("too many headers".into());
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err("body too large".into());
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Request { method, path, headers, body })
}

fn respond(mut stream: &TcpStream, code: u16, body: Value) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let text = serde_json::to_string(&body).unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )
}

fn serve(stream: TcpStream, port: u16, token: &str) -> std::io::Result<()> {
    let req = match read_request(&stream) {
        Ok(r) => r,
        Err(e) => return respond(&stream, 400, json!({ "ok": false, "error": e })),
    };
    let host_ok = req
        .header("host")
        .is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if !host_ok {
        return respond(&stream, 403, json!({ "ok": false, "error": "host not allowed" }));
    }
    let path = req.path.split('?').next().unwrap_or("");
    if req.method == "GET" && path == "/api/health" {
        return respond(&stream, 200, json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }));
    }
    let authed = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| same(t.trim(), token));
    if !authed {
        return respond(&stream, 401, json!({ "ok": false, "error": "missing or wrong token (Authorization: Bearer <token>)" }));
    }
    match (req.method.as_str(), path) {
        ("GET", "/api/tools") => respond(
            &stream,
            200,
            json!({ "ok": true, "tools": TOOLS.iter().map(|t| json!({ "name": t.name, "description": t.description, "input": (t.schema)() })).collect::<Vec<_>>() }),
        ),
        ("POST", p) if p.starts_with("/api/tools/") => {
            let name = &p["/api/tools/".len()..];
            let args: Value = if req.body.iter().all(|b| b.is_ascii_whitespace()) {
                json!({})
            } else {
                match serde_json::from_slice(&req.body) {
                    Ok(v) => v,
                    Err(e) => return respond(&stream, 400, json!({ "ok": false, "error": format!("invalid JSON: {e}") })),
                }
            };
            match call(name, &args) {
                Ok(result) => respond(&stream, 200, json!({ "ok": true, "result": result })),
                Err(e) => respond(&stream, 400, json!({ "ok": false, "error": e })),
            }
        }
        _ => respond(&stream, 404, json!({ "ok": false, "error": "not found" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, path: &str, host: &str, token: Option<&str>) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn requires_token_and_local_host() {
        let port = 47899;
        start(port, "secret".into()).unwrap();
        let host = format!("127.0.0.1:{port}");
        assert!(get(port, "/api/health", &host, None).contains("200 OK"));
        assert!(get(port, "/api/tools", &host, None).contains("401"));
        assert!(get(port, "/api/tools", &host, Some("wrong")).contains("401"));
        assert!(get(port, "/api/tools", &host, Some("secret")).contains("list_projects"));
        assert!(get(port, "/api/tools", &format!("evil.example:{port}"), Some("secret")).contains("403"));
        stop();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "멈추면 포트를 닫아야 합니다");
    }
}
