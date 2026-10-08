//! SFTP 로 파일 주고받기 — 셸 명령을 막아 둔 호스팅(카페24 등)용.
//!
//! 서버에서 명령을 돌릴 수 없으니 rsync 대신 디렉토리를 훑어 크기·수정 시각이 다른 파일만 주고받는다.
//! 받는 쪽에만 있는 파일은 지우지 않는다. 서버 호스트 키는 처음 본 것을 기억하고 바뀌면 멈춘다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, load_secret_key};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::ProgressFn;
use super::deploy::DeployTarget;
use super::lan::human_bytes;
use super::settings::data_dir;
use crate::i18n::{tr, trf};

/// 웹 경로를 못 찾을 때 홈에서 차례로 볼 폴더
const WEB_DIRS: [&str; 5] = ["www", "public_html", "html", "htdocs", "web"];

fn known_hosts_path() -> PathBuf {
    data_dir().join("sftp_known_hosts.json")
}

fn load_known() -> HashMap<String, String> {
    std::fs::read_to_string(known_hosts_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

struct Checker {
    host: String,
    /// 처음 본 키 (접속이 끝난 뒤 저장)
    seen: Arc<std::sync::Mutex<Option<String>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        match load_known().get(&self.host) {
            Some(known) => Ok(*known == fp),
            None => {
                *self.seen.lock().unwrap() = Some(fp);
                Ok(true)
            }
        }
    }
}

/// 열린 SFTP 세션과 실제 웹 경로
pub struct Sftp {
    pub sftp: SftpSession,
    /// 서버에서 실제로 찾은 웹 경로
    pub root: String,
    _session: client::Handle<Checker>,
}

fn host_key(t: &DeployTarget) -> String {
    format!("{}:{}", t.ssh_host.trim(), t.ssh_port)
}

/// 접속하고 웹 경로를 찾는다. 적힌 경로가 없으면 홈 아래(…/www 등)에서 찾는다.
pub async fn connect(t: &DeployTarget) -> Result<Sftp, String> {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let checker = Checker { host: host_key(t), seen: seen.clone() };
    let config = Arc::new(client::Config { inactivity_timeout: Some(std::time::Duration::from_secs(60)), ..Default::default() });
    let mut session = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client::connect(config, (t.ssh_host.trim(), t.ssh_port), checker),
    )
    .await
    .map_err(|_| tr("서버 응답이 없습니다 (15초)").to_string())?
    .map_err(|e| match e {
        russh::Error::UnknownKey => tr("서버 호스트 키가 예전과 다릅니다 — 서버가 바뀐 게 아니면 접속하지 마세요").to_string(),
        e => trf("SFTP 접속 실패: {0}", &[&e]),
    })?;

    let user = t.ssh_user.trim();
    let ok = if !t.ssh_password.is_empty() {
        session.authenticate_password(user, &t.ssh_password).await.map_err(|e| e.to_string())?.success()
    } else {
        let path = if t.ssh_key.trim().is_empty() {
            let home = dirs::home_dir().unwrap_or_default();
            ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|k| home.join(".ssh").join(k)).find(|p| p.exists())
        } else {
            Some(PathBuf::from(t.ssh_key.trim()))
        }
        .ok_or_else(|| tr("SSH 키가 없습니다 — 비밀번호를 넣거나 키 경로를 적으세요").to_string())?;
        let key = load_secret_key(&path, None).map_err(|e| trf("SSH 키를 읽지 못했습니다 (암호가 걸린 키는 비밀번호 로그인을 쓰세요): {0}", &[&e]))?;
        let hash = session.best_supported_rsa_hash().await.map_err(|e| e.to_string())?.flatten();
        session
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|e| e.to_string())?
            .success()
    };
    if !ok {
        return Err(tr("SFTP 로그인 실패 — 사용자·비밀번호(또는 키)를 확인하세요.").into());
    }
    // 로그인까지 된 뒤에만 처음 본 호스트 키를 기억한다
    if let Some(fp) = seen.lock().unwrap().take() {
        let mut all = load_known();
        all.insert(host_key(t), fp);
        let _ = super::write_atomic(&known_hosts_path(), serde_json::to_string_pretty(&all).unwrap_or_default().as_bytes());
    }

    let channel = session.channel_open_session().await.map_err(|e| e.to_string())?;
    channel.request_subsystem(true, "sftp").await.map_err(|e| e.to_string())?;
    let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| trf("SFTP를 열지 못했습니다: {0}", &[&e]))?;
    let root = find_root(&sftp, t.remote_path.trim()).await?;
    Ok(Sftp { sftp, root, _session: session })
}

async fn is_dir(sftp: &SftpSession, path: &str) -> bool {
    sftp.metadata(path).await.map(|m| m.is_dir()).unwrap_or(false)
}

/// 웹 경로 찾기: 적힌 그대로 → 홈 기준(/www → ~/www) → 홈 아래 www·public_html… 순서
async fn find_root(sftp: &SftpSession, wanted: &str) -> Result<String, String> {
    let wanted = wanted.trim_end_matches('/');
    if !wanted.is_empty() && is_dir(sftp, wanted).await {
        return Ok(wanted.to_string());
    }
    let home = sftp.canonicalize(".").await.map_err(|e| e.to_string())?;
    let home = home.trim_end_matches('/');
    if !wanted.is_empty() {
        let rel = format!("{home}/{}", wanted.trim_start_matches('/'));
        if is_dir(sftp, &rel).await {
            return Ok(rel);
        }
    }
    for d in WEB_DIRS {
        let p = format!("{home}/{d}");
        if is_dir(sftp, &p).await {
            return Ok(p);
        }
    }
    Err(trf("서버에서 웹 경로를 찾지 못했습니다: {0} (홈: {1})", &[&wanted, &home]))
}

/// 웹 경로만 찾아 본다 (연결 시험·자동 찾기)
pub fn probe(t: &DeployTarget) -> Result<String, String> {
    let t = t.clone();
    run(move || async move { connect(&t).await.map(|s| s.root) })
}

/// 별도 스레드의 런타임에서 돌린다 — 화면(iced)·MCP 어느 쪽에서 불러도 안전하게
/// 퓨처는 그 스레드 안에서 만든다 (Send 가 아니어도 된다).
pub fn run<T, F, Fut>(make: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(make())
    })
    .join()
    .map_err(|_| "sftp thread panicked".to_string())?
}

// ── 제외 규칙 (rsync --exclude 와 같은 뜻으로) ──────────────────────────

/// `/files/config/`(앞 / 는 웹 경로 기준), `.git/`(끝 / 는 폴더만), `*.log`(이름 끝), `node_modules`(어디서든 같은 이름)
pub fn excluded(rel: &str, is_dir: bool, patterns: &[String]) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    patterns.iter().any(|raw| {
        let pat = raw.trim();
        if pat.is_empty() {
            return false;
        }
        let dir_only = pat.ends_with('/');
        if dir_only && !is_dir {
            return false;
        }
        let anchored = pat.starts_with('/');
        let p = pat.trim_matches('/');
        if let Some(ext) = p.strip_prefix('*') {
            return name.ends_with(ext);
        }
        if anchored || p.contains('/') { rel == p } else { name == p }
    })
}

// ── 파일 목록 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Entry {
    pub rel: String,
    pub size: u64,
    pub mtime: u64,
}

/// 서버 파일 목록. 폴더 목록을 한 단계씩 16개까지 동시에 읽는다 (한 번에 하나씩 읽으면 왕복 시간이 쌓여 느리다).
async fn list_remote(sftp: &SftpSession, root: &str, excludes: &[String], progress: &ProgressFn) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut level = vec![String::new()];
    let mut dirs_read = 0usize;
    while !level.is_empty() {
        let mut next = Vec::new();
        for chunk in level.chunks(16) {
            let reads = chunk.iter().map(|dir| async move {
                let full = if dir.is_empty() { root.to_string() } else { format!("{root}/{dir}") };
                (dir, full.clone(), sftp.read_dir(full).await)
            });
            for (dir, full, rd) in futures::future::join_all(reads).await {
                let rd = rd.map_err(|e| trf("{0} 읽기 실패: {1}", &[&full, &e]))?;
                for e in rd {
                    let name = e.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    let rel = if dir.is_empty() { name } else { format!("{dir}/{name}") };
                    let m = e.metadata();
                    if m.is_dir() {
                        if !excluded(&rel, true, excludes) {
                            next.push(rel);
                        }
                    } else if m.is_regular() && !excluded(&rel, false, excludes) {
                        out.push(Entry { rel, size: m.size.unwrap_or(0), mtime: m.mtime.unwrap_or(0) as u64 });
                    }
                }
            }
        }
        dirs_read += level.len();
        progress(trf("서버 파일 목록 읽는 중… 폴더 {0}개 · 파일 {1}개", &[&dirs_read, &out.len()]), None);
        level = next;
    }
    Ok(out)
}

fn list_local(root: &Path, excludes: &[String]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(if dir.is_empty() { root.to_path_buf() } else { root.join(&dir) }) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let rel = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !excluded(&rel, true, excludes) {
                    stack.push(rel);
                }
            } else if ft.is_file() && !excluded(&rel, false, excludes) {
                if let Ok(m) = e.metadata() {
                    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                    out.push(Entry { rel, size: m.len(), mtime });
                }
            }
        }
    }
    out
}

/// 크기와 수정 시각(초)이 같으면 같은 파일로 본다 (rsync 기본 판단과 같다)
fn same(a: &Entry, size: u64, mtime: u64) -> bool {
    a.size == size && a.mtime == mtime
}

// ── 받기 / 올리기 ───────────────────────────────────────────────────────

async fn download(sftp: &SftpSession, root: &str, local: &Path, e: &Entry) -> Result<u64, String> {
    let dest = local.join(&e.rel);
    // 이름 뒤에 붙인다 — 확장자만 바꾸면 a.php·a.txt 를 동시에 받을 때 임시 파일이 겹친다
    let tmp = PathBuf::from(format!("{}.localman-part", dest.display()));
    let mut f = sftp.open(format!("{root}/{}", e.rel)).await.map_err(|err| trf("{0} 받기 실패: {1}", &[&e.rel, &err]))?;
    let mut out = tokio::fs::File::create(&tmp).await.map_err(|err| trf("{0}: {1}", &[&tmp.display(), &err]))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await.map_err(|err| trf("{0} 받기 실패: {1}", &[&e.rel, &err]))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).await.map_err(|err| err.to_string())?;
    }
    out.flush().await.map_err(|err| err.to_string())?;
    drop(out);
    std::fs::rename(&tmp, &dest).map_err(|err| trf("{0}: {1}", &[&dest.display(), &err]))?;
    // 다음에 같은 파일로 알아보도록 서버 시각을 그대로 붙인다
    let _ = filetime::set_file_mtime(&dest, filetime::FileTime::from_unix_time(e.mtime as i64, 0));
    Ok(e.size)
}

async fn upload(sftp: &SftpSession, root: &str, local: &Path, e: &Entry) -> Result<u64, String> {
    let remote = format!("{root}/{}", e.rel);
    let mut src = tokio::fs::File::open(local.join(&e.rel)).await.map_err(|err| err.to_string())?;
    let mut f = sftp.create(remote.clone()).await.map_err(|err| trf("{0} 올리기 실패: {1}", &[&e.rel, &err]))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = src.read(&mut buf).await.map_err(|err| err.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).await.map_err(|err| trf("{0} 올리기 실패: {1}", &[&e.rel, &err]))?;
    }
    f.shutdown().await.map_err(|err| err.to_string())?;
    let attrs = FileAttributes { mtime: Some(e.mtime as u32), atime: Some(e.mtime as u32), ..FileAttributes::empty() };
    let _ = sftp.set_metadata(remote, attrs).await;
    Ok(e.size)
}

/// 8개씩 동시에 돌리며 끝날 때마다 진행 상황을 알린다
async fn transfer_all<'a, F, Fut>(todo: &'a [Entry], label: &'static str, progress: &ProgressFn, f: F) -> Result<(), String>
where
    F: Fn(&'a Entry) -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    let total: u64 = todo.iter().map(|e| e.size).sum();
    let (mut files, mut bytes) = (0usize, 0u64);
    let mut st = futures::stream::iter(todo.iter().map(f)).buffer_unordered(8);
    while let Some(r) = st.next().await {
        bytes += r?;
        files += 1;
        progress(
            trf(label, &[&files, &todo.len(), &human_bytes(bytes), &human_bytes(total)]),
            Some((bytes, total.max(1))),
        );
    }
    Ok(())
}

/// 서버 → 로컬. dry_run 이면 받을 목록만. (받은 파일 목록, 실제 웹 경로, 서버 파일 수)
pub fn pull(t: &DeployTarget, local: &Path, excludes: Vec<String>, dry_run: bool, progress: ProgressFn) -> Result<(Vec<String>, String, usize), String> {
    let t = t.clone();
    let local = local.to_path_buf();
    run(move || async move {
        let s = connect(&t).await?;
        progress(tr("SFTP 접속됨 — 서버 파일 목록을 읽습니다").to_string(), None);
        let remote = list_remote(&s.sftp, &s.root, &excludes, &progress).await?;
        let here: HashMap<String, Entry> = list_local(&local, &[]).into_iter().map(|e| (e.rel.clone(), e)).collect();
        let total = remote.len();
        let todo: Vec<Entry> = remote.into_iter().filter(|r| here.get(&r.rel).is_none_or(|l| !same(l, r.size, r.mtime))).collect();
        let names: Vec<String> = todo.iter().map(|e| e.rel.clone()).collect();
        if dry_run {
            return Ok((names, s.root, total));
        }
        // 폴더를 먼저 만들고 8개씩 동시에 받는다
        for e in &todo {
            if let Some(dir) = local.join(&e.rel).parent() {
                std::fs::create_dir_all(dir).map_err(|err| trf("{0}: {1}", &[&dir.display(), &err]))?;
            }
        }
        transfer_all(&todo, "받는 중 {0}/{1}개 · {2} / {3}", &progress, |e| download(&s.sftp, &s.root, &local, e)).await?;
        Ok((names, s.root, total))
    })
}

/// 로컬 → 서버. dry_run 이면 올릴 목록만. (올린 파일 목록, 실제 웹 경로, 로컬 파일 수)
pub fn push(t: &DeployTarget, local: &Path, excludes: Vec<String>, dry_run: bool, progress: ProgressFn) -> Result<(Vec<String>, String, usize), String> {
    let t = t.clone();
    let local = local.to_path_buf();
    run(move || async move {
        let s = connect(&t).await?;
        let mine = list_local(&local, &excludes);
        let there: HashMap<String, Entry> = list_remote(&s.sftp, &s.root, &[], &progress).await?.into_iter().map(|e| (e.rel.clone(), e)).collect();
        let total = mine.len();
        let todo: Vec<Entry> = mine.into_iter().filter(|l| there.get(&l.rel).is_none_or(|r| !same(r, l.size, l.mtime))).collect();
        let names: Vec<String> = todo.iter().map(|e| e.rel.clone()).collect();
        if dry_run {
            return Ok((names, s.root, total));
        }
        // 폴더를 위에서부터 만든 뒤 8개씩 동시에 올린다
        let mut dirs: Vec<String> = todo
            .iter()
            .filter_map(|e| e.rel.rsplit_once('/').map(|(d, _)| d.to_string()))
            .flat_map(|d| {
                let parts: Vec<&str> = d.split('/').collect();
                (1..=parts.len()).map(|i| parts[..i].join("/")).collect::<Vec<_>>()
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        for d in &dirs {
            let p = format!("{}/{d}", s.root);
            if !is_dir(&s.sftp, &p).await {
                s.sftp.create_dir(p.clone()).await.map_err(|err| trf("{0} 폴더 만들기 실패: {1}", &[&p, &err]))?;
            }
        }
        transfer_all(&todo, "올리는 중 {0}/{1}개 · {2} / {3}", &progress, |e| upload(&s.sftp, &s.root, &local, e)).await?;
        Ok((names, s.root, total))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_rules_match_rsync_meaning() {
        let pats: Vec<String> = ["/files/config/", ".git/", "*.log", "node_modules", ".env"].iter().map(|s| s.to_string()).collect();
        assert!(excluded("files/config", true, &pats));
        assert!(!excluded("modules/files/config", true, &pats), "앞 / 는 웹 경로 기준");
        assert!(excluded(".git", true, &pats));
        assert!(excluded("a/b/.git", true, &pats));
        assert!(!excluded(".git", false, &pats), "끝 / 는 폴더만");
        assert!(excluded("logs/x.log", false, &pats));
        assert!(excluded("x/node_modules", true, &pats));
        assert!(excluded(".env", false, &pats));
        assert!(!excluded("index.php", false, &pats));
    }
}


