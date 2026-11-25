//! 把 [`CrosshairSpec`] 光栅化成像素。
//!
//! 渲染只用 tiny-skia（纯 CPU 矢量光栅化，自带抗锯齿），不依赖 GPU，
//! 因此覆盖层进程不会碰到任何显卡驱动兼容性问题。

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::color::{self, Rgba};
use crate::config::{CapKind, CrosshairSpec, Shape};

/// 2D 仿射矩阵，字段顺序与 SVG `matrix(a b c d e f)` 一致：
/// `x' = a*x + c*y + e`，`y' = b*x + d*y + f`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Mat {
    pub const IDENTITY: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Mat {
        Mat {
            e: x,
            f: y,
            ..Mat::IDENTITY
        }
    }

    pub fn scale(s: f32) -> Mat {
        Mat {
            a: s,
            d: s,
            ..Mat::IDENTITY
        }
    }

    pub fn rotate_deg(deg: f32) -> Mat {
        let (sin, cos) = deg.to_radians().sin_cos();
        Mat {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// 矩阵乘法：先应用 `rhs`，再应用 `self`。
    pub fn chain(self, rhs: Mat) -> Mat {
        Mat {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    /// 该矩阵对长度的统一缩放系数（用于把线宽换算成设备像素）。
    pub fn scale_factor(self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }

    pub fn to_transform(self) -> Transform {
        Transform::from_row(self.a, self.b, self.c, self.d, self.e, self.f)
    }
}

/// 一个展开后的图元。
struct Prim {
    path: Path,
    matrix: Mat,
    /// 描边宽度（本地单位，绘制时乘 `matrix.scale_factor()`）
    width: f32,
    filled: bool,
    cap: LineCap,
}

/// 渲染时真正使用的颜色。
struct Style {
    main: Color,
    outline: Option<Color>,
    outline_width: f32,
}

fn make_style(spec: &CrosshairSpec) -> Style {
    let main_rgba = color::parse_or(&spec.color, Rgba::opaque(0, 255, 0));
    let outline = if spec.outline.enabled && spec.outline.width > 0.0 && spec.outline.opacity > 0.0
    {
        let c = color::parse_or(&spec.outline.color, Rgba::opaque(0, 0, 0))
            .with_opacity(spec.outline.opacity * spec.opacity);
        Some(to_skia(c))
    } else {
        None
    };
    Style {
        main: to_skia(main_rgba.with_opacity(spec.opacity)),
        outline,
        outline_width: spec.outline.width,
    }
}

fn to_skia(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// 在 `pixmap` 上把准星画到设备坐标 `center`。
///
/// `dpi_scale`：显示器缩放（96 DPI = 1.0）。返回是否真的画了东西。
pub fn draw(pixmap: &mut Pixmap, spec: &CrosshairSpec, center: (f32, f32), dpi_scale: f32) -> bool {
    let style = make_style(spec);
    let mut prims: Vec<Prim> = Vec::new();

    let total_scale = spec.scale * dpi_scale.clamp(0.1, 8.0);
    let root = Mat::translate(
        center.0 + spec.offset[0] * total_scale,
        center.1 + spec.offset[1] * total_scale,
    )
    .chain(Mat::rotate_deg(spec.rotation_deg))
    .chain(Mat::scale(total_scale));

    collect(&spec.shapes, root, &mut prims);
    if prims.is_empty() {
        return false;
    }

    // 第一遍画描边：把轮廓铺在所有图元下面，交叉处才是统一的黑边而不是互相压掉。
    if let Some(outline) = style.outline {
        let mut paint = Paint::default();
        paint.set_color(outline);
        paint.anti_alias = true;
        for prim in &prims {
            let local = if prim.filled {
                style.outline_width * 2.0
            } else {
                prim.width + style.outline_width * 2.0
            };
            let stroke = Stroke {
                width: (local * prim.matrix.scale_factor()).max(0.01),
                miter_limit: 4.0,
                line_cap: prim.cap,
                line_join: LineJoin::Round,
                dash: None,
            };
            // 描边先画、本体后画，这样本体会盖住描边的内侧一半，
            // 看上去就是一圈均匀的黑边而不是「加粗了」。
            pixmap.stroke_path(
                &prim.path,
                &paint,
                &stroke,
                prim.matrix.to_transform(),
                None,
            );
        }
    }

    // 第二遍画本体。
    let mut paint = Paint::default();
    paint.set_color(style.main);
    paint.anti_alias = true;
    for prim in &prims {
        let transform = prim.matrix.to_transform();
        if prim.filled {
            pixmap.fill_path(&prim.path, &paint, FillRule::Winding, transform, None);
        } else {
            let stroke = Stroke {
                width: (prim.width * prim.matrix.scale_factor()).max(0.01),
                miter_limit: 4.0,
                line_cap: prim.cap,
                line_join: LineJoin::Round,
                dash: None,
            };
            pixmap.stroke_path(&prim.path, &paint, &stroke, transform, None);
        }
    }
    true
}

fn collect(shapes: &[Shape], parent: Mat, out: &mut Vec<Prim>) {
    for shape in shapes {
        match shape {
            Shape::Group {
                shapes,
                rotation_deg,
                translate,
                scale,
            } => {
                let child = parent
                    .chain(Mat::translate(translate[0], translate[1]))
                    .chain(Mat::rotate_deg(*rotation_deg))
                    .chain(Mat::scale(scale.max(0.001)));
                collect(shapes, child, out);
            }
            Shape::Line {
                from,
                to,
                width,
                cap,
            } => {
                if *width <= 0.0 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(from[0], from[1]);
                pb.line_to(to[0], to[1]);
                push(out, pb, parent, *width, false, cap_of(*cap));
            }
            Shape::Dot { center, radius } => {
                if *radius <= 0.0 {
                    continue;
                }
                push(
                    out,
                    circle_pb(center[0], center[1], *radius),
                    parent,
                    0.0,
                    true,
                    LineCap::Butt,
                );
            }
            Shape::Circle {
                center,
                radius,
                filled,
                width,
            } => {
                if *radius <= 0.0 {
                    continue;
                }
                push(
                    out,
                    circle_pb(center[0], center[1], *radius),
                    parent,
                    *width,
                    *filled,
                    LineCap::Butt,
                );
            }
            Shape::Arc {
                center,
                radius,
                start_deg,
                sweep_deg,
                width,
                cap,
            } => {
                if *radius <= 0.0 || sweep_deg.abs() < 0.01 || *width <= 0.0 {
                    continue;
                }
                push(
                    out,
                    arc_pb(center[0], center[1], *radius, *start_deg, *sweep_deg),
                    parent,
                    *width,
                    false,
                    cap_of(*cap),
                );
            }
            Shape::Rect {
                center,
                size,
                corner_radius,
                filled,
                width,
                rotation_deg,
            } => {
                let (w, h) = (size[0].abs(), size[1].abs());
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                let m = parent.chain(Mat::rotate_deg(*rotation_deg));
                let r = corner_radius.min(w * 0.5).min(h * 0.5).max(0.0);
                push(
                    out,
                    rounded_rect_pb(center[0], center[1], w, h, r),
                    m,
                    *width,
                    *filled,
                    LineCap::Butt,
                );
            }
            Shape::Polygon {
                points,
                closed,
                filled,
                width,
            } => {
                if points.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(points[0][0], points[0][1]);
                for p in &points[1..] {
                    pb.line_to(p[0], p[1]);
                }
                if *closed {
                    pb.close();
                }
                push(out, pb, parent, *width, *filled, LineCap::Butt);
            }
        }
    }
}

fn push(out: &mut Vec<Prim>, pb: PathBuilder, matrix: Mat, width: f32, filled: bool, cap: LineCap) {
    let path = match pb.finish() {
        Some(path) => path,
        None => return,
    };
    out.push(Prim {
        path,
        matrix,
        width,
        filled,
        cap,
    });
}

fn cap_of(cap: CapKind) -> LineCap {
    match cap {
        CapKind::Butt => LineCap::Butt,
        CapKind::Round => LineCap::Round,
        CapKind::Square => LineCap::Square,
    }
}

/// 四段三次贝塞尔的整圆。
fn circle_pb(cx: f32, cy: f32, r: f32) -> PathBuilder {
    const K: f32 = 0.552_284_8;
    let k = r * K;
    let mut pb = PathBuilder::new();
    pb.move_to(cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.close();
    pb
}

/// 圆弧（角度制，0° 指向 +X，顺时针为正），用足够密的折线近似。
fn arc_pb(cx: f32, cy: f32, r: f32, start_deg: f32, sweep_deg: f32) -> PathBuilder {
    let steps = ((sweep_deg.abs() / 3.0).ceil() as usize).clamp(4, 720);
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (sin, cos) = (start_deg + sweep_deg * t).to_radians().sin_cos();
        let (x, y) = (cx + r * cos, cy + r * sin);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb
}

/// 圆角矩形。`r <= 0` 时退化成直角矩形。
fn rounded_rect_pb(cx: f32, cy: f32, w: f32, h: f32, r: f32) -> PathBuilder {
    const K: f32 = 0.552_284_8;
    let (hw, hh) = (w * 0.5, h * 0.5);
    let (l, t, rt, b) = (cx - hw, cy - hh, cx + hw, cy + hh);
    let mut pb = PathBuilder::new();
    if r <= 0.01 {
        pb.move_to(l, t);
        pb.line_to(rt, t);
        pb.line_to(rt, b);
        pb.line_to(l, b);
        pb.close();
        return pb;
    }
    let k = r * K;
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb
}

// ---------------------------------------------------------------------------
// 像素格式转换 / 预览
// ---------------------------------------------------------------------------

/// tiny-skia 输出的是预乘 RGBA；`UpdateLayeredWindow` 要的是预乘 BGRA。
pub fn rgba_to_bgra_premultiplied(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// 预乘 → 非预乘（egui 的 `ColorImage::from_rgba_unmultiplied` 需要）。
pub fn unpremultiply(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        let a = px[3];
        if a == 0 || a == 255 {
            continue;
        }
        let inv = 255.0 / a as f32;
        for c in &mut px[0..3] {
            *c = ((*c as f32 * inv).round()).clamp(0.0, 255.0) as u8;
        }
    }
}

/// 生成一张预览图（预乘 RGBA，宽高均为 `size`），给设置界面用。
pub fn render_preview(spec: &CrosshairSpec, size: u32, dpi_scale: f32) -> Vec<u8> {
    let size = size.max(8);
    let mut pixmap = Pixmap::new(size, size).expect("预览图分配失败");
    let center = (size as f32 * 0.5, size as f32 * 0.5);
    draw(&mut pixmap, spec, center, dpi_scale);
    pixmap.data().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn count_opaque(pixmap: &Pixmap) -> usize {
        pixmap.data().chunks_exact(4).filter(|px| px[3] > 8).count()
    }

    fn bbox(pixmap: &Pixmap) -> Option<(u32, u32, u32, u32)> {
        let w = pixmap.width();
        let h = pixmap.height();
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let data = pixmap.data();
        for y in 0..h {
            for x in 0..w {
                if data[((y * w + x) * 4) as usize + 3] > 8 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x0 == u32::MAX {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }

    #[test]
    fn matrix_chain_applies_child_first() {
        // 先放大 2 倍，再平移 10：点 (1,0) → (2,0) → (12,0)
        let m = Mat::translate(10.0, 0.0).chain(Mat::scale(2.0));
        let x = m.a * 1.0 + m.c * 0.0 + m.e;
        let y = m.b * 1.0 + m.d * 0.0 + m.f;
        assert!((x - 12.0).abs() < 1e-4, "x={x}");
        assert!(y.abs() < 1e-4);
    }

    #[test]
    fn rotation_keeps_uniform_scale() {
        let m = Mat::rotate_deg(37.0).chain(Mat::scale(3.0));
        assert!((m.scale_factor() - 3.0).abs() < 1e-3);
    }

    #[test]
    fn every_preset_renders_something() {
        for (name, _) in presets::BUILTINS {
            let spec = presets::builtin(name).unwrap();
            let mut pixmap = Pixmap::new(160, 160).unwrap();
            assert!(
                draw(&mut pixmap, &spec, (80.0, 80.0), 1.0),
                "预设 {name} 什么都没画出来"
            );
            assert!(count_opaque(&pixmap) > 0, "预设 {name} 没有任何可见像素");
        }
    }

    #[test]
    fn crosshair_is_centered() {
        let spec = presets::builtin("cross_dot").unwrap();
        let mut pixmap = Pixmap::new(128, 128).unwrap();
        draw(&mut pixmap, &spec, (64.0, 64.0), 1.0);
        let (x0, y0, x1, y1) = bbox(&pixmap).expect("应有像素");
        let cx = (x0 + x1) as f32 * 0.5;
        let cy = (y0 + y1) as f32 * 0.5;
        assert!((cx - 63.5).abs() <= 2.5, "水平中心偏移: {cx}");
        assert!((cy - 63.5).abs() <= 2.5, "垂直中心偏移: {cy}");
    }

    #[test]
    fn scale_enlarges_footprint() {
        let mut small = presets::builtin("circle").unwrap();
        small.scale = 1.0;
        let mut large = small.clone();
        large.scale = 3.0;

        let mut p1 = Pixmap::new(256, 256).unwrap();
        let mut p2 = Pixmap::new(256, 256).unwrap();
        draw(&mut p1, &small, (128.0, 128.0), 1.0);
        draw(&mut p2, &large, (128.0, 128.0), 1.0);

        let a = bbox(&p1).unwrap();
        let b = bbox(&p2).unwrap();
        let w1 = (a.2 - a.0) as f32;
        let w2 = (b.2 - b.0) as f32;
        assert!(w2 > w1 * 2.0, "放大 3 倍后宽度应该明显变大: {w1} -> {w2}");
    }

    #[test]
    fn stroke_thickness_grows_with_scale() {
        // 一条 2px 宽的线放大 4 倍，粗细也应随之变粗（而不是恒为 2px）。
        let mk = |scale: f32| {
            let mut spec =
                CrosshairSpec::new("#FFFFFF", vec![Shape::line([-10.0, 0.0], [10.0, 0.0], 2.0)]);
            spec.outline.enabled = false;
            spec.scale = scale;
            let mut pixmap = Pixmap::new(200, 200).unwrap();
            draw(&mut pixmap, &spec, (100.0, 100.0), 1.0);
            let data = pixmap.data();
            let mut rows = 0;
            for y in 0..200u32 {
                if data[((y * 200 + 100) * 4) as usize + 3] > 8 {
                    rows += 1;
                }
            }
            rows
        };
        let thin = mk(1.0);
        let thick = mk(4.0);
        assert!((2..=4).contains(&thin), "1 倍时应是 2px 左右，实际 {thin}");
        assert!(
            thick > thin * 2,
            "4 倍缩放时线宽应跟着变粗: {thin} -> {thick}"
        );
    }

    #[test]
    fn opacity_scales_alpha() {
        let mut spec = presets::builtin("dot").unwrap();
        spec.opacity = 0.5;
        spec.outline.enabled = false;
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        draw(&mut pixmap, &spec, (32.0, 32.0), 1.0);
        let max_alpha = pixmap
            .data()
            .chunks_exact(4)
            .map(|px| px[3])
            .max()
            .unwrap_or(0);
        assert!((100..=140).contains(&max_alpha), "alpha={max_alpha}");
    }

    #[test]
    fn outline_surrounds_shape() {
        let mut spec = presets::builtin("dot").unwrap();
        spec.color = "#00FF00".into();
        spec.outline.color = "#FF0000".into();
        spec.outline.enabled = true;
        spec.outline.width = 2.0;
        spec.outline.opacity = 1.0;
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        draw(&mut pixmap, &spec, (32.0, 32.0), 1.0);
        let data = pixmap.data();
        let idx = ((32 * 64 + 32) * 4) as usize;
        assert!(data[idx + 1] > data[idx], "中心应以绿色为主");
        let mut red_pixels = 0;
        for px in data.chunks_exact(4) {
            if px[3] > 8 && px[0] as u16 > px[1] as u16 + 30 {
                red_pixels += 1;
            }
        }
        assert!(red_pixels > 10, "描边应产生红色像素，实际 {red_pixels}");
    }

    #[test]
    fn empty_spec_draws_nothing_but_normalize_fills_it() {
        let mut spec = CrosshairSpec::new("#FFFFFF", vec![]);
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        assert!(
            !draw(&mut pixmap, &spec, (32.0, 32.0), 1.0),
            "空 shapes 不应画出任何东西"
        );

        spec.normalize();
        assert!(!spec.shapes.is_empty(), "normalize 应回退到默认预设");
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        assert!(draw(&mut pixmap, &spec, (32.0, 32.0), 1.0));
    }

    #[test]
    fn group_transform_moves_shape() {
        let spec = CrosshairSpec::new(
            "#FFFFFF",
            vec![Shape::Group {
                shapes: vec![Shape::dot(2.0)],
                rotation_deg: 0.0,
                translate: [20.0, 0.0],
                scale: 1.0,
            }],
        );
        let mut pixmap = Pixmap::new(128, 128).unwrap();
        draw(&mut pixmap, &spec, (64.0, 64.0), 1.0);
        let (x0, _y0, x1, _y1) = bbox(&pixmap).unwrap();
        let cx = (x0 + x1) as f32 * 0.5;
        assert!(cx > 78.0, "组合平移没生效，中心在 {cx}");
    }

    #[test]
    fn bgra_swap_and_unpremultiply_are_correct() {
        let mut data = vec![10u8, 20, 30, 255];
        rgba_to_bgra_premultiplied(&mut data);
        assert_eq!(data, vec![30, 20, 10, 255]);

        let mut data = vec![128u8, 64, 0, 128];
        unpremultiply(&mut data);
        assert_eq!(data[3], 128);
        assert!(data[0] >= 250, "反预乘后应接近 255，实际 {}", data[0]);
    }

    #[test]
    fn preview_has_expected_size() {
        let spec = presets::builtin("cross").unwrap();
        let data = render_preview(&spec, 64, 1.0);
        assert_eq!(data.len(), 64 * 64 * 4);
    }
}
