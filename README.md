# LocalMan

로컬 개발 환경 관리자 — PHP/Python 프로젝트를 Apache vhost + MariaDB로 관리합니다.

## 기능

- **프로젝트 관리**: PHP/Python 프로젝트를 `*.localhost` 도메인으로 등록
  - `/etc/hosts` 자동 수정
  - Apache 가상호스트 자동 생성 (`/etc/apache2/sites-available/`)
  - Python 프로젝트: ProxyPass로 dev server 포트로 포워드
- **Python 환경 자동 설정**: venv 생성, `requirements.txt` / `pyproject.toml` 자동 감지, npm 프론트엔드 빌드 포함
- **서버 제어**: Python dev server 시작/중지 (포트 자동 할당 5001~6000)
- **서비스 제어**: Apache2, MariaDB systemctl 시작/중지
- **데이터베이스 관리**: DB 생성/삭제, 백업/복원, 사용자 관리

## 프로젝트 데이터

프로젝트 목록은 `~/.local/share/localman/projects.json`에 저장됩니다.

```json
[
  {
    "id": "myproject",
    "name": "My Project",
    "path": "/home/user/projects/myproject",
    "domain": "myproject.localhost",
    "project_type": "Python",
    "port": 5001,
    "start_command": "venv/bin/python3 run_server.py 5001"
  }
]
```

다른 도구에서 이 파일을 읽어 포트/상태를 연동할 수 있습니다.

## 요구사항

- Apache2 (`sudo apt install apache2`)
- MariaDB (`sudo apt install mariadb-server`)
- Python3, Node.js/npm
- sudoers 설정 (Apache 제어용)

```bash
# install-sudoers.sh 실행으로 설정
sudo ./install-sudoers.sh
```

## 빌드

```bash
cargo build --release
./target/release/localman
```

또는 개발 실행:

```bash
./run.sh
```

## 기술 스택

- Rust + [Iced](https://github.com/iced-rs/iced) GUI
- Apache2 vhost 기반 로컬 도메인
- MariaDB CLI 래핑
