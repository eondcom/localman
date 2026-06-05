mod system;
mod ui;

use iced::{application, Settings, Size, Theme};
use ui::App;

fn main() -> iced::Result {
    application("LocalMan", App::update, App::view)
        .theme(|_| Theme::Dark)
        .window_size(Size::new(1100.0, 700.0))
        .settings(Settings::default())
        .run_with(App::new)
}
