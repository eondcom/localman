//! 로컬 HTTPS: localman 전용 인증기관(CA)을 만들어 OS에 한 번 신뢰시키고,
//! 프로젝트 도메인마다 인증서를 발급·자동 갱신한다 (mkcert 와 같은 방식).
//!
//! Let's Encrypt 는 인터넷에서 도메인 소유를 확인하므로 *.localhost 에는 발급하지 않는다.
//!
//! CA 는 시스템 전체에서 신뢰되므로 키가 새면 아무 사이트나 위조할 수 있다. 그래서
//! 이름 제약(Name Constraints)으로 localhost 와 그 하위 도메인에만 발급할 수 있게 묶는다.
//! 키는 이 PC 밖으로 내보내지 않는다(백업 묶음에도 담지 않는다).

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, KeyPair, KeyUsagePurpose, NameConstraints,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::{Duration, OffsetDateTime};

use super::settings::data_dir;

/// 도메인 인증서 유효 기간. macOS 는 사설 CA 인증서도 825일을 넘으면 거부한다.
const LEAF_DAYS: i64 = 365;
/// 만료까지 이만큼 남으면 새로 발급한다.
const RENEW_BEFORE_DAYS: i64 = 30;
const CA_YEARS: i64 = 10;

fn tls_dir() -> PathBuf {
    let d = data_dir().join("tls");
    let _ = fs::create_dir_all(d.join("certs"));
    d
}

pub fn ca_cert_path() -> PathBuf {
    tls_dir().join("ca.pem")
}

fn ca_key_path() -> PathBuf {
    tls_dir().join("ca.key")
}

fn ca_meta_path() -> PathBuf {
    tls_dir().join("ca.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    common_name: String,
    /// 만료 시각 (unix 초)
    not_after: i64,
}

/// 비밀 키 파일은 소유자만 읽게 쓴다.
fn write_private(path: &Path, data: &str) -> Result<(), String> {
    fs::write(path, data).map_err(|e| format!("{} 쓰기 실패: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn ca_params(common_name: &str, not_after: OffsetDateTime) -> CertificateParams {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "LocalMan");
    dn.push(DnType::CommonName, common_name);
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    p.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName("localhost".into())],
        excluded_subtrees: vec![],
    });
    p.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    p.not_after = not_after;
    p
}

/// 저장된 CA 를 서명에 쓸 수 있는 형태로 불러온다.
///
/// rcgen 이 발급자로 쓰는 건 DN·키·키 식별 방식뿐이라, 같은 이름과 같은 키로
/// 매개변수를 다시 만들면 원래 CA 와 같은 발급자가 된다(인증서를 파싱할 필요가 없다).
fn load_ca() -> Option<(Certificate, KeyPair)> {
    let meta: Meta = serde_json::from_str(&fs::read_to_string(ca_meta_path()).ok()?).ok()?;
    let key = KeyPair::from_pem(&fs::read_to_string(ca_key_path()).ok()?).ok()?;
    if !ca_cert_path().exists() {
        return None;
    }
    let not_after = OffsetDateTime::from_unix_timestamp(meta.not_after).ok()?;
    let cert = ca_params(&meta.common_name, not_after).self_signed(&key).ok()?;
    Some((cert, key))
}

pub fn ca_exists() -> bool {
    load_ca().is_some()
}

/// CA 가 없으면 만든다. 이미 있으면 그대로 둔다(새로 만들면 다시 신뢰 등록해야 하므로).
pub fn ensure_ca() -> Result<(), String> {
    if ca_exists() {
        return Ok(());
    }
    let host = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let common_name = format!("LocalMan Local CA ({host})");
    let not_after = OffsetDateTime::now_utc() + Duration::days(365 * CA_YEARS);
    let key = KeyPair::generate().map_err(|e| format!("CA 키 생성 실패: {e}"))?;
    let cert = ca_params(&common_name, not_after)
        .self_signed(&key)
        .map_err(|e| format!("CA 인증서 생성 실패: {e}"))?;
    write_private(&ca_key_path(), &key.serialize_pem())?;
    fs::write(ca_cert_path(), cert.pem()).map_err(|e| e.to_string())?;
    let meta = Meta { common_name, not_after: not_after.unix_timestamp() };
    fs::write(ca_meta_path(), serde_json::to_string_pretty(&meta).unwrap()).map_err(|e| e.to_string())?;
    eprintln!("[localman] 로컬 CA 생성: {}", ca_cert_path().display());
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CertPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

fn cert_paths(domain: &str) -> (CertPaths, PathBuf) {
    let dir = tls_dir().join("certs");
    (
        CertPaths { cert: dir.join(format!("{domain}.pem")), key: dir.join(format!("{domain}.key")) },
        dir.join(format!("{domain}.json")),
    )
}

/// 인증서가 없거나 곧 만료되면 새로 발급해야 한다.
fn needs_issue(domain: &str) -> bool {
    let (paths, meta_path) = cert_paths(domain);
    if !paths.cert.exists() || !paths.key.exists() {
        return true;
    }
    let Some(meta) = fs::read_to_string(meta_path).ok().and_then(|s| serde_json::from_str::<Meta>(&s).ok()) else {
        return true;
    };
    meta.not_after - OffsetDateTime::now_utc().unix_timestamp() < RENEW_BEFORE_DAYS * 86_400
}

fn issue(domain: &str) -> Result<CertPaths, String> {
    let (ca, ca_key) = load_ca().ok_or("로컬 인증기관이 없습니다.")?;
    let mut p = CertificateParams::new(vec![domain.to_string()]).map_err(|e| e.to_string())?;
    p.distinguished_name = DistinguishedName::new();
    p.distinguished_name.push(DnType::CommonName, domain);
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    let not_after = OffsetDateTime::now_utc() + Duration::days(LEAF_DAYS);
    p.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    p.not_after = not_after;

    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = p.signed_by(&key, &ca, &ca_key).map_err(|e| format!("{domain} 인증서 발급 실패: {e}"))?;
    let (paths, meta_path) = cert_paths(domain);
    write_private(&paths.key, &key.serialize_pem())?;
    fs::write(&paths.cert, cert.pem()).map_err(|e| e.to_string())?;
    let meta = Meta { common_name: domain.to_string(), not_after: not_after.unix_timestamp() };
    fs::write(meta_path, serde_json::to_string_pretty(&meta).unwrap()).map_err(|e| e.to_string())?;
    eprintln!("[localman] 인증서 발급: {domain}");
    Ok(paths)
}

/// 도메인 인증서를 보장한다. 없거나 만료가 다가오면 발급하고, 아니면 기존 것을 쓴다.
pub fn ensure_cert(domain: &str) -> Result<CertPaths, String> {
    ensure_ca()?;
    if needs_issue(domain) {
        issue(domain)
    } else {
        Ok(cert_paths(domain).0)
    }
}

/// 만료가 다가온 인증서를 새로 발급한다. 갱신한 도메인 목록을 돌려준다.
/// (Apache 는 시작할 때 인증서를 읽으므로, 하나라도 갱신했으면 호출한 쪽이 reload 해야 한다)
pub fn renew_due(domains: &[String]) -> Vec<Result<String, String>> {
    if !ca_exists() {
        return Vec::new();
    }
    domains
        .iter()
        .filter(|d| needs_issue(d))
        .map(|d| issue(d).map(|_| d.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 다시 불러온 CA 로 서명한 인증서가 처음 만든 CA 인증서로 검증돼야 하고,
    /// 이름 제약 때문에 localhost 밖의 도메인은 검증에 실패해야 한다. (openssl 로 확인)
    #[test]
    fn reloaded_ca_chain_verifies_and_is_name_constrained() {
        if std::process::Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl 없음 — 건너뜀");
            return;
        }
        let dir = std::env::temp_dir().join(format!("localman-tls-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let key = KeyPair::generate().unwrap();
        let na = OffsetDateTime::now_utc() + Duration::days(30);
        let ca = ca_params("LocalMan Test CA", na).self_signed(&key).unwrap();
        fs::write(dir.join("ca.pem"), ca.pem()).unwrap();

        // 저장된 PEM 키로 다시 만든 발급자
        let key2 = KeyPair::from_pem(&key.serialize_pem()).unwrap();
        let ca2 = ca_params("LocalMan Test CA", na).self_signed(&key2).unwrap();

        let verify = |domain: &str| -> bool {
            let mut p = CertificateParams::new(vec![domain.to_string()]).unwrap();
            p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            p.use_authority_key_identifier_extension = true;
            let leaf_key = KeyPair::generate().unwrap();
            let leaf = p.signed_by(&leaf_key, &ca2, &key2).unwrap();
            let f = dir.join(format!("{domain}.pem"));
            fs::write(&f, leaf.pem()).unwrap();
            std::process::Command::new("openssl")
                .args(["verify", "-CAfile"])
                .arg(dir.join("ca.pem"))
                .arg(&f)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        assert!(verify("demo.localhost"), "다시 불러온 CA 로 서명한 인증서가 검증되지 않습니다");
        assert!(!verify("example.com"), "이름 제약이 걸려 있지 않습니다");
        let _ = fs::remove_dir_all(&dir);
    }
}

