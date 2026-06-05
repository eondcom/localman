mod system;
mod ui;

use iced::{application, Font, Settings, Size, Theme};
use ui::App;

const NANUM_GOTHIC: &[u8] =
    include_bytes!("../assets/NanumGothic.ttf");

fn main() -> iced::Result {
    application("LocalMan", App::update, App::view)
        .theme(|_| Theme::Dark)
        .window_size(Size::new(1100.0, 700.0))
        .settings(Settings {
            fonts: vec![NANUM_GOTHIC.into()],
            default_font: Font::with_name("NanumGothic"),
            ..Settings::default()
        })
        .run_with(App::new)
}
