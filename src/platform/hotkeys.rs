//! 全局热键注册。
//!
//! 每个热键单独注册、单独报告失败原因：`RegisterHotKey` 会在组合已被
//! 别的程序占用时返回 0，这种情况必须让用户知道，而不是静默失效。
//!
//! 热键表是**运行时**拼出来的（[`resolve`]）：内置默认表 + 配置里的覆盖 +
//! 「总开关」三者合成最终要注册的列表。所以本模块提供的是
//! 「给一组定义就注册」的纯执行者，不再假设只有一套固定键位。

use crate::config::{Config, HotkeyCombo};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_HOTKEY;

/// 一次热键触发的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    NextPreset,
    PrevPreset,
    ToggleVisible,
    OpacityUp,
    OpacityDown,
    ScaleUp,
    ScaleDown,
    /// 准星位置微调，单位是「100% DPI 下的像素」，原点在屏幕中心。
    ///
    /// 用 `i32` 而不是 `f32` 是因为枚举要能 `Eq`（面板/菜单里会拿来比）。
    Nudge {
        dx: i32,
        dy: i32,
    },
    /// 准星回到屏幕正中。
    ResetOffset,
    OpenSettings,
    Reload,
    Quit,
}

impl HotkeyAction {
    pub fn describe(self) -> String {
        match self {
            HotkeyAction::NextPreset => "切换到下一个预设".to_string(),
            HotkeyAction::PrevPreset => "切换到上一个预设".to_string(),
            HotkeyAction::ToggleVisible => "显示 / 隐藏准星".to_string(),
            HotkeyAction::OpacityUp => "提高不透明度".to_string(),
            HotkeyAction::OpacityDown => "降低不透明度".to_string(),
            HotkeyAction::ScaleUp => "放大准星".to_string(),
            HotkeyAction::ScaleDown => "缩小准星".to_string(),
            HotkeyAction::Nudge { dx, dy } => {
                let dir = match (dx.signum(), dy.signum()) {
                    (-1, _) => "左",
                    (1, _) => "右",
                    (_, -1) => "上",
                    (_, 1) => "下",
                    _ => "不",
                };
                format!("准星向{dir}移动 {} 像素", dx.abs().max(dy.abs()))
            }
            HotkeyAction::ResetOffset => "准星回到屏幕中心".to_string(),
            HotkeyAction::OpenSettings => "打开设置界面".to_string(),
            HotkeyAction::Reload => "重新载入配置".to_string(),
            HotkeyAction::Quit => "退出程序".to_string(),
        }
    }

    /// 稳定的短标识，用来在配置/日志里指代一个动作。
    ///
    /// 不参与热键解析（id 才是键），但排查「哪个动作没注册上」时比中文描述好搜。
    pub fn key(self) -> &'static str {
        match self {
            HotkeyAction::NextPreset => "next_preset",
            HotkeyAction::PrevPreset => "prev_preset",
            HotkeyAction::ToggleVisible => "toggle_visible",
            HotkeyAction::OpacityUp => "opacity_up",
            HotkeyAction::OpacityDown => "opacity_down",
            HotkeyAction::ScaleUp => "scale_up",
            HotkeyAction::ScaleDown => "scale_down",
            HotkeyAction::Nudge { dx: -1, .. } => "nudge_left",
            HotkeyAction::Nudge { dx: 1, .. } => "nudge_right",
            HotkeyAction::Nudge { dy: -1, .. } => "nudge_up",
            HotkeyAction::Nudge { dy: 1, .. } => "nudge_down",
            HotkeyAction::Nudge { .. } => "nudge",
            HotkeyAction::ResetOffset => "reset_offset",
            HotkeyAction::OpenSettings => "open_settings",
            HotkeyAction::Reload => "reload",
            HotkeyAction::Quit => "quit",
        }
    }
}

/// 一条热键定义。
#[derive(Debug, Clone, Copy)]
pub struct HotkeyDef {
    /// 注册用的 id（1 起）
    pub id: i32,
    pub action: HotkeyAction,
    pub combo: HotkeyCombo,
}

impl HotkeyDef {
    /// 展示用文本，如 `"Ctrl+Shift+D  准星向右移动 1 像素"`。
    ///
    /// 由组合实时生成，所以改键后提示文字自动跟着变——这正是原来把
    /// `label` 写成 `&'static str` 做不到的事。
    pub fn label(&self) -> String {
        format!("{}  {}", self.combo.label(), self.action.describe())
    }

    /// 只给组合，如 `"Ctrl+Shift+D"`。
    pub fn combo_label(&self) -> String {
        self.combo.label()
    }
}

const fn hk(
    id: i32,
    action: HotkeyAction,
    ctrl: bool,
    alt: bool,
    shift: bool,
    vk: u32,
) -> HotkeyDef {
    HotkeyDef {
        id,
        action,
        combo: HotkeyCombo::new(ctrl, alt, shift, vk),
    }
}

pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_0: u32 = 0x30;
pub const VK_A: u32 = 0x41;
pub const VK_D: u32 = 0x44;
pub const VK_H: u32 = 0x48;
pub const VK_O: u32 = 0x4F;
pub const VK_Q: u32 = 0x51;
pub const VK_R: u32 = 0x52;
pub const VK_S: u32 = 0x53;
pub const VK_W: u32 = 0x57;

/// 默认热键表。全部用 `Ctrl+Alt+…`，避免和游戏里的单键操作冲突。
///
/// 这张表只是**默认值**：用户改过的项在 [`resolve`] 里被配置覆盖。
pub const DEFAULT_HOTKEYS: &[HotkeyDef] = &[
    hk(1, HotkeyAction::NextPreset, true, true, false, VK_RIGHT),
    hk(2, HotkeyAction::PrevPreset, true, true, false, VK_LEFT),
    hk(3, HotkeyAction::ToggleVisible, true, true, false, VK_H),
    hk(4, HotkeyAction::OpacityUp, true, true, false, VK_UP),
    hk(5, HotkeyAction::OpacityDown, true, true, false, VK_DOWN),
    hk(6, HotkeyAction::ScaleUp, true, true, true, VK_UP),
    hk(7, HotkeyAction::ScaleDown, true, true, true, VK_DOWN),
    hk(8, HotkeyAction::OpenSettings, true, true, false, VK_O),
    hk(9, HotkeyAction::Reload, true, true, false, VK_R),
    hk(10, HotkeyAction::Quit, true, true, false, VK_Q),
    // 位置微调走 WASD：方向键已经被预设 / 不透明度占用了，再挤到
    // Ctrl+Alt+Shift+方向键会和「缩放」撞车。WASD 在 Ctrl+Alt 前缀下
    // 几乎不会和游戏冲突，而且「W 上 / A 左 / S 下 / D 右」不用记。
    //
    // 但 1px 那四个**不能**带 Alt：实测本机 `Ctrl+Alt+A/D/W/S` 全部
    // `RegisterHotKey` 失败（被别的程序占了，`Ctrl+Alt+E/L/P/X` 也一样），
    // 只有加上 Shift 才拿得到。所以规则是「Shift = 1px / Ctrl+Alt+Shift = 10px」，
    // 少按一个 Alt 就是小幅微调。
    hk(
        11,
        HotkeyAction::Nudge { dx: -1, dy: 0 },
        true,
        false,
        true,
        VK_A,
    ),
    hk(
        12,
        HotkeyAction::Nudge { dx: 1, dy: 0 },
        true,
        false,
        true,
        VK_D,
    ),
    hk(
        13,
        HotkeyAction::Nudge { dx: 0, dy: -1 },
        true,
        false,
        true,
        VK_W,
    ),
    hk(
        14,
        HotkeyAction::Nudge { dx: 0, dy: 1 },
        true,
        false,
        true,
        VK_S,
    ),
    hk(
        15,
        HotkeyAction::Nudge { dx: -10, dy: 0 },
        true,
        true,
        true,
        VK_A,
    ),
    hk(
        16,
        HotkeyAction::Nudge { dx: 10, dy: 0 },
        true,
        true,
        true,
        VK_D,
    ),
    hk(
        17,
        HotkeyAction::Nudge { dx: 0, dy: -10 },
        true,
        true,
        true,
        VK_W,
    ),
    hk(
        18,
        HotkeyAction::Nudge { dx: 0, dy: 10 },
        true,
        true,
        true,
        VK_S,
    ),
    hk(19, HotkeyAction::ResetOffset, true, true, false, VK_0),
];

// ---------------------------------------------------------------------------
// 组合 <-> 文本 / Win32 位
// ---------------------------------------------------------------------------

impl HotkeyCombo {
    /// 转成 `RegisterHotKey` 要的 `MOD_*` 位图。
    ///
    /// 恒带 `MOD_NOREPEAT`：按住不放时系统会每秒发几十条 `WM_HOTKEY`，
    /// 位置微调那种「按一下挪一像素」的动作会瞬间把准星甩到屏幕外。
    pub fn modifiers(&self) -> u32 {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        m
    }

    /// 展示用文本，如 `"Ctrl+Alt+Shift+D"`。
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        let key = key_name(self.vk);
        if parts.is_empty() {
            key
        } else {
            format!("{}+{key}", parts.join("+"))
        }
    }

    /// 这个组合是否还需要（至少一个修饰键 + 合法的键码）。
    pub fn is_usable(&self) -> bool {
        self.vk != 0 && self.vk <= 0xFE && (self.ctrl || self.alt || self.shift)
    }
}

/// 虚拟键码 -> 人能读的键名。
pub fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 => ((b'0' + (vk - 0x30) as u8) as char).to_string(),
        0x41..=0x5A => ((b'A' + (vk - 0x41) as u8) as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x25 => "←".to_string(),
        0x26 => "↑".to_string(),
        0x27 => "→".to_string(),
        0x28 => "↓".to_string(),
        0x20 => "Space".to_string(),
        0x0D => "Enter".to_string(),
        0x09 => "Tab".to_string(),
        0x08 => "Backspace".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "PageUp".to_string(),
        0x22 => "PageDown".to_string(),
        // 不认识的键照样给个能读的名字，而不是空串——空串会让界面看起来像
        // 「这个热键没设置」。下划线是刻意加的：`VKE2` 会被读成单词，
        // `VK_E2` 一眼就能看出是「虚拟键码 0xE2」。
        _ => format!("VK_{vk:02X}"),
    }
}

// ---------------------------------------------------------------------------
// 配置 -> 要注册的表
// ---------------------------------------------------------------------------

/// 某一条热键当前生效的组合（用户改过的优先，否则用默认值）。
pub fn combo_for(config: &Config, id: i32) -> Option<HotkeyCombo> {
    if let Some(combo) = config.hotkey_override(id) {
        return Some(combo);
    }
    DEFAULT_HOTKEYS.iter().find(|d| d.id == id).map(|d| d.combo)
}

/// 界面用的一行：动作 + 当前组合 + 默认组合。
#[derive(Debug, Clone, Copy)]
pub struct HotkeyRow {
    pub id: i32,
    pub action: HotkeyAction,
    pub combo: HotkeyCombo,
    pub default_combo: HotkeyCombo,
}

impl HotkeyRow {
    pub fn customized(&self) -> bool {
        self.combo != self.default_combo
    }
}

/// 列出全部热键（含当前生效组合），供设置界面渲染。
pub fn rows(config: &Config) -> Vec<HotkeyRow> {
    DEFAULT_HOTKEYS
        .iter()
        .map(|d| HotkeyRow {
            id: d.id,
            action: d.action,
            combo: config.hotkey_override(d.id).unwrap_or(d.combo),
            default_combo: d.combo,
        })
        .collect()
}

/// 找出「多个动作抢同一个组合」的项，返回重复的组合。
///
/// 只在用户改键后才可能非空。界面拿它显示红字警告——否则表现是
/// 「寄存器里只有第一个生效，另一个键按了没反应」，非常难查。
pub fn conflicts(config: &Config) -> Vec<HotkeyCombo> {
    let rows = rows(config);
    let mut out: Vec<HotkeyCombo> = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        if out.contains(&a.combo) {
            continue;
        }
        if rows[i + 1..].iter().any(|b| b.combo == a.combo) {
            out.push(a.combo);
        }
    }
    out
}

/// 默认表 + 配置覆盖 + 总开关 = 真正要注册的热键表。
///
/// 组合重复的项会被跳过（先出现的赢），因为 `RegisterHotKey` 对同一组合
/// 只会成功一次。冲突本身由 [`conflicts`] 报给界面，而不是在这里静默修掉。
pub fn resolve(config: &Config) -> Vec<HotkeyDef> {
    if !config.hotkeys_enabled {
        return Vec::new();
    }
    let mut out: Vec<HotkeyDef> = Vec::with_capacity(DEFAULT_HOTKEYS.len());
    for def in DEFAULT_HOTKEYS {
        let combo = config.hotkey_override(def.id).unwrap_or(def.combo);
        if !combo.is_usable() {
            continue;
        }
        if out.iter().any(|d| d.combo == combo) {
            continue;
        }
        out.push(HotkeyDef { combo, ..*def });
    }
    out
}

// ---------------------------------------------------------------------------
// 注册
// ---------------------------------------------------------------------------

/// 已注册热键的集合；`Drop` 时自动注销。
///
/// 重建（改键 / 开关热键）时的正确姿势是先 `drop` 旧的再用新表 `register`：
/// 同一个 id 重复注册会失败，所以注销必须发生在注册之前。
pub struct Hotkeys {
    hwnd: HWND,
    defs: Vec<HotkeyDef>,
    registered: Vec<i32>,
    /// 注册失败的项：`(定义, 失败原因)`
    pub failures: Vec<(HotkeyDef, String)>,
}

impl Hotkeys {
    /// 在 `hwnd` 上注册给定的热键表，逐个记录失败项。
    ///
    /// `HWND` 在这套 API 里只是个不透明句柄，`RegisterHotKey` 不会去解引用它；
    /// 调用者也不可能凭空造出一个非法窗口（句柄来自 [`crate::platform::overlay`]）。
    /// 所以这里保留安全函数签名，而不是强迫每个调用点包 `unsafe`。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn register(hwnd: HWND, defs: &[HotkeyDef]) -> Self {
        let mut registered = Vec::new();
        let mut failures = Vec::new();
        // 这里**不需要**特殊排序，`RegisterHotKey` 自己能区分这两个组合。
        //
        // 曾经因为自测里 `Ctrl+Alt+Shift+↑` 总是收到 id 4（不透明度）而误判成
        // 「先注册的先命中、被 `Ctrl+Alt+↑` 吃掉了」，白改了一版排序。真相是
        // **注入脚本用了 `keybd_event`**：它注入的修饰键会进入
        // `GetAsyncKeyState` 的物理键状态，却不参与热键匹配，于是系统只看到
        // Ctrl+Alt。换成官方 `SendInput` 后立刻就命中 id 6、Shift 状态也正确。
        // 结论：产品代码原本就是对的，不要为它加任何工作区。
        for def in defs {
            let ok = unsafe { RegisterHotKey(hwnd, def.id, def.combo.modifiers(), def.combo.vk) };
            if ok != 0 {
                registered.push(def.id);
            } else {
                let err = std::io::Error::last_os_error();
                failures.push((*def, format!("{err}（多半已被其它程序占用）")));
            }
        }
        Hotkeys {
            hwnd,
            defs: defs.to_vec(),
            registered,
            failures,
        }
    }

    pub fn ok_count(&self) -> usize {
        self.registered.len()
    }

    /// 注销全部热键并清空记录（保留 `failures`，下次注册会重构它）。
    ///
    /// 改键 / 关开关时必须在重新注册**之前**调用：`RegisterHotKey` 对同一个
    /// id 重复注册会失败，而已注册的旧组合也不会自己让出来，不注销的话
    /// 表现是「改了键位还是原来那个键生效，新键按了没反应」。
    pub fn unregister_all(&mut self) {
        for id in self.registered.drain(..) {
            unsafe {
                UnregisterHotKey(self.hwnd, id);
            }
        }
        self.defs.clear();
        self.failures.clear();
    }

    /// 这次实际尝试注册的条目数。
    pub fn attempted(&self) -> usize {
        self.defs.len()
    }

    /// 当前生效的键位表（已合并用户覆盖）。
    pub fn defs(&self) -> &[HotkeyDef] {
        &self.defs
    }

    /// 把失败项汇总成一句给界面看的话。
    pub fn failure_summary(&self) -> Option<String> {
        if self.failures.is_empty() {
            return None;
        }
        let names: Vec<String> = self
            .failures
            .iter()
            .map(|(def, _)| def.combo_label())
            .collect();
        Some(format!(
            "{} 个热键注册失败（可能被其它程序占用）：{}",
            self.failures.len(),
            names.join("、")
        ))
    }

    /// 从 `WM_HOTKEY` 的 `wparam` 解出对应动作。
    ///
    /// 走静态默认表：动作和 id 的对应关系由程序定义，和用户改了什么键无关。
    pub fn action_of(id: i32) -> Option<HotkeyAction> {
        DEFAULT_HOTKEYS
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.action)
    }

    /// 是否是一条热键消息。
    pub fn is_hotkey(msg: u32) -> bool {
        msg == WM_HOTKEY
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        for id in &self.registered {
            unsafe {
                UnregisterHotKey(self.hwnd, *id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn hotkey_ids_are_unique() {
        let mut ids: Vec<i32> = DEFAULT_HOTKEYS.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DEFAULT_HOTKEYS.len());
    }

    #[test]
    fn hotkey_combos_are_unique() {
        let mut combos: Vec<HotkeyCombo> = DEFAULT_HOTKEYS.iter().map(|d| d.combo).collect();
        combos.sort_unstable_by_key(|c| (c.ctrl, c.alt, c.shift, c.vk));
        combos.dedup();
        assert_eq!(combos.len(), DEFAULT_HOTKEYS.len(), "热键组合重复");
    }

    #[test]
    fn lookup_by_id() {
        assert_eq!(Hotkeys::action_of(1), Some(HotkeyAction::NextPreset));
        assert_eq!(Hotkeys::action_of(10), Some(HotkeyAction::Quit));
        assert_eq!(
            Hotkeys::action_of(11),
            Some(HotkeyAction::Nudge { dx: -1, dy: 0 })
        );
        assert_eq!(
            Hotkeys::action_of(18),
            Some(HotkeyAction::Nudge { dx: 0, dy: 10 })
        );
        assert_eq!(Hotkeys::action_of(19), Some(HotkeyAction::ResetOffset));
        assert_eq!(Hotkeys::action_of(999), None);
    }

    /// 位置微调必须四向齐全，且细/粗两档步长成比例。
    /// 少写一条的症状是「某个方向按了没反应」，而热键注册本身还会成功。
    #[test]
    fn nudges_cover_all_directions_in_two_magnitudes() {
        let nudges: Vec<(i32, i32)> = DEFAULT_HOTKEYS
            .iter()
            .filter_map(|d| match d.action {
                HotkeyAction::Nudge { dx, dy } => Some((dx, dy)),
                _ => None,
            })
            .collect();
        assert_eq!(nudges.len(), 8);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            assert!(nudges.contains(&(dx, dy)), "缺少 {dx},{dy} 方向");
            assert!(
                nudges.contains(&(dx * 10, dy * 10)),
                "缺少 {dx},{dy} 方向的 10 像素档"
            );
        }
    }

    #[test]
    fn every_hotkey_has_a_readable_label() {
        for d in DEFAULT_HOTKEYS {
            let label = d.label();
            assert!(!label.is_empty(), "热键 {} 没有说明文字", d.id);
            // 标签必须真的包含组合，否则界面上看不出按什么键
            assert!(
                label.starts_with(&d.combo_label()),
                "热键 {} 标签缺组合",
                d.id
            );
        }
    }

    #[test]
    fn combo_label_renders_modifiers_and_key() {
        let c = HotkeyCombo::new(true, true, false, VK_D);
        assert_eq!(c.label(), "Ctrl+Alt+D");
        let c = HotkeyCombo::new(true, true, true, VK_UP);
        assert_eq!(c.label(), "Ctrl+Alt+Shift+↑");
        assert_eq!(HotkeyCombo::new(true, false, false, VK_0).label(), "Ctrl+0");
    }

    #[test]
    fn modifiers_bits_follow_flags() {
        let plain = HotkeyCombo::new(true, true, false, VK_H);
        assert_eq!(plain.modifiers() & MOD_CONTROL, MOD_CONTROL);
        assert_eq!(plain.modifiers() & MOD_ALT, MOD_ALT);
        assert_eq!(plain.modifiers() & MOD_SHIFT, 0);
        // 恒带 NOREPEAT：少这一位，按住不放会把准星甩飞
        assert_eq!(plain.modifiers() & MOD_NOREPEAT, MOD_NOREPEAT);
    }

    /// 裸键不能拿去注册：那会把普通打字变成全局热键。
    #[test]
    fn bare_key_is_not_usable() {
        assert!(!HotkeyCombo::new(false, false, false, VK_D).is_usable());
        assert!(!HotkeyCombo::new(true, true, false, 0).is_usable());
        assert!(!HotkeyCombo::new(false, true, false, 0xFF).is_usable());
        assert!(HotkeyCombo::new(true, true, false, VK_D).is_usable());
    }

    #[test]
    fn resolve_without_overrides_matches_defaults() {
        let config = Config::default();
        let defs = resolve(&config);
        assert_eq!(defs.len(), DEFAULT_HOTKEYS.len());
        for (got, want) in defs.iter().zip(DEFAULT_HOTKEYS) {
            assert_eq!(got.id, want.id);
            assert_eq!(got.combo, want.combo);
        }
    }

    #[test]
    fn resolve_applies_user_override() {
        let mut config = Config::default();
        // 默认表里没人用 Ctrl+Alt+F5，换到 id 12 上应能独立生效
        let custom = HotkeyCombo::new(true, true, false, 0x74);
        config.set_hotkey(12, custom);
        let defs = resolve(&config);
        let hit = defs.iter().find(|d| d.id == 12).expect("应保留 id 12");
        assert_eq!(hit.combo, custom);
        assert_eq!(defs.len(), DEFAULT_HOTKEYS.len());
    }

    /// 改键改到别人正在用的组合上时，只保留先出现的那个。
    /// 不去重的话 `RegisterHotKey` 第二条会失败，症状是「按了没反应」。
    #[test]
    fn resolve_dedupes_conflicting_combos() {
        let mut config = Config::default();
        // id 3 默认就是 Ctrl+Alt+H；把 id 12 也指到 Ctrl+Alt+H
        config
            .hotkeys
            .insert(12, HotkeyCombo::new(true, true, false, VK_H));
        let defs = resolve(&config);
        let with_h: Vec<i32> = defs
            .iter()
            .filter(|d| d.combo == HotkeyCombo::new(true, true, false, VK_H))
            .map(|d| d.id)
            .collect();
        assert_eq!(with_h, vec![3], "同组合只应保留先出现的 id");
    }

    #[test]
    fn disabled_config_yields_no_hotkeys() {
        let config = Config {
            hotkeys_enabled: false,
            ..Default::default()
        };
        assert!(resolve(&config).is_empty());
    }

    #[test]
    fn set_hotkey_bumps_previous_owner() {
        let mut config = Config::default();
        let combo = HotkeyCombo::new(true, true, false, VK_0);
        // id 19 默认就是 Ctrl+Alt+0，显式绑到 id 12 应把 19 的覆盖清掉
        config.set_hotkey(19, combo);
        let bumped = config.set_hotkey(12, combo);
        assert_eq!(bumped, vec![19]);
        assert_eq!(config.hotkey_override(12), Some(combo));
        assert_eq!(config.hotkey_override(19), None);
    }

    #[test]
    fn reset_hotkey_falls_back_to_default() {
        let mut config = Config::default();
        config.set_hotkey(12, HotkeyCombo::new(true, true, false, VK_LEFT));
        assert!(config.has_custom_hotkeys());
        config.reset_hotkey(12);
        assert!(!config.has_custom_hotkeys());
        assert_eq!(
            combo_for(&config, 12),
            Some(HotkeyCombo::new(true, false, true, VK_D))
        );
    }

    #[test]
    fn conflicts_detects_duplicate_overrides() {
        let mut config = Config::default();
        config
            .hotkeys
            .insert(12, HotkeyCombo::new(true, true, false, VK_H));
        let found = conflicts(&config);
        assert_eq!(found, vec![HotkeyCombo::new(true, true, false, VK_H)]);
    }

    #[test]
    fn key_name_covers_letters_digits_arrows_and_fkeys() {
        assert_eq!(key_name(VK_A), "A");
        assert_eq!(key_name(VK_0), "0");
        assert_eq!(key_name(VK_UP), "↑");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x7B), "F12");
        assert_eq!(key_name(0x2E), "Delete");
        assert_eq!(key_name(0xE2), "VK_E2");
    }
}
