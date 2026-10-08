mod domain;
mod i18n;
mod platform;
mod ui;

use iced::{application, window, Settings, Size};
use ui::App;

const APP_ICON: &[u8] = include_bytes!("../assets/localman.png");

fn main() -> iced::Result {
    platform::init_env();
    // 앱이 설치한 Node.js(LTS)를 먼저 찾게 한다
    domain::tools::prepend_path(&domain::tools::node_bin_dir());

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
        .theme(App::theme)
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
            fonts: ui::theme::FONT_FILES.iter().map(|f| (*f).into()).collect(),
            default_font: ui::theme::REGULAR,
            ..Settings::default()
        })
        .run_with(App::new)
}
