#!/bin/bash
# localman sudoers 규칙 설치 (한 번만 실행)
set -e
RULES=/tmp/localman-sudoers

# systemctl / cp / ln / rm 실제 경로 확인
SYSTEMCTL=$(which systemctl)
CP=$(which cp)
LN=$(which ln)
RM=$(which rm)

cat > "$RULES" << EOF
# localman: 비밀번호 없이 Apache/MariaDB 제어
dell ALL=(root) NOPASSWD: $SYSTEMCTL start apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL reload apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL start mariadb
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop mariadb
dell ALL=(root) NOPASSWD: /usr/sbin/a2enmod *
dell ALL=(root) NOPASSWD: $CP /tmp/localman_vhost_* /etc/apache2/sites-available/*
dell ALL=(root) NOPASSWD: $CP /tmp/localman_hosts /etc/hosts
dell ALL=(root) NOPASSWD: $LN -sf /etc/apache2/sites-available/* /etc/apache2/sites-enabled/*
dell ALL=(root) NOPASSWD: $RM -f /etc/apache2/sites-available/* /etc/apache2/sites-enabled/*
EOF

visudo -cf "$RULES" && sudo install -m 440 "$RULES" /etc/sudoers.d/localman
echo "sudoers 규칙 설치 완료: /etc/sudoers.d/localman"
