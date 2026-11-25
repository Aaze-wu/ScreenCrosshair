//! 内置设置界面（eframe / egui），运行在独立线程里。
//!
//! 它和覆盖层共享同一个 [`Shared`]：界面上任何改动都只写进 `Shared`
//! 的配置并置 `dirty`，重绘与落盘由主消息循环负责。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::app::Shared;
use crate::color;
use crate::config::{CrosshairSpec, MonitorSel};
use crate::crosshair;
use crate::platform::hotkeys;
use crate::presets;

/// 设置窗口是否已经创建过。
///
/// **必须记住这件事，因为 winit 的事件循环一个进程只能创建一次**
/// （`EventLoop::new` 内部是个一次性开关，第二次直接返回
/// `EventLoopError::Recreation`）。所以设置窗口一旦被销毁，
/// 就再也建不出新的来了——表现为「第一次能打开，关掉以后
/// 热键、托盘菜单全都没反应，只能重启程序」。
///
/// 对策：窗口永远不销毁，只隐藏。再次打开时把同一个窗口
/// 重新显示出来（见 [`spawn`]）。
static CREATED: AtomicBool = AtomicBool::new(false);

/// 用户把窗口藏起来了（点「关闭窗口」或标题栏 X）。
///
/// 注意这里不能只用一个「可见」布尔：`ui()` 每帧都会跑，如果它每帧写
/// 「我现在可见」，那么手工藏起来之后只要来一帧重绘就会把状态翻回去，
/// 覆盖层跟着错误地取消置顶。所以可见性改由两个只增不减的**原因**推导。
static HIDDEN: AtomicBool = AtomicBool::new(false);

/// 窗口被最小化了（任务栏里点的最小化）。最小化时屏幕上看不到它，
/// 覆盖层也该恢复置顶，否则准星会被一个看不见的窗口「压」在下面。
static MINIMIZED: AtomicBool = AtomicBool::new(false);

/// 常驻线程里那个 egui 上下文。`egui::Context` 是 `Send + Sync`，
/// 主线程可以拿它直接发视口命令把隐藏的窗口叫回来，
/// 不需要额外造一套跨线程唤醒机制。
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

/// 由三个标志推导「设置窗口是否真的占着屏幕」。
///
/// `created` 必须参与判断：`HIDDEN` / `MINIMIZED` 的初值都是 `false`，
/// 光看这两个的话，**还没创建过窗口时**函数就返回 `true`，主循环会在启动后
/// 第一帧就误判「设置界面开着」而取消覆盖层置顶 —— 表现为准星被别的窗口盖住、
/// 只有开关过一次设置窗口才恢复。抽成纯函数也方便单测这条边界。
fn visible_from_state(created: bool, hidden: bool, minimized: bool) -> bool {
    created && !hidden && !minimized
}

/// 设置窗口是否真的占着屏幕。主循环靠它决定要不要给覆盖层让位（取消置顶）。
pub fn is_visible() -> bool {
    visible_from_state(
        CREATED.load(Ordering::SeqCst),
        HIDDEN.load(Ordering::SeqCst),
        MINIMIZED.load(Ordering::SeqCst),
    )
}

/// 把窗口藏起来：不销毁，只是不可见，随时能被 [`spawn`] 叫回来。
fn hide(ctx: &egui::Context) {
    HIDDEN.store(true, Ordering::SeqCst);
    MINIMIZED.store(false, Ordering::SeqCst);
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}

/// 窗口图标（标题栏 / Alt+Tab / 任务栏）。
///
/// 和托盘、文件图标共用同一张自绘准星图，只是这里用 64px——标题栏和
/// Alt+Tab 会把图放大，32px 看着发虚。
fn window_icon() -> std::sync::Arc<egui::IconData> {
    let size = crate::platform::icon::WINDOW_ICON_SIZE;
    std::sync::Arc::new(egui::IconData {
        rgba: crate::platform::icon::app_icon_rgba(size),
        width: size,
        height: size,
    })
}

/// 后台打开设置窗口；已经开过则把原有窗口重新显示出来。
pub fn spawn(shared: Arc<Mutex<Shared>>) {
    if CREATED.swap(true, Ordering::SeqCst) {
        // 窗口之前建过：事件循环不可能重建，只能把已有窗口亮出来。
        let ctx = CTX.lock().ok().and_then(|c| c.clone());
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // 窗口隐藏期间不会自己刷新，得踢一帧让上面几条命令生效。
            ctx.request_repaint();
            HIDDEN.store(false, Ordering::SeqCst);
            MINIMIZED.store(false, Ordering::SeqCst);
        }
        return;
    }
    // 失败时要把原因写回状态栏：release 构建没有控制台，eprintln 谁也看不到。
    let report = Arc::clone(&shared);
    std::thread::spawn(move || {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([960.0, 700.0])
                .with_min_inner_size([760.0, 540.0])
                .with_title("ScreenCrosshair 设置")
                .with_icon(window_icon()),
            // 主线程被覆盖层的 Win32 消息循环占着，winit 默认拒绝在主线程之外
            // 创建事件循环（会直接 panic），这里显式放行。
            event_loop_builder: Some(Box::new(|builder| {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };
        let result = eframe::run_native(
            "ScreenCrosshair 设置",
            options,
            Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, shared)))),
        );
        // 走到这里说明窗口真的没了（正常路径下窗口只会隐藏，不会走到这里）。
        // 复位状态：万一还有救就允许用户再试一次；救不了会显示失败原因。
        CREATED.store(false, Ordering::SeqCst);
        HIDDEN.store(false, Ordering::SeqCst);
        MINIMIZED.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = CTX.lock() {
            *slot = None;
        }
        if let Err(err) = result {
            eprintln!("设置界面启动失败：{err}");
            if let Ok(mut s) = report.lock() {
                s.status = Some(format!("设置界面启动失败：{err}"));
            }
        }
    });
}

struct SettingsApp {
    shared: Arc<Mutex<Shared>>,
    /// 自定义图元 JSON
    script: String,
    script_error: Option<String>,
    editing_name: String,
    /// 预览纹理与其「指纹」
    texture: Option<egui::TextureHandle>,
    texture_key: String,
    /// 颜色输入框（`None` 表示还没初始化，跟随当前准星颜色）
    hex_input: Option<String>,
    outline_hex_input: Option<String>,
    toast: Option<String>,
    /// 中文字体加载失败时的提示（成功则为 `None`）
    font_note: Option<String>,
    /// 底部脚本面板的高度（逻辑像素），由顶部拖拽条调节。
    ///
    /// 显式保存而不是交给 egui 的 `resizable`：内置手柄放在这里完全拖不动
    /// （面板高度靠内容撑、内容又反过来读可用高度，egui 的抓取区也只剩
    /// 工具栏那 20px），已经用真实鼠标拖拽反证过。
    script_height: f32,
    /// 正在等待用户按新键位的热键 id（`None` = 没在捕获）。
    capturing: Option<i32>,
}

/// 脚本面板的默认高度。
pub(crate) const SCRIPT_DEFAULT_HEIGHT: f32 = 240.0;

/// 把脚本面板高度约束到「看得见、不过分占屏」的范围。
///
/// 抽成纯函数是因为它必须可测：Compose/egui 布局在单测里跑不起来，
/// 而「拖到底后面板没影了」或「拖到顶把预览区挤没」都是这行 clamp 的责任。
///
/// * 下限 96：拖拽条(7) + 工具栏一行 + 两行编辑器。再小就只剩个标题，
///   拖进去就再也拖不回来（看不到抓取条）。
/// * 上限 = 窗口高的 75%：永远给中间的预览区留一席之地。
fn clamp_script_height(h: f32, window_h: f32) -> f32 {
    const MIN_H: f32 = 96.0;
    let max_h = (window_h * 0.75).max(MIN_H);
    if h.is_nan() {
        return SCRIPT_DEFAULT_HEIGHT.clamp(MIN_H, max_h);
    }
    h.clamp(MIN_H, max_h)
}

/// 脚本面板顶部的自绘拖拽条，返回「双击恢复默认高度」是否被触发。
///
/// 两个设计要点：
/// * **命中区占满整行宽度**，视觉上只画中间一小段横杠。原来 egui 内置手柄的
///   抓取区只有工具栏那一行高，实测怎么拖都抓不住；
/// * **占用的是面板内的第一行**，而不是画在面板外。画在外面会和顶栏/预览区
///   抢输入，而面板内的第一行天然跟着面板走，面板被拖高时它一直在同一个地方。
fn drag_handle(ui: &mut egui::Ui, height: &mut f32, window_h: f32) -> bool {
    const ROW_H: f32 = 10.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::click_and_drag(),
    );
    let bar_w = (rect.width() * 0.3).clamp(48.0, 240.0);
    let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(bar_w, 3.0));
    let col = if resp.dragged() {
        ui.visuals().widgets.active.fg_stroke.color
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    ui.painter().rect_filled(bar, 1.5, col);

    if resp.dragged() {
        // `drag_delta()` 是「本帧鼠标位移」。面板向上长高时鼠标的屏幕坐标并不变，
        // 所以这里用绝对位移即可，不需要记住按下时的锚点。
        *height = clamp_script_height(*height - resp.drag_delta().y, window_h);
        // 面板改高会让整个布局重排，必须主动请求重绘，否则要等下一次输入才刷新。
        ui.ctx().request_repaint();
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    resp.double_clicked()
}

/// egui 的逻辑按键 -> Windows 虚拟键码（`VK_*`）。
///
/// 抽成纯函数是为了可测：捕获热键的逻辑在 egui 里跑不起来，但「按下的键对应
/// 哪个 VK」是一次查表，错了的后果是「改完键后新键按了没反应」，而这条路径
/// 在界面上一眼看不出来。返回 `None` 表示这个键不支持当热键。
fn vk_of_key(key: egui::Key) -> Option<u32> {
    use egui::Key as K;
    Some(match key {
        K::A => 0x41,
        K::B => 0x42,
        K::C => 0x43,
        K::D => 0x44,
        K::E => 0x45,
        K::F => 0x46,
        K::G => 0x47,
        K::H => 0x48,
        K::I => 0x49,
        K::J => 0x4A,
        K::K => 0x4B,
        K::L => 0x4C,
        K::M => 0x4D,
        K::N => 0x4E,
        K::O => 0x4F,
        K::P => 0x50,
        K::Q => 0x51,
        K::R => 0x52,
        K::S => 0x53,
        K::T => 0x54,
        K::U => 0x55,
        K::V => 0x56,
        K::W => 0x57,
        K::X => 0x58,
        K::Y => 0x59,
        K::Z => 0x5A,
        K::Num0 => 0x30,
        K::Num1 => 0x31,
        K::Num2 => 0x32,
        K::Num3 => 0x33,
        K::Num4 => 0x34,
        K::Num5 => 0x35,
        K::Num6 => 0x36,
        K::Num7 => 0x37,
        K::Num8 => 0x38,
        K::Num9 => 0x39,
        K::F1 => 0x70,
        K::F2 => 0x71,
        K::F3 => 0x72,
        K::F4 => 0x73,
        K::F5 => 0x74,
        K::F6 => 0x75,
        K::F7 => 0x76,
        K::F8 => 0x77,
        K::F9 => 0x78,
        K::F10 => 0x79,
        K::F11 => 0x7A,
        K::F12 => 0x7B,
        K::ArrowLeft => 0x25,
        K::ArrowUp => 0x26,
        K::ArrowRight => 0x27,
        K::ArrowDown => 0x28,
        K::Space => 0x20,
        K::Enter => 0x0D,
        K::Tab => 0x09,
        K::Backspace => 0x08,
        K::Insert => 0x2D,
        K::Delete => 0x2E,
        K::Home => 0x24,
        K::End => 0x23,
        K::PageUp => 0x21,
        K::PageDown => 0x22,
        K::Minus => 0xBD,
        K::Equals => 0xBB,
        K::Comma => 0xBC,
        K::Period => 0xBE,
        K::Slash => 0xBF,
        K::Semicolon => 0xBA,
        K::Quote => 0xDE,
        K::OpenBracket => 0xDB,
        K::CloseBracket => 0xDD,
        K::Backslash => 0xDC,
        K::Backtick => 0xC0,
        // 修饰键本身当热键没有意义，其余也不映射
        _ => return None,
    })
}

/// egui 自带字体不含任何中日韩字形，不额外装字体的话整个界面都是「豆腐块」。
///
/// 中文字体只追加到候选列表末尾：拉丁字母仍由 egui 自带的 Ubuntu-Light 渲染
/// （字形更协调），只在中文字符缺字形时才回退到这里。
fn install_cjk_font(ctx: &egui::Context) -> Option<String> {
    /// (文件名, 字体面序号)。`.ttc` 是字体集合，必须挑一个面。
    const CANDIDATES: &[(&str, u32)] = &[
        ("msyh.ttc", 0),   // 微软雅黑（Win7+ 默认中文界面字体）
        ("Deng.ttf", 0),   // 等线（Win10+）
        ("simhei.ttf", 0), // 黑体
        ("msyh.ttf", 0),   // 极少数精简系统里是单字体文件
        ("simsun.ttc", 0), // 宋体（兜底）
        ("msjh.ttc", 0),   // 微软正黑（繁体系统）
    ];

    let font_dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join("Fonts");

    for (file, index) in CANDIDATES {
        let path = font_dir.join(file);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let mut data = egui::FontData::from_owned(bytes);
        data.index = *index;

        let mut defs = egui::FontDefinitions::default();
        defs.font_data
            .insert("cjk".to_owned(), std::sync::Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            defs.families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        ctx.set_fonts(defs);
        return None;
    }

    Some(format!(
        "未找到中文字体（{}），界面里的中文会显示成方块。",
        font_dir.display()
    ))
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Shared>>) -> Self {
        let font_note = install_cjk_font(&cc.egui_ctx);
        // 把上下文挂到全局：主线程要靠它把隐藏的窗口重新亮出来。
        if let Ok(mut slot) = CTX.lock() {
            *slot = Some(cc.egui_ctx.clone());
        }
        HIDDEN.store(false, Ordering::SeqCst);
        SettingsApp {
            shared,
            script: String::new(),
            script_error: None,
            editing_name: String::new(),
            texture: None,
            texture_key: String::new(),
            hex_input: None,
            outline_hex_input: None,
            toast: None,
            font_note,
            script_height: SCRIPT_DEFAULT_HEIGHT,
            capturing: None,
        }
    }

    fn mutate(&mut self, f: impl FnOnce(&mut Shared)) {
        if let Ok(mut s) = self.shared.lock() {
            f(&mut s);
            s.mark_dirty();
        }
    }

    /// 进入 / 退出「改键捕获」状态，同时请求主线程挂起全部热键。
    ///
    /// 挂起是必须的：`RegisterHotKey` 抢走的组合键**不会再进任何窗口的键盘队列**，
    /// 所以捕获态里按下一个已被本项目占用的组合，设置窗口连事件都收不到——
    /// 包括用户想换掉的那个旧键本身。实测按 `Ctrl+Alt+Shift+S` 不进捕获态，
    /// 而是直接触发了「下移 10px」。所以捕获期间要先注销热键，退出后重新注册。
    ///
    /// 这里**不**走 `mutate`：挂起标志不是配置，没必要触发一次落盘。
    fn set_capturing(&mut self, id: Option<i32>) {
        self.capturing = id;
        if let Ok(mut s) = self.shared.lock() {
            s.hotkeys_suspend = id.is_some();
        }
    }

    fn config(&self) -> crate::config::Config {
        self.shared
            .lock()
            .map(|s| s.config.clone())
            .unwrap_or_default()
    }

    fn status(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.status.clone())
    }

    /// 仅在 spec 变化时重建预览纹理。
    fn ensure_texture(&mut self, ctx: &egui::Context, spec: &CrosshairSpec) {
        // 预览只反映「长什么样」，不反映「摆在哪」：偏移是相对整个屏幕的位置，
        // 在 200px 的小方块里按原值画会直接把准星推出画布，看着像渲染坏了。
        // 把偏移抹平后也能让拖动位置时不反复重建纹理。
        let mut preview = spec.clone();
        preview.offset = [0.0, 0.0];
        let key = serde_json::to_string(&preview).unwrap_or_default();
        if key == self.texture_key && self.texture.is_some() {
            return;
        }
        const SIZE: u32 = 200;
        let mut data = crosshair::render_preview(&preview, SIZE, 1.0);
        crosshair::unpremultiply(&mut data);
        let image = egui::ColorImage::from_rgba_unmultiplied([SIZE as usize, SIZE as usize], &data);
        self.texture = Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
        self.texture_key = key;
    }

    /// 「全局热键」区块：总开关 + 每个动作一行（改键 / 恢复默认）。
    ///
    /// 捕获态必须在绘制任何控件之前处理：这段代码一旦排到后面，键盘事件会先被
    /// TextEdit 之类的控件吃掉，热键就永远捕获不到（表现为「点了改键没反应」）。
    fn hotkeys_ui(&mut self, ui: &mut egui::Ui, config: &crate::config::Config) {
        use crate::config::HotkeyCombo;

        // ---- 捕获新键位 ----
        if let Some(id) = self.capturing {
            let mut captured: Option<HotkeyCombo> = None;
            let mut cancel = false;
            ui.ctx().input(|i| {
                for ev in &i.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = ev
                    {
                        if *key == egui::Key::Escape {
                            cancel = true;
                        } else if let Some(vk) = vk_of_key(*key) {
                            captured = Some(HotkeyCombo::new(
                                modifiers.ctrl,
                                modifiers.alt,
                                modifiers.shift,
                                vk,
                            ));
                        }
                    }
                }
            });
            if cancel {
                self.set_capturing(None);
                self.toast = Some("已取消改键".to_string());
            } else if let Some(combo) = captured {
                if combo.is_usable() {
                    // 结束捕获、恢复热键、提交新键位必须在**同一把锁**里做完：
                    // 若先单独置 `hotkeys_suspend = false`，主线程可能抢先按
                    // 旧配置注册一轮，然后再按新配置注册一轮。
                    self.capturing = None;
                    let label = combo.label();
                    let mut msg = String::new();
                    self.mutate(|s| {
                        s.hotkeys_suspend = false;
                        let bumped = s.config.set_hotkey(id, combo);
                        s.hotkeys_dirty = true;
                        msg = if bumped.is_empty() {
                            format!("热键已改为 {label}")
                        } else {
                            format!(
                                "热键已改为 {label}（同时取消了 {} 个冲突的旧绑定）",
                                bumped.len()
                            )
                        };
                    });
                    self.toast = Some(msg);
                } else {
                    // `RegisterHotKey` 允许裸键，但那会把普通打字变成全局热键
                    self.toast = Some("至少要按住 Ctrl / Alt / Shift 中的一个".to_string());
                }
            }
        }

        ui.separator();
        ui.heading("全局热键");

        let mut enabled = config.hotkeys_enabled;
        if ui.checkbox(&mut enabled, "启用全局热键").changed() {
            self.mutate(|s| {
                s.config.hotkeys_enabled = enabled;
                s.hotkeys_dirty = true;
            });
            self.toast = Some(
                if enabled {
                    "全局热键已启用"
                } else {
                    "全局热键已关闭"
                }
                .to_string(),
            );
        }
        ui.label(
            egui::RichText::new("关闭后所有组合键都不再响应（托盘菜单里也能切换）")
                .small()
                .weak(),
        );

        // 关闭状态下仍然把列表画出来（置灰）：藏起来会让用户以为「设置丢了」
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(id) = self.capturing {
                let desc = hotkeys::rows(config)
                    .into_iter()
                    .find(|r| r.id == id)
                    .map(|r| r.action.describe())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("正在为「{desc}」捕获新键位：请按下组合键（Esc 取消）"),
                    );
                });
            }

            for c in hotkeys::conflicts(config) {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{} 被多个动作占用（只有排在前面的那个生效）", c.label()),
                );
            }

            let rows = hotkeys::rows(config);
            let mut edit: Option<i32> = None;
            let mut reset: Option<i32> = None;
            egui::Grid::new("hotkey_grid")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for row in &rows {
                        ui.label(row.action.describe());
                        if row.customized() {
                            // 改过的键位用高亮色，一眼看出哪些不是默认值
                            ui.colored_label(ui.visuals().hyperlink_color, row.combo.label());
                        } else {
                            ui.label(row.combo.label());
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("改键").clicked() {
                                edit = Some(row.id);
                            }
                            if row.customized() {
                                if ui.small_button("恢复默认").clicked() {
                                    reset = Some(row.id);
                                }
                            } else {
                                ui.add_enabled(false, egui::Button::new("默认").small());
                            }
                        });
                        ui.end_row();
                    }
                });
            if let Some(id) = edit {
                self.set_capturing(Some(id));
            }
            if let Some(id) = reset {
                self.mutate(|s| {
                    s.config.reset_hotkey(id);
                    s.hotkeys_dirty = true;
                });
                self.toast = Some("已恢复该热键的默认键位".to_string());
            }

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        config.has_custom_hotkeys(),
                        egui::Button::new("全部恢复默认"),
                    )
                    .clicked()
                {
                    self.mutate(|s| {
                        s.config.reset_all_hotkeys();
                        s.hotkeys_dirty = true;
                    });
                    self.set_capturing(None);
                    self.toast = Some("所有热键已恢复默认键位".to_string());
                }
                ui.label(
                    egui::RichText::new("点「改键」后按下新的组合；同一个组合只保留一个动作")
                        .small()
                        .weak(),
                );
            });
        });
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 能跑到这里就说明窗口没被 X 销毁。记下最小化状态：
        // 最小化时窗口看不见，覆盖层该恢复置顶。
        let minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        MINIMIZED.store(minimized, Ordering::SeqCst);
        // 界面看不见了就不能还揣着「正在捕获」：热键一直挂着，用户会以为
        // 整个程序的快捷键都坏了（捕获提示也看不到，根本不知道发生了什么事）。
        if minimized && self.capturing.is_some() {
            self.set_capturing(None);
        }
        // 标题栏的 X 也要拦下来：销毁窗口 = 设置界面再也打不开
        // （winit 的事件循环不可重建），所以「关闭」一律降级为「隐藏」。
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.set_capturing(None);
            hide(&ctx);
        }

        let config = self.config();
        let spec = config.effective();
        let status = self.status();
        self.ensure_texture(&ctx, &spec);

        if self.hex_input.is_none() {
            self.hex_input = Some(spec.color.clone());
        }
        if self.outline_hex_input.is_none() {
            self.outline_hex_input = Some(spec.outline.color.clone());
        }
        if self.script.trim().is_empty() {
            self.script = serde_json::to_string_pretty(&spec).unwrap_or_default();
        }

        // ---------------- 顶栏 ----------------
        egui::Panel::top(egui::Id::new("top")).show(ui, |ui| {
            // 顶栏分两行：标题和关闭按钮一行，预设名和切换按钮一行。
            //
            // 原来全挤在一个 `horizontal` 里，中文标题（雅黑字号比 egui 自带字体大）
            // 加上后面一串按钮后就超出窗口宽度，而 `horizontal` 不会换行，
            // 结果是右半边被面板边缘硬裁掉——窗口一窄就看不见「关闭窗口」。
            ui.horizontal(|ui| {
                ui.heading("ScreenCrosshair 设置");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("关闭窗口").clicked() {
                        // 只是隐藏，不是销毁——见 `CREATED` 的注释。
                        self.set_capturing(None);
                        hide(&ctx);
                    }
                });
            });
            // `horizontal_wrapped`：预设名可能是用户自取的长名字，允许换行而不是被裁掉。
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("当前预设：{}", presets::label(&config.active)));
                ui.separator();
                if ui.button("下一预设").clicked() {
                    self.mutate(|s| {
                        s.config.cycle(1);
                    });
                }
                if ui.button("上一预设").clicked() {
                    self.mutate(|s| {
                        s.config.cycle(-1);
                    });
                }
            });
            if let Some(text) = self.toast.clone().or(status) {
                ui.label(egui::RichText::new(text).small().weak());
            }
            if let Some(note) = self.font_note.clone() {
                ui.label(
                    egui::RichText::new(note)
                        .small()
                        .color(egui::Color32::from_rgb(220, 120, 120)),
                );
            }
            ui.add_space(2.0);
        });

        // ---------------- 底部：自定义图元脚本 ----------------
        //
        // 高度完全由 `script_height` 决定（不再用 `.resizable(true)`，实测那个
        // 内置手柄在这里拖不动）：面板高度原本靠内容撑，内容里的 ScrollArea
        // 又反过来读 `available_height()`，于是「谁定谁」成了循环依赖；
        // egui 的抓取区也只剩 `horizontal_wrapped` 工具栏那 20px 高。
        // 现在高度是显式量，拖拽条自绘，`max_height` 也一并去掉。
        let window_h = ctx.viewport_rect().height();
        self.script_height = clamp_script_height(self.script_height, window_h);
        let mut want_reset_height = false;
        egui::Panel::bottom(egui::Id::new("script"))
            .exact_size(self.script_height)
            .show(ui, |ui| {
                want_reset_height = drag_handle(ui, &mut self.script_height, window_h);
                // 工具按钮可能多到一行放不下，允许换行；不换行会被面板右边缘裁掉。
                ui.horizontal_wrapped(|ui| {
                    ui.strong("自定义准星脚本（JSON 图元）");
                    if ui.button("载入当前预设").clicked() {
                        match serde_json::to_string_pretty(&spec) {
                            Ok(text) => {
                                self.script = text;
                                self.script_error = None;
                            }
                            Err(err) => self.script_error = Some(err.to_string()),
                        }
                    }
                    if ui.button("插入示例").clicked() {
                        self.script = example_script();
                        self.script_error = None;
                    }
                    if ui.button("校验").clicked() {
                        self.script_error =
                            match serde_json::from_str::<CrosshairSpec>(&self.script) {
                                Ok(_) => None,
                                Err(err) => Some(err.to_string()),
                            };
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.editing_name)
                            .hint_text("预设名")
                            .desired_width(110.0),
                    );
                    if ui.button("另存为自定义预设").clicked() {
                        match serde_json::from_str::<CrosshairSpec>(&self.script) {
                            Ok(mut parsed) => {
                                let name = self.editing_name.trim().to_string();
                                let name = if name.is_empty() {
                                    format!("custom_{}", config.custom_presets.len() + 1)
                                } else {
                                    name
                                };
                                parsed.normalize();
                                self.script_error = None;
                                self.toast = Some(format!("已保存自定义预设「{name}」"));
                                self.mutate(|s| {
                                    s.config.custom_presets.insert(name.clone(), parsed);
                                    s.config.active = name;
                                });
                            }
                            Err(err) => self.script_error = Some(err.to_string()),
                        }
                    }
                    ui.label(
                        egui::RichText::new(format!("图元类型：{}", shape_kinds().join(" / ")))
                            .small()
                            .weak(),
                    );
                });
                if let Some(err) = &self.script_error {
                    ui.colored_label(egui::Color32::from_rgb(230, 90, 90), err);
                }
                // `auto_shrink([false, false])`：必须占满面板剩余高度。
                // 默认的 auto_shrink 会让 ScrollArea 缩到内容高度，内容比面板高时
                // 多出来的部分直接被面板底边裁掉——表现就是编辑器最后一行看不全。
                //
                // 这里不再写 `.max_height(ui.available_height() - 8.0)`：面板高度
                // 已经是显式量，ScrollArea 只要吃掉剩余空间即可；反过来读可用高度
                // 会让「面板高度 ← 内容 ← 可用高度 ← 面板高度」打成一个环。
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.script)
                                .code_editor()
                                .desired_rows(8)
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
        if want_reset_height {
            self.script_height = clamp_script_height(SCRIPT_DEFAULT_HEIGHT, window_h);
        }

        // ---------------- 左侧：预设列表 ----------------
        egui::Panel::left(egui::Id::new("presets"))
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ui.heading("预设");
                ui.add_space(4.0);
                let mut chosen: Option<String> = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ui.available_height() - 90.0)
                    .show(ui, |ui| {
                        for name in config.preset_names() {
                            let custom = config.custom_presets.contains_key(&name);
                            let label = if custom {
                                format!("{}（自定义）", presets::label(&name))
                            } else {
                                presets::label(&name)
                            };
                            let selected = config.active == name;
                            if ui.selectable_label(selected, label).clicked() {
                                chosen = Some(name.clone());
                            }
                        }
                    });
                if let Some(name) = chosen {
                    self.mutate(|s| s.config.active = name);
                }
                ui.separator();
                let is_custom = config.custom_presets.contains_key(&config.active);
                if ui
                    .add_enabled(is_custom, egui::Button::new("删除当前自定义预设"))
                    .clicked()
                {
                    let active = config.active.clone();
                    self.toast = Some(format!("已删除「{active}」"));
                    self.mutate(|s| {
                        s.config.custom_presets.remove(&active);
                        s.config.active = presets::FALLBACK.to_string();
                    });
                }
            });

        // ---------------- 主区域 ----------------
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // ---- 预览 ----
                ui.vertical(|ui| {
                    let size = 200.0;
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 8.0, egui::Color32::from_rgb(34, 34, 38));
                    if let Some(tex) = &self.texture {
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("实际大小 100% DPI 预览").small().weak());
                });

                ui.add_space(16.0);

                // ---- 参数 ----
                ui.vertical(|ui| {
                    // 参数 + 19 行热键列表肯定超出窗口高度，整体放进滚动区，
                    // 否则底部几张控件永远拿不到可见区域（窗口本身不允许变大）。
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().slider_width = 240.0;
                            ui.spacing_mut().item_spacing.y = 6.0;

                            ui.label(egui::RichText::new("颜色").strong());
                            ui.horizontal_wrapped(|ui| {
                                for (label, hex) in color::PALETTE {
                                    let is_current = hex.eq_ignore_ascii_case(&spec.color);
                                    let rgba =
                                        color::parse(hex).unwrap_or(color::Rgba::opaque(0, 255, 0));
                                    let fill = egui::Color32::from_rgb(rgba.r, rgba.g, rgba.b);
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(24.0, 24.0),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().rect_filled(rect, 4.0, fill);
                                    if is_current {
                                        ui.painter().rect_stroke(
                                            rect,
                                            4.0,
                                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if resp.clicked() {
                                        let hex = (*hex).to_string();
                                        self.hex_input = Some(hex.clone());
                                        self.mutate(|s| s.config.overrides.color = Some(hex));
                                    }
                                    resp.on_hover_text(format!("{label}  {hex}"));
                                }
                            });

                            ui.horizontal(|ui| {
                                if let Some(input) = self.hex_input.as_mut() {
                                    ui.add(
                                        egui::TextEdit::singleline(input)
                                            .desired_width(96.0)
                                            .hint_text("#RRGGBB"),
                                    );
                                }
                                if ui.button("应用颜色").clicked() {
                                    let picked = self.hex_input.clone().unwrap_or_default();
                                    match color::parse(&picked) {
                                        Some(rgba) => {
                                            let normalized = rgba.to_hex();
                                            self.hex_input = Some(normalized.clone());
                                            self.toast = None;
                                            self.mutate(|s| {
                                                s.config.overrides.color = Some(normalized)
                                            });
                                        }
                                        None => {
                                            self.toast =
                                                Some(format!("颜色格式无法识别：{picked}"));
                                        }
                                    }
                                }
                                ui.label(
                                    egui::RichText::new("支持 #RGB / #RRGGBB / 具名色")
                                        .small()
                                        .weak(),
                                );
                            });

                            ui.add_space(4.0);
                            ui.separator();

                            let mut opacity = spec.opacity;
                            if ui
                                .add(egui::Slider::new(&mut opacity, 0.1..=1.0).text("不透明度"))
                                .changed()
                            {
                                self.mutate(|s| s.config.overrides.opacity = Some(opacity));
                            }

                            let mut scale = spec.scale;
                            if ui
                                .add(egui::Slider::new(&mut scale, 0.3..=6.0).text("尺寸"))
                                .changed()
                            {
                                self.mutate(|s| s.config.overrides.scale = Some(scale));
                            }

                            let mut rotation = spec.rotation_deg;
                            if ui
                                .add(
                                    egui::Slider::new(&mut rotation, -180.0..=180.0)
                                        .text("旋转（度）"),
                                )
                                .changed()
                            {
                                self.mutate(|s| s.config.overrides.rotation_deg = Some(rotation));
                            }

                            // ---- 位置 ----
                            // 单位是「100% DPI 下的像素」，和预设里的 offset（本地单位、
                            // 跟着 scale 缩放）不是一回事：用户说「往右挪 3 像素」就应该是 3 像素。
                            let offset = config.offset_px();
                            let (mut ox, mut oy) = (offset[0], offset[1]);
                            let mut moved = false;
                            ui.horizontal(|ui| {
                                ui.label("位置");
                                moved |= ui
                                    .add(
                                        egui::DragValue::new(&mut ox)
                                            .speed(1.0)
                                            .range(-1000.0..=1000.0)
                                            .prefix("X "),
                                    )
                                    .changed();
                                moved |= ui
                                    .add(
                                        egui::DragValue::new(&mut oy)
                                            .speed(1.0)
                                            .range(-1000.0..=1000.0)
                                            .prefix("Y "),
                                    )
                                    .changed();
                                if ui.button("回到中心").clicked() {
                                    self.mutate(|s| s.config.reset_offset());
                                }
                            });
                            if moved {
                                self.mutate(|s| {
                                    s.config.set_offset_px([ox, oy]);
                                });
                            }
                            ui.label(
                                egui::RichText::new("相对屏幕中心，单位像素（X 右为正，Y 下为正）")
                                    .small()
                                    .weak(),
                            );

                            ui.separator();

                            let mut outline = spec.outline.enabled;
                            if ui
                                .checkbox(&mut outline, "描边（黑边，提高可见度）")
                                .changed()
                            {
                                self.mutate(|s| s.config.overrides.outline = Some(outline));
                            }
                            if outline {
                                let mut width = spec.outline.width;
                                if ui
                                    .add(egui::Slider::new(&mut width, 0.5..=6.0).text("描边宽度"))
                                    .changed()
                                {
                                    self.mutate(|s| {
                                        s.config.overrides.outline_width = Some(width);
                                    });
                                }

                                ui.horizontal(|ui| {
                                    ui.label("描边颜色");
                                    let mut current = color::parse(&spec.outline.color)
                                        .unwrap_or(color::Rgba::opaque(0, 0, 0));
                                    let mut c32 =
                                        egui::Color32::from_rgb(current.r, current.g, current.b);
                                    if ui.color_edit_button_srgba(&mut c32).changed() {
                                        current = color::Rgba::opaque(c32.r(), c32.g(), c32.b());
                                        let hex = current.to_hex();
                                        self.outline_hex_input = Some(hex.clone());
                                        self.mutate(|s| {
                                            s.config.overrides.outline_color = Some(hex)
                                        });
                                    }
                                    if let Some(input) = self.outline_hex_input.as_mut() {
                                        ui.add(
                                            egui::TextEdit::singleline(input).desired_width(90.0),
                                        );
                                    }
                                    if ui.button("应用").clicked() {
                                        let picked =
                                            self.outline_hex_input.clone().unwrap_or_default();
                                        if let Some(rgba) = color::parse(&picked) {
                                            let hex = rgba.to_hex();
                                            self.outline_hex_input = Some(hex.clone());
                                            self.mutate(|s| {
                                                s.config.overrides.outline_color = Some(hex)
                                            });
                                        } else {
                                            self.toast =
                                                Some(format!("描边颜色无法识别：{picked}"));
                                        }
                                    }
                                });
                            }

                            ui.separator();

                            // ---- 显示器 ----
                            ui.horizontal(|ui| {
                                ui.label("显示器");
                                let monitors = crate::platform::monitor::enumerate();
                                let mut picked: Option<MonitorSel> = None;
                                egui::ComboBox::from_id_salt("monitor_select")
                                    .selected_text(config.monitor.describe())
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                config.monitor.index().is_none(),
                                                "跟随主显示器",
                                            )
                                            .clicked()
                                        {
                                            picked = Some(MonitorSel::default());
                                        }
                                        for (i, m) in monitors.iter().enumerate() {
                                            let is_current = config.monitor.index() == Some(i);
                                            if ui
                                                .selectable_label(is_current, m.describe())
                                                .clicked()
                                            {
                                                picked = Some(MonitorSel::from_index(Some(i)));
                                            }
                                        }
                                    });
                                if let Some(sel) = picked {
                                    self.mutate(|s| {
                                        s.config.monitor = sel;
                                        s.rebuild_pending = true;
                                    });
                                }
                                if ui.button("重新枚举").clicked() {
                                    self.mutate(|s| s.rebuild_pending = true);
                                }
                            });

                            ui.add_space(4.0);
                            let mut visible = config.visible;
                            if ui.checkbox(&mut visible, "显示准星").changed() {
                                self.mutate(|s| s.config.visible = visible);
                            }

                            let mut autostart = crate::platform::autostart::is_enabled();
                            if ui.checkbox(&mut autostart, "开机自启").changed() {
                                let result = if autostart {
                                    match crate::platform::exe_path() {
                                        Some(exe) => {
                                            let cfg = self
                                                .shared
                                                .lock()
                                                .map(|s| s.config_path.clone())
                                                .unwrap_or_default();
                                            crate::platform::autostart::enable(&exe, Some(&cfg))
                                        }
                                        None => Err("取不到程序路径".to_string()),
                                    }
                                } else {
                                    crate::platform::autostart::disable()
                                };
                                self.toast = Some(match result {
                                    Ok(()) => format!(
                                        "开机自启已{}",
                                        if autostart { "开启" } else { "关闭" }
                                    ),
                                    Err(err) => format!("设置开机自启失败：{err}"),
                                });
                            }

                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if ui.button("清除临时调整").clicked() {
                                    self.toast =
                                        Some("已清除颜色/透明度/尺寸的临时调整".to_string());
                                    self.hex_input = None;
                                    self.outline_hex_input = None;
                                    self.mutate(|s| {
                                        s.config.overrides = Default::default();
                                    });
                                }
                                if ui.button("重新载入配置文件").clicked() {
                                    self.mutate(|s| s.reload_pending = true);
                                }
                            });

                            self.hotkeys_ui(ui, &config);
                        });
                });
            });
        });

        // 30fps 足够，不必跑满刷新率
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// `Shape` 的所有变体名，显示在脚本编辑器旁边作为提示。
pub fn shape_kinds() -> Vec<&'static str> {
    vec!["line", "dot", "circle", "arc", "rect", "polygon", "group"]
}

/// 一段最小的可用示例脚本（十字 + 中心点）。
pub fn example_script() -> String {
    let spec = CrosshairSpec::new(
        "#00FF00",
        vec![
            crate::config::Shape::round_line([-14.0, 0.0], [-3.0, 0.0], 2.0),
            crate::config::Shape::round_line([3.0, 0.0], [14.0, 0.0], 2.0),
            crate::config::Shape::round_line([0.0, -14.0], [0.0, -3.0], 2.0),
            crate::config::Shape::round_line([0.0, 3.0], [0.0, 14.0], 2.0),
            crate::config::Shape::dot(1.5),
        ],
    );
    serde_json::to_string_pretty(&spec).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_script_round_trips() {
        let text = example_script();
        let spec: CrosshairSpec = serde_json::from_str(&text).expect("示例脚本必须能解析");
        assert_eq!(spec.shapes.len(), 5);
    }

    #[test]
    fn shape_kinds_cover_all_variants() {
        let kinds = shape_kinds();
        assert_eq!(kinds.len(), 7);
        assert!(kinds.contains(&"group"));
    }

    /// 设置窗口没创建过时绝不能算「占着屏幕」。
    ///
    /// 这条用例锁住的是一个真实缺陷：早先 `is_visible()` 只看隐藏/最小化两个标志，
    /// 它们的初值都是 `false`，于是启动后第一帧就判定「设置界面开着」，
    /// 把覆盖层的 `WS_EX_TOPMOST` 取消掉，准星于是一直被别的窗口压着。
    #[test]
    fn not_created_is_not_visible() {
        assert!(!visible_from_state(false, false, false));
    }

    #[test]
    fn visible_only_when_created_and_neither_hidden_nor_minimized() {
        assert!(visible_from_state(true, false, false));
        assert!(!visible_from_state(true, true, false));
        assert!(!visible_from_state(true, false, true));
        assert!(!visible_from_state(true, true, true));
    }

    /// 拖拽条的高度夹取。
    ///
    /// 下限必须真的能露出编辑器的几行，上限不能把面板顶到整屏（否则上面的
    /// 参数区一点地方都不剩）；`NaN` 必须能自愈——拖动中一旦被污染，
    /// 面板高度会直接变成「不可见」，而用户只能重启程序才能救回来。
    #[test]
    fn script_height_is_clamped_to_sane_range() {
        assert_eq!(clamp_script_height(1000.0, 800.0), 600.0);
        assert_eq!(clamp_script_height(10.0, 800.0), 96.0);
        assert_eq!(clamp_script_height(300.0, 2000.0), 300.0);
        // 窗口矮到 `window_h * 0.75 < MIN_H` 时上下限不能交叉
        assert_eq!(clamp_script_height(10.0, 60.0), 96.0);
        assert_eq!(clamp_script_height(f32::NAN, 800.0), SCRIPT_DEFAULT_HEIGHT);
    }

    /// 键位映射错了的后果是「改完键后新键按了没反应」，而这在界面上
    /// 一眼看不出来（旧键位已经注销、新键位没注册上）。
    #[test]
    fn vk_of_key_maps_letters_digits_arrows_and_fkeys() {
        assert_eq!(vk_of_key(egui::Key::A), Some(0x41));
        assert_eq!(vk_of_key(egui::Key::D), Some(0x44));
        assert_eq!(vk_of_key(egui::Key::Z), Some(0x5A));
        assert_eq!(vk_of_key(egui::Key::Num0), Some(0x30));
        assert_eq!(vk_of_key(egui::Key::Num9), Some(0x39));
        assert_eq!(vk_of_key(egui::Key::F1), Some(0x70));
        assert_eq!(vk_of_key(egui::Key::F12), Some(0x7B));
        assert_eq!(vk_of_key(egui::Key::ArrowLeft), Some(0x25));
        assert_eq!(vk_of_key(egui::Key::ArrowUp), Some(0x26));
        assert_eq!(vk_of_key(egui::Key::ArrowRight), Some(0x27));
        assert_eq!(vk_of_key(egui::Key::ArrowDown), Some(0x28));
        // 修饰键自己当热键没有意义，必须落到 None
        assert_eq!(vk_of_key(egui::Key::Escape), None);
        assert_eq!(vk_of_key(egui::Key::ShiftLeft), None);
        assert_eq!(vk_of_key(egui::Key::ControlLeft), None);
        assert_eq!(vk_of_key(egui::Key::AltLeft), None);
    }

    /// 映射出来的 VK 必须能被 `key_name` 翻译成可读键名，否则界面上显示的
    /// 键名和实际注册的键不是同一个东西。
    #[test]
    fn mapped_vks_have_readable_names() {
        for key in [
            egui::Key::A,
            egui::Key::D,
            egui::Key::Num0,
            egui::Key::Num9,
            egui::Key::F1,
            egui::Key::F12,
            egui::Key::ArrowLeft,
            egui::Key::Space,
        ] {
            let vk = vk_of_key(key).expect("应该能映射");
            let name = hotkeys::key_name(vk);
            assert!(!name.starts_with("VK"), "{key:?} -> {name} 应该是可读键名");
        }
    }
}
