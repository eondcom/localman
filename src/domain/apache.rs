//! Apache vhost 설정 생성. 설정 내용은 OS 무관하고, 파일을 어디에 두고 어떻게
//! 활성화하는지만 platform 쪽에서 OS별로 처리한다.

use super::project::{ProjectType, VhostProject, list_projects};
use super::settings::{load_settings, save_settings};
use super::tls::{self, CertPaths};
use crate::platform;

/// 이 PC 에서 온 요청만 받는다.
///
/// Apache 는 기본으로 모든 인터페이스(0.0.0.0:80)에서 받으므로, 공용 와이파이 같은
/// 망에서 다른 사람이 `Host: pma.localhost` 헤더만 붙이면 Adminer·dev 프로젝트에 닿는다.
/// `<Location />` 은 `<Directory>` 보다 나중에 병합되고 ProxyPass 요청에도 적용되므로
/// PHP·프록시 vhost 모두 이것 하나로 막힌다. (:443 은 :80 설정을 복제하므로 함께 막힌다)
const LOCAL_ONLY: &str = "\x20   <Location />\n\
                          \x20       Require local\n\
                          \x20   </Location>\n";

pub(crate) fn build_vhost_conf(p: &VhostProject) -> String {
    let conf = build_vhost_body(p);
    // 모든 템플릿이 "</VirtualHost>\n" 으로 끝난다
    let end = conf.rfind("</VirtualHost>").expect("vhost 템플릿에 </VirtualHost> 가 없다");
    format!("{}{LOCAL_ONLY}{}", &conf[..end], &conf[end..])
}

fn build_vhost_body(p: &VhostProject) -> String {
    match p.project_type {
        ProjectType::Php => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   DocumentRoot {path}\n\
             \x20   <Directory {path}>\n\
             \x20       AllowOverride All\n\
             \x20       Require all granted\n\
             \x20   </Directory>\n\
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            // public/ 같은 하위 디렉토리를 DocumentRoot로 쓸 수 있게 work_dir 사용
            path = p.work_dir(),
            id = p.id,
        ),
        // Next.js dev server는 HMR에 웹소켓을 쓴다. HMR 경로는 Next 버전과 번들러
        // (webpack/turbopack)에 따라 달라지므로 경로를 고정하지 않고 Upgrade 헤더로 판별한다.
        // RewriteRule [P]가 먼저 처리되고, 웹소켓이 아닌 요청만 아래 ProxyPass로 내려간다.
        ProjectType::NextJs => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   ProxyPreserveHost On\n\
             \x20   ProxyRequests Off\n\
             \x20   RewriteEngine On\n\
             \x20   RewriteCond %{{HTTP:Upgrade}} =websocket [NC]\n\
             \x20   RewriteRule ^/?(.*) ws://127.0.0.1:{port}/$1 [P,L]\n\
             \x20   ProxyPass / http://127.0.0.1:{port}/\n\
             \x20   ProxyPassReverse / http://127.0.0.1:{port}/\n\
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            port = p.port,
            id = p.id,
        ),
        ProjectType::Python => format!(
            "<VirtualHost *:80>\n\
             \x20   ServerName {domain}\n\
             \x20   ProxyPreserveHost On\n\
             \x20   ProxyPass / http://127.0.0.1:{port}/\n\
             \x20   ProxyPassReverse / http://127.0.0.1:{port}/\n\
             \x20   ErrorLog ${{APACHE_LOG_DIR}}/{id}.error.log\n\
             \x20   CustomLog ${{APACHE_LOG_DIR}}/{id}.access.log combined\n\
             </VirtualHost>\n",
            domain = p.domain,
            port = p.port,
            id = p.id,
        ),
    }
}

/// :80 설정을 그대로 복제해 :443 가상호스트를 만든다. http 도 계속 열어 둔다
/// (dev server 나 외부 콜백이 http 를 쓰는 경우가 있어 강제로 넘기지 않는다).
pub(crate) fn tls_vhost(conf80: &str, tls: &CertPaths) -> String {
    // 맥은 인증서 경로에 공백("Application Support")이 있어 따옴표로 감싼다.
    conf80.replacen(
        "<VirtualHost *:80>\n",
        &format!(
            "<VirtualHost *:443>\n\
             \x20   SSLEngine on\n\
             \x20   SSLCertificateFile \"{}\"\n\
             \x20   SSLCertificateKeyFile \"{}\"\n\
             \x20   # 프록시 뒤 앱이 https 로 접속된 걸 알도록 (리다이렉트·쿠키 Secure 판단)\n\
             \x20   RequestHeader set X-Forwarded-Proto https\n",
            tls.cert.display(),
            tls.key.display(),
        ),
        1,
    )
}

pub(crate) fn write_vhost(p: &VhostProject) -> Result<(), String> {
    let mut conf = build_vhost_conf(p);
    if load_settings().https {
        let paths = tls::ensure_cert(&p.domain)?;
        platform::ensure_ssl_module()?;
        let tls_conf = tls_vhost(&conf, &paths);
        conf.push('\n');
        conf.push_str(&tls_conf);
    }
    platform::write_site(&p.id, &conf)
}

/// 등록된 모든 사이트의 vhost 를 다시 쓴다 (https 켜기/끄기 후). 줄 단위 결과를 돌려준다.
fn rewrite_all_sites() -> Vec<String> {
    list_projects()
        .iter()
        .map(|p| match write_vhost(p) {
            Ok(()) => format!("✓ {}", p.domain),
            Err(e) => format!("✗ {}: {e}", p.domain),
        })
        .collect()
}

/// 모든 프로젝트의 https 를 켜거나 끈다.
/// 켤 때는 로컬 인증기관을 만들고, 아직 신뢰 등록이 안 됐으면 OS 에 등록한다.
pub fn set_https(enabled: bool) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    if enabled {
        tls::ensure_ca()?;
        let ca = tls::ca_cert_path();
        if !platform::ca_trusted(&ca) {
            log.push(platform::trust_ca(&ca)?);
        }
    }
    let mut s = load_settings();
    s.https = enabled;
    save_settings(&s)?;
    log.extend(rewrite_all_sites());
    Ok(log)
}

/// 만료가 다가온 인증서를 갱신하고, 하나라도 바뀌었으면 웹 서버를 reload 한다.
/// 앱 시작 때와 주기적으로 불린다.
pub fn renew_certs() -> Vec<String> {
    if !load_settings().https {
        return Vec::new();
    }
    let domains: Vec<String> = list_projects().into_iter().map(|p| p.domain).collect();
    let results = tls::renew_due(&domains);
    if results.iter().any(|r| r.is_ok()) {
        platform::reload_web_server();
    }
    results
        .into_iter()
        .map(|r| match r {
            Ok(d) => format!("✓ 인증서 갱신: {d}"),
            Err(e) => format!("✗ 인증서 갱신 실패: {e}"),
        })
        .collect()
}

pub(crate) fn remove_vhost(p: &VhostProject) -> Result<(), String> {
    platform::remove_site(&p.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::test_util::project;

    #[test]
    fn next_vhost_proxies_websocket_before_root() {
        let conf = build_vhost_conf(&project(ProjectType::NextJs, "/srv/x", "app", 5005));
        // HMR 경로를 고정하지 않고 Upgrade 헤더로 판별한다 (webpack/turbopack 모두 대응)
        assert!(conf.contains("RewriteCond %{HTTP:Upgrade} =websocket [NC]"));
        assert!(conf.contains("RewriteRule ^/?(.*) ws://127.0.0.1:5005/$1 [P,L]"));
        assert!(conf.contains("ProxyPass / http://127.0.0.1:5005/"));
        // 웹소켓 규칙이 루트 프록시보다 먼저 와야 매칭된다
        let ws = conf.find("RewriteRule").unwrap();
        let root = conf.find("ProxyPass / http").unwrap();
        assert!(ws < root, "웹소켓 규칙이 루트 프록시보다 뒤에 있습니다");
    }

    #[test]
    fn php_vhost_uses_app_dir_as_document_root() {
        let conf = build_vhost_conf(&project(ProjectType::Php, "/srv/laravel", "public", 80));
        assert!(conf.contains("DocumentRoot /srv/laravel/public"));
        // 하위 디렉토리가 없으면 경로 그대로
        let conf2 = build_vhost_conf(&project(ProjectType::Php, "/srv/plain", "", 80));
        assert!(conf2.contains("DocumentRoot /srv/plain\n"));
    }

    #[test]
    fn tls_vhost_mirrors_http_block_on_443() {
        let conf = build_vhost_conf(&project(ProjectType::NextJs, "/srv/x", "", 5005));
        let paths = CertPaths {
            cert: "/Users/me/Library/Application Support/localman/tls/certs/demo.localhost.pem".into(),
            key: "/Users/me/Library/Application Support/localman/tls/certs/demo.localhost.key".into(),
        };
        let tls = tls_vhost(&conf, &paths);
        assert!(tls.starts_with("<VirtualHost *:443>\n"));
        assert!(!tls.contains("*:80"));
        // 공백이 있는 경로는 따옴표로 감싸야 Apache 가 읽는다
        assert!(tls.contains("SSLCertificateFile \"/Users/me/Library/Application Support/"));
        // 웹소켓(HMR) 프록시 규칙도 그대로 있어야 한다
        assert!(tls.contains("RewriteRule ^/?(.*) ws://127.0.0.1:5005/$1 [P,L]"));
    }

    #[test]
    fn every_vhost_accepts_local_requests_only() {
        for t in [ProjectType::Php, ProjectType::Python, ProjectType::NextJs] {
            let conf = build_vhost_conf(&project(t.clone(), "/srv/x", "", 5001));
            let loc = conf.find("    <Location />\n        Require local\n    </Location>\n").expect("Require local 없음");
            assert!(loc < conf.find("</VirtualHost>").unwrap());
            assert!(conf.ends_with("</VirtualHost>\n"));
            // https 가상호스트도 같은 제한을 갖는다
            let paths = CertPaths { cert: "/c.pem".into(), key: "/k.pem".into() };
            assert!(tls_vhost(&conf, &paths).contains("Require local"));
        }
    }

    #[test]
    fn python_vhost_unchanged() {
        let conf = build_vhost_conf(&project(ProjectType::Python, "/srv/x", "", 5001));
        assert!(conf.contains("ProxyPass / http://127.0.0.1:5001/"));
        assert!(!conf.contains("ws://"));
    }
}
