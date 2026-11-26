//! ScreenCrosshair —— 屏幕中心准星覆盖层。
//!
//! 分成两层：
//! * **纯逻辑**（`color` / `config` / `presets` / `crosshair`）：不碰任何 Win32 API，
//!   因此可以在任意平台上 `cargo test`；
//! * **平台层**（`overlay` / `hotkeys` / `tray` / `autostart` / `settings`）：只在 Windows 上编译。

pub mod color;
pub mod config;
pub mod crosshair;
pub mod presets;
pub mod update;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod platform;
#[cfg(windows)]
pub mod settings;
