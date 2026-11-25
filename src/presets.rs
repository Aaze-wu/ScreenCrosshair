//! 内置准星预设库。
//!
//! 每个预设都是纯数据（[`CrosshairSpec`]），用户可以在 `config.json` 的
//! `custom_presets` 里用同样的 JSON 结构画出任意形状。

use crate::config::{CrosshairSpec, Shape};

/// 找不到预设、或配置为空时的兜底预设名。
pub const FALLBACK: &str = "cross_dot";

/// (名字, 中文标签)，顺序即切换顺序。
pub const BUILTINS: &[(&str, &str)] = &[
    ("dot", "圆点"),
    ("dot_small", "小圆点"),
    ("cross", "十字"),
    ("cross_thin", "细十字"),
    ("cross_dot", "十字 + 点"),
    ("circle", "圆环"),
    ("circle_dot", "圆环 + 点"),
    ("cross_circle", "十字 + 圆环"),
    ("t_shape", "T 形"),
    ("chevron", "V 形"),
    ("triangle", "三角箭头"),
    ("brackets", "四角括号"),
    ("square", "方框"),
    ("x_cross", "斜十字"),
];

/// 预设名列表（与 [`BUILTINS`] 同序）。
pub fn names() -> Vec<String> {
    BUILTINS.iter().map(|(name, _)| name.to_string()).collect()
}

/// 中文标签，未知名返回原名。
pub fn label(name: &str) -> String {
    BUILTINS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| name.to_string())
}

/// 查表构造内置预设。
pub fn builtin(name: &str) -> Option<CrosshairSpec> {
    let spec = match name {
        "dot" => spec("#00FF00", vec![Shape::dot(2.5)]),
        "dot_small" => spec("#00FF00", vec![Shape::dot(1.5)]),
        "cross" => spec("#00FF00", rays(4, 0.0, 14.0, 2.0)),
        "cross_thin" => spec("#00FF00", rays(4, 5.0, 13.0, 1.2)),
        "cross_dot" => {
            let mut shapes = arms(4, 3.5, 13.0, 2.0, 0.0);
            shapes.push(Shape::dot(1.6));
            spec("#00FF00", shapes)
        }
        "circle" => spec("#00FF00", vec![Shape::ring(10.0, 1.6)]),
        "circle_dot" => spec("#00E5FF", vec![Shape::ring(12.0, 2.0), Shape::dot(2.0)]),
        "cross_circle" => {
            let mut shapes = arms(4, 3.0, 9.0, 1.8, 0.0);
            shapes.push(Shape::ring(12.0, 1.8));
            spec("#00FF00", shapes)
        }
        "t_shape" => spec(
            "#00FF00",
            vec![
                Shape::line([-8.0, -8.0], [8.0, -8.0], 2.0),
                Shape::line([0.0, -8.0], [0.0, 8.0], 2.0),
            ],
        ),
        "chevron" => spec(
            "#FFD60A",
            vec![
                Shape::round_line([-8.0, 4.0], [0.0, -6.0], 2.0),
                Shape::round_line([0.0, -6.0], [8.0, 4.0], 2.0),
            ],
        ),
        "triangle" => spec(
            "#00FF00",
            vec![Shape::Polygon {
                points: vec![[0.0, -9.0], [-7.0, 5.0], [7.0, 5.0]],
                closed: true,
                filled: false,
                width: 2.0,
            }],
        ),
        "brackets" => spec("#00FF00", brackets(10.0, 5.0, 2.0)),
        "square" => spec(
            "#00FF00",
            vec![Shape::Rect {
                center: [0.0, 0.0],
                size: [15.0, 15.0],
                corner_radius: 0.0,
                filled: false,
                width: 1.8,
                rotation_deg: 0.0,
            }],
        ),
        "x_cross" => spec("#00FF00", arms(4, 4.0, 13.0, 2.0, 45.0)),
        _ => return None,
    };
    Some(spec)
}

fn spec(color: &str, shapes: Vec<Shape>) -> CrosshairSpec {
    let mut spec = CrosshairSpec::new(color, shapes);
    spec.normalize();
    spec
}

/// 以 `angle0` 为起点、均匀分布的 `count` 条射线，从 `gap` 伸到 `outer`。
fn arms(count: usize, gap: f32, outer: f32, width: f32, angle0: f32) -> Vec<Shape> {
    (0..count)
        .map(|i| {
            let angle = angle0 + 360.0 * i as f32 / count as f32;
            Shape::ray(angle, gap, outer, width)
        })
        .collect()
}

/// `arms(4, …)` 的别名，语义上更接近“十字/斜十字”。
fn rays(count: usize, gap: f32, outer: f32, width: f32) -> Vec<Shape> {
    arms(count, gap, outer, width, 0.0)
}

/// 四个角括号。
fn brackets(radius: f32, arm: f32, width: f32) -> Vec<Shape> {
    let r = radius;
    let inner = radius - arm;
    let mut shapes = Vec::with_capacity(8);
    for &sx in &[-1.0f32, 1.0] {
        for &sy in &[-1.0f32, 1.0] {
            shapes.push(Shape::line([sx * r, sy * r], [sx * inner, sy * r], width));
            shapes.push(Shape::line([sx * r, sy * r], [sx * r, sy * inner], width));
        }
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CapKind;

    #[test]
    fn every_builtin_name_resolves() {
        for (name, label) in BUILTINS {
            let spec = builtin(name).unwrap_or_else(|| panic!("预设 {name} 缺失"));
            assert!(!spec.shapes.is_empty(), "预设 {name} 没有任何图元");
            assert!(
                crate::color::parse(&spec.color).is_some(),
                "预设 {name} 颜色非法"
            );
            assert!(spec.scale > 0.0);
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn fallback_exists() {
        assert!(builtin(FALLBACK).is_some());
    }

    #[test]
    fn brackets_have_eight_lines() {
        assert_eq!(brackets(10.0, 5.0, 2.0).len(), 8);
    }

    #[test]
    fn rays_use_round_cap_none_and_expected_count() {
        let shapes = arms(4, 4.0, 13.0, 2.0, 45.0);
        assert_eq!(shapes.len(), 4);
        for shape in shapes {
            match shape {
                Shape::Line { cap, .. } => assert_eq!(cap, CapKind::Butt),
                other => panic!("期望 line，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(builtin("nope").is_none());
        assert_eq!(label("nope"), "nope");
    }
}
