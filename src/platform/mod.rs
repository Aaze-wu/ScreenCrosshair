//! Windows 平台层。
//!
//! 全部代码只在 Windows 上编译；纯逻辑部分（配置、颜色、渲染）在
//! [`crate`] 的其它模块里，可以跨平台跑测试。

pub mod autostart;
pub mod hotkeys;
pub mod icon;
pub mod monitor;
pub mod overlay;
pub mod single;
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

/// 用系统默认程序打开一个链接或文件。
///
/// 只用于「打开发布页」这类动作：**程序自己从不下载任何东西**，把链接交给
/// 浏览器是唯一不会踩「静默替换可执行文件」这条红线的做法。
///
/// `ShellExecuteW` 的返回值是个 `HINSTANCE`，但语义是整数：大于 32 才算成功。
pub fn open_url(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // 只放行 http(s)：`ShellExecuteW` 也会执行 `file:///…exe`、`mailto:` 甚至
    // 自定义协议，这个函数的输入来自网络返回体，不该有这种权力。
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("拒绝打开非 http(s) 链接：{url}"));
    }
    let verb = wide("open");
    let target = wide(url);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("打开链接失败（ShellExecuteW 返回 {code}）"))
    }
}
