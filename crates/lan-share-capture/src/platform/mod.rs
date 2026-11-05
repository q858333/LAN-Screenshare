#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "windows")]
pub use win::{open_primary_display, open_system_audio};

#[cfg(target_os = "macos")]
pub use mac::{open_primary_display, open_system_audio};
