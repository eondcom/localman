# 서비스 "설치하기" 기능 — 구현 스펙

작성: 2026-09-13 / 작성자: Claude(스펙) → 구현: Codex → 검증: 사용자

## 왜

PostgreSQL 카드에서 [시작]을 눌러도 아무 일이 없다. 이 시스템에는
`postgresql-client` 만 깔려 있고 **서버 패키지가 없다.** 그런데 화면은
"중지됨" 으로 보여 준다. 사용자는 멈춘 서비스를 켜려고 계속 시도하게 된다.

목표: 안 깔린 서비스는 **"설치되지 않음"** 으로 보여 주고, 그 카드에서 바로
설치할 수 있게 한다.

## 실측한 사실 (추측 아님 — 2026-09-13 이 기기에서 확인)

| 명령 | apache2 | postgresql(미설치) | 없는 서비스 |
|---|---|---|---|
| `systemctl is-active` stdout | `active` | `inactive` | `inactive` |
| `systemctl is-active` 종료코드 | 0 | **4** | **4** |
| `systemctl cat X.service` 종료코드 | 0 | **1** | 1 |
| `systemctl list-unit-files X.service` | 한 줄 출력 | **빈 출력** | 빈 출력 |

**함정 1 — 종료코드로는 못 가른다.** 미설치 postgresql 과 존재하지 않는
서비스가 둘 다 exit 4 다. 종료코드 4 만 보고 "미설치" 라고 하면 안 된다.
유닛 파일 존재 여부(`systemctl cat`)가 실제 판별 근거다.

**함정 2 — `dpkg -l` 성공 여부로 판단하면 오판한다.**

```
dpkg-query -W -f='${db:Status-Status}' postgresql-16   →  not-installed
```

패키지를 지워도 dpkg 항목은 남는다. 명령은 성공(exit 0)하는데 실제로는
안 깔려 있다. 반드시 `${db:Status-Status}` 값이 `installed` 인지 봐야 한다.

**함정 3 — 현재 코드는 종료코드를 아예 안 본다.** `service.rs:18` 의 match 는
stdout 만 보고 `_ => Stopped` 로 떨어뜨린다. 이게 지금 증상의 직접 원인이다.

## 1. `ServiceStatus` 에 변이 추가

`src/system/service.rs`

```rust
pub enum ServiceStatus {
    Running,
    Stopped,
    NotInstalled,   // 새로 추가
    Unknown,
}
```

`NotInstalled` 는 **유닛 파일이 없을 때만** 준다:

```rust
/// 유닛 파일이 존재하나. 설치 여부의 근거는 이것이다.
fn unit_exists(name: &str) -> bool {
    Command::new("systemctl")
        .args(["cat", &format!("{name}.service")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)   // systemctl 자체가 없으면 판단 보류 → 아래에서 Unknown
}
```

`get_service_status` 순서:

1. `systemctl is-active` 가 `active` → `Running` (유닛 검사 불필요, 가장 흔한 경로)
2. `systemctl` 실행 자체가 실패 → `Unknown`
3. `unit_exists()` 가 false → `NotInstalled`
4. 그 외 → `Stopped`

`activating` / `reloading` 도 `is-active` 가 낼 수 있다. 지금은 `Stopped` 로
떨어지는데 그대로 둔다 — 새로고침하면 곧 `active` 가 된다. 범위 밖이다.

## 2. 설치 대상 패키지를 서비스별로 고정한다

`src/system/service.rs`

```rust
/// 서비스마다 설치할 패키지. 임의 문자열을 apt 에 넘기지 않기 위해
/// **여기 적힌 것만** 설치할 수 있다.
pub fn install_package_for(service: &str) -> Option<&'static str> {
    match service {
        "postgresql" => Some("postgresql"),
        "mariadb"    => Some("mariadb-server"),
        "apache2"    => Some("apache2"),
        _ => None,
    }
}
```

`postgresql` 메타패키지를 쓴다(apt 후보 `16+257build1.1` 확인). 버전 번호를
박지 않는다 — OS 를 올리면 깨진다.

## 3. 설치 실행

```rust
pub fn install_service(service: &str) -> Result<(), String> {
    let pkg = install_package_for(service)
        .ok_or_else(|| format!("설치할 수 있는 서비스가 아닙니다: {service}"))?;

    let out = Command::new("sudo")
        .args(["-n", "/usr/bin/apt-get", "install", "-y", pkg])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .map_err(|e| e.to_string())?;

    if out.status.success() { return Ok(()); }

    let err = String::from_utf8_lossy(&out.stderr);
    // sudo -n 은 비밀번호가 필요하면 여기로 떨어진다. 원인을 알려 준다.
    if err.contains("password is required") || err.contains("a password is required") {
        return Err("설치 권한이 없습니다. install-sudoers.sh 를 다시 실행하십시오.".into());
    }
    Err(err.to_string())
}
```

`DEBIAN_FRONTEND=noninteractive` 가 없으면 apt 가 대화형 프롬프트에서 멈춰
GUI 가 영원히 응답을 기다린다. 반드시 넣는다.

## 4. sudoers 규칙 — 범위를 좁힌다

`install-sudoers.sh` 에 추가한다.

```bash
APTGET=$(which apt-get)
...
# 설치는 아래 세 패키지에만 허용한다. 와일드카드를 쓰지 않는다.
dell ALL=(root) NOPASSWD: $APTGET install -y postgresql
dell ALL=(root) NOPASSWD: $APTGET install -y mariadb-server
dell ALL=(root) NOPASSWD: $APTGET install -y apache2
dell ALL=(root) NOPASSWD: $SYSTEMCTL start postgresql
dell ALL=(root) NOPASSWD: $SYSTEMCTL stop postgresql
```

**`apt-get install *` 로 쓰지 말 것.** 그렇게 하면 임의 패키지 설치가 열리고,
이는 사실상 무제한 root 권한이다(악성 패키지의 postinst 스크립트는 root 로 돈다).
sudoers 는 인자를 문자열로 정확히 대조하므로, 위처럼 적으면 그 세 줄 외의
어떤 apt 호출도 비밀번호를 요구한다.

기존 규칙에 postgresql start/stop 이 **아예 없다.** 설치해도 [시작]이 안 될
것이므로 같이 넣는다.

## 5. 화면

`src/ui/services.rs`

- `service_card` 의 match 에 `NotInstalled => ("설치되지 않음", 회색빛 주황
  `Color::from_rgb(0.85, 0.6, 0.2)`)` 추가.
- 그 상태에서는 [시작] 대신 **[설치하기]** 를 낸다 → `ServicesMessage::Install(sid)`.
- 설치는 오래 걸린다(수십 초). 누른 뒤에는 단추를 `on_press` 없이 비활성으로
  두고 라벨을 "설치 중…" 으로 바꾼다. `ServicesState` 에 `installing: Option<String>`
  (설치 중인 서비스 id) 를 둔다.
- `ServicesMessage::Installed(name, Result)` 를 받으면 `installing = None`,
  성공이면 `refresh()`, 실패면 `error` 에 넣는다.
- 설치 직후 상태는 apt 가 서비스를 자동 기동하므로 대개 `Running` 이 된다.
  자동 기동을 가정하지 말고 그냥 `refresh()` 결과를 보여 준다.

`Task::perform` 안에서 `install_service` 를 호출한다 — 기존 `Toggle` 과 같은 모양이다.
**동기로 호출하면 설치 내내 창이 얼어붙는다.**

## 6. 검증 (구현자가 반드시 할 것)

컴파일 통과는 검증이 아니다. 아래를 실제로 돌린 증거를 남긴다.

1. `cargo build` 통과.
2. **설치 전** 실행 → PostgreSQL 카드가 "설치되지 않음" + [설치하기] 로 보이는지
   스크린샷. Apache/MariaDB 는 "실행 중" 그대로인지 같이 확인(회귀 없음).
3. `install-sudoers.sh` 재실행 후 [설치하기] 클릭 → 설치되고 카드가 바뀌는지.
4. 설치 뒤 `psql -h 127.0.0.1 -U postgres -c 'select 1'` 로 실제 접속 확인.
5. sudoers 범위 확인: `sudo -n apt-get install -y cowsay` 가 **비밀번호를 요구하며
   거부되는지**. 통과해 버리면 규칙이 너무 넓은 것이다.

## 범위 밖

- PostgreSQL 초기 계정/비밀번호 설정. 설치 직후 `postgres` 역할은 peer 인증만
  된다. 이건 별도 작업이다.
- 패키지 제거 기능. 넣지 않는다.
