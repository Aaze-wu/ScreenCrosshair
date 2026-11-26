//! 单实例检测。
//!
//! 覆盖层是常驻程序：用户双击两次图标就会有两个进程、两套全局热键、
//! 两个托盘图标、两层透明覆盖层叠在一起。更糟的是**第二个进程看起来一切正常**
//! （窗口都建起来了，只是 `RegisterHotKey` 静默失败），用户完全不知道自己
//! 多开了一个，只会觉得「热键时灵时不灵」。
//!
//! 做法是最经典的两步：
//! 1. **命名互斥体**：`CreateMutexW` + 检查 `ERROR_ALREADY_EXISTS`，
//!    第二个进程立刻知道「已经有一个在跑」；
//! 2. **给老实例发个消息**：`FindWindowW` 找到它的消息窗口，`PostMessageW`
//!    一个自定义消息，让它把设置界面弹出来。
//!
//! 第 2 步是体验关键：用户双击图标的意图是「把程序叫出来」。如果第二个进程
//! 只是默默退出，用户会觉得双击没反应，然后继续双击。

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};

use crate::platform::wide;

/// 互斥体名字。
///
/// `Local\` 前缀表示「每个登录会话一个」：覆盖层画在**当前用户的桌面**上，
/// 用 `Global\` 会让同一台机器上的另一个用户（或另一条远程桌面会话）永远
/// 打不开程序，而他们的桌面其实是完全独立的。
pub const MUTEX_NAME: &str = r"Local\ScreenCrosshair.SingleInstance";

/// 第二个进程发给已运行实例的自定义消息：把设置界面叫出来。
///
/// 用 `WM_APP + n` 而不是注册窗口消息（`RegisterWindowMessageW`）：
/// 双方在同一个 exe 里，类名都是自己注册的，不存在第三方撞车的问题。
///
/// **必须避开 `tray::WM_TRAY`（`WM_APP + 1`）**：两者都会打到同一个消息窗口上，
/// 撞号的表现是「双击图标弹出的是托盘菜单」。
pub const WM_SHOW_SETTINGS: u32 = WM_APP + 2;

/// 请求已运行的实例**退出**。
///
/// 给安装包用的：覆盖层没有任务栏窗口，安装/升级时用户根本找不到它，
/// 让安装程序自己喊一声「请你退出」比弹一句「请先关闭程序」友好得多
/// （后者在这个程序上等于无解——用户看不见任何窗口）。
pub const WM_EXIT: u32 = WM_APP + 3;

/// 互斥体的 RAII 句柄：进程存活期间一直持有，退出时由系统自动释放。
///
/// 注意**不能**提前 drop：一旦句柄关掉，后一个进程就能立刻抢到锁。
/// 所以调用方必须把它绑在一个活到 `main` 结束的变量上。
pub struct InstanceGuard {
    handle: HANDLE,
}

impl InstanceGuard {
    /// 一个「什么都没守住」的守卫。
    ///
    /// 用于「检测机制本身失败了」的情况：这时候正确的做法是**放行**
    /// （宁可多开一次让用户能用，也不要因为检测不出就把程序挡在门外），
    /// 但仍然要有个值占住变量的位置，让调用方的代码只有一条路径。
    pub fn none() -> Self {
        Self {
            handle: std::ptr::null_mut(),
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

/// 抢单实例锁失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireError {
    /// 已经有一个实例在跑。
    AlreadyRunning,
    /// 系统层面失败（极少见）。调用方应该**放行**，别因为检测不到就不让用户用。
    Failed(String),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::AlreadyRunning => write!(f, "已经有一个实例在运行"),
            AcquireError::Failed(err) => write!(f, "{err}"),
        }
    }
}

/// 尝试成为「唯一的那个实例」。
pub fn acquire() -> Result<InstanceGuard, AcquireError> {
    let name = wide(MUTEX_NAME);
    // bInitialOwner = 0：不需要「拥有」互斥体，只要它存在就够了。
    // 抢占所有权会引入「谁该 ReleaseMutex」的额外正确性问题，而这里
    // 我们只想回答一个是非题。
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(AcquireError::Failed(format!(
            "CreateMutexW 失败（错误码 {}）",
            unsafe { GetLastError() }
        )));
    }
    // 必须在 CloseHandle 之前读，之后错误码就可能被覆盖。
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if exists {
        unsafe {
            CloseHandle(handle);
        }
        return Err(AcquireError::AlreadyRunning);
    }
    Ok(InstanceGuard { handle })
}

/// 通知已运行的实例「有人又想启动你」，让它把设置界面弹出来。
///
/// 返回 `false` 表示没找到（互斥体还在但窗口已经没了，属于启动/退出瞬间的
/// 竞态）。这种情况不必报错：用户再点一次就好。
pub fn signal_existing() -> bool {
    post_to_existing(WM_SHOW_SETTINGS)
}

/// 请已运行的实例退出（安装包 / 升级前用）。
///
/// 返回 `false` 表示没有找到运行中的实例——对调用方来说这是**正常**的，
/// 没在跑就不需要关。
pub fn signal_exit() -> bool {
    post_to_existing(WM_EXIT)
}

/// 给消息窗口投递一条自定义消息。
fn post_to_existing(msg: u32) -> bool {
    let class = wide(crate::platform::overlay::MSG_CLASS_NAME);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return false;
        }
        PostMessageW(hwnd, msg, 0, 0) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 互斥体名字必须是会话局部且带项目前缀。
    ///
    /// 这两点都是**改了会静默出事**的：
    /// * 去掉 `Local\` 前缀 → 多用户机器上第二个用户打不开程序；
    /// * 名字太通用（比如 `ScreenCrosshair`）→ 和别的软件撞锁，双方都启动不了。
    #[test]
    fn mutex_name_is_session_local_and_specific() {
        assert!(MUTEX_NAME.starts_with(r"Local\"), "必须是会话局部互斥体");
        assert!(MUTEX_NAME.contains("ScreenCrosshair"));
        // 反斜杠后面除了前缀那一个之外不该再有分隔符（`Local\` 里那一个除外）
        assert_eq!(MUTEX_NAME.matches('\\').count(), 1);
    }

    /// 自定义消息必须落在 `WM_APP` 区间，避免和系统消息号撞车
    /// （`WM_USER` 以下的号段被系统占满，拿来做进程间信令会误触发系统行为）。
    ///
    /// 这里全部用 `const { … }` 包着：参与比较的都是常量，放在运行时取值的
    /// `assert!` 里等于「测一个已经写死的真值」，clippy 会指出来。写成 const 块
    /// 反而更强——**写错就是编译错误**，不用等 `cargo test` 才发现。
    #[test]
    fn custom_message_is_in_wm_app_range() {
        const {
            assert!(WM_SHOW_SETTINGS >= WM_APP);
            assert!(WM_SHOW_SETTINGS < WM_APP + 0x4000);
            // 和托盘回调撞号的话，双击图标弹出的会是托盘菜单。
            assert!(WM_SHOW_SETTINGS != crate::platform::tray::WM_TRAY);
        }
    }

    /// 两条自定义消息必须互不相同，且都避开托盘消息。
    ///
    /// 撞号的表现很难查：`WM_EXIT` 撞上 `WM_SHOW_SETTINGS` 会变成
    /// 「安装时程序弹出设置界面但没退出」。
    #[test]
    fn exit_message_is_distinct_from_the_others() {
        const {
            assert!(WM_EXIT != WM_SHOW_SETTINGS);
            assert!(WM_EXIT != crate::platform::tray::WM_TRAY);
            assert!(WM_EXIT >= WM_APP);
            assert!(WM_EXIT < WM_APP + 0x4000);
        }
    }
}
