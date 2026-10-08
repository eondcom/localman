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

### 리눅스 (데비안/우분투)

- Apache2 (`sudo apt install apache2`)
  - Next.js의 HMR에는 `proxy_wstunnel`·`rewrite` 모듈이 필요하다 (없으면 프로젝트 추가 시 자동 활성화)
- MariaDB (`sudo apt install mariadb-server`)
- PostgreSQL (`sudo apt install postgresql postgresql-client`)
- Python3, Node.js/npm
- sudoers 설정 (Apache 제어용)

```bash
# install-sudoers.sh 실행으로 설정
sudo ./scripts/linux/install-sudoers.sh
```

### 맥

- [Homebrew](https://brew.sh) — httpd·PostgreSQL은 서비스 탭의 [설치하기]로 `brew install` 된다
- DB 서버(MariaDB 자리): Homebrew가 Intel 맥용 MariaDB를 더 이상 미리 빌드해 주지 않아(Xcode 없이는 빌드 불가),
  [설치하기]는 **MySQL 8.4 LTS 공식 바이너리**를 `<데이터 폴더>/tools/mysql`에 설치한다 (sudo 불필요, 3306·`/tmp/mysql.sock`,
  root 비밀번호 `root`). 서비스 탭에는 "MySQL"로 보인다. Homebrew MariaDB가 이미 있으면 그것을 쓴다
- 개발 도구의 PHP는 Apache 모듈까지 본다 — MAMP 등의 php 명령만 있고 Homebrew `libphp`가 없으면 "Apache 연결 안 됨"
- Apache는 Homebrew `httpd`를 80포트로 띄운다. vhost는 `$(brew --prefix)/etc/httpd/localman/*.conf`에 생기고,
  `httpd.conf` 끝에 localman 관리 블록(Listen 80, 실행 사용자, proxy·rewrite 모듈, mod_php, Include)이 들어간다
- PHP 프로젝트는 `brew install php`가 있으면 mod_php로 연결된다
- sudoers 설정 (80포트 httpd 제어, /etc/hosts, 로그):

```bash
sudo ./scripts/macos/install-sudoers.sh
```

## 개발 도구 설치

서비스 탭 아래 [개발 도구]에서 버전을 보고, 없거나 오래됐으면 설치한다.

- Node.js: nodejs.org 배포 목록에서 **그때의 LTS**를 골라 공식 SHA-256으로 검증한 뒤 `<데이터 폴더>/tools/node`에 푼다.
  sudo가 필요 없고 맥·리눅스가 같으며, 앱이 시작할 때 PATH 맨 앞에 둔다. 깔린 Node는 공식 릴리스 일정
  (nodejs/Release `schedule.json`)으로 Active LTS · 유지보수 LTS(종료 시기) · LTS 아님 · 지원 종료를 구분해 보여준다
- Python 3, PHP, rsync: 맥은 Homebrew, 리눅스는 apt (`install-sudoers.sh`에 고정된 패키지만 허용). PHP를 깔면 Apache에 mod_php를 연결한다

## 사이트 찾기·용량

- 프로젝트 탭 위 검색창에서 도메인·이름·ID로 걸러낸다
- [용량 계산]: 사이트마다 파일(그중 node_modules·venv 등 의존성 몫)·DB·합계, 맨 위에 전체 합계. 앱을 켤 때 한 번 잰다
- 사이트의 DB는 수정 화면에서 지정한다. 비우면 라이믹스 `files/config/config.php`, `.env`(DB_DATABASE·DATABASE_URL)에서
  자동으로 찾는다 (비밀번호는 읽지 않는다)

## 서버 연결 — 가져오기·배포 (SSH·SFTP)

프로젝트 ⋯ → [서버 배포·가져오기]. 서버(주소·포트·사용자·웹 경로, 비밀번호 또는 SSH 키)와 서버 DB(호스트·포트·이름·사용자·비밀번호)를 넣는다.
호스팅 DB가 외부 접속을 막으면 **[SSH로 서버에 들어가서 접속]**을 켠다 — 서버 안에서 `mysqldump`/`mysql`을 돌린다.

**서버에서 가져오기 (↓)** — 실서버를 로컬로 받아 테스트할 때

- [받을 파일 보기] / [파일 가져오기]: 서버 웹 경로를 프로젝트 폴더로 rsync (서버에 rsync가 없으면 tar로 전부).
  **로컬에만 있는 파일은 지우지 않고**, 로컬 설정 파일(`files/config/`, `.env`, `wp-config.php` …)이 이미 있으면 덮지 않는다
- [DB 가져오기]: 서버 DB를 받아(`<데이터 폴더>/pull-backups/`에 보관) 고른 로컬 DB에 넣는다. 같은 이름의 로컬 DB는 먼저 백업한다.
  사이트에 DB를 연결하고, 사이트 설정에 적힌 DB 계정도 만든다
- [모두 가져오기]: 파일 → DB 순서로 한 번에

**서버로 올리기 (↑)**

- [연결 시험]: SSH·웹 경로·서버 rsync·서버 DB 접속 확인
- [미리보기]: 올라갈 파일 목록만 본다 (`rsync --dry-run`)
- [파일 올리기]: 바뀐 파일만 rsync로 올린다. **서버에만 있는 파일은 지우지 않는다**. `.git/`, `node_modules/`, `.env` 등은
  기본으로 빼며(라이믹스는 `files/config/`·`files/cache/` 등도) 목록은 고칠 수 있다
- [DB 올리기]: 확인을 한 번 더 받은 뒤, **서버 DB를 이 PC의 `<데이터 폴더>/deploy-backups/`에 먼저 백업**하고 로컬 DB로 덮어쓴다.
  백업이 실패하면 덮어쓰지 않는다

SSH 비밀번호는 `SSH_ASKPASS`로 넘겨 명령줄에 남기지 않는다. 키 로그인이면 `ssh-copy-id -p 포트 사용자@서버`로 한 번 등록한다.
접속 정보는 `deploy.json`(권한 600)에 저장되고 백업 묶음에는 담기지 않는다. 기록은 ⋯ 메뉴의 이전 기록에 "↓ 가져옴"·"↑ 배포"로 남는다.

## 자동화 — MCP·HTTP API

설정 → 자동화. AI나 스크립트가 사이트 목록, 서버 켜기·끄기, 프로젝트 추가, DB 목록·만들기·백업, 사이트 DB 계정 만들기,
서버에서 가져오기를 할 수 있다. **DB·프로젝트 삭제와 서버로 올리기는 열지 않는다.**

```bash
# Claude Code 에 MCP 서버로 등록 (설정 화면의 [Claude Code 명령 복사]와 같다)
claude mcp add localman -- /Applications/LocalMan.app/Contents/MacOS/LocalMan --mcp

# HTTP API — 설정에서 켜면 앱이 떠 있는 동안 127.0.0.1:47810 에서만 듣는다 (토큰 필요)
curl -H "Authorization: Bearer <토큰>" http://127.0.0.1:47810/api/tools
curl -H "Authorization: Bearer <토큰>" -X POST http://127.0.0.1:47810/api/tools/pull_from_server \
     -d '{"id":"eond","files":true,"db":true}'
```

## HTTPS (로컬 인증기관)

서비스 탭의 [HTTPS 켜기]로 모든 프로젝트를 `https://<도메인>`으로도 연다 (http도 그대로 열려 있다).

- Let's Encrypt는 `*.localhost`에 발급하지 않으므로 mkcert와 같은 방식을 쓴다: localman 전용 인증기관을
  만들어 OS에 한 번 신뢰시키고, 도메인마다 인증서(1년)를 발급한다. 만료 30일 전이면 앱 시작 때와
  6시간마다 자동으로 다시 발급하고 Apache를 reload한다
- 인증기관은 이름 제약으로 `localhost`와 `*.localhost`에만 발급할 수 있다 — 키가 새도 다른 사이트를 위조할 수 없다
- 인증기관 키는 이 PC에만 있다(`<데이터 폴더>/tls/`, 권한 600). 백업 묶음에도 담지 않는다
- 신뢰 등록: 맥은 로그인 키체인(암호 확인 창이 뜬다), 리눅스는 시스템 저장소 + 크롬·파이어폭스 NSS DB
  (`libnss3-tools`의 `certutil` 필요)
- 앱 안에서는 `X-Forwarded-Proto: https`가 붙어 프록시 뒤 앱도 https로 접속된 걸 안다

## 백업·이전 (리눅스 ↔ 맥)

### 같은 네트워크로 바로 보내기

1. 받는 PC: [백업·이전] → [받기 대기]. 6자리 코드가 나온다
2. 보내는 PC: [받는 PC 찾기] → PC 선택(못 찾으면 IP 입력) → 코드 입력 → [보내기]
   - 보낼 것: 전체 또는 프로젝트 하나. 프로젝트 목록의 더보기(⋯) → [다른 PC로 보내기]로도 시작한다
   - 함께 옮길 DB와 DB 접속 정보는 [함께 옮길 데이터베이스]에서 고른다

- rsync처럼 크기·수정 시각을 비교해 바뀐 파일만 보낸다. 받는 쪽에만 있는 파일은 지우지 않는다
- `node_modules`, `venv`, `.venv`, `__pycache__`, `.next` 등 의존성·빌드 폴더는 보내지 않는다 → 받은 PC에서 패키지 설치
- 받는 쪽 파일이 더 최신이면 기본으로 건너뛰고 결과에 알린다 (덮어쓰기를 켜면 보내는 쪽 기준)
- 받는 PC는 자기 홈 폴더 안에만 쓴다. 보내는 쪽 홈 밖의 프로젝트는 `~/localman-received/<id>`로 받는다
- 설정·DB는 아래 백업 파일과 같은 형식으로 함께 보내고, 받는 PC가 같은 가져오기 규칙으로 적용한다
- 보안: 코드로 SPAKE2 키 합의 후 ChaCha20-Poly1305로 암호화. 엿본 트래픽으로 코드를 대입해 풀 수 없고,
  받는 쪽은 틀린 코드 3번이면 멈춘다. 코드는 한 번만 쓴다
- 포트: 발견 UDP 47800, 전송 TCP 47801. 맥은 처음 받을 때 방화벽이 수신 연결 허용을 물을 수 있다
- 보내고 받은 기록은 [최근 이전 기록]과 프로젝트 줄("↗ 10/03 14:20 맥북(으)로 보냄"), 더보기 메뉴에 남는다

### MAMP에서 옮기기 (맥)

MAMP가 있으면 [백업·이전]에 나타난다. MAMP의 가상호스트(폴더가 남은 것)와 MySQL DB를 골라
**로컬맨 백업 파일과 같은 형식**으로 만든다 → [이 PC로 바로 가져오기] 또는 리눅스로 옮겨 가져오기.

- DB 크기는 MAMP 데이터 폴더에서 바로 잰다 (MySQL을 켜지 않는다)
- MAMP MySQL이 꺼져 있으면 덤프하는 동안만 포트 없이 소켓으로 띄웠다가 다시 내린다 (로컬맨 MySQL 3306과 충돌 없음)
- MAMP 폴더는 읽기만 한다. MAMP MySQL 계정 기본값 root/root

### 파일로 옮기기

[백업·이전] 탭에서 한 파일(`localman-backup-<호스트>-<날짜>.tar.gz`)로 내보내고 다른 PC에서 가져온다.

- 담기는 것: 프로젝트 설정(`projects.json`), 고른 DB의 덤프, (선택) DB 접속 정보 — 비밀번호가 평문이므로 파일 관리에 주의
- 담기지 않는 것: 프로젝트 소스 폴더. git 등으로 따로 옮긴다
- 가져올 때 경로 앞부분을 바꾼다 (기본값: 만든 PC의 홈 → 이 PC의 홈, 예 `/home/dell` → `/Users/eond`)
- 이미 있는 프로젝트·DB는 기본으로 건너뛰고, 덮어쓰기를 켜면 바꾼다. 포트가 겹치면 빈 포트로 옮긴다
- 프로젝트마다 이 PC 방식으로 vhost·hosts를 새로 만든다 (Apache가 설치돼 있어야 한다)
- Python venv는 OS 간 호환되지 않는다. 다른 OS에서 온 venv는 지우고 패키지 설치를 다시 한다

## 빌드

```bash
cargo build --release
./target/release/localman
```

또는 개발 실행 (리눅스, kime 입력기 충돌 회피용 X11 강제):

```bash
./scripts/linux/run.sh
```

맥 — Homebrew로 설치 (Intel·Apple Silicon 겸용):

```bash
brew install --cask eondcom/tap/localman
sudo /Applications/LocalMan.app/Contents/Resources/install-sudoers.sh   # 처음 한 번
```

LocalMan은 Apple 미서명 앱이라 처음 실행할 때 우클릭 → **열기**, 또는 시스템 설정 → 개인정보 보호 및 보안에서 허용한다.
화면 언어는 설정 탭에서 한국어·English·日本語(기본: 시스템 언어)로 바꾼다.

소스에서 맥 앱으로 설치 (`/Applications/LocalMan.app`, 아이콘·임시 서명 포함, 다시 실행하면 덮어쓴다):

```bash
./scripts/macos/make-app.sh
```

맥에서 리눅스 대상 타입 검사: `./scripts/check-linux.sh`

## 디자인

맥북 팬 관리(mac-fan-control)와 같은 EOND UI App 0.2 체계를 쓴다 — `src/ui/theme.rs`.

- 테두리 대신 밝기로 층을 나눈다 (바탕 → 카드 c1 → 겹침 c2 → 선택 c3). 색은 토큰으로만 쓴다 (HeroUI 팔레트)
- 다크/라이트/시스템 (설정 탭). 시스템이면 맥은 화면 모드, 리눅스는 GNOME color-scheme 을 따른다
- 글꼴 Pretendard, 아이콘 Lucide 를 앱에 넣는다 (`assets/fonts/`, 각 LICENSE 포함).
  iced 가 화면을 직접 그리므로 맥·리눅스가 같은 화면이다 (창 테두리·파일 선택 창만 OS 것)

## 기술 스택

- Rust + [Iced](https://github.com/iced-rs/iced) GUI
- Apache2 vhost 기반 로컬 도메인
- MariaDB/PostgreSQL CLI 래핑
