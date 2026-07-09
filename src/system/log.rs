use std::path::Path;
use std::process::Command;

/// Apache 로그 디렉토리. ${APACHE_LOG_DIR}는 데비안/우분투에서 이 경로로 해석된다.
const APACHE_LOG_DIR: &str = "/var/log/apache2";

/// 프로젝트별 에러 로그 경로. vhost의 `ErrorLog ${APACHE_LOG_DIR}/{id}.error.log`와 일치한다.
pub fn error_log_path(id: &str) -> String {
    format!("{APACHE_LOG_DIR}/{id}.error.log")
}

/// 로그 파일의 마지막 `max_lines`줄을 읽어 반환한다.
///
/// 로그 파일은 보통 `-rw-r--r--`(전체 읽기 가능)이므로 sudo 없이 직접 읽는다.
/// 권한 문제로 직접 읽기에 실패하면 `sudo tail`로 한 번 더 시도한다.
pub fn read_log(path: &str, max_lines: usize) -> Result<String, String> {
    if !Path::new(path).exists() {
        return Err(format!(
            "로그 파일이 아직 없습니다:\n{path}\n\n\
             (프로젝트를 한 번 '수정 → 저장'하면 vhost에 전용 ErrorLog가 추가됩니다.)"
        ));
    }

    // 1) 직접 읽기 시도
    match std::fs::read(path) {
        Ok(bytes) => Ok(tail_lines(&String::from_utf8_lossy(&bytes), max_lines)),
        Err(_) => {
            // 2) 권한 문제 → sudo tail 폴백
            let out = Command::new("sudo")
                .args(["-n", "tail", "-n", &max_lines.to_string(), path])
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).to_string())
            } else {
                Err(format!(
                    "로그를 읽을 수 없습니다 (권한). sudoers 설정이 필요할 수 있습니다.\n{}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ))
            }
        }
    }
}

/// 로그 파일을 비운다(truncate). 파일 소유자가 root이므로 sudo가 필요하다.
pub fn clear_log(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err("비울 로그 파일이 없습니다.".to_string());
    }
    let out = Command::new("sudo")
        .args(["-n", "truncate", "-s", "0", path])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!(
            "로그를 비우지 못했습니다. install-sudoers.sh를 다시 실행해 truncate 규칙을 추가하세요.\n{err}"
        ))
    }
}

/// 문자열에서 마지막 n줄만 추출한다.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}
