mod api;
mod domain;
mod i18n;
mod mcp;
mod platform;
mod ui;

use iced::{application, window, Settings, Size};
use ui::App;

const APP_ICON: &[u8] = include_bytes!("../assets/localman.png");

fn main() -> iced::Result {
    platform::init_env();
    // 아래 "이미 실행 중" 안내부터 고른 언어로 나오게 한다 (설정 화면 상태보다 먼저 실행됨)
    apply_saved_lang();
    // 앱이 설치한 Node.js(LTS)를 먼저 찾게 한다
    domain::tools::prepend_path(&domain::tools::node_bin_dir());

    // `LocalMan --mcp`: 화면 없이 MCP 서버로 (Claude Code·Claude 데스크톱이 띄운다). 앱과 함께 떠 있어도 된다.
    if std::env::args().any(|a| a == "--mcp") {
        mcp::run();
        return Ok(());
    }

    if !platform::acquire_single_instance() {
        rfd::MessageDialog::new()
            .set_title("LocalMan")
            .set_description(i18n::tr("LocalMan이 이미 실행 중입니다."))
            .set_level(rfd::MessageLevel::Info)
            .show();
        return Ok(());
    }

    application("LocalMan", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .scale_factor(App::scale_factor)
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

/// 저장된 언어 설정을 적용한다 (ui::settings::apply_lang 과 같은 규칙).
fn apply_saved_lang() {
    use domain::settings::LangMode;
    i18n::set_lang(match domain::settings::load_settings().lang {
        LangMode::Ko => i18n::Lang::Ko,
        LangMode::En => i18n::Lang::En,
        LangMode::Ja => i18n::Lang::Ja,
        LangMode::System => i18n::from_locale(&platform::system_language()),
    });
}
