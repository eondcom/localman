#!/bin/bash
# localman sudoers 규칙 설치 (한 번만 실행)
set -e
RULES=/tmp/localman-sudoers
cat > "$RULES" << 'EOF'
# localman: 비밀번호 없이 Apache/MariaDB 제어
dell ALL=(root) NOPASSWD: /usr/bin/systemctl start apache2
dell ALL=(root) NOPASSWD: /usr/bin/systemctl stop apache2
dell ALL=(root) NOPASSWD: /usr/bin/systemctl reload apache2
dell ALL=(root) NOPASSWD: /usr/bin/systemctl start mariadb
dell ALL=(root) NOPASSWD: /usr/bin/systemctl stop mariadb
dell ALL=(root) NOPASSWD: /usr/bin/a2enmod *
dell ALL=(root) NOPASSWD: /usr/bin/cp /tmp/localman_vhost_* /etc/apache2/sites-available/*
dell ALL=(root) NOPASSWD: /usr/bin/cp /tmp/localman_hosts /etc/hosts
dell ALL=(root) NOPASSWD: /usr/bin/ln -sf /etc/apache2/sites-available/* /etc/apache2/sites-enabled/*
dell ALL=(root) NOPASSWD: /usr/bin/rm -f /etc/apache2/sites-available/* /etc/apache2/sites-enabled/*
EOF
visudo -cf "$RULES" && sudo install -m 440 "$RULES" /etc/sudoers.d/localman
echo "sudoers 규칙 설치 완료: /etc/sudoers.d/localman"
