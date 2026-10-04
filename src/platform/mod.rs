//! OS마다 달라지는 작업만 모아 둔 계층.
//!
//! 이 모듈 밖에서는 OS를 직접 다루지 않는다(`systemctl`, `/proc`, `/etc/apache2` 등).
//! 새 OS를 지원할 때는 같은 이름의 함수를 가진 모듈을 하나 추가하고 아래에서 cfg로 고른다.

mod unix;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_mysql;

pub use unix::*;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    NotInstalled,
    Unknown,
}
