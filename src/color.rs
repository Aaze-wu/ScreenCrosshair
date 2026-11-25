//! 颜色解析与格式化。
//!
//! 支持的写法：
//! - `#RGB`            → 每个分量重复一次，不透明
//! - `#RRGGBB`         → 不透明
//! - `#RRGGBBAA`       → 带透明度（CSS 顺序，最后两位是 alpha）
//! - 具名颜色          → 见 [`NAMED`]

/// 一个不预乘的 RGBA 颜色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// 用额外的透明度（0..=1）调制本颜色的 alpha。
    pub fn with_opacity(self, opacity: f32) -> Self {
        let o = opacity.clamp(0.0, 1.0);
        let a = (self.a as f32 * o).round().clamp(0.0, 255.0) as u8;
        Self { a, ..self }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub fn to_hex_with_alpha(self) -> String {
        if self.a == 255 {
            self.to_hex()
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

/// 具名颜色表（全部不透明）。
pub const NAMED: &[(&str, (u8, u8, u8))] = &[
    ("black", (0, 0, 0)),
    ("white", (255, 255, 255)),
    ("red", (255, 59, 48)),
    ("green", (0, 255, 0)),
    ("lime", (0, 255, 0)),
    ("yellow", (255, 214, 10)),
    ("cyan", (48, 213, 200)),
    ("magenta", (255, 45, 149)),
    ("orange", (255, 159, 10)),
    ("blue", (10, 132, 255)),
    ("pink", (255, 45, 149)),
    ("gray", (142, 142, 147)),
    ("grey", (142, 142, 147)),
];

/// 托盘/设置界面里提供的常用颜色。
pub const PALETTE: &[(&str, &str)] = &[
    ("绿", "#00FF00"),
    ("青", "#30D5C8"),
    ("红", "#FF3B30"),
    ("黄", "#FFD60A"),
    ("品红", "#FF2D95"),
    ("橙", "#FF9F0A"),
    ("蓝", "#0A84FF"),
    ("白", "#FFFFFF"),
    ("灰", "#8E8E93"),
    ("黑", "#000000"),
];

/// 宽松解析颜色字符串；无法解析时返回 `None`。
pub fn parse(input: &str) -> Option<Rgba> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }

    let lower = s.to_ascii_lowercase();
    for (name, (r, g, b)) in NAMED {
        if *name == lower {
            return Some(Rgba::opaque(*r, *g, *b));
        }
    }
    // 允许不带 '#' 的十六进制
    parse_hex(s)
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    let all_hex = hex.chars().all(|c| c.is_ascii_hexdigit());
    if !all_hex {
        return None;
    }
    let nib = |c: char| c.to_digit(16).unwrap_or(0) as u8;
    match hex.len() {
        3 => {
            let v: Vec<char> = hex.chars().collect();
            let (r, g, b) = (nib(v[0]), nib(v[1]), nib(v[2]));
            Some(Rgba::opaque(r * 17, g * 17, b * 17))
        }
        4 => {
            let v: Vec<char> = hex.chars().collect();
            let (r, g, b, a) = (nib(v[0]), nib(v[1]), nib(v[2]), nib(v[3]));
            Some(Rgba::new(r * 17, g * 17, b * 17, a * 17))
        }
        6 => Some(Rgba::opaque(
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        )),
        8 => Some(Rgba::new(
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        )),
        _ => None,
    }
}

/// 解析颜色，失败时用回退色（保证渲染路径永远不会因为一个手写错的颜色而崩掉）。
pub fn parse_or(input: &str, fallback: Rgba) -> Rgba {
    parse(input).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_forms() {
        assert_eq!(parse("#0f0"), Some(Rgba::opaque(0, 255, 0)));
        assert_eq!(parse("#00FF00"), Some(Rgba::opaque(0, 255, 0)));
        assert_eq!(parse("00FF00"), Some(Rgba::opaque(0, 255, 0)));
        assert_eq!(parse("#00FF0080"), Some(Rgba::new(0, 255, 0, 0x80)));
        assert_eq!(parse("  #f00  "), Some(Rgba::opaque(255, 0, 0)));
        assert_eq!(parse("red"), Some(Rgba::opaque(255, 59, 48)));
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("not-a-color"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn opacity_modulates_alpha_only() {
        let c = Rgba::opaque(10, 20, 30).with_opacity(0.5);
        assert_eq!(c, Rgba::new(10, 20, 30, 128));
        let c = Rgba::opaque(1, 2, 3).with_opacity(0.0);
        assert_eq!(c.a, 0);
        let c = Rgba::opaque(1, 2, 3).with_opacity(9.0);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn hex_roundtrip() {
        let c = Rgba::opaque(0x12, 0x34, 0x56);
        assert_eq!(c.to_hex(), "#123456");
        assert_eq!(parse(&c.to_hex()), Some(c));
    }

    #[test]
    fn palette_entries_are_parseable() {
        for (label, hex) in PALETTE {
            assert!(parse(hex).is_some(), "调色板颜色 {label} ({hex}) 无法解析");
        }
    }
}
