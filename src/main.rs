mod system;
mod ui;

use iced::{application, window, Font, Settings, Size, Theme};
use ui::App;

const NANUM_GOTHIC: &[u8] =
    include_bytes!("../assets/NanumGothic.ttf");
const APP_ICON: &[u8] = include_bytes!("../assets/localman.png");

fn main() -> iced::Result {
    ignore_sigchld();

    if !acquire_single_instance() {
        rfd::MessageDialog::new()
            .set_title("LocalMan")
            .set_description("LocalMan이 이미 실행 중입니다.")
            .set_level(rfd::MessageLevel::Info)
            .show();
        return Ok(());
    }

    application("LocalMan", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Dark)
        .window(window::Settings {
            size: Size::new(1100.0, 700.0),
            icon: window::icon::from_file_data(APP_ICON, None).ok(),
            platform_specific: window::settings::PlatformSpecific {
                // .desktop 파일 이름(localman.desktop)과 일치해야 독바에서 같은 앱으로 인식됨
                application_id: String::from("localman"),
                ..Default::default()
            },
            ..window::Settings::default()
        })
        .settings(Settings {
            fonts: vec![NANUM_GOTHIC.into()],
            default_font: Font::with_name("NanumGothic"),
            ..Settings::default()
        })
        .run_with(App::new)
}

/// SIGCHLD를 SIG_IGN으로 설정한다.
///
/// vhost::spawn_server가 자식(dev server)을 detach(mem::forget)하므로
/// 아무도 wait()를 호출하지 않는다. 커널 기본 동작은 자식이 죽어도
/// 부모가 reap할 때까지 좀비로 남기는 것인데, SIGCHLD를 SIG_IGN으로
/// 두면 POSIX 규정에 따라 커널이 종료된 자식을 즉시 자동 회수한다.
fn ignore_sigchld() {
    unsafe extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    const SIGCHLD: i32 = 17;
    const SIG_IGN: usize = 1;
    unsafe {
        signal(SIGCHLD, SIG_IGN);
    }
}

/// 중복 실행 방지: abstract unix socket을 잠금으로 사용 (프로세스 종료 시 커널이 자동 해제)
fn acquire_single_instance() -> bool {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let Ok(addr) = SocketAddr::from_abstract_name(b"localman.single-instance") else {
        return true;
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
