//! 같은 네트워크(와이파이)의 다른 localman 으로 바로 이전한다.
//!
//! 흐름
//!   1. 받는 PC: [받기 대기] → 6자리 코드 표시, TCP 47801 대기 + UDP 47800 발견 응답
//!   2. 보내는 PC: UDP 브로드캐스트로 받기 대기 중인 PC 를 찾고, 코드를 입력해 접속
//!   3. 코드로 SPAKE2 키 합의 → 이후 모든 프레임을 ChaCha20-Poly1305 로 암호화
//!      (6자리 코드여도 엿본 트래픽으로 오프라인 대입이 안 된다. 받는 쪽은 3번 틀리면 멈춘다)
//!   4. 프로젝트마다 파일 목록(크기·수정 시각)을 보내면 받는 쪽이 필요한 파일만 고른다 (rsync 의 quick check)
//!   5. 설정·DB 는 백업 묶음(transfer.rs)을 그대로 실어 보내고, 받는 쪽이 같은 가져오기 로직으로 적용한다
//!
//! 받는 쪽은 자기 홈 폴더 안에만 쓴다. 보내는 쪽 홈 밖의 프로젝트는 ~/localman-received/<id> 로 받는다.
//! 받는 쪽에만 있는 파일은 지우지 않는다.

use chacha20poly1305::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::database::DbEngine;
use super::history::{Direction, TransferRecord, append_history, now};
use super::project::{VhostProject, list_projects};
use super::transfer::{ExportOptions, ImportOptions, export_bundle, import_bundle, open_bundle, remap_path};

pub const DISCOVERY_PORT: u16 = 47800;
pub const TRANSFER_PORT: u16 = 47801;
const MAGIC: &[u8] = b"LOCALMAN-LAN-1\n";
const DISCOVER_REQ: &[u8] = b"LOCALMAN-DISCOVER-1";
const SPAKE_ID: &[u8] = b"localman-lan-v1";
const CHUNK: usize = 1 << 20;
/// 암호문 한 프레임의 최대 크기 (청크 + 태그·헤더 여유)
const MAX_FRAME: u32 = (CHUNK + 4096) as u32;
/// 받는 쪽이 코드가 틀렸다고 알릴 때 쓰는 평문 표식 (길이 자리에 넣는다)
const BAD_CODE: u32 = u32::MAX;
const MAX_CODE_FAILURES: u32 = 3;
/// 받는 쪽이 DB 를 복원하는 동안 보내는 쪽이 기다리는 시간까지 넉넉히 잡는다
const IO_TIMEOUT: Duration = Duration::from_secs(1800);

/// 다른 OS 에서 쓸 수 없거나 다시 만들면 되는 폴더는 보내지 않는다.
pub const EXCLUDED_DIRS: &[&str] = &[
    "node_modules", "venv", ".venv", "__pycache__", ".next", ".turbo", ".cache",
    ".pytest_cache", ".mypy_cache",
];

/// 화면으로 보내는 진행 상황
#[derive(Debug, Clone)]
pub enum LanEvent {
    /// 받기 대기 시작: (코드, 이 PC 이름)
    Listening(String, String),
    Status(String),
    Progress { files: u64, bytes: u64, total_bytes: u64 },
    Log(String),
    Done(Result<Vec<String>, String>),
}

pub fn pc_name() -> String {
    let h = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let h = h.strip_suffix(".local").unwrap_or(&h).to_string();
    if h.is_empty() { "localman".into() } else { h }
}

fn home() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn new_code() -> String {
    format!("{:06}", OsRng.next_u32() % 1_000_000)
}

// ── 메시지 ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// 프로젝트 폴더 기준 상대 경로 ('/' 구분)
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: u32,
    pub mode: u32,
    /// 심볼릭 링크면 대상
    pub link: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
enum Msg {
    Hello { name: String, os: String, home: String, projects: Vec<String> },
    Welcome { name: String, os: String, home: String },
    Files { project: String, root: String, entries: Vec<FileEntry> },
    Need { indices: Vec<u32>, notes: Vec<String> },
    FileStart { index: u32, size: u64 },
    FilesDone,
    Bundle { size: u64 },
    Result { ok: bool, lines: Vec<String> },
    Error { message: String },
}

enum Frame {
    Msg(Msg),
    Data(Vec<u8>),
}

// ── 암호화 채널 ────────────────────────────────────────────────────────

struct Channel {
    stream: TcpStream,
    tx: ChaCha20Poly1305,
    rx: ChaCha20Poly1305,
    tx_n: u64,
    rx_n: u64,
}

fn nonce(n: u64) -> Nonce {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_be_bytes());
    Nonce::from(b)
}

fn write_plain(s: &mut TcpStream, data: &[u8]) -> Result<(), String> {
    s.write_all(&(data.len() as u32).to_be_bytes()).map_err(|e| e.to_string())?;
    s.write_all(data).map_err(|e| e.to_string())
}

fn read_len(s: &mut TcpStream) -> Result<u32, String> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).map_err(|e| format!("연결이 끊겼습니다: {e}"))?;
    Ok(u32::from_be_bytes(len))
}

fn read_plain(s: &mut TcpStream, max: u32) -> Result<Vec<u8>, String> {
    let len = read_len(s)?;
    if len > max {
        return Err("잘못된 데이터를 받았습니다.".into());
    }
    let mut buf = vec![0u8; len as usize];
    s.read_exact(&mut buf).map_err(|e| format!("연결이 끊겼습니다: {e}"))?;
    Ok(buf)
}

/// 코드로 SPAKE2 를 해 방향별 키를 만든다. 코드가 다르면 키가 달라 첫 프레임 복호화가 실패한다.
fn handshake(mut stream: TcpStream, code: &str, is_sender: bool) -> Result<Channel, String> {
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_nodelay(true).ok();
    let (state, outbound) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.trim().as_bytes()),
        &Identity::new(SPAKE_ID),
    );
    write_plain(&mut stream, &outbound)?;
    let inbound = read_plain(&mut stream, 1024)?;
    let key = state.finish(&inbound).map_err(|_| "연결 협상에 실패했습니다.".to_string())?;
    let derive = |label: &[u8]| {
        let mut h = Sha256::new();
        h.update(&key);
        h.update(label);
        ChaCha20Poly1305::new(Key::from_slice(&h.finalize()))
    };
    let (tx, rx) = if is_sender {
        (derive(b"sender->receiver"), derive(b"receiver->sender"))
    } else {
        (derive(b"receiver->sender"), derive(b"sender->receiver"))
    };
    Ok(Channel { stream, tx, rx, tx_n: 0, rx_n: 0 })
}

#[derive(Debug)]
enum RecvError {
    /// 상대가 코드가 틀렸다고 알림 (보내는 쪽에서만)
    BadCode,
    /// 복호화 실패 — 코드가 다르다 (받는 쪽 첫 프레임)
    Decrypt,
    Other(String),
}

impl From<RecvError> for String {
    fn from(e: RecvError) -> String {
        match e {
            RecvError::BadCode | RecvError::Decrypt => "코드가 맞지 않습니다.".into(),
            RecvError::Other(s) => s,
        }
    }
}

impl Channel {
    fn send_raw(&mut self, plain: &[u8]) -> Result<(), String> {
        let ct = self
            .tx
            .encrypt(&nonce(self.tx_n), plain)
            .map_err(|_| "암호화 실패".to_string())?;
        self.tx_n += 1;
        write_plain(&mut self.stream, &ct)
    }

    fn send(&mut self, m: &Msg) -> Result<(), String> {
        let mut p = vec![0u8];
        p.extend(serde_json::to_vec(m).map_err(|e| e.to_string())?);
        self.send_raw(&p)
    }

    fn send_data(&mut self, d: &[u8]) -> Result<(), String> {
        let mut p = Vec::with_capacity(d.len() + 1);
        p.push(1u8);
        p.extend_from_slice(d);
        self.send_raw(&p)
    }

    fn recv(&mut self) -> Result<Frame, RecvError> {
        let len = read_len(&mut self.stream).map_err(RecvError::Other)?;
        if len == BAD_CODE {
            return Err(RecvError::BadCode);
        }
        if len > MAX_FRAME {
            return Err(RecvError::Other("잘못된 데이터를 받았습니다.".into()));
        }
        let mut ct = vec![0u8; len as usize];
        self.stream
            .read_exact(&mut ct)
            .map_err(|e| RecvError::Other(format!("연결이 끊겼습니다: {e}")))?;
        let plain = self.rx.decrypt(&nonce(self.rx_n), ct.as_ref()).map_err(|_| RecvError::Decrypt)?;
        self.rx_n += 1;
        match plain.split_first() {
            Some((0, json)) => serde_json::from_slice(json)
                .map(Frame::Msg)
                .map_err(|e| RecvError::Other(format!("메시지 형식 오류: {e}"))),
            Some((1, data)) => Ok(Frame::Data(data.to_vec())),
            _ => Err(RecvError::Other("알 수 없는 프레임".into())),
        }
    }

    fn recv_msg(&mut self) -> Result<Msg, String> {
        match self.recv()? {
            Frame::Msg(Msg::Error { message }) => Err(format!("상대 PC: {message}")),
            Frame::Msg(m) => Ok(m),
            Frame::Data(_) => Err("예상하지 못한 데이터".into()),
        }
    }

    /// size 바이트를 데이터 프레임으로 받아 w 에 쓴다.
    fn recv_into(&mut self, size: u64, w: &mut impl Write, mut on_chunk: impl FnMut(u64)) -> Result<(), String> {
        let mut left = size;
        while left > 0 {
            match self.recv()? {
                Frame::Data(d) => {
                    if d.len() as u64 > left {
                        return Err("파일 크기가 맞지 않습니다.".into());
                    }
                    w.write_all(&d).map_err(|e| e.to_string())?;
                    left -= d.len() as u64;
                    on_chunk(d.len() as u64);
                }
                Frame::Msg(Msg::Error { message }) => return Err(format!("상대 PC: {message}")),
                Frame::Msg(_) => return Err("파일 데이터 대신 메시지를 받았습니다.".into()),
            }
        }
        Ok(())
    }

    /// 파일을 청크로 보낸다. 보낸 바이트 수를 돌려준다.
    fn send_file(&mut self, r: &mut impl Read, size: u64, mut on_chunk: impl FnMut(u64)) -> Result<(), String> {
        let mut buf = vec![0u8; CHUNK];
        let mut left = size;
        while left > 0 {
            let want = (left as usize).min(CHUNK);
            r.read_exact(&mut buf[..want]).map_err(|e| format!("파일 읽기 실패: {e}"))?;
            self.send_data(&buf[..want])?;
            left -= want as u64;
            on_chunk(want as u64);
        }
        Ok(())
    }
}

// ── 발견 (UDP 브로드캐스트) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub name: String,
    pub os: String,
    pub addr: SocketAddr,
}

#[derive(Serialize, Deserialize)]
struct Announce {
    name: String,
    os: String,
    port: u16,
}

/// 같은 네트워크에서 받기 대기 중인 localman 을 찾는다.
pub fn discover(wait: Duration) -> Vec<Peer> {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
        return Vec::new();
    };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = sock.send_to(DISCOVER_REQ, (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    // 같은 PC 에서 시험할 때를 위해 루프백에도 묻는다
    let _ = sock.send_to(DISCOVER_REQ, (Ipv4Addr::LOCALHOST, DISCOVERY_PORT));

    let mut peers: Vec<Peer> = Vec::new();
    let deadline = Instant::now() + wait;
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Ok(a) = serde_json::from_slice::<Announce>(&buf[..n]) {
                let peer = Peer { name: a.name, os: a.os, addr: SocketAddr::new(from.ip(), a.port) };
                // 루프백과 실제 IP 로 두 번 답하는 경우 이름으로 하나만 남긴다
                if !peers.iter().any(|p| p.name == peer.name && p.addr.port() == peer.addr.port()) {
                    peers.push(peer);
                }
            }
        }
    }
    peers
}

fn run_responder(stop: Arc<AtomicBool>, port: u16) {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)) else {
        eprintln!("[localman] 발견 응답 포트({DISCOVERY_PORT})를 열 수 없습니다 — IP 직접 입력으로 접속하세요");
        return;
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let reply = serde_json::to_vec(&Announce { name: pc_name(), os: std::env::consts::OS.into(), port }).unwrap();
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::Relaxed) {
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if &buf[..n] == DISCOVER_REQ {
                let _ = sock.send_to(&reply, from);
            }
        }
    }
}

// ── 파일 목록 ─────────────────────────────────────────────────────────

/// 프로젝트 폴더를 훑어 보낼 파일 목록을 만든다 (EXCLUDED_DIRS 는 어느 깊이에서든 제외).
pub fn scan(root: &Path) -> Result<Vec<FileEntry>, String> {
    let mut out = Vec::new();
    walk(root, root, &mut out).map_err(|e| format!("{} 읽기 실패: {e}", root.display()))?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<FileEntry>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        let ft = meta.file_type();
        if ft.is_dir() {
            if !EXCLUDED_DIRS.contains(&name.as_str()) {
                walk(root, &path, out)?;
            }
            continue;
        }
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        #[cfg(unix)]
        let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o777;
        #[cfg(not(unix))]
        let mode = 0o644;
        if ft.is_symlink() {
            let target = fs::read_link(&path)?.to_string_lossy().to_string();
            out.push(FileEntry { path: rel, size: 0, mtime: mtime.unix_seconds(), mtime_ns: 0, mode, link: Some(target) });
        } else if ft.is_file() {
            out.push(FileEntry {
                path: rel,
                size: meta.len(),
                mtime: mtime.unix_seconds(),
                mtime_ns: mtime.nanoseconds(),
                mode,
                link: None,
            });
        }
        // 소켓·FIFO 등은 보내지 않는다
    }
    Ok(())
}

/// 상대 경로가 폴더 밖을 가리키지 않는지 (.., 절대 경로 금지)
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(p.to_path_buf())
}

/// 받는 쪽에서 이 파일이 필요한가. Err 는 받지 않는 이유(충돌 등).
fn needs_file(dest: &Path, e: &FileEntry, overwrite: bool) -> Result<bool, String> {
    match fs::symlink_metadata(dest) {
        Err(_) => Ok(true),
        Ok(m) if m.is_dir() => Err(format!("{}: 같은 이름의 폴더가 있어 건너뜀", e.path)),
        Ok(m) => {
            let mine = filetime::FileTime::from_last_modification_time(&m).unix_seconds();
            if m.len() == e.size && mine == e.mtime {
                Ok(false) // 같은 파일
            } else if mine > e.mtime && !overwrite {
                Err(format!("{}: 이 PC 파일이 더 최신이라 건너뜀", e.path))
            } else {
                Ok(true)
            }
        }
    }
}

/// 받을 프로젝트 폴더. 보내는 쪽 홈 기준 경로를 이 PC 홈으로 바꾸고,
/// 결과가 이 PC 홈 밖이면 ~/localman-received/<id> 로 받는다(시스템 경로에 쓰지 않는다).
fn dest_root(src_root: &str, sender_home: &str, my_home: &str, id: &str) -> PathBuf {
    let mapped = remap_path(src_root, sender_home, my_home);
    let home = Path::new(my_home);
    let p = Path::new(&mapped);
    if !my_home.is_empty() && p.starts_with(home) && p != home {
        p.to_path_buf()
    } else {
        home.join("localman-received").join(id)
    }
}

// ── 보내기 ────────────────────────────────────────────────────────────

pub struct SendPlan {
    /// 보낼 프로젝트 id. None 이면 전부.
    pub projects: Option<Vec<String>>,
    pub databases: Vec<(DbEngine, String)>,
    pub include_credentials: bool,
}

pub fn send(addr: SocketAddr, code: &str, plan: SendPlan, ev: impl Fn(LanEvent)) -> Result<Vec<String>, String> {
    let projects: Vec<VhostProject> = list_projects()
        .into_iter()
        .filter(|p| plan.projects.as_ref().is_none_or(|ids| ids.contains(&p.id)))
        .collect();

    // 1) 연결 전에 준비를 끝낸다 (상대가 기다리다 끊기지 않게)
    ev(LanEvent::Status("파일 목록을 만드는 중…".into()));
    let mut lists: Vec<(VhostProject, Vec<FileEntry>)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for p in &projects {
        match scan(Path::new(&p.path)) {
            Ok(entries) => lists.push((p.clone(), entries)),
            Err(e) => notes.push(format!("✗ {} 파일: {e}", p.id)),
        }
    }

    ev(LanEvent::Status("설정·DB를 묶는 중…".into()));
    let bundle_path = std::env::temp_dir().join(format!("localman-lan-{}-{}.tar.gz", std::process::id(), now()));
    let export_summary = export_bundle(
        &bundle_path,
        &ExportOptions {
            projects: Some(projects.iter().map(|p| p.id.clone()).collect()),
            databases: plan.databases.clone(),
            include_credentials: plan.include_credentials,
        },
    )?;
    notes.extend(export_summary.lines().filter(|l| l.starts_with('✗')).map(String::from));

    let result = send_over_network(addr, code, &lists, &bundle_path, &ev);
    let _ = fs::remove_file(&bundle_path);

    let (peer, peer_os, files, bytes, outcome) = match result {
        Ok(r) => r,
        Err(e) => {
            ev(LanEvent::Status(String::new()));
            return Err(e);
        }
    };
    let mut lines = notes;
    let (ok, remote) = outcome;
    lines.extend(remote);
    append_history(TransferRecord {
        at: now(),
        direction: Direction::Sent,
        peer,
        peer_os,
        projects: projects.iter().map(|p| p.id.clone()).collect(),
        databases: plan.databases.iter().map(|d| d.1.clone()).collect(),
        files,
        bytes,
        ok: ok && !lines.iter().any(|l| l.starts_with('✗')),
        notes: lines.clone(),
    });
    Ok(lines)
}

type SendOutcome = (String, String, u64, u64, (bool, Vec<String>));

fn send_over_network(
    addr: SocketAddr,
    code: &str,
    lists: &[(VhostProject, Vec<FileEntry>)],
    bundle_path: &Path,
    ev: &impl Fn(LanEvent),
) -> Result<SendOutcome, String> {
    ev(LanEvent::Status(format!("{addr} 에 연결하는 중…")));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("연결 실패 ({addr}): {e}\n받는 PC에서 [받기 대기] 중인지, 방화벽이 막지 않는지 확인하세요."))?;
    stream.write_all(MAGIC).map_err(|e| e.to_string())?;
    let mut ch = handshake(stream, code, true)?;

    ch.send(&Msg::Hello {
        name: pc_name(),
        os: std::env::consts::OS.into(),
        home: home(),
        projects: lists.iter().map(|(p, _)| p.id.clone()).collect(),
    })?;
    let (peer, peer_os) = match ch.recv_msg()? {
        Msg::Welcome { name, os, .. } => (name, os),
        _ => return Err("상대가 이상한 응답을 보냈습니다.".into()),
    };
    ev(LanEvent::Status(format!("{peer}(으)로 보내는 중…")));

    let (mut files, mut bytes) = (0u64, 0u64);
    for (p, entries) in lists {
        ch.send(&Msg::Files { project: p.id.clone(), root: p.path.clone(), entries: entries.clone() })?;
        let (indices, notes) = match ch.recv_msg()? {
            Msg::Need { indices, notes } => (indices, notes),
            _ => return Err("상대가 이상한 응답을 보냈습니다.".into()),
        };
        for n in notes {
            ev(LanEvent::Log(format!("· {}: {n}", p.id)));
        }
        let total_bytes: u64 = indices.iter().filter_map(|&i| entries.get(i as usize)).map(|e| e.size).sum();
        let mut done_bytes = 0u64;
        ev(LanEvent::Status(format!("{} — 바뀐 파일 {}개 보내는 중", p.id, indices.len())));
        for i in indices {
            let e = entries.get(i as usize).ok_or("잘못된 파일 번호")?;
            let path = Path::new(&p.path).join(&e.path);
            let mut f = fs::File::open(&path).map_err(|err| format!("{} 열기 실패: {err}", e.path))?;
            // 목록을 만든 뒤 바뀌었을 수 있으니 지금 크기로 보낸다
            let size = f.metadata().map(|m| m.len()).unwrap_or(e.size);
            ch.send(&Msg::FileStart { index: i, size })?;
            ch.send_file(&mut f, size, |n| {
                done_bytes += n;
                bytes += n;
                ev(LanEvent::Progress { files, bytes: done_bytes, total_bytes });
            })?;
            files += 1;
        }
    }
    ch.send(&Msg::FilesDone)?;

    ev(LanEvent::Status("설정·DB 보내는 중…".into()));
    let size = fs::metadata(bundle_path).map_err(|e| e.to_string())?.len();
    ch.send(&Msg::Bundle { size })?;
    let mut f = fs::File::open(bundle_path).map_err(|e| e.to_string())?;
    ch.send_file(&mut f, size, |_| {})?;

    ev(LanEvent::Status("받는 PC에서 적용하는 중…".into()));
    match ch.recv_msg()? {
        Msg::Result { ok, lines } => Ok((peer, peer_os, files, bytes, (ok, lines))),
        _ => Err("상대가 이상한 응답을 보냈습니다.".into()),
    }
}

// ── 받기 ──────────────────────────────────────────────────────────────

pub struct ReceiveOptions {
    /// 이미 있는 프로젝트·DB, 이 PC 쪽이 더 최신인 파일까지 보내는 쪽 기준으로 덮어쓸지
    pub overwrite: bool,
}

/// 받기 대기. 한 번 받으면 끝난다(코드는 한 번만 쓴다). stop 이 켜지면 대기를 멈춘다.
pub fn receive(opts: ReceiveOptions, stop: Arc<AtomicBool>, ev: impl Fn(LanEvent)) -> Result<Vec<String>, String> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, TRANSFER_PORT))
        .map_err(|e| format!("포트 {TRANSFER_PORT}를 열 수 없습니다: {e} (다른 localman이 받기 대기 중인지 확인)"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let responder_stop = Arc::new(AtomicBool::new(false));
    {
        let s = responder_stop.clone();
        std::thread::spawn(move || run_responder(s, TRANSFER_PORT));
    }
    let result = accept_loop(&listener, &opts, &stop, &ev);
    responder_stop.store(true, Ordering::Relaxed);
    result
}

fn accept_loop(
    listener: &TcpListener,
    opts: &ReceiveOptions,
    stop: &AtomicBool,
    ev: &impl Fn(LanEvent),
) -> Result<Vec<String>, String> {
    let mut code = new_code();
    let mut failures = 0;
    ev(LanEvent::Listening(code.clone(), pc_name()));
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("받기 대기를 멈췄습니다.".into());
        }
        let (mut stream, from) = match listener.accept() {
            Ok(x) => x,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        stream.set_nonblocking(false).ok();
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let mut magic = vec![0u8; MAGIC.len()];
        if stream.read_exact(&mut magic).is_err() || magic != MAGIC {
            continue; // localman 이 아닌 접속
        }
        let mut ch = match handshake(stream, &code, false) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let hello = match ch.recv() {
            Ok(Frame::Msg(m @ Msg::Hello { .. })) => m,
            Err(RecvError::Decrypt) => {
                // 코드가 틀렸다고 평문으로 알리고 끊는다. 대입 공격을 막으려 3번이면 멈춘다.
                let _ = ch.stream.write_all(&BAD_CODE.to_be_bytes());
                failures += 1;
                ev(LanEvent::Log(format!("✗ {} 에서 틀린 코드로 접속 시도 ({failures}/{MAX_CODE_FAILURES})", from.ip())));
                if failures >= MAX_CODE_FAILURES {
                    return Err("코드가 3번 틀려 받기를 멈췄습니다. 다시 [받기 대기]를 누르면 새 코드가 나옵니다.".into());
                }
                // 틀린 시도 뒤에는 새 코드로 바꿔 같은 코드를 계속 맞혀 보지 못하게 한다
                code = new_code();
                ev(LanEvent::Listening(code.clone(), pc_name()));
                continue;
            }
            _ => continue,
        };
        return session(ch, hello, opts, ev);
    }
}

fn session(mut ch: Channel, hello: Msg, opts: &ReceiveOptions, ev: &impl Fn(LanEvent)) -> Result<Vec<String>, String> {
    let Msg::Hello { name: peer, os: peer_os, home: sender_home, projects } = hello else {
        unreachable!()
    };
    ch.send(&Msg::Welcome { name: pc_name(), os: std::env::consts::OS.into(), home: home() })?;
    ev(LanEvent::Status(format!("{peer}에서 받는 중…")));

    let my_home = home();
    let mut lines: Vec<String> = Vec::new();
    // 프로젝트 id → 이 PC 에 받은 폴더
    let mut placed: Vec<(String, PathBuf)> = Vec::new();
    let (mut files, mut bytes) = (0u64, 0u64);

    let bundle_file = loop {
        match ch.recv_msg()? {
            Msg::Files { project, root, entries } => {
                let dest = dest_root(&root, &sender_home, &my_home, &project);
                fs::create_dir_all(&dest).map_err(|e| format!("{} 만들기 실패: {e}", dest.display()))?;
                let mut need: Vec<u32> = Vec::new();
                let mut notes: Vec<String> = Vec::new();
                for (i, e) in entries.iter().enumerate() {
                    let Some(rel) = safe_rel(&e.path) else {
                        notes.push(format!("{}: 허용되지 않는 경로라 건너뜀", e.path));
                        continue;
                    };
                    let target = dest.join(&rel);
                    if let Some(link) = &e.link {
                        if fs::symlink_metadata(&target).is_err() {
                            if let Some(parent) = target.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            #[cfg(unix)]
                            if let Err(err) = std::os::unix::fs::symlink(link, &target) {
                                notes.push(format!("{}: 링크 만들기 실패 ({err})", e.path));
                            }
                        }
                        continue;
                    }
                    match needs_file(&target, e, opts.overwrite) {
                        Ok(true) => need.push(i as u32),
                        Ok(false) => {}
                        Err(n) => notes.push(n),
                    }
                }
                let total_bytes: u64 = need.iter().map(|&i| entries[i as usize].size).sum();
                ev(LanEvent::Status(format!("{project} — 바뀐 파일 {}개 받는 중", need.len())));
                for n in &notes {
                    lines.push(format!("· {project}: {n}"));
                }
                ch.send(&Msg::Need { indices: need.clone(), notes })?;

                let mut done_bytes = 0u64;
                for _ in 0..need.len() {
                    let (index, size) = match ch.recv_msg()? {
                        Msg::FileStart { index, size } => (index, size),
                        _ => return Err("파일 시작 신호를 받지 못했습니다.".into()),
                    };
                    if !need.contains(&index) {
                        return Err("요청하지 않은 파일을 받았습니다.".into());
                    }
                    let e = &entries[index as usize];
                    let target = dest.join(safe_rel(&e.path).unwrap());
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
                    }
                    // 다 받은 뒤에 바꿔치기해서, 도중에 끊겨도 원래 파일이 깨지지 않게 한다
                    let part = target.with_file_name(format!(
                        ".{}.localman-part",
                        target.file_name().unwrap().to_string_lossy()
                    ));
                    let mut f = fs::File::create(&part).map_err(|err| format!("{} 쓰기 실패: {err}", e.path))?;
                    let recv = ch.recv_into(size, &mut f, |n| {
                        done_bytes += n;
                        bytes += n;
                        ev(LanEvent::Progress { files, bytes: done_bytes, total_bytes });
                    });
                    drop(f);
                    if let Err(err) = recv {
                        let _ = fs::remove_file(&part);
                        return Err(err);
                    }
                    fs::rename(&part, &target).map_err(|err| err.to_string())?;
                    let _ = filetime::set_file_mtime(&target, filetime::FileTime::from_unix_time(e.mtime, e.mtime_ns));
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(e.mode));
                    }
                    files += 1;
                }
                placed.push((project, dest));
            }
            Msg::FilesDone => {
                let size = match ch.recv_msg()? {
                    Msg::Bundle { size } => size,
                    _ => return Err("설정 묶음을 받지 못했습니다.".into()),
                };
                let path = std::env::temp_dir().join(format!("localman-recv-{}-{}.tar.gz", std::process::id(), now()));
                let mut f = fs::File::create(&path).map_err(|e| e.to_string())?;
                ch.recv_into(size, &mut f, |_| {})?;
                break path;
            }
            _ => return Err("예상하지 못한 메시지".into()),
        }
    };

    // 설정·DB 적용 — 백업 묶음 가져오기와 같은 로직. 경로는 실제로 받은 폴더로 맞춘다.
    ev(LanEvent::Status("설정·DB 적용 중…".into()));
    let applied = (|| -> Result<(Vec<String>, Vec<String>), String> {
        let mut bundle = open_bundle(&bundle_file)?;
        for p in &mut bundle.projects {
            p.path = match placed.iter().find(|(id, _)| id == &p.id) {
                Some((_, dest)) => dest.to_string_lossy().to_string(),
                None => dest_root(&p.path, &sender_home, &my_home, &p.id).to_string_lossy().to_string(),
            };
        }
        let out = import_bundle(
            &bundle,
            &ImportOptions {
                path_from: String::new(),
                path_to: String::new(),
                overwrite: opts.overwrite,
                restore_databases: true,
                import_credentials: true,
            },
        );
        let dbs = bundle.manifest.databases.iter().map(|d| d.name.clone()).collect();
        bundle.close();
        Ok((out, dbs))
    })();
    let _ = fs::remove_file(&bundle_file);

    let mut databases = Vec::new();
    match applied {
        Ok((out, dbs)) => {
            lines.extend(out);
            databases = dbs;
        }
        Err(e) => lines.push(format!("✗ 설정 적용 실패: {e}")),
    }
    for (id, dest) in &placed {
        lines.insert(0, format!("✓ {id} 파일 위치: {}", dest.display()));
    }
    lines.insert(0, format!("✓ 파일 {files}개 ({}) 받음", human_bytes(bytes)));
    let ok = !lines.iter().any(|l| l.starts_with('✗'));
    let _ = ch.send(&Msg::Result { ok, lines: lines.clone() });

    append_history(TransferRecord {
        at: now(),
        direction: Direction::Received,
        peer,
        peer_os,
        projects,
        databases,
        files,
        bytes,
        ok,
        notes: lines.clone(),
    });
    Ok(lines)
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::TmpDir;

    #[test]
    fn rejects_paths_escaping_the_project() {
        assert!(safe_rel("src/main.rs").is_some());
        assert!(safe_rel("../etc/passwd").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("a/../../b").is_none());
        assert!(safe_rel("").is_none());
    }

    #[test]
    fn destination_stays_inside_receiver_home() {
        assert_eq!(dest_root("/home/dell/dev/x", "/home/dell", "/Users/eond", "x"), PathBuf::from("/Users/eond/dev/x"));
        // 홈 밖에서 온 프로젝트는 시스템 경로가 아니라 받은 폴더로
        assert_eq!(dest_root("/var/www/x", "/home/dell", "/Users/eond", "x"),
            PathBuf::from("/Users/eond/localman-received/x"));
        // 홈 자체를 덮어쓰지 않는다
        assert_eq!(dest_root("/home/dell", "/home/dell", "/Users/eond", "x"),
            PathBuf::from("/Users/eond/localman-received/x"));
    }

    #[test]
    fn scan_skips_dependency_folders() {
        let t = TmpDir::new("lanscan");
        t.write("src/a.rs", "a");
        t.write(".env", "SECRET=1");
        t.write("node_modules/x/index.js", "x");
        t.write("web/node_modules/y.js", "y");
        t.write("venv/bin/python3", "");
        let list = scan(&t.0).unwrap();
        let paths: Vec<&str> = list.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec![".env", "src/a.rs"]);
    }

    /// 같은 PC 안에서 실제 TCP 로 보내고 받는다: 바뀐 파일만 오가는지, 틀린 코드는 거부되는지.
    #[test]
    fn channel_round_trip_and_bad_code() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut ch = handshake(s, "123456", false).unwrap();
            let got = match ch.recv() { Ok(Frame::Msg(Msg::FilesDone)) => true, _ => false };
            let mut buf = Vec::new();
            ch.recv_into(3 * CHUNK as u64 + 5, &mut buf, |_| {}).unwrap();
            ch.send(&Msg::Result { ok: got, lines: vec![format!("{}", buf.len())] }).unwrap();
            // 두 번째 접속: 틀린 코드
            let (s, _) = l.accept().unwrap();
            let mut ch = handshake(s, "123456", false).unwrap();
            let bad = matches!(ch.recv(), Err(RecvError::Decrypt));
            let _ = ch.stream.write_all(&BAD_CODE.to_be_bytes());
            bad
        });
        let mut ch = handshake(TcpStream::connect(addr).unwrap(), "123456", true).unwrap();
        ch.send(&Msg::FilesDone).unwrap();
        let data = vec![7u8; 3 * CHUNK + 5];
        ch.send_file(&mut data.as_slice(), data.len() as u64, |_| {}).unwrap();
        match ch.recv_msg().unwrap() {
            Msg::Result { ok, lines } => {
                assert!(ok);
                assert_eq!(lines, vec![(3 * CHUNK + 5).to_string()]);
            }
            _ => panic!("결과를 받지 못했습니다"),
        }

        let mut wrong = handshake(TcpStream::connect(addr).unwrap(), "000000", true).unwrap();
        wrong.send(&Msg::FilesDone).unwrap();
        assert!(matches!(wrong.recv(), Err(RecvError::BadCode)));
        assert!(server.join().unwrap(), "받는 쪽이 틀린 코드를 알아채지 못했습니다");
    }

    #[test]
    fn quick_check_skips_unchanged_and_protects_newer() {
        let t = TmpDir::new("lanneed");
        t.write("a.txt", "hello");
        let dest = t.0.join("a.txt");
        let m = fs::metadata(&dest).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&m).unix_seconds();
        let same = FileEntry { path: "a.txt".into(), size: 5, mtime, mtime_ns: 0, mode: 0o644, link: None };
        assert_eq!(needs_file(&dest, &same, false), Ok(false));
        let older = FileEntry { size: 6, mtime: mtime - 100, ..same.clone() };
        assert!(needs_file(&dest, &older, false).is_err(), "이 PC 쪽이 더 최신이면 건너뛰어야 합니다");
        assert_eq!(needs_file(&dest, &older, true), Ok(true));
        let newer = FileEntry { size: 6, mtime: mtime + 100, ..same };
        assert_eq!(needs_file(&dest, &newer, false), Ok(true));
        assert_eq!(needs_file(&t.0.join("missing.txt"), &older, false), Ok(true));
    }
}

