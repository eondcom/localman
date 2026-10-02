#!/bin/bash
# localman sudoers 규칙 설치 (한 번만 실행)
set -e
RULES=/tmp/localman-sudoers

# 배포판별 경로 차이로 sudoers 규칙이 빗나가지 않게 실행 파일 위치를 확인한다.
SYSTEMCTL=$(which systemctl)
APTGET=$(which apt-get)
CP=$(which cp)
LN=$(which ln)
RM=$(which rm)
TRUNCATE=$(which truncate)
TAIL=$(which tail)

cat > "$RULES" << EOF
# localman: 비밀번호 없이 웹·데이터베이스 서비스를 제어
dell ALL=(root) NOPASSWD: $SYSTEMCTL start apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL reload apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL start mariadb
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop mariadb
# 임의 패키지 설치는 postinst를 통한 root 실행으로 이어질 수 있어, 허용 대상만 정확히 제한한다.
dell ALL=(root) NOPASSWD: $APTGET install -y postgresql
dell ALL=(root) NOPASSWD: $APTGET install -y mariadb-server
dell ALL=(root) NOPASSWD: $APTGET install -y apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL start postgresql
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop postgresql
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
