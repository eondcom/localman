mod domain;
mod platform;
mod ui;

use iced::{application, window, Font, Settings, Size, Theme};
use ui::App;

const NANUM_GOTHIC: &[u8] =
    include_bytes!("../assets/NanumGothic.ttf");
const APP_ICON: &[u8] = include_bytes!("../assets/localman.png");

fn main() -> iced::Result {
    platform::init_env();

    if !platform::acquire_single_instance() {
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
            #[cfg(target_os = "linux")]
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
