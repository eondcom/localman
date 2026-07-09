#!/bin/bash
# localman sudoers 규칙 설치 (한 번만 실행)
set -e
RULES=/tmp/localman-sudoers

# systemctl / cp / ln / rm / truncate / tail 실제 경로 확인
SYSTEMCTL=$(which systemctl)
CP=$(which cp)
LN=$(which ln)
RM=$(which rm)
TRUNCATE=$(which truncate)
TAIL=$(which tail)

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
# 로그 비우기/읽기 (프로젝트별 에러 로그)
dell ALL=(root) NOPASSWD: $TRUNCATE -s 0 /var/log/apache2/*
dell ALL=(root) NOPASSWD: $TAIL -n * /var/log/apache2/*
EOF

visudo -cf "$RULES" && sudo install -m 440 "$RULES" /etc/sudoers.d/localman
echo "sudoers 규칙 설치 완료: /etc/sudoers.d/localman"
