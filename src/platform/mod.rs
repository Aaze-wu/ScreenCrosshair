//! Windows 平台层。
//!
//! 全部代码只在 Windows 上编译；纯逻辑部分（配置、颜色、渲染）在
//! [`crate`] 的其它模块里，可以跨平台跑测试。

pub mod autostart;
pub mod hotkeys;
pub mod icon;
pub mod monitor;
pub mod overlay;
pub mod tray;

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// 把 Rust 字符串转成以 NUL 结尾的 UTF-16，供 Win32 的 `*W` 接口使用。
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    let mut v: Vec<u16> = s.as_ref().encode_wide().collect();
    v.push(0);
    v
}

/// 从定长 UTF-16 缓冲区里读出字符串（遇到 NUL 截断）。
pub fn from_wide_buf(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 启用 Per-Monitor V2 DPI 感知。必须在创建任何窗口之前调用。
///
/// 返回是否成功；老系统（Win7/8）会失败，此时程序仍能运行，只是
/// 高 DPI 下尺寸由系统拉伸，可能略糊。
pub fn enable_per_monitor_dpi() -> bool {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 }
}

/// 取当前进程程序的完整路径。
pub fn exe_path() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()
}
