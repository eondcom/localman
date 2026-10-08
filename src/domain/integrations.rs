//! 외부 도구 연동: Adminer(pma.localhost), 라이믹스 rx-cli.

use std::fs;
use std::path::{Path, PathBuf};

use super::apache::write_vhost;
use super::project::{ProjectType, VhostProject};
use crate::i18n::{tr, trf};
use crate::platform::update_hosts;
use super::settings::PmaTool;

/// 앱에 넣어 둔 Adminer (Apache-2.0 / GPL-2.0, assets/adminer/LICENSE-Apache-2.0.txt).
/// 최신판을 받지 못할 때(오프라인 등) 이것을 쓴다.
const BUNDLED_ADMINER: &[u8] = include_bytes!("../../assets/adminer/adminer.php");

fn data_root() -> PathBuf {
    let mut dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.push("localman");
    dir
}

/// pma.localhost 에 고른 DB 관리 도구(Adminer 또는 phpMyAdmin)를 설치하고 URL 을 돌려준다.
/// 이미 설치돼 있으면 vhost·hosts 만 보장한다(멱등). 도구를 바꾸면 pma.localhost 가 그 도구를 가리킨다.
pub fn ensure_pma_site(tool: PmaTool) -> Result<String, String> {
    let (root, name) = match tool {
        PmaTool::Adminer => (ensure_adminer()?, "Adminer"),
        PmaTool::PhpMyAdmin => (ensure_phpmyadmin()?, "phpMyAdmin"),
    };
    let pma = VhostProject {
        id: "pma".to_string(),
        name: name.to_string(),
        path: root.to_string_lossy().to_string(),
        domain: "pma.localhost".to_string(),
        project_type: ProjectType::Php,
        port: 80,
        start_command: String::new(),
        app_dir: String::new(),
        db: None,
    };
    write_vhost(&pma)?;
    update_hosts(&pma.domain, true)?;
    Ok(format!("http://{}", pma.domain))
}

/// <데이터 폴더>/pma/index.php 에 Adminer. 최신판을 받고, 못 받으면 앱에 넣어 둔 판을 쓴다.
fn ensure_adminer() -> Result<PathBuf, String> {
    let dir = data_root().join("pma");
    fs::create_dir_all(&dir).map_err(|e| trf("pma 디렉토리 생성 실패: {0}", &[&e]))?;
    let index = dir.join("index.php");
    if !index.exists() && download_adminer(&index).is_err() {
        fs::write(&index, BUNDLED_ADMINER).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// adminer.org에서 최신 Adminer를 내려받아 dest에 저장한다(curl 우선, 실패 시 wget).
fn download_adminer(dest: &Path) -> Result<(), String> {
    let url = "https://www.adminer.org/latest.php";
    let dest_str = dest.to_str().ok_or_else(|| tr("경로 변환 실패").to_string())?;

    let curl = std::process::Command::new("curl")
        .args(["-fsSL", url, "-o", dest_str])
        .output();
    if let Ok(o) = curl {
        if o.status.success() && dest.exists() {
            return Ok(());
        }
    }
    let wget = std::process::Command::new("wget")
        .args(["-q", url, "-O", dest_str])
        .output();
    if let Ok(o) = wget {
        if o.status.success() && dest.exists() {
            return Ok(());
        }
    }
    let _ = fs::remove_file(dest);
    Err(tr("Adminer 다운로드 실패 (curl/wget·인터넷 연결 확인)").to_string())
}

fn curl_bytes(url: &str) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--retry", "2", url])
        .output()
        .map_err(|e| trf("curl 실행 실패: {0}", &[&e]))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(trf("{0} 받기 실패: {1}", &[&url, &String::from_utf8_lossy(&out.stderr).trim()]))
    }
}

/// phpMyAdmin 최신판을 <데이터 폴더>/phpmyadmin/<버전> 에 설치한다.
/// 공식 version.json 으로 버전을 고르고, 공식 SHA-256 으로 검증한 뒤 푼다. 설정 파일도 만든다.
fn ensure_phpmyadmin() -> Result<PathBuf, String> {
    let base = data_root().join("phpmyadmin");
    let latest = curl_bytes("https://www.phpmyadmin.net/home_page/version.json")
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("version")?.as_str().map(str::to_string));
    let Some(version) = latest else {
        // 오프라인 — 이미 설치된 판이 있으면 그것을 쓴다
        let installed = fs::read_dir(&base).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).filter(|p| p.join("index.php").exists()).max()
        });
        return match installed {
            Some(root) => {
                write_phpmyadmin_config(&root)?;
                Ok(root)
            }
            None => Err(tr("phpMyAdmin 버전을 알 수 없습니다.").to_string()),
        };
    };
    let name = format!("phpMyAdmin-{version}-all-languages");
    let root = base.join(&name);
    if !root.join("index.php").exists() {
        let url = format!("https://files.phpmyadmin.net/phpMyAdmin/{version}/{name}.tar.gz");
        let data = curl_bytes(&url)?;
        let sums = String::from_utf8_lossy(&curl_bytes(&format!("{url}.sha256"))?).to_string();
        let expected = sums.split_whitespace().next().unwrap_or("").to_lowercase();
        let actual = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&data));
        if expected.is_empty() || expected != actual {
            return Err(tr("phpMyAdmin 체크섬이 맞지 않아 설치를 멈췄습니다.").to_string());
        }
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        // tar 크레이트는 폴더 밖으로 나가는 경로(..)를 풀지 않는다
        tar::Archive::new(flate2::read::GzDecoder::new(data.as_slice()))
            .unpack(&base)
            .map_err(|e| trf("압축 풀기 실패: {0}", &[&e]))?;
    }
    write_phpmyadmin_config(&root)?;
    Ok(root)
}

/// config.inc.php — 쿠키 로그인, 이 PC 의 DB 서버(localhost), 임시 폴더, 쿠키 암호화 키(32자).
fn write_phpmyadmin_config(root: &Path) -> Result<(), String> {
    let conf = root.join("config.inc.php");
    if conf.exists() {
        return Ok(());
    }
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let secret: String = (0..32).map(|_| CHARS[(OsRng.next_u32() as usize) % CHARS.len()] as char).collect();
    let tmp = root.join("tmp");
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let php = format!(
        "<?php\n// localman 이 만든 설정 — pma.localhost (이 PC 에서만 열린다)\n\
         $cfg['blowfish_secret'] = '{secret}';\n\
         $i = 1;\n\
         $cfg['Servers'][$i]['auth_type'] = 'cookie';\n\
         $cfg['Servers'][$i]['host'] = 'localhost';\n\
         $cfg['Servers'][$i]['AllowNoPassword'] = false;\n\
         $cfg['TempDir'] = '{}';\n",
        tmp.display()
    );
    fs::write(&conf, php).map_err(|e| e.to_string())
}

