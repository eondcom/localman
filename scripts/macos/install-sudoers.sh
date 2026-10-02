#!/bin/bash
# localman sudoers 규칙 설치 (macOS, 한 번만 실행): sudo ./scripts/macos/install-sudoers.sh
#
# 맥에서 root가 필요한 건 80포트로 httpd를 띄우는 것, /etc/hosts 수정, root 소유 로그뿐이다.
# vhost 파일은 Homebrew prefix(사용자 소유) 아래에 쓰므로 규칙이 필요 없다.
set -e
TARGET_USER=${SUDO_USER:-$(whoami)}
if [ -x /opt/homebrew/bin/brew ]; then PREFIX=/opt/homebrew; else PREFIX=/usr/local; fi
RULES=/tmp/localman-sudoers

cat > "$RULES" << RULES_EOF
# localman: 비밀번호 없이 Homebrew httpd 제어
$TARGET_USER ALL=(root) NOPASSWD: $PREFIX/bin/apachectl start
$TARGET_USER ALL=(root) NOPASSWD: $PREFIX/bin/apachectl stop
$TARGET_USER ALL=(root) NOPASSWD: $PREFIX/bin/apachectl graceful
$TARGET_USER ALL=(root) NOPASSWD: /bin/cp /tmp/localman_hosts /etc/hosts
# 로그 비우기/읽기 (프로젝트별 에러 로그)
$TARGET_USER ALL=(root) NOPASSWD: /bin/cp /dev/null $PREFIX/var/log/httpd/*
$TARGET_USER ALL=(root) NOPASSWD: /usr/bin/tail -n * $PREFIX/var/log/httpd/*
RULES_EOF

visudo -cf "$RULES" && install -m 440 "$RULES" /etc/sudoers.d/localman
echo "sudoers 규칙 설치 완료 ($TARGET_USER): /etc/sudoers.d/localman"
