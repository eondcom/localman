# macOS 지원 — 플랜

작성: 2026-10-02 / 작성자: Claude(플랜) → 구현: Codex(단계별 스펙 확정 후) → 검증: 사용자(Mac 실기)

## 왜

LocalMan 은 지금 **Ubuntu/Debian 전용**이다. 같은 프로젝트(PHP·Python·Next.js)를
Mac 에서도 `*.localhost` 로 띄우고 DB 를 관리하고 싶다.

GUI(iced·rfd·NanumGothic)는 그대로 Mac 에서 돈다. 깨지는 곳은 **시스템 연동 계층
(`src/system/`)과 `main.rs` 일부**뿐이다. UI 는 거의 손대지 않는다.

## 결론 먼저

1. **Mac 스택은 Homebrew 로 통일한다.** httpd / php / mariadb / postgresql@16 /
   node / python 을 전부 brew 로 받는다. Mac 기본 Apache(`/usr/sbin/httpd`)는 쓰지 않는다
   — SIP 영역이라 설정 파일 수정이 불편하고, macOS 업데이트 때 설정이 초기화되며,
   PHP 모듈이 빠져 있다.
2. **플랫폼 차이는 `Platform` 계층 하나로 모은다.** 경로·명령을 코드 곳곳에서
   `#[cfg(target_os)]` 로 가르지 말고, `src/system/platform/{linux,macos}.rs` 에
   몰아넣는다. 나머지 코드는 `platform::` 함수만 부른다.
3. **Mac 은 sudo 를 거의 없앨 수 있다.** brew 프리픽스(`/opt/homebrew`)는 사용자 소유라서
   vhost 파일 쓰기·모듈 활성화·로그 읽기/비우기·서비스 시작/중지가 전부 일반 권한이다.
   root 가 필요한 건 `/etc/hosts` 수정 하나뿐이다.
4. **빌드는 Mac 에서 해야 한다.** Linux 에서 Mac 바이너리를 크로스 컴파일하는 건
   SDK 라이선스·링커 문제로 비현실적이다. 실기 Mac 이 있으면 거기서 `cargo build`,
   배포까지 하려면 GitHub Actions `macos-latest` 러너로 `.app`/`.dmg` 를 만든다.

## 실측한 Linux 의존 지점 (2026-10-02 코드 기준)

| # | 위치 | Linux 전용인 것 | Mac 대응 |
|---|---|---|---|
| 1 | `main.rs:44` `acquire_single_instance` | `std::os::linux::net::SocketAddrExt` abstract socket — **Mac 에선 컴파일 자체가 안 된다** | 데이터 디렉토리의 lock 파일에 `flock`(양쪽 공통) |
| 2 | `main.rs:29` `application_id` | `PlatformSpecific` 필드가 OS 마다 다름 → **컴파일 에러** | `#[cfg(target_os = "linux")]` 로 감싼다 |
| 3 | `vhost.rs:69` `proc_stat_fields` | `/proc/<pid>/stat` 에서 pgrp·starttime | `ps -o pgid=,lstart= -p PID` 또는 `libc::proc_pidinfo`. starttime 은 "PID 재사용 판별용 비교 키"로만 쓰므로 형식이 달라도 된다 |
| 4 | `vhost.rs:202` `process_alive` | `/proc` 존재 + state != Z | `kill(pid, 0)` + `ps -o stat=` 로 좀비 판별 |
| 5 | `vhost.rs:100` `signal_group` | `kill -- -PGID` 에러문구 "No such process" 매칭 | Mac `kill` 도 문법 동일. 에러문구는 다를 수 있음 → `libc::killpg` 로 바꿔 errno(ESRCH) 로 판별하면 양쪽 공통 |
| 6 | `vhost.rs:658` `.process_group(0)` | — | Unix 공통, 그대로 동작 |
| 7 | `vhost.rs:847` `open_url` | `xdg-open` | `open` |
| 8 | `vhost.rs:1070` `write_vhost` | `/etc/apache2/sites-{available,enabled}` + sudo cp/ln + `systemctl reload apache2` | `$(brew --prefix)/etc/httpd/localman/<id>.conf` 에 직접 쓰기 + `httpd.conf` 에 `Include .../localman/*.conf` 한 줄 + `apachectl -k graceful` (sudo 없음) |
| 9 | `vhost.rs:1022` `build_vhost_conf` | `${APACHE_LOG_DIR}` — Debian `envvars` 가 정의하는 변수. **Mac httpd 엔 없다 → 설정 로드 실패** | 플랫폼별 절대 경로를 넣는다 (`$(brew --prefix)/var/log/httpd`) |
| 10 | `vhost.rs:962,988` `apache2ctl`, `a2enmod` | Debian 래퍼 | `apachectl`. `a2enmod` 는 없음 → `httpd.conf` 의 `#LoadModule proxy_module …` 줄 주석 해제 (proxy, proxy_http, proxy_wstunnel, rewrite) |
| 11 | `log.rs:5` | `/var/log/apache2` | `$(brew --prefix)/var/log/httpd` (사용자 소유라 sudo tail/truncate 폴백 불필요) |
| 12 | `service.rs` 전체 | `systemctl is-active/cat/start/stop`, `apt-get install` | `brew services list --json` / `brew services start|stop X` / `brew install X`. 서비스명 매핑: apache2→`httpd`, mariadb→`mariadb`, postgresql→`postgresql@16` |
| 13 | `vhost.rs:1126` `/etc/hosts` 수정 | sudo cp | 동일하게 sudo 필요(Mac 도 root 소유). 아래 "권한" 참고 |
| 14 | `database.rs` 복원 파이프라인의 `sed -E` | GNU sed | BSD sed 도 `-E`·개행 구분 스크립트는 OK. **단 BSD sed 는 UTF-8 이 아닌 바이트에서 `illegal byte sequence` 로 죽는다** → `LC_ALL=C` 환경변수 필수 |
| 15 | `install-sudoers.sh`, `run.sh` | apt·systemctl 경로, `WINIT_UNIX_BACKEND` | Mac 용은 `install-sudoers-macos.sh`(hosts 한 줄만). `run.sh` 의 env 는 Mac 에서 무해하지만 의미 없음 |
| 16 | `dirs::data_dir()` | `~/.local/share/localman` | Mac 은 `~/Library/Application Support/localman` 으로 자동. 코드 변경 불필요, README 만 갱신 |

PHP 프로젝트 vhost 는 Debian 에선 `libapache2-mod-php` 가 자동으로 붙지만,
**brew httpd 는 PHP 를 모른다.** 두 방법 중 하나를 고른다:

- (권장) **php-fpm + `SetHandler "proxy:fcgi://127.0.0.1:9000"`** — `brew services start php`
  가 fpm 을 9000 포트로 띄운다. mod_php 보다 버전 교체가 쉽고 brew 가 계속 지원한다.
- mod_php: `LoadModule php_module $(brew --prefix)/opt/php/lib/httpd/modules/libphp.so`.
  설정이 단순하지만 PHP 버전을 바꿀 때마다 경로를 고쳐야 한다.

## Mac 쪽 함정 (구현 전에 알아둘 것)

**함정 1 — Finder/Dock 에서 띄운 `.app` 은 PATH 가 `/usr/bin:/bin:/usr/sbin:/sbin` 뿐이다.**
`/opt/homebrew/bin` 이 없으므로 `mysql`, `psql`, `brew`, `node`, `npx` 가 전부
"실행 실패: No such file" 로 떨어진다. 터미널에서 `cargo run` 할 때는 멀쩡해서
놓치기 쉽다. → 시작 시 `PATH` 앞에 `/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin`
을 붙인다(Intel Mac 은 `/usr/local`). dev server 자식 프로세스도 이 PATH 를 물려받는다.

**함정 2 — Apple Silicon 과 Intel 의 brew 프리픽스가 다르다.** `/opt/homebrew` vs
`/usr/local`. 하드코딩하지 말고 시작 시 `brew --prefix` 를 한 번 실행해 캐시한다.

**함정 3 — 80 포트.** Mojave(10.14) 이후 macOS 는 **0.0.0.0 에 바인딩하면** root 가 아니어도
1024 미만 포트를 열 수 있다. 그래서 `brew services start httpd`(사용자 권한)로 `Listen 80`
이 가능해야 한다. **이건 Mac 실기에서 1단계에 반드시 확인한다.** 안 되면 대안은
`Listen 8080` + pf 리다이렉트, 또는 `sudo brew services start httpd`(root 실행 — 비권장,
이후 brew 파일 소유권이 꼬인다).
brew httpd 기본값은 `Listen 8080` 이므로 `httpd.conf` 에서 80 으로 바꿔야 한다.

**함정 4 — `*.localhost` 해석.** Chrome 은 `.localhost` 를 자체적으로 127.0.0.1 로 풀지만
Safari·curl·Python 은 시스템 리졸버를 쓴다. 지금처럼 `/etc/hosts` 에 쓰는 방식을
유지한다(양쪽 동작이 같아진다). `/etc/hosts` 수정 후 `dscacheutil -flushcache` 를 같이
실행해야 즉시 반영된다.

**함정 5 — MariaDB 인증.** brew mariadb 는 설치한 Mac 사용자 계정과 `root` 를
unix_socket 인증으로 만든다. 즉 `mysql -u root` 는 Mac 사용자명이 root 가 아니면 실패하고,
`mysql -u $(whoami)` 는 비번 없이 된다. 첫 연결 화면의 기본 사용자를 Mac 에선 `whoami`
로 채워 준다.

**함정 6 — PostgreSQL.** brew postgresql 은 `postgres` 롤이 없고 Mac 사용자명 롤이
슈퍼유저다. 코드는 `-h 127.0.0.1 -U <user>` 로 TCP 접속하므로 brew 기본 `pg_hba.conf`
(127.0.0.1 trust)면 그대로 된다. 기본 사용자만 `whoami` 로.

**함정 7 — Gatekeeper.** 서명 안 된 `.app` 을 다른 Mac 에 옮기면 "손상되었습니다" 로
막힌다. 본인 Mac 에서 직접 빌드해 쓰면 문제없다. 배포하려면 Developer ID 서명 +
공증(notarization)이 필요하고, 이때 인증서·암호를 **GitHub repo secret** 으로 넣는다
(`MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID`,
`APPLE_APP_PASSWORD`). 코드·워크플로우 파일에 직접 쓰지 않는다.

## 권한 (sudo)

| 작업 | Linux | Mac |
|---|---|---|
| vhost 쓰기·활성화 | sudo cp/ln | 일반 권한 (brew 프리픽스) |
| 웹서버 reload | sudo systemctl | `apachectl -k graceful` (일반) |
| 서비스 시작/중지 | sudo systemctl | `brew services` (일반) |
| 패키지 설치 | sudo apt-get | `brew install` (일반, **sudo 로 돌리면 안 됨**) |
| 로그 읽기/비우기 | sudo 폴백 | 일반 |
| `/etc/hosts` | sudo cp | **sudo cp + dscacheutil — 유일하게 root 필요** |

Mac 의 `/etc/hosts` 는 `install-sudoers-macos.sh` 로 아래 두 줄만 NOPASSWD 등록한다
(macOS `/etc/sudoers` 는 기본으로 `#includedir /private/etc/sudoers.d` 를 포함한다 —
실기 확인 항목):

```
<user> ALL=(root) NOPASSWD: /bin/cp /tmp/localman_hosts /etc/hosts
<user> ALL=(root) NOPASSWD: /usr/bin/dscacheutil -flushcache
```

사용자명은 지금 Linux 스크립트처럼 `dell` 하드코딩하지 말고 `$(whoami)`(sudo 로 돌리면
`$SUDO_USER`) 로 채운다. Linux 스크립트도 같이 고친다.

대안: sudoers 대신 `osascript -e 'do shell script "…" with administrator privileges'` 로
그때그때 비번 창을 띄울 수 있다. 등록 빈도가 낮으면 이쪽이 설치 단계가 없어 더 간단하다.
→ **기본은 osascript, sudoers 는 선택**으로 권장.

## 구현 단계

각 단계는 Linux 에서 `cargo test` + 실행 회귀가 깨지지 않아야 다음으로 넘어간다.
Mac 확인이 필요한 단계는 ★ 표시.

### 0단계 — Mac 에서 컴파일만 되게 (반나절)
- `main.rs`: single-instance 를 `flock` 기반으로 교체(양쪽 공통), `application_id` cfg 처리.
- `vhost.rs`: `/proc` 사용 3곳을 `platform::proc_info(pid) -> Option<ProcInfo{pgid, start_key, zombie}>`
  로 추출. Linux 구현은 기존 코드 그대로 이동, Mac 구현은 `ps`.
- `signal_group` → `libc::killpg` (Cargo 에 `libc` 추가).
- `open_url` cfg 분기.
- ★ Mac 에서 `cargo build` 성공, 앱 창이 뜨는지 확인.

### 1단계 — Platform 계층 도입 (Linux 동작 불변 리팩터)
- `src/system/platform/mod.rs` 에 아래를 정의하고 Linux 구현은 **현재 값 그대로** 옮긴다:
  - 경로: `vhost_dir()`, `apache_log_dir()`, `hosts_path()`
  - 서비스: `service_status(Service)`, `service_toggle(Service, bool)`, `service_install(Service)`
    (`Service` = `Web | MariaDb | Postgres` enum — 문자열 "apache2" 를 여기저기 쓰는 걸 없앤다)
  - 웹서버: `enable_modules(&[..])`, `reload_web()`, `configtest()`
  - `write_hosts(content)`, `open_url(url)`
- `build_vhost_conf` 는 로그 디렉토리를 인자로 받는다(`${APACHE_LOG_DIR}` 제거).
  **기존 vhost 테스트(`php_vhost_uses_app_dir_as_document_root` 등)가 그대로 통과해야 한다.**
- 이 단계는 Mac 없이 Linux 에서만 완료·검증 가능 → Codex 에 넘기기 적합.

### 2단계 — macOS 구현 ★
- `platform/macos.rs`: brew 프리픽스 캐시, PATH 보강(함정 1·2), `brew services` 파싱
  (`brew services list --json` 의 `status`: `started`/`stopped`/`none`/`error`),
  미설치 판별은 `brew list --formula <name>` 종료코드.
- `httpd.conf` 1회 셋업 함수 `ensure_httpd_setup()`:
  `Listen 80`, 모듈 4개 주석 해제, `Include …/localman/*.conf`, php-fpm 핸들러.
  수정 전 `httpd.conf.localman-backup` 으로 백업, 이미 적용돼 있으면 건드리지 않음(멱등).
- PHP vhost 템플릿에 Mac 일 때 `<FilesMatch \.php$> SetHandler "proxy:fcgi://127.0.0.1:9000" </FilesMatch>`.
- 복원 파이프라인 `sed` 에 `LC_ALL=C`.
- DB 기본 사용자 `whoami`.

### 3단계 — Mac 실기 검증 ★ (아래 체크리스트)

### 4단계 — 배포 (선택)
- `cargo-bundle` 로 `LocalMan.app` (아이콘 `assets/localman.png` → `.icns` 변환).
- `.github/workflows/macos.yml`: `macos-latest` 에서 빌드 → 서명 → 공증 → dmg 를 Release 에 첨부.
  서명 정보는 repo secret 으로만(함정 7). **본인 Mac 에서만 쓸 거면 이 단계는 불필요.**

## 검증 체크리스트 (Mac 실기)

```
# 사전
brew install httpd php mariadb postgresql@16 node python
brew services start mariadb postgresql@16 php

# 1. 80 포트 (함정 3)
apachectl -t && brew services start httpd && curl -sI http://127.0.0.1/ | head -1

# 2. 앱 기동: Finder 에서 .app 더블클릭으로 띄운 뒤(함정 1)
#    DB 탭 연결 → DB 목록이 뜨는가 (mysql 이 PATH 에서 잡혔는가)

# 3. 프로젝트 3종 등록 후
curl -s -o /dev/null -w '%{http_code}\n' http://phpsample.localhost/     # 200, PHP 실행 결과
curl -s -o /dev/null -w '%{http_code}\n' http://pysample.localhost/      # dev server 시작 후 200
curl -s -o /dev/null -w '%{http_code}\n' http://nextsample.localhost/    # 200, HMR 소켓 연결(브라우저 콘솔)

# 4. 서버 중지 → 포트 해제 확인 (프로세스 그룹 정리)
lsof -nP -iTCP:<port> -sTCP:LISTEN   # 아무것도 없어야 함

# 5. 앱 재시작 후 실행 중 서버 상태가 유지되는가 (starttime 키 대체가 맞는가)
# 6. 중복 실행 → "이미 실행 중" 다이얼로그
# 7. MySQL 8 덤프(utf8mb4_0900_ai_ci, 비UTF-8 바이너리 포함) 복원 (함정 14)
# 8. 프로젝트 삭제 → vhost 파일·/etc/hosts 줄이 지워지는가
```

## 범위 밖

- Windows 지원 (프로세스 그룹·hosts·서비스 모델이 완전히 달라 별도 플랜).
- Valet/Herd/MAMP 등 기존 Mac 도구 연동.
- Mac 기본 Apache(`/usr/sbin/httpd`) 지원.

## 열린 결정 (사용자 확인 필요)

1. **용도** — 본인 Mac 에서만 쓰나, 다른 사람에게도 배포하나? → 4단계(서명·공증·secret) 필요 여부가 갈린다.
2. **Mac 칩** — Apple Silicon 인가 Intel 인가? (둘 다면 universal 빌드)
3. **`/etc/hosts` 권한** — osascript 비번 창(권장) vs sudoers 등록.
