//! 系统托盘图标与右键菜单。
//!
//! 菜单在每次右键时按当前配置**重新构建**（Windows 的菜单项勾选状态不能在
//! 弹出后再改），因此不需要维护任何长期菜单句柄。

use std::mem::size_of;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, IDI_APPLICATION, LoadIconW,
    MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterWindowMessageW,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_NULL,
};

use super::wide;

/// 托盘回调用的自定义消息号。
pub const WM_TRAY: u32 = WM_APP + 1;

/// 资源管理器启动/重启时向所有顶层窗口广播的消息号。
///
/// 只能在运行时用 `RegisterWindowMessageW` 取（值不固定），所以缓存一份。
pub fn taskbar_created_message() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe {
        let name = wide("TaskbarCreated");
        RegisterWindowMessageW(name.as_ptr())
    })
}

const TIP: &str = "ScreenCrosshair 屏幕准星";

// 菜单 id 段
const ID_OUTLINE: u16 = 100;
const ID_VISIBLE: u16 = 101;
const ID_SETTINGS: u16 = 102;
const ID_AUTOSTART: u16 = 103;
const ID_OPEN_CONFIG: u16 = 104;
const ID_RELOAD: u16 = 105;
const ID_QUIT: u16 = 106;
const ID_RESET: u16 = 107;
const ID_RESET_OFFSET: u16 = 108;
const ID_HOTKEYS: u16 = 109;

const BASE_PRESET: u16 = 1000;
const BASE_COLOR: u16 = 2000;
const BASE_OPACITY: u16 = 3000;
const BASE_SCALE: u16 = 4000;
const BASE_MONITOR: u16 = 5000;
const BASE_OFFSET: u16 = 6000;

const OPACITY_STEPS: &[f32] = &[1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1];
const SCALE_STEPS: &[f32] = &[0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0];

/// 位置微调步长（像素）。
///
/// 和热键的 1 像素 / 10 像素两档保持一致：菜单里调的步长与快捷键不同的话，
/// 用户会以为「菜单那一项坏了」。
const OFFSET_STEPS: &[f32] = &[1.0, 10.0];

/// 位置微调方向：`(dx, dy, 文案)`，坐标向下为正。
const OFFSET_DIRS: &[(f32, f32, &str)] = &[
    (-1.0, 0.0, "左移"),
    (1.0, 0.0, "右移"),
    (0.0, -1.0, "上移"),
    (0.0, 1.0, "下移"),
];

/// 托盘菜单被点击后产生的意图。
#[derive(Debug, Clone, PartialEq)]
pub enum TrayCommand {
    SelectPreset(String),
    SetColor(String),
    SetOpacity(f32),
    SetScale(f32),
    /// 准星位置微调，单位「100% DPI 像素」。
    NudgeOffset(f32, f32),
    /// 准星回到屏幕中心。
    ResetOffset,
    ToggleOutline,
    ToggleVisible,
    ToggleHotkeys,
    SelectMonitor(Option<usize>),
    OpenSettings,
    ToggleAutostart,
    OpenConfig,
    ResetOverrides,
    Reload,
    Quit,
}

/// 构建菜单时需要的当前状态。
pub struct TrayState {
    pub visible: bool,
    pub active_preset: String,
    /// `(name, 中文标签)`
    pub presets: Vec<(String, String)>,
    pub color: String,
    pub color_palette: Vec<(&'static str, &'static str)>,
    pub opacity: f32,
    pub scale: f32,
    /// 当前偏移（100% DPI 像素），菜单里显示成「左 3 · 上 5」。
    pub offset: [f32; 2],
    pub outline_enabled: bool,
    /// `(下标, 描述)`
    pub monitors: Vec<(usize, String)>,
    pub monitor: Option<usize>,
    pub autostart: bool,
    /// 全局热键总开关。
    pub hotkeys_enabled: bool,
}

pub struct Tray {
    hwnd: HWND,
    /// 图标句柄由调用方持有（[`crate::platform::icon::Icon`]），这里只借用。
    icon: HICON,
    added: bool,
}

impl Tray {
    /// 添加托盘图标。`hwnd` 用来接收回调消息（见 [`WM_TRAY`]）。
    ///
    /// `icon` 传 `None` 时退回系统默认图标（[`IDI_APPLICATION`]）：拿不到
    /// 自绘图标是外观问题，不该让整个程序起不来。
    pub fn new(hwnd: HWND, icon: Option<HICON>) -> Result<Self, String> {
        let icon = icon
            .filter(|h| !h.is_null())
            .unwrap_or_else(|| unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) });
        if icon.is_null() {
            return Err("LoadIconW 失败".to_string());
        }

        let mut tray = Tray {
            hwnd,
            icon,
            added: false,
        };
        tray.attach().map(|()| tray)
    }

    /// 把图标挂到任务栏（`NIM_ADD`）。首次创建与「任务栏重建」后都用它。
    fn attach(&mut self) -> Result<(), String> {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon;
        copy_wide(&mut data.szTip, TIP);

        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err("Shell_NotifyIconW(NIM_ADD) 失败".to_string());
        }
        self.added = true;
        Ok(())
    }

    /// 资源管理器重启后重新挂上图标。
    ///
    /// 收到 `TaskbarCreated` 时任务栏里的图标已经随旧的 explorer.exe 一起没了，
    /// 只有重新 `NIM_ADD` 才会回来。漏掉这一步的症状是「重启资源管理器
    /// （或屏幕锁定解锁、Explorer 崩溃恢复）之后托盘图标永久消失」，
    /// 必须重启本程序才恢复——而 `NIM_ADD` 本身一直是成功的，很容易误判。
    pub fn reattach(&mut self) {
        self.added = false;
        let _ = self.attach();
    }

    /// 更新提示文字（切换预设后调用）。
    pub fn set_tip(&mut self, text: &str) {
        if !self.added {
            return;
        }
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_TIP;
        copy_wide(&mut data.szTip, text);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// 在鼠标位置弹出右键菜单，返回用户的选择（未选中返回 `None`）。
    pub fn popup(&self, state: &TrayState) -> Option<TrayCommand> {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }

            // ---- 预设 ----
            let presets_menu = CreatePopupMenu();
            for (i, (name, label)) in state.presets.iter().enumerate() {
                let is_active = *name == state.active_preset;
                let flags = MF_STRING | if is_active { MF_CHECKED } else { 0 };
                let text = wide(if is_active {
                    format!("• {label}")
                } else {
                    format!("   {label}")
                });
                AppendMenuW(
                    presets_menu,
                    flags,
                    (BASE_PRESET + i as u16) as usize,
                    text.as_ptr(),
                );
            }
            AppendMenuW(menu, MF_POPUP, presets_menu as usize, wide("预设").as_ptr());

            // ---- 颜色 ----
            let color_menu = CreatePopupMenu();
            for (i, (label, hex)) in state.color_palette.iter().enumerate() {
                let is_current = hex.eq_ignore_ascii_case(&state.color);
                let flags = MF_STRING | if is_current { MF_CHECKED } else { 0 };
                let text = wide(format!("{label}  {hex}"));
                AppendMenuW(
                    color_menu,
                    flags,
                    (BASE_COLOR + i as u16) as usize,
                    text.as_ptr(),
                );
            }
            AppendMenuW(menu, MF_POPUP, color_menu as usize, wide("颜色").as_ptr());

            // ---- 不透明度 ----
            let opacity_menu = CreatePopupMenu();
            for (i, value) in OPACITY_STEPS.iter().enumerate() {
                let is_current = (value - state.opacity).abs() < 0.006;
                let flags = MF_STRING | if is_current { MF_CHECKED } else { 0 };
                let text = wide(format!("{:>3}%", (value * 100.0).round() as i32));
                AppendMenuW(
                    opacity_menu,
                    flags,
                    (BASE_OPACITY + i as u16) as usize,
                    text.as_ptr(),
                );
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                opacity_menu as usize,
                wide("不透明度").as_ptr(),
            );

            // ---- 尺寸 ----
            let scale_menu = CreatePopupMenu();
            for (i, value) in SCALE_STEPS.iter().enumerate() {
                let is_current = (value - state.scale).abs() < 0.006;
                let flags = MF_STRING | if is_current { MF_CHECKED } else { 0 };
                let text = wide(format!("{value:.2}x"));
                AppendMenuW(
                    scale_menu,
                    flags,
                    (BASE_SCALE + i as u16) as usize,
                    text.as_ptr(),
                );
            }
            AppendMenuW(menu, MF_POPUP, scale_menu as usize, wide("尺寸").as_ptr());

            // ---- 显示器 ----
            let monitor_menu = CreatePopupMenu();
            for (i, (index, desc)) in state.monitors.iter().enumerate() {
                let is_current = state.monitor == Some(*index);
                let flags = MF_STRING | if is_current { MF_CHECKED } else { 0 };
                let text = wide(desc.clone());
                AppendMenuW(
                    monitor_menu,
                    flags,
                    (BASE_MONITOR + i as u16) as usize,
                    text.as_ptr(),
                );
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                monitor_menu as usize,
                wide("显示器").as_ptr(),
            );

            // ---- 位置 ----
            let offset_menu = CreatePopupMenu();
            let current = wide(format!(
                "当前：水平 {:+} · 垂直 {:+}",
                state.offset[0].round() as i32,
                state.offset[1].round() as i32
            ));
            AppendMenuW(
                offset_menu,
                MF_STRING | MF_GRAYED,
                BASE_OFFSET as usize,
                current.as_ptr(),
            );
            for (step_idx, step) in OFFSET_STEPS.iter().enumerate() {
                AppendMenuW(offset_menu, MF_SEPARATOR, 0, std::ptr::null());
                for (dir_idx, (_, _, name)) in OFFSET_DIRS.iter().enumerate() {
                    let id = BASE_OFFSET + 1 + (step_idx * OFFSET_DIRS.len() + dir_idx) as u16;
                    let text = wide(format!("{name} {step:.0} 像素"));
                    AppendMenuW(offset_menu, MF_STRING, id as usize, text.as_ptr());
                }
            }
            AppendMenuW(offset_menu, MF_SEPARATOR, 0, std::ptr::null());
            let center_text = wide("回到屏幕中心");
            AppendMenuW(
                offset_menu,
                MF_STRING,
                ID_RESET_OFFSET as usize,
                center_text.as_ptr(),
            );
            AppendMenuW(menu, MF_POPUP, offset_menu as usize, wide("位置").as_ptr());

            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

            let outline_text = wide("描边（黑边）");
            AppendMenuW(
                menu,
                MF_STRING | if state.outline_enabled { MF_CHECKED } else { 0 },
                ID_OUTLINE as usize,
                outline_text.as_ptr(),
            );
            let visible_text = wide("显示准星");
            AppendMenuW(
                menu,
                MF_STRING | if state.visible { MF_CHECKED } else { 0 },
                ID_VISIBLE as usize,
                visible_text.as_ptr(),
            );
            // 热键总开关必须能从托盘切：只靠热键本身的话，关掉就再也开不回来了
            let hotkeys_text = wide("启用全局热键");
            AppendMenuW(
                menu,
                MF_STRING | if state.hotkeys_enabled { MF_CHECKED } else { 0 },
                ID_HOTKEYS as usize,
                hotkeys_text.as_ptr(),
            );
            let reset_text = wide("清除临时调整（颜色/透明度/尺寸）");
            AppendMenuW(menu, MF_STRING, ID_RESET as usize, reset_text.as_ptr());

            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

            let settings_text = wide("设置界面…");
            AppendMenuW(
                menu,
                MF_STRING,
                ID_SETTINGS as usize,
                settings_text.as_ptr(),
            );
            let autostart_text = wide("开机自启");
            AppendMenuW(
                menu,
                MF_STRING | if state.autostart { MF_CHECKED } else { 0 },
                ID_AUTOSTART as usize,
                autostart_text.as_ptr(),
            );
            let open_cfg_text = wide("打开配置文件");
            AppendMenuW(
                menu,
                MF_STRING,
                ID_OPEN_CONFIG as usize,
                open_cfg_text.as_ptr(),
            );
            let reload_text = wide("重新载入配置");
            AppendMenuW(menu, MF_STRING, ID_RELOAD as usize, reload_text.as_ptr());

            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

            let quit_text = wide("退出");
            AppendMenuW(menu, MF_STRING, ID_QUIT as usize, quit_text.as_ptr());

            // TrackPopupMenu 要求先让窗口成为前台，否则点菜单外面不会关闭。
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            SetForegroundWindow(self.hwnd);
            let picked = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                0,
                self.hwnd,
                std::ptr::null(),
            );
            // 官方推荐的收尾：投一个空消息，把菜单消失后残留的鼠标捕获还给系统。
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);

            if picked == 0 {
                return None;
            }
            decode(picked as u16, state)
        }
    }
}

fn decode(id: u16, state: &TrayState) -> Option<TrayCommand> {
    match id {
        ID_OUTLINE => Some(TrayCommand::ToggleOutline),
        ID_VISIBLE => Some(TrayCommand::ToggleVisible),
        ID_HOTKEYS => Some(TrayCommand::ToggleHotkeys),
        ID_SETTINGS => Some(TrayCommand::OpenSettings),
        ID_AUTOSTART => Some(TrayCommand::ToggleAutostart),
        ID_OPEN_CONFIG => Some(TrayCommand::OpenConfig),
        ID_RELOAD => Some(TrayCommand::Reload),
        ID_RESET => Some(TrayCommand::ResetOverrides),
        ID_RESET_OFFSET => Some(TrayCommand::ResetOffset),
        ID_QUIT => Some(TrayCommand::Quit),
        _ => {
            // 菜单 id 分段编码：每类从各自的 BASE 开始递增，
            // 因此区间判断要用半开区间，否则相邻两类的首尾会重叠。
            if (BASE_PRESET..BASE_COLOR).contains(&id) {
                let i = (id - BASE_PRESET) as usize;
                state
                    .presets
                    .get(i)
                    .map(|(name, _)| TrayCommand::SelectPreset(name.clone()))
            } else if (BASE_COLOR..BASE_OPACITY).contains(&id) {
                let i = (id - BASE_COLOR) as usize;
                state
                    .color_palette
                    .get(i)
                    .map(|(_, hex)| TrayCommand::SetColor((*hex).to_string()))
            } else if (BASE_OPACITY..BASE_SCALE).contains(&id) {
                let i = (id - BASE_OPACITY) as usize;
                OPACITY_STEPS.get(i).map(|v| TrayCommand::SetOpacity(*v))
            } else if (BASE_SCALE..BASE_MONITOR).contains(&id) {
                let i = (id - BASE_SCALE) as usize;
                SCALE_STEPS.get(i).map(|v| TrayCommand::SetScale(*v))
            } else if (BASE_MONITOR..BASE_OFFSET).contains(&id) {
                let i = (id - BASE_MONITOR) as usize;
                state
                    .monitors
                    .get(i)
                    .map(|(index, _)| TrayCommand::SelectMonitor(Some(*index)))
            } else if id > BASE_OFFSET {
                // 第一项（id 恰为 BASE_OFFSET）是灰掉的「当前：…」占位，
                // 真正可点的是 +1 之后每档步长 × 四个方向。
                let slot = (id - BASE_OFFSET - 1) as usize;
                let dirs = OFFSET_DIRS.len();
                let step = OFFSET_STEPS.get(slot / dirs)?;
                let (dx, dy, _) = OFFSET_DIRS.get(slot % dirs)?;
                Some(TrayCommand::NudgeOffset(dx * step, dy * step))
            } else {
                None
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if !self.added {
            return;
        }
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
        self.added = false;
    }
}

/// 把字符串写进 Win32 的定长 `[u16; N]` 数组（含结尾 NUL）。
fn copy_wide<const N: usize>(dst: &mut [u16; N], src: &str) {
    let encoded: Vec<u16> = src.encode_utf16().collect();
    let n = encoded.len().min(N.saturating_sub(1));
    dst[..n].copy_from_slice(&encoded[..n]);
    dst[n] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TrayState {
        TrayState {
            visible: true,
            active_preset: "cross".to_string(),
            presets: vec![
                ("dot".to_string(), "圆点".to_string()),
                ("cross".to_string(), "十字".to_string()),
            ],
            color: "#00FF00".to_string(),
            color_palette: vec![("绿", "#00FF00"), ("红", "#FF3B30")],
            opacity: 0.9,
            scale: 1.0,
            offset: [0.0, 0.0],
            outline_enabled: true,
            monitors: vec![(0, "主显示器".to_string())],
            monitor: None,
            autostart: false,
            hotkeys_enabled: true,
        }
    }

    #[test]
    fn decode_maps_each_section() {
        let s = state();
        assert_eq!(
            decode(BASE_PRESET + 1, &s),
            Some(TrayCommand::SelectPreset("cross".to_string()))
        );
        assert_eq!(
            decode(BASE_COLOR, &s),
            Some(TrayCommand::SetColor("#00FF00".to_string()))
        );
        assert_eq!(
            decode(BASE_OPACITY + 1, &s),
            Some(TrayCommand::SetOpacity(0.9))
        );
        assert_eq!(decode(BASE_SCALE + 2, &s), Some(TrayCommand::SetScale(1.0)));
        assert_eq!(
            decode(BASE_MONITOR, &s),
            Some(TrayCommand::SelectMonitor(Some(0)))
        );
        assert_eq!(decode(ID_QUIT, &s), Some(TrayCommand::Quit));
        assert_eq!(decode(ID_HOTKEYS, &s), Some(TrayCommand::ToggleHotkeys));
    }

    /// 位置子菜单的 id 是「步长 × 方向」算出来的，算错一格就会变成
    /// 「点左移却往右跑」，这种错看代码很难发现，必须逐个校验。
    #[test]
    fn decode_maps_every_offset_item() {
        let s = state();
        for (step_idx, step) in OFFSET_STEPS.iter().enumerate() {
            for (dir_idx, (dx, dy, _)) in OFFSET_DIRS.iter().enumerate() {
                let id = BASE_OFFSET + 1 + (step_idx * OFFSET_DIRS.len() + dir_idx) as u16;
                assert_eq!(
                    decode(id, &s),
                    Some(TrayCommand::NudgeOffset(dx * step, dy * step)),
                    "步长 {step} 方向 ({dx}, {dy}) 解析不对"
                );
            }
        }
        // 两档步长之间没有空洞：越界一格必须是 None 而不是拐回第一档。
        let past = BASE_OFFSET + 1 + (OFFSET_STEPS.len() * OFFSET_DIRS.len()) as u16;
        assert_eq!(decode(past, &s), None);
        // 灰掉的状态占位项不该被当成命令。
        assert_eq!(decode(BASE_OFFSET, &s), None);
        assert_eq!(decode(ID_RESET_OFFSET, &s), Some(TrayCommand::ResetOffset));
    }

    #[test]
    fn decode_rejects_out_of_range() {
        let s = state();
        assert_eq!(decode(BASE_PRESET + 99, &s), None);
        assert_eq!(decode(1, &s), None);
    }

    #[test]
    fn copy_wide_never_overflows() {
        let mut buf = [0u16; 4];
        copy_wide(&mut buf, "abcdefgh");
        assert_eq!(buf, [b'a' as u16, b'b' as u16, b'c' as u16, 0]);
    }
}
