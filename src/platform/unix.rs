//! 리눅스·맥이 함께 쓰는 POSIX 작업: 프로세스 그룹, /etc/hosts, root 소유 로그 파일.

use std::fs;
use std::os::unix::process::CommandExt; // process_group
use std::path::Path;
use std::process::Command;

use crate::i18n::tr;

/// 명령을 새 프로세스 그룹의 리더로 띄우고 detach 한다. 자식 PID 를 돌려준다.
///
/// 표준입출력은 물려주지 않는다: 입력은 닫고, 출력은 `log` 파일(덧붙이기)로, 없으면 버린다.
/// 물려주면 `LocalMan --mcp` 에서 띄운 서버가 MCP 응답 채널에 로그를 섞고(프로토콜 깨짐),
/// MCP 가 끝나도 파이프를 쥐고 있어 부른 쪽이 끝을 못 받는다(2026-10-09 yasul-web 으로 멈춤).
pub fn spawn_in_new_group(program: &str, args: &[&str], dir: &str, log: Option<&Path>) -> Result<u32, String> {
    use std::process::Stdio;
    let (out, err) = match log {
        Some(path) => {
            if let Some(d) = path.parent() {
                let _ = fs::create_dir_all(d);
            }
            let f = fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let f2 = f.try_clone().map_err(|e| e.to_string())?;
            (Stdio::from(f), Stdio::from(f2))
        }
        None => (Stdio::null(), Stdio::null()),
    };
    let child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        // 자식을 새 프로세스 그룹의 리더로 만든다(pgid == 자식 pid).
        // 그래야 나중에 `kill -- -pgid` 한 방으로 그 서버가 파생시킨
        // vite·chromedriver·Chrome, npx 가 띄운 next-server 까지 통째로 정리할 수 있다.
        // 그러지 않으면 래퍼만 죽고 dev server 가 포트를 계속 물고 있어 재시작이 실패한다.
        .process_group(0)
        .spawn()
        .map_err(|e| crate::i18n::trf("실행 실패: {0}", &[&e]))?;
    let pid = child.id();
    // 서버가 끝나면 이 스레드가 회수(wait)해 좀비로 남지 않게 한다.
    // (SIGCHLD 를 SIG_IGN 으로 두는 방법은 Command::output() 까지 ECHILD 로
    //  실패시켜 systemctl·brew·mysql 호출 결과를 전부 잃게 만든다)
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(pid)
}

/// 프로세스 그룹 전체에 시그널을 보낸다 (`kill -SIG -- -PGID`).
///
/// 단일 PID 로는 `npm run dev` 가 띄운 vite, uvicorn 이 띄운 chromedriver 처럼
/// 손자 프로세스가 남는다. 그룹째 보내야 실제로 정리된다.
pub fn signal_group(pgid: u32, sig: &str) -> Result<(), String> {
    let out = Command::new("kill")
        .arg(format!("-{sig}"))
        .arg("--")
        .arg(format!("-{pgid}"))
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // 이미 다 죽어서 그룹이 없는 경우는 성공으로 친다.
    if err.contains("No such process") {
        Ok(())
    } else {
        Err(err)
    }
}

pub fn update_hosts(domain: &str, add: bool) -> Result<(), String> {
    let hosts = fs::read_to_string("/etc/hosts").map_err(|e| e.to_string())?;
    let entry = format!("127.0.0.1\t{domain}");

    let new_hosts = if add {
        if hosts.contains(domain) {
            return Ok(());
        }
        format!("{hosts}\n{entry}\n")
    } else {
        hosts
            .lines()
            .filter(|l| !l.contains(domain))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };

    let tmp = "/tmp/localman_hosts";
    fs::write(tmp, new_hosts).map_err(|e| e.to_string())?;
    Command::new("sudo")
        .args(["cp", tmp, "/etc/hosts"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 로그 파일의 마지막 `max_lines`줄을 읽어 반환한다.
///
/// 로그 파일은 보통 `-rw-r--r--`(전체 읽기 가능)이므로 sudo 없이 직접 읽는다.
/// 권한 문제로 직접 읽기에 실패하면 `sudo tail`로 한 번 더 시도한다.
pub fn read_log(path: &str, max_lines: usize) -> Result<String, String> {
    if !Path::new(path).exists() {
        // vhost ErrorLog 안내는 Apache 에러 로그에만 (서버 출력·예약 작업 로그는 실행해야 생긴다)
        let hint = if path.ends_with(".error.log") {
            format!("\n\n{}", tr("(프로젝트를 한 번 '수정 → 저장'하면 vhost에 전용 ErrorLog가 추가됩니다.)"))
        } else {
            String::new()
        };
        return Err(format!("{}\n{path}{hint}", tr("로그 파일이 아직 없습니다:")));
    }

    // 1) 직접 읽기 시도
    match std::fs::read(path) {
        // 색을 쓰는 프로그램(spoofdpi 등)의 로그는 ESC 코드가 섞여 읽기 어렵다
        Ok(bytes) => Ok(crate::domain::eondctl::strip_ansi(&tail_lines(&String::from_utf8_lossy(&bytes), max_lines))),
        Err(_) => {
            // 2) 권한 문제 → sudo tail 폴백
            let out = Command::new("sudo")
                .args(["-n", "tail", "-n", &max_lines.to_string(), path])
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(crate::domain::eondctl::strip_ansi(&String::from_utf8_lossy(&out.stdout)))
            } else {
                Err(format!(
                    "{}\n{}",
                    tr("로그를 읽을 수 없습니다 (권한). sudoers 설정이 필요할 수 있습니다."),
                    String::from_utf8_lossy(&out.stderr).trim()
                ))
            }
        }
    }
}

/// 로그 파일을 비운다(truncate). 파일 소유자가 root이므로 sudo가 필요하다.
pub fn clear_log(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err(tr("비울 로그 파일이 없습니다.").to_string());
    }
    // 맥에는 truncate 명령이 없어 /dev/null 을 덮어써서 비운다.
    #[cfg(target_os = "linux")]
    let args = ["-n", "truncate", "-s", "0", path];
    #[cfg(target_os = "macos")]
    let args = ["-n", "/bin/cp", "/dev/null", path];
    let out = Command::new("sudo")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!(
            "{}\n{err}",
            tr("로그를 비우지 못했습니다. install-sudoers.sh를 다시 실행해 truncate 규칙을 추가하세요.")
        ))
    }
}

/// 문자열에서 마지막 n줄만 추출한다.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 끝난 dev server 는 좀비로 남지 않아야 하고, 그 와중에도
    /// 다른 명령의 output() 은 종료 코드를 정상으로 받아야 한다.
    #[test]
    fn spawned_server_is_reaped_without_breaking_output() {
        let pid = spawn_in_new_group("true", &[], "/", None).unwrap();
        let mut reaped = false;
        for _ in 0..100 {
            if !crate::platform::process_alive(pid) {
                reaped = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(reaped, "종료된 서버가 좀비로 남았습니다");
        let out = Command::new("true").output().expect("output() 이 실패했습니다");
        assert!(out.status.success());
    }
}
