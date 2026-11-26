//! 运行时装配：把配置、渲染、覆盖层、热键、托盘串起来。
//!
//! 结构上刻意分成两半：
//! * [`Shared`] 是**纯数据**，放在 `Arc<Mutex<…>>` 里，供 Win32 回调、
//!   设置界面线程、主消息循环同时访问；
//! * [`App`] 持有所有 `HWND` / `HDC`，只在主线程里用。
//!
//! Win32 回调永远不会碰 [`App`]，只改 [`Shared`] 里的标志位；真正的重绘、
//! 写盘、弹菜单都回到主循环里做。这样避免了「回调里重入 Mutex」的死锁。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, SetTimer, TranslateMessage, WM_DISPLAYCHANGE, WM_HOTKEY,
    WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER,
};

use crate::config::{self, Config, MonitorSel, Overrides};
use crate::crosshair;
use crate::platform::hotkeys::{self, HotkeyAction, Hotkeys};
use crate::platform::icon::{self, Icon};
use crate::platform::monitor::{self, MonitorInfo};
use crate::platform::overlay::{MessageWindow, Overlay};
use crate::platform::tray::{Tray, TrayCommand, TrayState, WM_TRAY, taskbar_created_message};
use crate::presets;
use crate::update::UpdateState;

/// 定时器 id：轮询配置文件是否被外部改动。
const TIMER_POLL: usize = 1;
/// 轮询间隔（毫秒）
const POLL_MS: u32 = 400;

/// 所有线程共享的可变状态。
pub struct Shared {
    pub config: Config,
    pub config_path: PathBuf,
    /// 上次读/写时的文件内容哈希，用来发现外部改动
    pub file_hash: u64,
    /// 需要重绘
    pub dirty: bool,
    /// 需要写回配置
    pub needs_save: bool,
    pub quit: bool,
    pub tray_pending: bool,
    /// 需要重新挂托盘图标（资源管理器重启后任务栏会广播 `TaskbarCreated`）
    pub tray_reattach: bool,
    pub reload_pending: bool,
    pub settings_pending: bool,
    /// 需要重建覆盖层（换显示器 / 分辨率变化）
    pub rebuild_pending: bool,
    /// 需要按新配置重新注册全局热键（改键 / 开关热键）
    pub hotkeys_dirty: bool,
    /// 设置界面正在捕获新键位，期间必须把全部热键注销。
    ///
    /// `RegisterHotKey` 抢走的组合键不会再进任何窗口的键盘队列，所以捕获态里
    /// 按下**任何已被本项目占用**的组合都收不到——包括用户想换掉的那个旧键。
    /// 实测按 `Ctrl+Alt+Shift+S` 不进捕获态，而是直接触发了「下移 10px」。
    pub hotkeys_suspend: bool,
    /// 最近一条状态信息（错误或提示），供界面显示
    pub status: Option<String>,
    /// 更新检测结果（后台线程写入，设置界面读取）
    pub update: UpdateState,
}

impl Shared {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.needs_save = true;
    }

    /// 当前预设的中文标签 + 名称列表，供菜单使用。
    pub fn preset_items(&self) -> Vec<(String, String)> {
        self.config
            .preset_names()
            .into_iter()
            .map(|name| {
                let label = if self.config.custom_presets.contains_key(&name) {
                    format!("{}（自定义）", presets::label(&name))
                } else {
                    presets::label(&name)
                };
                (name, label)
            })
            .collect()
    }

    pub fn handle_hotkey(&mut self, action: HotkeyAction) {
        // `resolve()` 在关闭时已经返回空表，理论上这里收不到动作。
        // 但重注册是异步的（置 `hotkeys_dirty`，下一帧才生效），那几帧里
        // 旧绑定还在，所以要再挡一次。
        if !self.config.hotkeys_enabled {
            return;
        }
        match action {
            HotkeyAction::NextPreset => {
                let name = self.config.cycle(1);
                self.status = Some(format!("预设：{}", presets::label(&name)));
                self.mark_dirty();
            }
            HotkeyAction::PrevPreset => {
                let name = self.config.cycle(-1);
                self.status = Some(format!("预设：{}", presets::label(&name)));
                self.mark_dirty();
            }
            HotkeyAction::ToggleVisible => {
                self.config.visible = !self.config.visible;
                self.status = Some(if self.config.visible {
                    "准星已显示".to_string()
                } else {
                    "准星已隐藏".to_string()
                });
                self.mark_dirty();
            }
            HotkeyAction::OpacityUp => {
                let v = self.config.set_opacity_step(1);
                self.status = Some(format!("不透明度 {:.0}%", v * 100.0));
                self.mark_dirty();
            }
            HotkeyAction::OpacityDown => {
                let v = self.config.set_opacity_step(-1);
                self.status = Some(format!("不透明度 {:.0}%", v * 100.0));
                self.mark_dirty();
            }
            HotkeyAction::ScaleUp => {
                let v = self.config.set_scale_step(1);
                self.status = Some(format!("尺寸 {v:.1}x"));
                self.mark_dirty();
            }
            HotkeyAction::ScaleDown => {
                let v = self.config.set_scale_step(-1);
                self.status = Some(format!("尺寸 {v:.1}x"));
                self.mark_dirty();
            }
            HotkeyAction::Nudge { dx, dy } => {
                let off = self.config.nudge_offset(dx as f32, dy as f32);
                self.status = Some(format_offset(off));
                self.mark_dirty();
            }
            HotkeyAction::ResetOffset => {
                self.config.reset_offset();
                self.status = Some("准星已回到屏幕中心".to_string());
                self.mark_dirty();
            }
            HotkeyAction::OpenSettings => {
                self.settings_pending = true;
            }
            HotkeyAction::Reload => {
                self.reload_pending = true;
            }
            HotkeyAction::Quit => {
                self.quit = true;
            }
        }
    }

    pub fn handle_tray(&mut self, cmd: TrayCommand) {
        match cmd {
            TrayCommand::SelectPreset(name) => {
                self.config.active = name.clone();
                self.status = Some(format!("预设：{}", presets::label(&name)));
                self.mark_dirty();
            }
            TrayCommand::SetColor(hex) => {
                self.config.overrides.color = Some(hex);
                self.mark_dirty();
            }
            TrayCommand::SetOpacity(v) => {
                self.config.overrides.opacity = Some(v.clamp(0.0, 1.0));
                self.mark_dirty();
            }
            TrayCommand::SetScale(v) => {
                self.config.overrides.scale = Some(v.clamp(0.3, 6.0));
                self.mark_dirty();
            }
            TrayCommand::NudgeOffset(dx, dy) => {
                let off = self.config.nudge_offset(dx, dy);
                self.status = Some(format_offset(off));
                self.mark_dirty();
            }
            TrayCommand::ResetOffset => {
                self.config.reset_offset();
                self.status = Some("准星已回到屏幕中心".to_string());
                self.mark_dirty();
            }
            TrayCommand::ToggleOutline => {
                let current = self
                    .config
                    .overrides
                    .outline
                    .unwrap_or_else(|| self.config.effective().outline.enabled);
                self.config.overrides.outline = Some(!current);
                self.mark_dirty();
            }
            TrayCommand::ToggleVisible => {
                self.config.visible = !self.config.visible;
                self.mark_dirty();
            }
            TrayCommand::ToggleHotkeys => {
                self.config.hotkeys_enabled = !self.config.hotkeys_enabled;
                self.hotkeys_dirty = true;
                self.status = Some(if self.config.hotkeys_enabled {
                    "全局热键已启用".to_string()
                } else {
                    "全局热键已关闭".to_string()
                });
                self.mark_dirty();
            }
            TrayCommand::SelectMonitor(index) => {
                self.config.monitor = MonitorSel::from_index(index);
                self.rebuild_pending = true;
                self.mark_dirty();
            }
            TrayCommand::OpenSettings => {
                self.settings_pending = true;
            }
            TrayCommand::ToggleAutostart => {
                let result = if crate::platform::autostart::is_enabled() {
                    crate::platform::autostart::disable()
                } else {
                    match crate::platform::exe_path() {
                        Some(exe) => {
                            crate::platform::autostart::enable(&exe, Some(&self.config_path))
                        }
                        None => Err("取不到程序路径".to_string()),
                    }
                };
                self.status = Some(match result {
                    Ok(()) => format!(
                        "开机自启：{}",
                        if crate::platform::autostart::is_enabled() {
                            "已开启"
                        } else {
                            "已关闭"
                        }
                    ),
                    Err(err) => format!("设置开机自启失败：{err}"),
                });
            }
            TrayCommand::OpenConfig => {
                let _ = std::process::Command::new("notepad")
                    .arg(&self.config_path)
                    .spawn();
            }
            TrayCommand::ResetOverrides => {
                self.config.overrides = Overrides::default();
                self.status = Some("已清除临时调整".to_string());
                self.mark_dirty();
            }
            TrayCommand::Reload => {
                self.reload_pending = true;
            }
            TrayCommand::Quit => {
                self.quit = true;
            }
        }
    }
}

/// 状态栏 / 托盘提示里显示当前偏移。
///
/// 四舍五入到整数像素：热键和菜单都是整数步长，显示 `+3` 比 `+3.000001` 可信。
fn format_offset(off: [f32; 2]) -> String {
    format!(
        "位置 水平{:+} · 垂直{:+}",
        off[0].round() as i32,
        off[1].round() as i32
    )
}

/// 按当前配置注册全局热键，并把失败项写进状态栏。
///
/// 失败不能只 `eprintln!`：用户看不到控制台，表现就是「某个热键按了没反应」，
/// 而这正是之前 `Ctrl+Alt+D` 排查了半天的那类现象。写进 `status` 后，
/// 设置界面和托盘提示里都能直接看到是哪几个组合被别的程序占了。
fn register_hotkeys(
    hwnd: windows_sys::Win32::Foundation::HWND,
    shared: &Arc<Mutex<Shared>>,
) -> Hotkeys {
    let defs = match shared.lock() {
        Ok(s) => hotkeys::resolve(&s.config),
        Err(_) => Vec::new(),
    };
    let hotkeys = Hotkeys::register(hwnd, &defs);
    if let Some(summary) = hotkeys.failure_summary() {
        eprintln!("{summary}");
        if let Ok(mut s) = shared.lock() {
            s.status = Some(summary);
        }
    }
    hotkeys
}

/// 主线程独占的部分。
pub struct App {
    shared: Arc<Mutex<Shared>>,
    overlay: Overlay,
    msg_window: MessageWindow,
    tray: Tray,
    /// 托盘/窗口图标。必须比 `tray` 活得久（`tray` 只借用句柄）。
    _icon: Option<Icon>,
    /// 已注册的热键。改键 / 开关时需要先 `drop`（注销）再重建。
    hotkeys: Hotkeys,
    /// 上一帧是否因「捕获新键位」把所有热键挂起了。
    ///
    /// 退出捕获时**必须**重注册，哪怕期间一次改键都没发生（例如按 Esc 取消）
    /// ——否则热键会静悄悄地永久失效。
    hotkeys_suspended: bool,
    monitor: MonitorInfo,
    /// 复用的画布，避免每次重绘都分配几十 MB
    pixmap: Option<tiny_skia::Pixmap>,
    /// 上一轮 tick 时设置窗口是否开着（用来只在状态翻转时改 Z 序）
    settings_was_open: bool,
}

impl App {
    /// 启动：建窗口、注册热键与托盘，然后进入消息循环直到退出。
    pub fn run(config_override: Option<PathBuf>) -> Result<(), String> {
        crate::platform::enable_per_monitor_dpi();

        let config_path = config::resolve_path(config_override);
        config::ensure_exists(&config_path)?;
        let (config, load_err) = config::load_or_default(&config_path);
        let file_hash = config::file_hash(&config_path);

        let monitor =
            monitor::pick_sel(&config.monitor).ok_or_else(|| "没有检测到任何显示器".to_string())?;

        let shared = Arc::new(Mutex::new(Shared {
            config,
            config_path,
            file_hash,
            dirty: true,
            needs_save: false,
            quit: false,
            tray_pending: false,
            tray_reattach: false,
            reload_pending: false,
            settings_pending: false,
            rebuild_pending: false,
            hotkeys_dirty: false,
            hotkeys_suspend: false,
            status: load_err,
            // 启动时**不**联网：一个常驻覆盖层不该在每次开机时都发一次外部请求。
            // 用户点到「关于」里的「检查更新」按钮才会真的出去查。
            update: UpdateState::default(),
        }));

        // 覆盖层：只处理显示设置变化
        let overlay_shared = Arc::clone(&shared);
        let rect = monitor.rect;
        let overlay = Overlay::new(
            rect.left,
            rect.top,
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
            Box::new(move |msg, _wp, _lp| {
                if msg == WM_DISPLAYCHANGE {
                    if let Ok(mut s) = overlay_shared.lock() {
                        s.rebuild_pending = true;
                    }
                    return Some(0);
                }
                None
            }),
        )?;

        // 消息窗口：热键、托盘、定时器
        let msg_shared = Arc::clone(&shared);
        // 必须在建窗口之前取：这个值要搬进回调闭包里
        let taskbar_created = taskbar_created_message();
        let msg_window = MessageWindow::new(Box::new(move |msg, wp, _lp| {
            let mut s = match msg_shared.lock() {
                Ok(s) => s,
                Err(_) => return Some(0),
            };
            match msg {
                WM_HOTKEY => {
                    if let Some(action) = Hotkeys::action_of(wp as i32) {
                        s.handle_hotkey(action);
                    }
                    Some(0)
                }
                // 托盘图标的回调：`wparam` 是图标 id，`lparam` 才是鼠标消息。
                // 因为没有调用 NIM_SETVERSION，走的是这套「传统」格式；
                // 之前这里错写成直接匹配 WM_RBUTTONUP，于是菜单永远不会弹出。
                WM_TRAY => {
                    let mouse = _lp as u32;
                    if mouse == WM_RBUTTONUP || mouse == WM_LBUTTONUP {
                        s.tray_pending = true;
                    }
                    Some(0)
                }
                WM_TIMER if wp == TIMER_POLL => {
                    let hash = config::file_hash(&s.config_path);
                    if hash != 0 && hash != s.file_hash {
                        s.reload_pending = true;
                    }
                    Some(0)
                }
                // 又有人双击了图标：单实例检测把「被挤掉的那个进程」变成
                // 一条消息打到这里，用户看到的就是设置界面被叫了出来。
                crate::platform::single::WM_SHOW_SETTINGS => {
                    s.settings_pending = true;
                    Some(0)
                }
                // 安装 / 升级前安装程序会喊一声，让常驻覆盖层自己走人。
                // 覆盖层没有任务栏窗口，这是唯一能让用户「不看见它也能关掉它」的路。
                crate::platform::single::WM_EXIT => {
                    s.quit = true;
                    Some(0)
                }
                // 资源管理器（重新）启动：任务栏里的托盘图标已随它一起消失，
                // 必须让主线程重新 NIM_ADD，否则图标永久不见。
                _ if msg == taskbar_created => {
                    s.tray_reattach = true;
                    Some(0)
                }
                _ => None,
            }
        }))?;

        // 托盘图标自己画：没有 .ico 资源时系统给的是通用窗口图标，
        // 在 16px 的托盘里根本认不出是什么程序。
        let app_icon = icon::create_hicon();
        let tray = Tray::new(msg_window.hwnd(), app_icon.as_ref().map(Icon::handle))?;
        let hotkeys = register_hotkeys(msg_window.hwnd(), &shared);

        unsafe {
            SetTimer(msg_window.hwnd(), TIMER_POLL, POLL_MS, None);
        }

        let mut app = App {
            shared: Arc::clone(&shared),
            overlay,
            msg_window,
            tray,
            _icon: app_icon,
            hotkeys,
            hotkeys_suspended: false,
            monitor,
            pixmap: None,
            settings_was_open: false,
        };

        app.overlay.show();
        app.render();
        app.refresh_tip();

        // ---- 消息循环 ----
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if ret == 0 || ret == -1 {
                break;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // 常驻程序最怕「一条消息处理崩了，整个覆盖层直接消失」。
            // tick() 里同时做着重载、落盘、重建窗口、重绘，任何一处 panic
            // 都不该终止进程——把脏标记清掉，下一帧从磁盘状态重新开始即可。
            let alive = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.tick()))
            {
                Ok(alive) => alive,
                Err(_) => {
                    if let Ok(mut s) = app.shared.lock() {
                        s.status = Some("内部错误：本次界面刷新失败，已跳过".to_string());
                        s.reload_pending = false;
                        s.rebuild_pending = false;
                        s.needs_save = false;
                    }
                    true
                }
            };
            if !alive {
                break;
            }
        }
        Ok(())
    }

    fn refresh_tip(&mut self) {
        let s = match self.shared.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let label = presets::label(&s.config.active);
        let text = if s.config.visible {
            format!("ScreenCrosshair · {label}")
        } else {
            format!("ScreenCrosshair · {label}（已隐藏）")
        };
        drop(s);
        self.tray.set_tip(&text);
    }

    /// 处理完一条消息后调用的「事务」：重载、保存、弹菜单、重绘。
    /// 返回 `false` 表示该退出了。
    fn tick(&mut self) -> bool {
        let shared = Arc::clone(&self.shared);

        // ---- 退出 ----
        {
            let s = shared.lock().unwrap();
            if s.quit {
                return false;
            }
        }

        // ---- 任务栏重建（资源管理器重启 / shell 晚于本程序启动）----
        let want_reattach = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.tray_reattach)
        };
        if want_reattach {
            self.tray.reattach();
            // reattach 会把提示文字重置成默认值，这里补回当前预设名。
            self.refresh_tip();
        }

        // ---- 托盘菜单（阻塞，不能持锁）----
        let want_tray = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.tray_pending)
        };
        if want_tray {
            let state = self.build_tray_state();
            // 菜单是模态阻塞的，所以 `popup` 不能在持锁时调用：
            // 设置窗口线程要写的也是同一把锁，持锁弹菜单会把它卡住。
            if let Some(cmd) = self.tray.popup(&state)
                && let Ok(mut s) = shared.lock()
            {
                s.handle_tray(cmd);
            }
        }

        // ---- 设置界面 ----
        let want_settings = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.settings_pending)
        };
        if want_settings {
            crate::settings::spawn(Arc::clone(&shared));
        }

        // 设置窗口开着时把覆盖层降为非置顶，否则准星会直接画在设置界面上
        // （看上去像界面自己画错了）。窗口关闭后恢复置顶。
        // 只在状态翻转时调 SetWindowPos，避免每 400ms 反复动 Z 序去抢别的置顶窗口。
        let settings_open = crate::settings::is_visible();
        if settings_open != self.settings_was_open {
            self.settings_was_open = settings_open;
            self.overlay.set_topmost(!settings_open);
            self.overlay.show();
        }

        // ---- 手动 / 自动重载 ----
        let want_reload = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.reload_pending)
        };
        if want_reload {
            let mut s = shared.lock().unwrap();
            match config::load(&s.config_path) {
                Ok(Some(cfg)) => {
                    s.config = cfg;
                    s.file_hash = config::file_hash(&s.config_path);
                    s.rebuild_pending = true;
                    // 用户可能手改了配置里的 hotkeys，也得重新注册
                    s.hotkeys_dirty = true;
                    s.dirty = true;
                    s.status = Some("已重新载入配置".to_string());
                }
                Ok(None) => {
                    s.status = Some("配置文件不存在，已保留当前设置".to_string());
                }
                Err(err) => {
                    s.status = Some(err);
                }
            }
        }

        // ---- 重建覆盖层 ----
        let want_rebuild = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.rebuild_pending)
        };
        if want_rebuild {
            let sel = {
                let s = shared.lock().unwrap();
                s.config.monitor.clone()
            };
            if let Some(m) = monitor::pick_sel(&sel) {
                let rect = m.rect;
                let (w, h) = (
                    (rect.right - rect.left).max(1) as u32,
                    (rect.bottom - rect.top).max(1) as u32,
                );
                if w != self.monitor.width().max(1) as u32
                    || h != self.monitor.height().max(1) as u32
                    || rect.left != self.monitor.rect.left
                    || rect.top != self.monitor.rect.top
                {
                    if let Err(err) = self.overlay.resize(rect.left, rect.top, w, h) {
                        if let Ok(mut s) = shared.lock() {
                            s.status = Some(err);
                        }
                    } else {
                        self.pixmap = None;
                    }
                }
                self.monitor = m;
                if let Ok(mut s) = shared.lock() {
                    s.dirty = true;
                }
            }
        }

        // ---- 全局热键（改键 / 开关 / 重载后需要重新注册）----
        //
        // 注销必须发生在注册之前：`RegisterHotKey` 对同一个 id 重复注册会
        // 直接失败，而已注册的旧组合也不会自己让出来。所以先显式注销整张表。
        //
        // 设置界面在「捕获新键位」期间会置 `hotkeys_suspend`：被 `RegisterHotKey`
        // 抢走的组合键不再进任何窗口的键盘队列，不挂起的话用户连自己那个旧键
        // 都按不出来（实测被吞成了对应动作）。挂起期间不动 `hotkeys_dirty`，
        // 退出后一起补上。
        let (dirty, suspending) = {
            let s = shared.lock().unwrap();
            (s.hotkeys_dirty, s.hotkeys_suspend)
        };
        // 挂起结束的那一帧无条件重注册：捕获可能只是被 Esc 取消，
        // 期间一次改键都没发生，不能指望 `hotkeys_dirty` 来提醒。
        let want_hotkeys = dirty || (self.hotkeys_suspended && !suspending);
        self.hotkeys_suspended = suspending;
        if suspending {
            // 幂等：已经空表就不再注销一遍
            if self.hotkeys.ok_count() > 0 {
                self.hotkeys.unregister_all();
            }
        } else if want_hotkeys {
            if let Ok(mut s) = shared.lock() {
                s.hotkeys_dirty = false;
            }
            self.hotkeys.unregister_all();
            self.hotkeys = register_hotkeys(self.msg_window.hwnd(), &shared);
            let n = self.hotkeys.ok_count();
            let total = self.hotkeys.attempted();
            if let Ok(mut s) = shared.lock()
                && s.status.is_none()
            {
                s.status = Some(if total == 0 {
                    "全局热键已关闭".to_string()
                } else {
                    format!("已注册 {n} / {total} 个热键")
                });
            }
        }

        // ---- 保存 ----
        let want_save = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.needs_save)
        };
        if want_save {
            let mut s = shared.lock().unwrap();
            match config::save(&s.config_path, &s.config) {
                Ok(hash) => s.file_hash = hash,
                Err(err) => s.status = Some(err),
            }
        }

        // ---- 重绘 ----
        let want_render = {
            let mut s = shared.lock().unwrap();
            std::mem::take(&mut s.dirty)
        };
        if want_render {
            self.render();
            self.refresh_tip();
        }

        true
    }

    fn build_tray_state(&self) -> TrayState {
        let s = self.shared.lock().unwrap();
        let monitors: Vec<(usize, String)> = monitor::enumerate()
            .into_iter()
            .enumerate()
            .map(|(i, m)| (i, m.describe()))
            .collect();
        let spec = s.config.effective();
        TrayState {
            visible: s.config.visible,
            active_preset: s.config.active.clone(),
            presets: s.preset_items(),
            color: s
                .config
                .overrides
                .color
                .clone()
                .unwrap_or_else(|| spec.color.clone()),
            color_palette: crate::color::PALETTE.to_vec(),
            opacity: spec.opacity,
            scale: spec.scale,
            offset: s.config.offset_px(),
            outline_enabled: spec.outline.enabled,
            monitors,
            monitor: s.config.monitor.index(),
            autostart: crate::platform::autostart::is_enabled(),
            hotkeys_enabled: s.config.hotkeys_enabled,
        }
    }

    /// 按当前配置把准星画进位图并上传给系统合成。
    fn render(&mut self) {
        let (spec, visible) = match self.shared.lock() {
            Ok(s) => (s.config.effective(), s.config.visible),
            Err(_) => return,
        };

        if !visible {
            let _ = self.overlay.clear();
            return;
        }

        let (w, h) = self.overlay.size();
        let needs_new = match &self.pixmap {
            Some(p) => p.width() != w || p.height() != h,
            None => true,
        };
        if needs_new {
            match tiny_skia::Pixmap::new(w, h) {
                Some(p) => self.pixmap = Some(p),
                None => {
                    if let Ok(mut s) = self.shared.lock() {
                        s.status = Some("分配画布失败".to_string());
                    }
                    return;
                }
            }
        }
        let pixmap = match self.pixmap.as_mut() {
            Some(p) => p,
            None => return,
        };
        pixmap.fill(tiny_skia::Color::TRANSPARENT);

        let dpi_scale = self.monitor.scale();
        let center = (w as f32 / 2.0, h as f32 / 2.0);
        crosshair::draw(pixmap, &spec, center, dpi_scale);

        let mut data = pixmap.data().to_vec();
        crosshair::rgba_to_bgra_premultiplied(&mut data);
        if let Err(err) = self.overlay.upload(&data)
            && let Ok(mut s) = self.shared.lock()
        {
            s.status = Some(err);
        }
    }

    /// 供设置界面查询的共享句柄。
    pub fn shared(&self) -> Arc<Mutex<Shared>> {
        Arc::clone(&self.shared)
    }
}
