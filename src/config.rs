//! 配置模型与读写。
//!
//! 顶层配置（`config.json`）只保存“用户意图”，具体图形由
//! [`CrosshairSpec`] 描述；`overrides` 提供一层轻量覆盖，使得
//! 托盘 / 设置界面改颜色、透明度时不需要改写预设本身。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::presets;

pub const CONFIG_VERSION: u32 = 1;

/// 准星相对屏幕中心的偏移上限（100% DPI 像素）。
///
/// 存在的意义是防止手写配置写成 `1e9` 把准星丢到屏幕外面再也找不回来。
/// 取 10000 是为了容纳极端的多屏 / 超宽屏拼接布局，而不是「屏幕宽度」。
pub const OFFSET_LIMIT: f32 = 10_000.0;

/// 合法热键 id 的上限。
///
/// `WM_HOTKEY` 的 id 取值范围是 0x0000–0xBFFF，这里只用到 1–19；留一段余量
/// 是为了以后加动作时不必动这个常量的语义，同时仍然能挡住手改配置写成
/// `"999999": {...}` 这种明显无效的项。
pub const MAX_HOTKEY_ID: u32 = 999;

// ---------------------------------------------------------------------------
// 通用小工具
// ---------------------------------------------------------------------------

fn default_color() -> String {
    "#00FF00".to_string()
}
fn default_opacity() -> f32 {
    1.0
}
fn default_scale() -> f32 {
    1.0
}
fn default_width() -> f32 {
    2.0
}
fn default_true() -> bool {
    true
}
fn default_outline_color() -> String {
    "#000000".to_string()
}
fn default_outline_opacity() -> f32 {
    0.55
}
fn default_outline_width() -> f32 {
    1.0
}
fn default_zero_pair() -> [f32; 2] {
    [0.0, 0.0]
}

/// 线端样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapKind {
    #[default]
    Butt,
    Round,
    Square,
}

/// 描边（光晕/黑边）：在图形后面再画一层更粗的轮廓，让准星在任何背景上都看得清。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineSpec {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_outline_color")]
    pub color: String,
    #[serde(default = "default_outline_opacity")]
    pub opacity: f32,
    #[serde(default = "default_outline_width")]
    pub width: f32,
}

impl Default for OutlineSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            color: default_outline_color(),
            opacity: default_outline_opacity(),
            width: default_outline_width(),
        }
    }
}

/// 一个图元。坐标是“预设本地坐标”，原点在准星中心，单位是 100% DPI 下的像素，
/// 最终会乘以 `CrosshairSpec::scale` 与显示器 DPI 缩放。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// 线段
    Line {
        from: [f32; 2],
        to: [f32; 2],
        #[serde(default = "default_width")]
        width: f32,
        #[serde(default)]
        cap: CapKind,
    },
    /// 实心圆点
    Dot {
        #[serde(default = "default_zero_pair")]
        center: [f32; 2],
        radius: f32,
    },
    /// 圆 / 圆环
    Circle {
        #[serde(default = "default_zero_pair")]
        center: [f32; 2],
        radius: f32,
        #[serde(default)]
        filled: bool,
        #[serde(default = "default_width")]
        width: f32,
    },
    /// 圆弧（0° 指向右，顺时针为正）
    Arc {
        #[serde(default = "default_zero_pair")]
        center: [f32; 2],
        radius: f32,
        #[serde(default)]
        start_deg: f32,
        sweep_deg: f32,
        #[serde(default = "default_width")]
        width: f32,
        #[serde(default)]
        cap: CapKind,
    },
    /// 矩形（可圆角）
    Rect {
        #[serde(default = "default_zero_pair")]
        center: [f32; 2],
        size: [f32; 2],
        #[serde(default)]
        corner_radius: f32,
        #[serde(default)]
        filled: bool,
        #[serde(default = "default_width")]
        width: f32,
        #[serde(default)]
        rotation_deg: f32,
    },
    /// 多边形 / 折线
    Polygon {
        points: Vec<[f32; 2]>,
        #[serde(default = "default_true")]
        closed: bool,
        #[serde(default)]
        filled: bool,
        #[serde(default = "default_width")]
        width: f32,
    },
    /// 组合：给一组图元统一加变换（平移/旋转/缩放）
    Group {
        #[serde(default)]
        shapes: Vec<Shape>,
        #[serde(default)]
        rotation_deg: f32,
        #[serde(default = "default_zero_pair")]
        translate: [f32; 2],
        #[serde(default = "default_scale")]
        scale: f32,
    },
}

impl Shape {
    pub fn line(from: [f32; 2], to: [f32; 2], width: f32) -> Self {
        Shape::Line {
            from,
            to,
            width,
            cap: CapKind::Butt,
        }
    }

    pub fn round_line(from: [f32; 2], to: [f32; 2], width: f32) -> Self {
        Shape::Line {
            from,
            to,
            width,
            cap: CapKind::Round,
        }
    }

    pub fn dot(radius: f32) -> Self {
        Shape::Dot {
            center: [0.0, 0.0],
            radius,
        }
    }

    pub fn ring(radius: f32, width: f32) -> Self {
        Shape::Circle {
            center: [0.0, 0.0],
            radius,
            filled: false,
            width,
        }
    }

    pub fn disc(radius: f32) -> Self {
        Shape::Circle {
            center: [0.0, 0.0],
            radius,
            filled: true,
            width: 1.0,
        }
    }

    /// 一条以中心为原点、沿 `angle_deg` 方向的短线，从 `r0` 伸到 `r1`。
    pub fn ray(angle_deg: f32, r0: f32, r1: f32, width: f32) -> Self {
        let rad = angle_deg.to_radians();
        let (s, c) = rad.sin_cos();
        Shape::line([c * r0, s * r0], [c * r1, s * r1], width)
    }
}

/// 完整的准星描述。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrosshairSpec {
    #[serde(default = "default_color")]
    pub color: String,
    /// 0.0 - 1.0
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// 整体缩放（1.0 = 设计尺寸）
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default)]
    pub rotation_deg: f32,
    /// 相对屏幕中心的偏移（本地单位，会乘 scale）
    #[serde(default = "default_zero_pair")]
    pub offset: [f32; 2],
    #[serde(default)]
    pub outline: OutlineSpec,
    #[serde(default)]
    pub shapes: Vec<Shape>,
}

impl Default for CrosshairSpec {
    fn default() -> Self {
        Self {
            color: default_color(),
            opacity: default_opacity(),
            scale: default_scale(),
            rotation_deg: 0.0,
            offset: [0.0, 0.0],
            outline: OutlineSpec::default(),
            shapes: Vec::new(),
        }
    }
}

impl CrosshairSpec {
    pub fn new(color: &str, shapes: Vec<Shape>) -> Self {
        Self {
            color: color.to_string(),
            shapes,
            ..Default::default()
        }
    }

    /// 把数值收敛到合法范围，避免手写配置把渲染搞坏。
    pub fn normalize(&mut self) {
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self.scale = self.scale.clamp(0.05, 20.0);
        self.rotation_deg = normalize_deg(self.rotation_deg);
        self.outline.width = self.outline.width.clamp(0.0, 20.0);
        self.outline.opacity = self.outline.opacity.clamp(0.0, 1.0);
        if self.shapes.is_empty() {
            self.shapes = presets::builtin(presets::FALLBACK)
                .map(|s| s.shapes)
                .unwrap_or_else(|| vec![Shape::dot(1.5)]);
        }
    }
}

pub fn normalize_deg(deg: f32) -> f32 {
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d
}

// ---------------------------------------------------------------------------
// 覆盖层 / 顶层配置
// ---------------------------------------------------------------------------

/// 对当前生效预设的临时覆盖（托盘、热键、设置界面改的值都落在这里）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<f32>,
    /// 屏幕中心偏移，单位是「100% DPI 下的像素」。
    ///
    /// 和 [`CrosshairSpec::offset`] 不是一个单位：那个是预设本地单位、会跟着
    /// `scale` 一起放大缩小；这里是像素，用户挪 3 格就真的是 3 个像素。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_width: Option<f32>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.opacity.is_none()
            && self.scale.is_none()
            && self.rotation_deg.is_none()
            && self.offset.is_none()
            && self.outline.is_none()
            && self.outline_color.is_none()
            && self.outline_width.is_none()
    }
}

// ---------------------------------------------------------------------------
// 热键
// ---------------------------------------------------------------------------

/// 一个热键组合：三个修饰键 + 一个虚拟键码。
///
/// 存成三个布尔而不是 `MOD_*` 位图，是为了让配置文件能直接手改：
/// `{"ctrl": true, "alt": true, "shift": false, "vk": 68}` 比
/// `"modifiers": 3` 好读得多，也不依赖 `MOD_NOREPEAT` 这类只在注册时才
/// 有意义的位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyCombo {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    /// Windows 虚拟键码（`VK_*`），例如 `0x44` = D
    pub vk: u32,
}

impl HotkeyCombo {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, vk: u32) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            vk,
        }
    }
}

/// 显示器选择：`"primary"` / `"1"` / `1`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MonitorSel {
    Index(u32),
    Text(String),
}

impl Default for MonitorSel {
    fn default() -> Self {
        MonitorSel::Text("primary".to_string())
    }
}

impl MonitorSel {
    /// 返回 0 基的显示器下标；`None` 表示“跟随主屏”。
    pub fn index(&self) -> Option<usize> {
        match self {
            MonitorSel::Index(i) => Some(*i as usize),
            MonitorSel::Text(t) => {
                let t = t.trim();
                if t.eq_ignore_ascii_case("primary") || t.is_empty() {
                    None
                } else {
                    t.parse::<usize>().ok()
                }
            }
        }
    }

    pub fn from_index(index: Option<usize>) -> Self {
        match index {
            None => MonitorSel::Text("primary".to_string()),
            Some(i) => MonitorSel::Index(i as u32),
        }
    }

    /// 供日志 / 界面展示的描述文本。
    pub fn describe(&self) -> String {
        match self.index() {
            None => "主显示器".to_string(),
            Some(i) => format!("第 {} 个显示器", i + 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "config_version")]
    pub version: u32,
    /// 当前生效的预设名（内置名或 `custom_presets` 里的名字）
    #[serde(default = "default_active")]
    pub active: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub monitor: MonitorSel,
    #[serde(default)]
    pub overrides: Overrides,
    /// 全局热键总开关。关掉后所有热键都不注册（托盘菜单和设置界面仍可用，
    /// 否则一旦关掉就再也没法用热键把设置界面叫出来了）。
    #[serde(default = "default_true")]
    pub hotkeys_enabled: bool,
    /// 用户改过的热键：`热键 id -> 组合`。没列出的 id 用内置默认值。
    ///
    /// 用「只存差异」而不是整表快照：以后默认键位调整时，没改过的项会跟着
    /// 一起变好，不会把旧默认值钉死在用户的配置里。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hotkeys: BTreeMap<u32, HotkeyCombo>,
    /// 用户自定义预设（JSON 图元脚本）
    #[serde(default)]
    pub custom_presets: BTreeMap<String, CrosshairSpec>,
    /// 更新检测用的仓库坐标（`owner/repo`）。
    ///
    /// 留空就用编译期内置的值（见 `src/update.rs`）。把它放进配置而不是写死，
    /// 是因为「仓库搬家了」这件事不该需要重新编译一个 exe。
    /// 手改错了也不会出事：`is_slug` 会拒掉非法值，程序只是不做联网检查。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_repo: Option<String>,
}

fn config_version() -> u32 {
    CONFIG_VERSION
}

fn default_active() -> String {
    presets::FALLBACK.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            active: default_active(),
            visible: true,
            monitor: MonitorSel::default(),
            overrides: Overrides::default(),
            hotkeys_enabled: true,
            hotkeys: BTreeMap::new(),
            custom_presets: BTreeMap::new(),
            update_repo: None,
        }
    }
}

impl Config {
    /// 可选的预设名：内置在前，自定义在后，去重。
    pub fn preset_names(&self) -> Vec<String> {
        let mut names: Vec<String> = presets::names();
        for name in self.custom_presets.keys() {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }
        names
    }

    /// 解析某个预设名（先找自定义，再找内置），解析不到则回退到内置默认预设。
    pub fn resolve(&self, name: &str) -> CrosshairSpec {
        if let Some(spec) = self.custom_presets.get(name) {
            let mut spec = spec.clone();
            spec.normalize();
            return spec;
        }
        if let Some(spec) = presets::builtin(name) {
            return spec;
        }
        let mut spec = presets::builtin(presets::FALLBACK)
            .unwrap_or_else(|| CrosshairSpec::new("#00FF00", vec![Shape::dot(1.5)]));
        spec.normalize();
        spec
    }

    /// 叠加 overrides 后真正用来渲染的图形。
    pub fn effective(&self) -> CrosshairSpec {
        let mut spec = self.resolve(&self.active);
        if let Some(color) = &self.overrides.color {
            spec.color = color.clone();
        }
        if let Some(opacity) = self.overrides.opacity {
            spec.opacity = opacity;
        }
        if let Some(scale) = self.overrides.scale {
            spec.scale = scale;
        }
        if let Some(rot) = self.overrides.rotation_deg {
            spec.rotation_deg = rot;
        }
        if let Some(off) = self.overrides.offset {
            // 偏移按像素记账，所以先除掉 scale：渲染时 `draw` 会把它乘回来
            // （`center + offset * scale * dpi`），用户看到的位移就等于 off * dpi。
            // 不这么做的话「往右挪 5 像素」在 2x 缩放时会变成 10 像素。
            let scale = spec.scale.max(0.05);
            spec.offset = [off[0] / scale, off[1] / scale];
        }
        if let Some(outline) = self.overrides.outline {
            spec.outline.enabled = outline;
        }
        if let Some(color) = &self.overrides.outline_color {
            spec.outline.color = color.clone();
        }
        if let Some(width) = self.overrides.outline_width {
            spec.outline.width = width;
        }
        spec.normalize();
        spec
    }

    /// 把无效字段修正回来（加载外部编辑过的文件后调用）。
    pub fn normalize(&mut self) {
        if self.version == 0 {
            self.version = CONFIG_VERSION;
        }
        if self.active.trim().is_empty() {
            self.active = default_active();
        }
        if let Some(op) = self.overrides.opacity {
            self.overrides.opacity = Some(op.clamp(0.0, 1.0));
        }
        if let Some(s) = self.overrides.scale {
            self.overrides.scale = Some(s.clamp(0.05, 20.0));
        }
        if let Some(w) = self.overrides.outline_width {
            self.overrides.outline_width = Some(w.clamp(0.0, 20.0));
        }
        if let Some(off) = self.overrides.offset {
            self.overrides.offset = Some([
                off[0].clamp(-OFFSET_LIMIT, OFFSET_LIMIT),
                off[1].clamp(-OFFSET_LIMIT, OFFSET_LIMIT),
            ]);
        }
        for spec in self.custom_presets.values_mut() {
            spec.normalize();
        }
        self.custom_presets
            .retain(|name, _| !name.trim().is_empty());
        // 手改配置很容易写出 `"vk": 0` 或裸键（无修饰键）。这类项留着只会
        // 让 `RegisterHotKey` 静默失败或把打字变成热键，直接丢掉。
        self.hotkeys
            .retain(|id, combo| (1..=MAX_HOTKEY_ID).contains(id) && combo.is_usable());
        // 仓库坐标会被拼进 URL：非法值直接丢掉，界面会显示「未配置」而不是
        // 拿一个拼坏的地址去请求。
        let repo = self.update_repo.take().and_then(|slug| {
            let slug = slug.trim().to_string();
            if crate::update::is_slug(&slug) {
                Some(slug)
            } else {
                None
            }
        });
        self.update_repo = repo;
    }

    /// 在当前预设列表里前后循环。
    pub fn cycle(&mut self, delta: i32) -> String {
        let names = self.preset_names();
        if names.is_empty() {
            return self.active.clone();
        }
        let current = names.iter().position(|n| *n == self.active).unwrap_or(0) as i32;
        let len = names.len() as i32;
        let next = ((current + delta) % len + len) % len;
        self.active = names[next as usize].clone();
        self.active.clone()
    }

    pub fn set_opacity_step(&mut self, step: i32) -> f32 {
        let current = self
            .overrides
            .opacity
            .unwrap_or(self.resolve(&self.active).opacity);
        let next = (((current * 100.0).round() as i32 + step * 10).clamp(10, 100)) as f32 / 100.0;
        self.overrides.opacity = Some(next);
        next
    }

    pub fn set_scale_step(&mut self, step: i32) -> f32 {
        let current = self
            .overrides
            .scale
            .unwrap_or(self.resolve(&self.active).scale);
        let next = (current + step as f32 * 0.1).clamp(0.3, 6.0);
        // 避免浮点累积出 1.2000001 这种值
        let next = (next * 10.0).round() / 10.0;
        self.overrides.scale = Some(next);
        next
    }

    /// 当前生效的准星偏移，单位是「100% DPI 下的像素」，原点是屏幕中心。
    ///
    /// 没有位置覆盖时要换算预设自带的本地偏移（本地单位 × scale），
    /// 否则界面会显示成 0 而准星其实偏着。
    pub fn offset_px(&self) -> [f32; 2] {
        if let Some(off) = self.overrides.offset {
            return off;
        }
        let spec = self.resolve(&self.active);
        let scale = spec.scale.max(0.05);
        [spec.offset[0] * scale, spec.offset[1] * scale]
    }

    /// 直接设定偏移（像素），返回实际生效的值。
    pub fn set_offset_px(&mut self, off: [f32; 2]) -> [f32; 2] {
        let next = [
            off[0].clamp(-OFFSET_LIMIT, OFFSET_LIMIT),
            off[1].clamp(-OFFSET_LIMIT, OFFSET_LIMIT),
        ];
        self.overrides.offset = Some(next);
        next
    }

    /// 在当前偏移上叠加一个位移（像素），返回叠加后的值。
    pub fn nudge_offset(&mut self, dx: f32, dy: f32) -> [f32; 2] {
        let current = self.offset_px();
        self.set_offset_px([current[0] + dx, current[1] + dy])
    }

    /// 把准星摆回屏幕正中。
    ///
    /// 写的是显式 `[0, 0]` 而不是删掉覆盖：自定义预设可能自带偏移，
    /// 只删覆盖的话准星会跳回预设的那个位置，用户按「回到中心」就会看到
    /// 准星「自己跑到别处去」。
    pub fn reset_offset(&mut self) {
        self.overrides.offset = Some([0.0, 0.0]);
    }

    // ---- 热键 ----

    /// 用户给这个 id 改过的组合（没改过就是 `None`）。
    pub fn hotkey_override(&self, id: i32) -> Option<HotkeyCombo> {
        if id <= 0 {
            return None;
        }
        self.hotkeys.get(&(id as u32)).copied()
    }

    /// 改一个热键，返回被这次改键「顶掉」的其它热键 id。
    ///
    /// 同一个组合只能挂一个动作（`RegisterHotKey` 对重复组合只会成功一次），
    /// 所以改键时顺手把占着同一组合的旧项删掉——用户的意图明显是「这个组合
    /// 归我了」。删掉的项会退回它的默认组合；若默认组合又撞上，由
    /// [`crate::platform::hotkeys::resolve`] 的去重兜底，界面另给冲突警告。
    pub fn set_hotkey(&mut self, id: i32, combo: HotkeyCombo) -> Vec<i32> {
        let mut bumped = Vec::new();
        if id <= 0 || !combo.is_usable() {
            return bumped;
        }
        let key = id as u32;
        self.hotkeys.retain(|other, existing| {
            if *other != key && *existing == combo {
                bumped.push(*other as i32);
                false
            } else {
                true
            }
        });
        self.hotkeys.insert(key, combo);
        bumped
    }

    /// 把一个热键恢复成内置默认值。
    pub fn reset_hotkey(&mut self, id: i32) {
        if id > 0 {
            self.hotkeys.remove(&(id as u32));
        }
    }

    /// 所有热键恢复默认。
    pub fn reset_all_hotkeys(&mut self) {
        self.hotkeys.clear();
    }

    /// 是否有热键被改过。
    pub fn has_custom_hotkeys(&self) -> bool {
        !self.hotkeys.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 读写
// ---------------------------------------------------------------------------

/// 配置路径：`--config` 指定 > 环境变量 `SCREENCROSSHAIR_CONFIG` > `%APPDATA%\ScreenCrosshair\config.json`
pub fn resolve_path(cli: Option<PathBuf>) -> PathBuf {
    if let Some(path) = cli {
        return path;
    }
    // 空字符串当成没设：环境变量很容易被 `set X=` 之类清成空值。
    if let Ok(path) = std::env::var("SCREENCROSSHAIR_CONFIG")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Roaming")))
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("ScreenCrosshair").join("config.json")
}

/// 内容哈希，用来判断文件是否被外部改动过。
pub fn file_hash(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            hasher.finish()
        }
        Err(_) => 0,
    }
}

/// 读取配置。文件不存在时返回 `Ok(None)`；解析失败时返回错误信息。
pub fn load(path: &Path) -> Result<Option<Config>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("读取 {} 失败: {err}", path.display())),
    };
    let mut config: Config = serde_json::from_str(&text)
        .map_err(|err| format!("解析 {} 失败: {err}", path.display()))?;
    config.normalize();
    Ok(Some(config))
}

/// 读取配置；任何失败都退回到默认配置（并保留错误信息给界面显示）。
pub fn load_or_default(path: &Path) -> (Config, Option<String>) {
    match load(path) {
        Ok(Some(config)) => (config, None),
        Ok(None) => (Config::default(), None),
        Err(err) => (Config::default(), Some(err)),
    }
}

/// 原子写回配置（先写临时文件再替换，避免另一个进程读到半个文件）。
pub fn save(path: &Path, config: &Config) -> Result<u64, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建目录 {} 失败: {e}", dir.display()))?;
    }
    let mut config = config.clone();
    config.normalize();
    let text = serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("写 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败: {e}", path.display()))?;
    Ok(file_hash(path))
}

/// 首次运行时写出一份带注释性默认值的配置。
pub fn ensure_exists(path: &Path) -> Result<u64, String> {
    if path.exists() {
        return Ok(file_hash(path));
    }
    save(path, &Config::default())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_spec_uses_defaults() {
        let spec: CrosshairSpec =
            serde_json::from_str(r#"{"shapes":[{"kind":"dot","radius":2.0}]}"#)
                .expect("最小配置应能解析");
        assert_eq!(spec.color, "#00FF00");
        assert_eq!(spec.opacity, 1.0);
        assert_eq!(spec.scale, 1.0);
        assert!(spec.outline.enabled);
        assert_eq!(spec.shapes.len(), 1);
    }

    #[test]
    fn shapes_parse_from_json_script() {
        let json = r#"
        {
          "shapes": [
            {"kind":"line","from":[0,-4],"to":[0,-14],"width":2},
            {"kind":"circle","radius":8,"width":1.5},
            {"kind":"arc","radius":6,"start_deg":0,"sweep_deg":90},
            {"kind":"rect","size":[10,10],"corner_radius":2},
            {"kind":"polygon","points":[[0,-6],[5,4],[-5,4]]},
            {"kind":"group","rotation_deg":45,"shapes":[{"kind":"dot","radius":1}]}
          ]
        }"#;
        let spec: CrosshairSpec = serde_json::from_str(json).expect("图元脚本应能解析");
        assert_eq!(spec.shapes.len(), 6);
        match &spec.shapes[1] {
            Shape::Circle { radius, filled, .. } => {
                assert_eq!(*radius, 8.0);
                assert!(!*filled);
            }
            other => panic!("期望 circle，实际 {other:?}"),
        }
    }

    #[test]
    fn overrides_apply_on_top_of_preset() {
        let mut config = Config {
            active: "cross".to_string(),
            ..Default::default()
        };
        let base = config.resolve("cross");
        config.overrides.color = Some("#FF0000".into());
        config.overrides.opacity = Some(0.4);
        config.overrides.scale = Some(2.0);
        config.overrides.outline = Some(false);
        let eff = config.effective();
        assert_eq!(eff.color, "#FF0000");
        assert_eq!(eff.opacity, 0.4);
        assert_eq!(eff.scale, 2.0);
        assert!(!eff.outline.enabled);
        // 图形本身来自预设
        assert_eq!(eff.shapes, base.shapes);
    }

    #[test]
    fn custom_preset_shadows_builtin() {
        let mut config = Config::default();
        let mut custom = CrosshairSpec::new("#123456", vec![Shape::dot(3.0)]);
        custom.scale = 3.0;
        config.custom_presets.insert("mine".into(), custom);
        let spec = config.resolve("mine");
        assert_eq!(spec.color, "#123456");
        assert_eq!(spec.shapes.len(), 1);
    }

    #[test]
    fn unknown_preset_falls_back() {
        let config = Config::default();
        let spec = config.resolve("does-not-exist");
        assert!(!spec.shapes.is_empty());
    }

    #[test]
    fn cycle_wraps_both_directions() {
        let mut config = Config::default();
        let names = config.preset_names();
        assert!(names.len() >= 3);
        config.active = names[0].clone();
        config.cycle(-1);
        assert_eq!(config.active, names[names.len() - 1]);
        config.cycle(1);
        assert_eq!(config.active, names[0]);
    }

    #[test]
    fn normalize_clamps_out_of_range_values() {
        let mut spec = CrosshairSpec {
            opacity: 3.0,
            scale: 0.0,
            ..Default::default()
        };
        spec.shapes.clear();
        spec.normalize();
        assert_eq!(spec.opacity, 1.0);
        assert!(spec.scale >= 0.05);
        assert!(!spec.shapes.is_empty(), "空图形列表应回退到默认预设");
    }

    #[test]
    fn opacity_and_scale_steps_stay_in_range() {
        let mut config = Config::default();
        assert_eq!(config.set_opacity_step(-99), 0.1);
        assert_eq!(config.set_opacity_step(99), 1.0);
        assert_eq!(config.set_scale_step(-99), 0.3);
        assert_eq!(config.set_scale_step(99), 6.0);
    }

    #[test]
    fn nudge_offset_is_in_pixels_regardless_of_scale() {
        let mut config = Config::default();
        assert_eq!(config.offset_px(), [0.0, 0.0]);

        // 1 倍缩放下挪 3 像素：渲染用的本地偏移就是 3。
        config.overrides.scale = Some(1.0);
        assert_eq!(config.nudge_offset(3.0, -2.0), [3.0, -2.0]);
        let spec = config.effective();
        assert_eq!(spec.offset, [3.0, -2.0]);

        // 2 倍缩放下同样挪 3 像素：本地偏移要减半，乘回 scale 才是 3。
        config.overrides.scale = Some(2.0);
        config.reset_offset();
        config.nudge_offset(3.0, 0.0);
        let spec = config.effective();
        assert_eq!(spec.offset, [1.5, 0.0]);
        assert_eq!(spec.offset[0] * spec.scale, 3.0);
    }

    #[test]
    fn offset_override_wins_and_is_clamped() {
        let mut config = Config::default();
        config.nudge_offset(1.0e9, -1.0e9);
        assert_eq!(config.offset_px(), [OFFSET_LIMIT, -OFFSET_LIMIT]);
        assert_eq!(config.effective().offset[0], OFFSET_LIMIT);

        // 反序列化出来的越界值也要在 normalize 里被收回来。
        let mut config: Config =
            serde_json::from_str(r#"{"overrides":{"offset":[1e9,-1e9]}}"#).unwrap();
        config.normalize();
        assert_eq!(config.overrides.offset, Some([OFFSET_LIMIT, -OFFSET_LIMIT]));
    }

    #[test]
    fn offset_none_falls_back_to_preset_offset() {
        let mut config = Config::default();
        let mut custom = CrosshairSpec::new("#FFFFFF", vec![Shape::dot(2.0)]);
        custom.offset = [4.0, -6.0];
        custom.scale = 2.0;
        config.custom_presets.insert("shifted".into(), custom);
        config.active = "shifted".into();

        // 没有覆盖时按「本地偏移 × scale」换算成像素显示。
        assert_eq!(config.offset_px(), [8.0, -12.0]);
        assert!(config.effective().offset[0].is_finite());

        // 「回到中心」必须压过预设自带的偏移。
        config.reset_offset();
        assert_eq!(config.offset_px(), [0.0, 0.0]);
        assert_eq!(config.effective().offset, [0.0, 0.0]);
    }

    #[test]
    fn monitor_selection_parses() {
        assert_eq!(MonitorSel::from_index(None).index(), None);
        assert_eq!(MonitorSel::from_index(Some(2)).index(), Some(2));
        assert_eq!(
            serde_json::from_str::<MonitorSel>("\"primary\"")
                .unwrap()
                .index(),
            None
        );
        assert_eq!(
            serde_json::from_str::<MonitorSel>("1").unwrap().index(),
            Some(1)
        );
        assert_eq!(
            serde_json::from_str::<MonitorSel>("\"1\"").unwrap().index(),
            Some(1)
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sch-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut config = Config {
            active: "circle".into(),
            ..Default::default()
        };
        config.overrides.opacity = Some(0.5);
        let hash = save(&path, &config).expect("保存应成功");
        assert_eq!(hash, file_hash(&path));

        let (loaded, err) = load_or_default(&path);
        assert!(err.is_none());
        assert_eq!(loaded.active, "circle");
        assert_eq!(loaded.overrides.opacity, Some(0.5));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broken_file_reports_error_instead_of_panicking() {
        let dir = std::env::temp_dir().join(format!("sch-test-bad-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let (config, err) = load_or_default(&path);
        assert!(err.is_some());
        assert_eq!(config.active, presets::FALLBACK);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_config_serializes_to_stable_shape() {
        let text = serde_json::to_string_pretty(&Config::default()).unwrap();
        let reparsed: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(reparsed, Config::default());
        // overrides 全空时不应写出 null 字段
        assert!(!text.contains("\"overrides\": {\n    \"color\""));
    }

    /// 热键差异表必须能原样落盘再读回来。
    ///
    /// 丢了字段的表现是「重启后改过的键位全丢」，而程序运行期间一切正常，
    /// 只有下一次启动才暴露 —— 单测里唯一的抓手就是这条往返。
    #[test]
    fn hotkey_bindings_roundtrip() {
        let mut c = Config::default();
        let combo = HotkeyCombo::new(true, true, false, 0x74); // Ctrl+Alt+F5
        assert!(c.set_hotkey(12, combo).is_empty());
        c.hotkeys_enabled = false;

        let text = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert!(!back.hotkeys_enabled);
        assert_eq!(back.hotkey_override(12), Some(combo));
        // 没改过的 id 不写出字段，读回来也还是 None
        assert_eq!(back.hotkey_override(1), None);
        assert!(back.has_custom_hotkeys());
    }

    /// 用不到的绑定（裸键 / 越界 id）必须在 `normalize` 后被清掉。
    ///
    /// 裸键是重点：`RegisterHotKey` 接受它，但那会把普通打字变成全局热键，
    /// 用户只是手改了一下 JSON，不该被这种输入毁掉体验。
    #[test]
    fn normalize_drops_unusable_hotkeys() {
        let mut c = Config::default();
        c.hotkeys
            .insert(3, HotkeyCombo::new(false, false, false, 0x48)); // 裸 H
        c.hotkeys
            .insert(MAX_HOTKEY_ID + 1, HotkeyCombo::new(true, true, false, 0x48));
        c.hotkeys
            .insert(0, HotkeyCombo::new(true, true, false, 0x48));
        c.hotkeys
            .insert(5, HotkeyCombo::new(true, true, false, 0x48)); // 合法
        c.normalize();
        assert_eq!(c.hotkeys.len(), 1);
        assert!(c.hotkeys.contains_key(&5));
    }

    /// 只改颜色时，`effective()` 的几何部分必须**逐字段不变**。
    ///
    /// 这条用例和 `crosshair::tests::color_never_moves_the_crosshair` 配对：
    /// 那条守「渲染不改位置」，这条守「叠加覆盖值时不串味」。`effective()` 里
    /// 把偏移按尺寸除回去（`offset / scale`）之后才 `normalize()`，任何一处
    /// 顺序写错都会让「换个颜色」顺手改掉位置，而这种现象在界面上极难归因。
    #[test]
    fn changing_color_keeps_geometry() {
        let mut cfg = Config::default();
        cfg.overrides.offset = Some([12.0, -7.0]);
        cfg.overrides.scale = Some(1.75);
        cfg.overrides.opacity = Some(0.8);

        let before = cfg.effective();
        cfg.overrides.color = Some("#FF3B30".to_string());
        let after = cfg.effective();

        assert_eq!(after.color, "#FF3B30", "颜色本身要变");
        assert_eq!(before.offset, after.offset, "偏移被颜色改动了");
        assert_eq!(before.scale, after.scale, "尺寸被颜色改动了");
        assert_eq!(before.rotation_deg, after.rotation_deg);
        assert_eq!(before.opacity, after.opacity);
        assert_eq!(before.shapes, after.shapes, "图元被颜色改动了");
    }

    /// 所有内置 + 自定义预设都过一遍：换颜色后几何量完全一致。
    #[test]
    fn changing_color_keeps_geometry_for_every_preset() {
        let mut cfg = Config::default();
        cfg.overrides.offset = Some([-25.0, 40.0]);
        for name in cfg.preset_names() {
            cfg.active = name.clone();
            let before = cfg.effective();
            let offset_before = cfg.offset_px();
            cfg.overrides.color = Some("#FFD60A".to_string());
            let after = cfg.effective();
            assert_eq!(before.offset, after.offset, "预设 {name} 的偏移变了");
            assert_eq!(before.scale, after.scale, "预设 {name} 的尺寸变了");
            assert_eq!(offset_before, cfg.offset_px(), "预设 {name} 的像素偏移变了");
        }
    }
}
