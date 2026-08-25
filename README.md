# LocalMan

로컬 개발 환경 관리자 — PHP/Python/Next.js 프로젝트를 Apache vhost + MariaDB/PostgreSQL로 관리합니다.

## 기능

- **프로젝트 관리**: PHP/Python/Next.js 프로젝트를 `*.localhost` 도메인으로 등록
  - `/etc/hosts` 자동 수정
  - Apache 가상호스트 자동 생성 (`/etc/apache2/sites-available/`)
  - PHP: DocumentRoot 직접 서빙 / Python·Next.js: ProxyPass로 dev server 포트로 포워드
- **하위 디렉토리 지원**: 앱이 저장소 루트가 아니라 하위 폴더에 있어도 등록 가능
  (예: `easyhost/app`). PHP는 이 값이 DocumentRoot가 되므로 Laravel의 `public` 등에 쓸 수 있다
- **Python 환경 자동 설정**: venv 생성, `requirements.txt` / `pyproject.toml` 자동 감지, npm 프론트엔드 빌드 포함
- **Next.js 지원**: 폴더를 고르면 Next.js 앱 위치를 자동 감지해 타입·하위 디렉토리·실행 명령을 채운다
  - lockfile을 보고 pnpm/yarn/bun/npm 중 맞는 것으로 의존성 설치 (매니저가 없으면 corepack 경유)
  - HMR(웹소켓)까지 프록시하므로 개발 중 새로고침 없이 반영된다
- **서버 제어**: dev server 시작/중지 (포트 자동 할당 5001~6000)
- **서비스 제어**: Apache2, MariaDB, PostgreSQL systemctl 시작/중지
- **데이터베이스 관리**: MariaDB/PostgreSQL DB 생성/삭제, 백업/복원, 사용자 관리

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
  },
  {
    "id": "easyhost",
    "name": "easyhost",
    "path": "/home/user/dev/easyhost",
    "domain": "easyhost.localhost",
    "project_type": "NextJs",
    "port": 5002,
    "start_command": "npx next dev --port 5002",
    "app_dir": "app"
  }
]
```

- `project_type`: `Php` | `Python` | `NextJs`
- `app_dir`: 실행·DocumentRoot 기준이 되는 하위 디렉토리 (없으면 `path` 그대로)
- `start_command`는 셸을 거치지 않고 그대로 실행된다 → `&&`, 파이프, `FOO=1 cmd` 같은 셸 문법은 쓸 수 없다
- Next.js는 저장 시 명령어의 `--port` 값이 할당된 포트에 자동으로 맞춰진다

다른 도구에서 이 파일을 읽어 포트/상태를 연동할 수 있습니다.

## 요구사항

- Apache2 (`sudo apt install apache2`)
  - Next.js의 HMR에는 `proxy_wstunnel`·`rewrite` 모듈이 필요하다 (없으면 프로젝트 추가 시 자동 활성화)
- MariaDB (`sudo apt install mariadb-server`)
- PostgreSQL (`sudo apt install postgresql postgresql-client`)
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
- MariaDB/PostgreSQL CLI 래핑
