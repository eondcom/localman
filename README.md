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

- [Homebrew](https://brew.sh) — httpd·MariaDB·PostgreSQL은 서비스 탭의 [설치하기]로 `brew install` 된다
- Apache는 Homebrew `httpd`를 80포트로 띄운다. vhost는 `$(brew --prefix)/etc/httpd/localman/*.conf`에 생기고,
  `httpd.conf` 끝에 localman 관리 블록(Listen 80, 실행 사용자, proxy·rewrite 모듈, mod_php, Include)이 들어간다
- PHP 프로젝트는 `brew install php`가 있으면 mod_php로 연결된다
- sudoers 설정 (80포트 httpd 제어, /etc/hosts, 로그):

```bash
sudo ./scripts/macos/install-sudoers.sh
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
