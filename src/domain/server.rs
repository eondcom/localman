//! dev server 프로세스 수명 관리와 실행 상태(running.json) 영속화.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use super::detect::{auto_detect_next_command, auto_detect_start_command};
use super::project::{ProjectType, VhostProject};
use super::setup::{deps_ready, setup_project};
use crate::platform;

/// 실행 중인 dev server 한 건.
///
/// 예전에는 PID 하나만 메모리 HashMap 에 들고 있었다. 그 탓에
/// (1) localman 이 죽으면 목록이 통째로 증발해 서버들이 관리 불가능한 고아가 되고
/// (2) 다시 켜도 그 사실을 몰라 같은 서버를 또 띄웠으며
/// (3) `kill <pid>` 로는 npm→vite, uvicorn→chromedriver 같은 손자 프로세스가 살아남았다.
///
/// 실제로 2026-07-28 에 이 세 가지가 겹쳐, 6일간 방치된 freethread 크롤러가
/// Chrome 임시 프로필 5,074개(약 220G)를 쌓아 디스크를 100% 채웠다.
/// 그래서 pgid 를 함께 기록해 그룹째 죽이고, 파일로 남겨 재시작 후에도 회수한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunningProc {
    pid: u32,
    /// 프로세스 그룹 ID. `process_group(0)` 으로 띄우므로 최초에는 pid 와 같다.
    pgid: u32,
    /// `/proc/<pid>/stat` 의 starttime(22번째 필드).
    /// PID 는 재사용되므로 이것까지 맞아야 같은 프로세스로 인정한다.
    /// 이 검증이 없으면 재사용된 PID 를 죽여 엉뚱한 프로세스를 날릴 수 있다.
    starttime: u64,
}

// id → RunningProc. 디스크(running.json)가 원본이고 이 맵은 캐시다.
static RUNNING_PIDS: Mutex<Option<HashMap<String, RunningProc>>> = Mutex::new(None);

fn running_state_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    let _ = fs::create_dir_all(&p);
    p.push("running.json");
    p
}

fn load_running() -> HashMap<String, RunningProc> {
    let Ok(s) = fs::read_to_string(running_state_path()) else {
        return HashMap::new();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

fn save_running(map: &HashMap<String, RunningProc>) {
    let path = running_state_path();
    match serde_json::to_string_pretty(map) {
        Ok(s) => {
            if let Err(e) = super::write_atomic(&path, s.as_bytes()) {
                eprintln!("[localman] 실행 상태 저장 실패: {e}");
            }
        }
        Err(e) => eprintln!("[localman] 실행 상태 직렬화 실패: {e}"),
    }
}

fn pids() -> std::sync::MutexGuard<'static, Option<HashMap<String, RunningProc>>> {
    let mut g = RUNNING_PIDS.lock().unwrap();
    if g.is_none() {
        // 첫 접근 때 디스크에서 읽어온다. 이전 localman 세션이 띄워둔 서버를
        // 여기서 회수하므로, 재시작해도 고아가 생기지 않는다.
        *g = Some(load_running());
    }
    g
}

/// 기록된 프로세스가 "그때 그 프로세스" 그대로 살아 있는지.
///
/// 두 가지를 모두 본다.
/// - starttime 이 같은가: PID 는 재사용되므로 이게 없으면 엉뚱한 프로세스를 죽일 수 있다.
/// - 좀비가 아닌가: localman 은 자식을 wait 하지 않아 종료된 서버가 좀비로 남는데,
///   좀비도 /proc/<pid>/stat 이 그대로 있어 starttime 만 보면 "실행 중"으로 오판한다.
fn is_alive(rec: &RunningProc) -> bool {
    platform::process_alive(rec.pid)
        && platform::proc_stat_fields(rec.pid).is_some_and(|(_, starttime)| starttime == rec.starttime)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Running(u32), // pid
    Stopped,
}

pub fn server_status(id: &str) -> ServerStatus {
    let mut g = pids();
    let map = g.as_mut().unwrap();
    if let Some(rec) = map.get(id).cloned() {
        // PID 존재만으로는 부족하다. PID 는 재사용되므로 starttime 까지 맞아야
        // "그때 띄운 그 서버"다.
        if is_alive(&rec) {
            return ServerStatus::Running(rec.pid);
        }
        // 죽은 기록은 남겨두지 않는다.
        map.remove(id);
        save_running(map);
    }
    ServerStatus::Stopped
}

pub fn start_server(project: &VhostProject) -> Result<u32, String> {
    // 이미 떠 있으면 또 띄우지 않는다.
    // 예전에는 localman 이 재시작되면 기존 서버를 기억하지 못해 같은 프로젝트를
    // 중복 실행했다(실제로 redholics viewer 가 7/28·7/30 두 벌 떠 있었다).
    // 이제 running.json 에서 회수하므로 여기서 걸러진다.
    if let ServerStatus::Running(pid) = server_status(&project.id) {
        eprintln!("[localman] 이미 실행 중 PID={pid} — 중복 실행하지 않음");
        return Ok(pid);
    }

    let dir = project.work_dir();
    if !std::path::Path::new(&dir).is_dir() {
        return Err(format!("디렉토리가 없습니다: {dir}"));
    }

    // 의존성이 없으면 자동 설치 (Python: venv, Next.js: node_modules)
    match project.project_type {
        ProjectType::Php => {
            return Err("PHP 프로젝트는 Apache가 직접 서빙하므로 dev server가 없습니다.".to_string());
        }
        _ if !deps_ready(project) => {
            eprintln!("[localman] 의존성 없음 → 자동 설치");
            setup_project(project)?;
        }
        _ => {}
    }

    let command = if project.start_command.is_empty() {
        match project.project_type {
            ProjectType::NextJs => auto_detect_next_command(&dir, project.port),
            _ => auto_detect_start_command(&dir, project.port),
        }
    } else {
        project.start_command.clone()
    };

    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("실행 명령어가 올바르지 않습니다.".to_string());
    }
    eprintln!("[localman] 서버 시작: {command} in {dir}");
    let pid = platform::spawn_in_new_group(parts[0], &parts[1..], &dir)?;

    // starttime 을 즉시 읽어 기록한다. 이후 이 값이 PID 재사용을 걸러낸다.
    // 못 읽으면(이미 즉사한 경우 등) 0 으로 둔다 — is_alive 가 false 가 되어
    // "죽은 것"으로 취급되므로 안전한 쪽으로 기운다.
    let (pgid, starttime) = platform::proc_stat_fields(pid).unwrap_or((pid, 0));
    let rec = RunningProc { pid, pgid, starttime };

    let mut g = pids();
    let map = g.as_mut().unwrap();
    map.insert(project.id.clone(), rec);
    // 메모리에만 두면 localman 이 죽는 순간 사라진다. 반드시 디스크에 남긴다.
    save_running(map);

    eprintln!("[localman] 서버 시작됨 PID={pid} PGID={pgid}");
    Ok(pid)
}

/// 서버를 프로세스 그룹째 정지한다.
///
/// SIGTERM 으로 정상 종료를 먼저 시도하고, 유예 시간이 지나도 살아 있으면
/// SIGKILL 로 확실히 끝낸다. 단일 PID 가 아니라 그룹에 보내므로
/// `npm run dev` → vite → esbuild, uvicorn → chromedriver → Chrome 처럼
/// 손자까지 함께 정리된다.
pub fn stop_server(id: &str) -> Result<(), String> {
    let rec = {
        let g = pids();
        g.as_ref().and_then(|m| m.get(id).cloned())
    };
    let Some(rec) = rec else {
        return Err("실행 중인 서버가 없습니다.".to_string());
    };

    // 기록만 남고 실제로는 이미 죽은 경우. 기록만 지우고 정상 처리한다.
    if !is_alive(&rec) {
        let mut g = pids();
        let map = g.as_mut().unwrap();
        map.remove(id);
        save_running(map);
        return Ok(());
    }

    eprintln!("[localman] 서버 중지 PID={} PGID={}", rec.pid, rec.pgid);
    platform::signal_group(rec.pgid, "TERM").map_err(|e| format!("kill 실패: {e}"))?;

    // 정상 종료를 최대 5초 기다린다.
    let mut terminated = false;
    for _ in 0..50 {
        if !is_alive(&rec) {
            terminated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if !terminated {
        eprintln!("[localman] SIGTERM 무응답 → SIGKILL PGID={}", rec.pgid);
        platform::signal_group(rec.pgid, "KILL").map_err(|e| format!("kill -9 실패: {e}"))?;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let mut g = pids();
    let map = g.as_mut().unwrap();
    map.remove(id);
    save_running(map);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::{TmpDir, project};
    use crate::platform::process_alive;
    use std::fs;

    fn pgrep(args: &[&str]) -> Vec<u32> {
        let out = match std::process::Command::new("pgrep").args(args).output() {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    /// npx가 실제 dev server를 자식으로 띄우는 구조에서, 중지하면 자식까지 정리돼야 한다.
    /// (래퍼만 죽으면 dev server가 포트를 물고 있어 재시작이 실패한다)
    #[test]
    fn stop_server_terminates_whole_process_group() {
        use std::os::unix::fs::PermissionsExt;

        let t = TmpDir::new("group");
        t.write("venv/bin/python3", ""); // deps_ready 통과용 마커
        t.write("wrapper.sh", "#!/bin/sh\nsleep 91 &\nwait\n");
        let script = t.0.join("wrapper.sh");
        let mut perm = fs::metadata(&script).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&script, perm).unwrap();

        let mut p = project(ProjectType::Python, &t.path(), "", 5099);
        p.id = "group-test".into();
        p.start_command = "./wrapper.sh".into();

        let pid = start_server(&p).expect("서버 시작 실패");

        // 래퍼가 띄운 자식(실제 dev server에 해당)의 PID를 잡아둔다
        let mut child = 0u32;
        assert!(
            wait_until(|| match pgrep(&["-P", &pid.to_string()]).first() {
                Some(&c) => { child = c; true }
                None => false,
            }),
            "래퍼가 자식 프로세스를 띄우지 않아 검증할 수 없습니다"
        );
        assert_eq!(server_status(&p.id), ServerStatus::Running(pid));

        stop_server(&p.id).expect("중지 실패");

        // 래퍼뿐 아니라 자식까지 정리돼야 한다
        assert!(wait_until(|| !process_alive(child)),
            "자식 프로세스가 살아남았습니다 (그룹 종료 실패)");
        assert!(wait_until(|| !process_alive(pid)), "래퍼 프로세스가 살아남았습니다");
        assert_eq!(server_status(&p.id), ServerStatus::Stopped);

        // 실패에 대비한 정리
        let _ = std::process::Command::new("kill").arg(child.to_string()).output();
    }
}
