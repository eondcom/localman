# Linux GUI 앱: 중복 실행 방지 & 독바 아이콘 통합 매뉴얼

LocalMan에 적용한 방법을 정리한 문서. Linux 데스크톱 앱(특히 Rust/iced/winit 기반)을 만들 때 공통으로 적용할 수 있다.

## 1. 중복 실행 방지 (Single Instance)

### 원리

Linux의 **abstract unix socket**을 잠금으로 사용한다. 일반 파일 잠금(lock file) 방식과 달리:

- 소켓 이름이 파일시스템에 존재하지 않음 (`@`로 시작하는 추상 네임스페이스)
- 프로세스가 죽으면(강제 종료, 크래시 포함) **커널이 자동으로 해제** → 찌꺼기 잠금 파일 문제가 원천적으로 없음
- 외부 크레이트 불필요, Rust 표준 라이브러리만으로 구현 (`SocketAddrExt::from_abstract_name`, Rust 1.70+)

같은 이름으로 bind를 시도해서 실패하면 = 이미 다른 인스턴스가 실행 중.

### 구현 (Rust)

```rust
/// 중복 실행 방지: abstract unix socket을 잠금으로 사용 (프로세스 종료 시 커널이 자동 해제)
fn acquire_single_instance() -> bool {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let Ok(addr) = SocketAddr::from_abstract_name(b"myapp.single-instance") else {
        return true; // 주소 생성 실패 시 실행은 막지 않음
    };
    match UnixListener::bind_addr(&addr) {
        Ok(listener) => {
            // drop되면 잠금이 풀리므로 프로세스 수명 동안 유지
            std::mem::forget(listener);
            true
        }
        Err(_) => false,
    }
}

fn main() {
    if !acquire_single_instance() {
        rfd::MessageDialog::new()
            .set_title("MyApp")
            .set_description("MyApp이 이미 실행 중입니다.")
            .set_level(rfd::MessageLevel::Info)
            .show();
        return;
    }
    // ... 앱 시작
}
```

- 소켓 이름(`myapp.single-instance`)은 앱마다 고유하게 지정
- 확인 방법: `ss -xl | grep myapp` → `@myapp.single-instance` 항목이 보이면 잠금 활성

### 다른 언어/환경

원리는 동일. abstract socket bind 시도 → 실패 시 종료. Python이면
`socket.socket(AF_UNIX).bind("\0myapp.single-instance")` 식으로 `\0` 접두사를 쓴다.

## 2. 독바(Dock) 아이콘 통합

### 증상

`.desktop` 파일을 만들어 독에 고정했는데, 앱을 실행하면 **고정한 아이콘과 별개로 기본(톱니바퀴) 아이콘이 하나 더 생김**.

### 원인

GNOME 독은 실행 중인 창을 `.desktop` 파일과 매칭할 때 창의 식별자를 사용한다:

- **Wayland**: 창의 `app_id`
- **X11**: 창의 `WM_CLASS`

앱이 이 값을 설정하지 않으면(또는 `.desktop` 파일 이름과 다르면) 독이 "알 수 없는 앱"으로 취급해서 별도 아이콘을 띄운다.

### 해결: 세 값을 일치시킨다

| 항목 | 값 (예: localman) |
|---|---|
| `.desktop` 파일 이름 | `localman.desktop` |
| `.desktop` 내 `StartupWMClass=` | `localman` |
| 앱이 설정하는 `app_id` / `WM_CLASS` | `localman` |

### 구현 (iced 0.13)

```rust
use iced::{application, window, Settings, Size, Theme};

application("MyApp", App::update, App::view)
    .window(window::Settings {
        size: Size::new(1100.0, 700.0),
        // 창/작업표시줄/Alt-Tab 아이콘 (PNG를 바이너리에 내장)
        icon: window::icon::from_file_data(APP_ICON_PNG, None).ok(),
        platform_specific: window::settings::PlatformSpecific {
            // .desktop 파일 이름과 일치해야 독바에서 같은 앱으로 인식됨
            application_id: String::from("myapp"),
            ..Default::default()
        },
        ..window::Settings::default()
    })
    .run_with(App::new)
```

- `application_id`는 iced(winit)가 Wayland에선 `app_id`, X11에선 `WM_CLASS`로 동일하게 적용해줌
- `window::icon::from_file_data`는 SVG를 못 읽으므로 PNG 필요 (iced의 `image` feature 필요):
  ```bash
  convert -background none -density 384 assets/myapp.svg -resize 256x256 assets/myapp.png
  ```
  ```rust
  const APP_ICON_PNG: &[u8] = include_bytes!("../assets/myapp.png");
  ```

### .desktop 파일 예시

`~/.local/share/applications/myapp.desktop`:

```ini
[Desktop Entry]
Version=1.0
Type=Application
Name=MyApp
Comment=설명
Exec=/home/USER/.local/share/myapp/myapp
Icon=/home/USER/.local/share/icons/hicolor/scalable/apps/myapp.svg
Terminal=false
Categories=Development;
StartupWMClass=myapp
```

- 독/런처용 아이콘은 SVG 가능: `~/.local/share/icons/hicolor/scalable/apps/`에 설치
- 설치 후 `update-desktop-database ~/.local/share/applications` (없어도 보통 자동 반영)

### 검증 방법

- **X11/XWayland**: `xprop WM_CLASS` 후 창 클릭 → `"myapp", "myapp"` 확인
- **Wayland 네이티브**: xprop으로 안 보임. 독에서 아이콘이 하나로 합쳐지는지 직접 확인
- 앱이 어느 백엔드로 떠 있는지: `lsof -p <PID> | grep wayland` → 결과 있으면 Wayland

## 3. 주의: `WINIT_UNIX_BACKEND`은 더 이상 동작하지 않음

구버전 winit에서 X11을 강제하던 `WINIT_UNIX_BACKEND=x11` 환경변수는 **winit 0.29+에서 제거됨** (iced 0.13은 winit 0.30 사용). 설정해도 무시되고 Wayland 세션에선 네이티브 Wayland로 뜬다.

XWayland를 강제해야 하면(예: kime 등 입력기 호환 문제) `WAYLAND_DISPLAY`를 비워서 실행한다:

```bash
env -u WAYLAND_DISPLAY ./myapp   # 또는 WAYLAND_DISPLAY= ./myapp
```

## 적용 사례

- LocalMan `src/main.rs` — `acquire_single_instance()`, `application_id` 설정 부분 참고
