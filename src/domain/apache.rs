//! Apache vhost 설정 생성. 설정 내용은 OS 무관하고, 파일을 어디에 두고 어떻게
//! 활성화하는지만 platform 쪽에서 OS별로 처리한다.

use super::project::{ProjectType, VhostProject};
use crate::platform;

pub(crate) fn build_vhost_conf(p: &VhostProject) -> String {
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

pub(crate) fn write_vhost(p: &VhostProject) -> Result<(), String> {
    platform::write_site(&p.id, &build_vhost_conf(p))
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
    fn python_vhost_unchanged() {
        let conf = build_vhost_conf(&project(ProjectType::Python, "/srv/x", "", 5001));
        assert!(conf.contains("ProxyPass / http://127.0.0.1:5001/"));
        assert!(!conf.contains("ws://"));
    }
}
