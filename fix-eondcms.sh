#!/bin/bash
# eondcms vhost 복구 + proxy 모듈 활성화
set -e
echo "1. proxy_http 모듈 활성화..."
sudo a2enmod proxy proxy_http

echo "2. eondcms vhost 생성..."
sudo cp /tmp/localman_vhost_eondcms.conf /etc/apache2/sites-available/eondcms.conf
sudo ln -sf /etc/apache2/sites-available/eondcms.conf /etc/apache2/sites-enabled/eondcms.conf

echo "3. Apache reload..."
sudo systemctl reload apache2

echo "완료! eondcms.localhost → 127.0.0.1:5001 프록시 설정됨"
echo ""
echo "실행 명령어: cd ~/IdeaProjects/eondcms && python run_server.py 5001"
