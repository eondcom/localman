use std::path::Path;

use super::detect::detect_package_manager;
use crate::i18n::{tr, trf};
use super::project::{ProjectType, VhostProject};

/// 해당 명령이 PATH에 있는지 확인
fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Next.js(Node) 프로젝트 의존성 설치 — venv의 Node 버전.
/// lockfile에 맞는 매니저를 쓰고, 매니저가 없으면 corepack, 그래도 없으면 npm으로 폴백한다.
pub fn setup_node_modules(project: &VhostProject) -> Result<String, String> {
    let dir = project.work_dir();
    if !Path::new(&format!("{dir}/package.json")).exists() {
        return Err(format!(
            "{}\n{}",
            trf("package.json이 없습니다: {0}", &[&dir]),
            tr("하위 디렉토리 설정을 확인하세요.")
        ));
    }

    let pm = detect_package_manager(&dir);
    eprintln!("[localman] 패키지 매니저: {pm} ({dir})");

    // (프로그램, 앞에 붙는 인자들) 결정
    let (program, prefix): (&str, Vec<&str>) = if pm == "npm" || command_exists(pm) {
        (pm, vec![])
    } else if command_exists("corepack") {
        eprintln!("[localman] {pm} 없음 → corepack 경유");
        ("corepack", vec![pm])
    } else {
        eprintln!("[localman] {pm} / corepack 모두 없음 → npm 폴백");
        ("npm", vec![])
    };

    let mut args: Vec<&str> = prefix.clone();
    args.push("install");

    eprintln!("[localman] {program} {} 실행", args.join(" "));
    let r = std::process::Command::new(program)
        .args(&args)
        // corepack이 최초 실행 시 대화형 다운로드 동의를 물으면 GUI에서 멈추므로 비활성화.
        // CI=1은 설정하지 않는다 — pnpm/yarn이 그걸 보고 frozen-lockfile을 강제해서,
        // lockfile이 package.json보다 오래되면 설치가 통째로 실패한다.
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("{}\n{}", trf("{0} 실행 실패: {1}", &[&program, &e]), tr("PATH에 Node.js가 있는지 확인하세요.")))?;

    if !r.status.success() {
        let err = String::from_utf8_lossy(&r.stderr);
        let tail: String = err.lines().rev().take(15).collect::<Vec<_>>()
            .into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!("{}\n{tail}", trf("의존성 설치 실패 ({0}):", &[&program])));
    }

    let via = if program == "corepack" { format!("corepack {pm}") } else { program.to_string() };
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
    if let Some(pkgs) = ignored_build_scripts(&out) {
        // 설치는 성공했지만 반쪽이다. postinstall로 실제 코드를 받아오는 패키지
        // (@heroui-pro/react 등)가 여기 걸리면 import가 조용히 깨진다.
        return Ok(format!(
            "{}\n{}",
            trf("의존성 설치 완료 ({0}) — 다만 빌드 스크립트가 차단된 패키지가 있습니다: {1}", &[&via, &pkgs]),
            tr("postinstall로 코드를 받아오는 패키지면 import가 실패합니다. 해당 폴더에서 `pnpm approve-builds` 실행 후 다시 설치하세요.")
        ));
    }
    Ok(trf("의존성 설치 완료 ({0})", &[&via]))
}

/// pnpm이 "빌드 스크립트를 무시했다"고 알릴 때 그 패키지 목록을 뽑는다.
/// 설치는 성공(exit 0)으로 끝나므로 이 경고를 놓치면 반쪽 설치를 모른 채 넘어간다.
fn ignored_build_scripts(output: &str) -> Option<String> {
    let plain = strip_ansi(output);
    let lower = plain.to_lowercase();
    if !lower.contains("ignored build scripts") && !lower.contains("build scripts were ignored") {
        return None;
    }
    // 경고 줄 뒤에 붙는 패키지 이름들을 모은다 (형식이 버전마다 달라 느슨하게 훑는다)
    let names: Vec<&str> = plain
        .lines()
        .skip_while(|l| !l.to_lowercase().contains("ignored build scripts")
            && !l.to_lowercase().contains("build scripts were ignored"))
        .take(4)
        .flat_map(|l| l.split(|c: char| c == ',' || c == ':'))
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| {
            !s.is_empty()
                && (s.starts_with('@') || s.chars().next().is_some_and(|c| c.is_ascii_lowercase()))
                && !s.contains(' ')
                && s.len() < 60
        })
        .collect();
    if names.is_empty() {
        Some(tr("(이름 확인 불가 — 설치 로그를 확인하세요)").to_string())
    } else {
        Some(names.join(", "))
    }
}

/// 터미널 색상 escape sequence 제거 (pnpm 출력엔 ANSI 코드가 섞여 있다)
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ ... <최종 바이트> 형태를 통째로 건너뛴다
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 타입에 맞는 의존성 설치 진입점
pub fn setup_project(project: &VhostProject) -> Result<String, String> {
    match project.project_type {
        ProjectType::Python => setup_venv(project),
        ProjectType::NextJs => setup_node_modules(project),
        ProjectType::Php => Ok(tr("PHP 프로젝트는 설치할 의존성이 없습니다.").to_string()),
    }
}

/// 의존성이 이미 설치돼 있는지 (UI의 "패키지설치" 버튼 노출 판단용)
pub fn deps_ready(project: &VhostProject) -> bool {
    let dir = project.work_dir();
    match project.project_type {
        ProjectType::Python => Path::new(&format!("{dir}/venv/bin/python3")).exists(),
        ProjectType::NextJs => Path::new(&format!("{dir}/node_modules")).exists(),
        ProjectType::Php => true,
    }
}

/// venv 생성 + 패키지 설치 (동기, 시간이 걸릴 수 있음)
pub fn setup_venv(project: &VhostProject) -> Result<String, String> {
    let base = project.work_dir();
    let venv = format!("{base}/venv");
    let venv_python = format!("{venv}/bin/python3");
    let pip = format!("{venv}/bin/pip");

    // venv 생성
    if !std::path::Path::new(&venv_python).exists() {
        eprintln!("[localman] venv 생성 중: {venv}");
        let r = std::process::Command::new("python3")
            .args(["-m", "venv", &venv])
            .output()
            .map_err(|e| trf("venv 생성 실패: {0}", &[&e]))?;
        if !r.status.success() {
            return Err(format!("{}\n{}", tr("venv 생성 실패:"), String::from_utf8_lossy(&r.stderr)));
        }
        eprintln!("[localman] venv 생성 완료");
    } else {
        eprintln!("[localman] venv 이미 존재");
    }

    // pip 업그레이드
    let _ = std::process::Command::new(&pip)
        .args(["install", "--upgrade", "pip"])
        .output();

    // requirements.txt 우선
    let req = format!("{base}/requirements.txt");
    if std::path::Path::new(&req).exists() {
        eprintln!("[localman] pip install -r requirements.txt");
        let r = std::process::Command::new(&pip)
            .args(["install", "-r", &req])
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            return Err(format!("{}\n{}", tr("패키지 설치 실패:"), String::from_utf8_lossy(&r.stderr)));
        }
        build_frontend_if_needed(project)?;
        return Ok(tr("패키지 설치 완료 (requirements.txt)").to_string());
    }

    // pyproject.toml (poetry/pip editable)
    let pyproject = format!("{base}/pyproject.toml");
    if std::path::Path::new(&pyproject).exists() {
        eprintln!("[localman] pip install -e .");
        // poetry 의존성이 있으면 pip install 가능하도록 먼저 pip install poetry-core
        let _ = std::process::Command::new(&pip)
            .args(["install", "pip-tools", "setuptools", "wheel"])
            .output();
        let r = std::process::Command::new(&pip)
            .args(["install", "-e", "."])
            .current_dir(&base)
            .output()
            .map_err(|e| e.to_string())?;
        if !r.status.success() {
            // poetry인 경우 pip install . 로도 시도
            let r2 = std::process::Command::new(&pip)
                .args(["install", "."])
                .current_dir(&base)
                .output()
                .map_err(|e| e.to_string())?;
            if !r2.status.success() {
                return Err(format!("{}\n{}", tr("패키지 설치 실패:"), String::from_utf8_lossy(&r2.stderr)));
            }
        }
        build_frontend_if_needed(project)?;
        return Ok(tr("패키지 설치 완료 (pyproject.toml)").to_string());
    }

    // web/ 디렉토리가 있으면 프론트엔드 빌드
    build_frontend_if_needed(project)?;

    Ok(tr("venv 생성 완료 (설치할 패키지 파일 없음)").to_string())
}

/// web/ 디렉토리에 package.json이 있으면 npm install + npm run build
pub fn build_frontend_if_needed(project: &VhostProject) -> Result<(), String> {
    let web_dir = format!("{}/web", project.work_dir());
    let pkg_json = format!("{web_dir}/package.json");
    if !std::path::Path::new(&pkg_json).exists() {
        return Ok(());
    }
    eprintln!("[localman] 프론트엔드 빌드 시작: {web_dir}");

    // npm install
    let install = std::process::Command::new("npm")
        .args(["install", "--legacy-peer-deps"])
        .current_dir(&web_dir)
        .output()
        .map_err(|e| trf("npm install 실패: {0}", &[&e]))?;
    if !install.status.success() {
        return Err(format!("{}\n{}", tr("npm install 실패:"), String::from_utf8_lossy(&install.stderr)));
    }

    // npm run build
    let build = std::process::Command::new("npm")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .output()
        .map_err(|e| trf("npm run build 실패: {0}", &[&e]))?;
    if !build.status.success() {
        return Err(format!("{}\n{}", tr("npm run build 실패:"), String::from_utf8_lossy(&build.stderr)));
    }

    eprintln!("[localman] 프론트엔드 빌드 완료");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::{TmpDir, project};

    #[test]
    fn deps_ready_checks_the_right_marker_per_type() {
        let t = TmpDir::new("deps");
        let path = t.path();
        assert!(deps_ready(&project(ProjectType::Php, &path, "", 80)));
        assert!(!deps_ready(&project(ProjectType::NextJs, &path, "app", 5001)));
        t.mkdir("app/node_modules");
        assert!(deps_ready(&project(ProjectType::NextJs, &path, "app", 5001)));
        assert!(!deps_ready(&project(ProjectType::Python, &path, "", 5001)));
        t.write("venv/bin/python3", "");
        assert!(deps_ready(&project(ProjectType::Python, &path, "", 5001)));
    }

    /// pnpm은 빌드 스크립트를 차단해도 설치를 성공(exit 0)으로 끝낸다.
    /// 이 경고를 놓치면 @heroui-pro/react처럼 postinstall로 코드를 받아오는 패키지가
    /// 껍데기만 설치된 채 넘어가 import가 깨진다. (실제 pnpm 11 출력으로 검증)
    #[test]
    fn detects_pnpm_ignored_build_scripts() {
        let real = "\u{1b}[41m\u{1b}[31m[\u{1b}[39m\u{1b}[49m\u{1b}[41m\u{1b}[30mERR_PNPM_IGNORED_BUILDS\u{1b}[39m\u{1b}[49m\u{1b}[41m\u{1b}[31m]\u{1b}[39m\u{1b}[49m \u{1b}[31mIgnored build scripts: esbuild@0.28.2\u{1b}[39m\n\nRun \"pnpm approve-builds\" to pick which dependencies should be allowed to run scripts.\n";
        assert_eq!(ignored_build_scripts(real).as_deref(), Some("esbuild@0.28.2"));
    }

    #[test]
    fn lists_every_ignored_package() {
        let out = "Ignored build scripts: @heroui-pro/react@1.0.0-beta.8, unrs-resolver@1.7.2.\n\
                   Run \"pnpm approve-builds\" to pick which dependencies should be allowed.\n";
        let got = ignored_build_scripts(out).unwrap();
        assert!(got.contains("@heroui-pro/react@1.0.0-beta.8"), "got: {got}");
        assert!(got.contains("unrs-resolver@1.7.2"), "got: {got}");
    }

    #[test]
    fn no_warning_on_clean_install() {
        let out = "Packages: +2\ndependencies:\n+ esbuild 0.28.2\nDone in 3.6s using pnpm v11.23.0\n";
        assert_eq!(ignored_build_scripts(out), None);
    }

    #[test]
    fn strips_ansi_escape_codes() {
        assert_eq!(strip_ansi("\u{1b}[32m✓\u{1b}[39m ok"), "✓ ok");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
