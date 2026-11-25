//! 应用图标：一份图形，三处使用。
//!
//! 仓库里没有手工绘制的 `.ico`，图标是**用准星渲染器现算出来的**：
//! 深色圆角底板 + 亮绿十字（中间留缺口）+ 中心圆点，缩到 16px 仍能看出
//! 「准星」这个形状，而不用认文字。
//!
//! 三个使用者：
//! * **文件图标**——`assets/app.ico` 由 `cargo run --example mkicon` 生成，
//!   再由 `build.rs` 调 `rc.exe` 嵌进 exe（资源管理器、任务栏、Alt+Tab 用它）；
//! * **窗口图标**——[`app_icon_rgba`] 喂给 egui 的 `IconData`；
//! * **托盘图标**——[`create_hicon`] 把同一份像素包成 `HICON`。
//!
//! 之前托盘用的是 `IDI_APPLICATION`：系统那张「通用窗口」图在 16px 的托盘里
//! 既认不出是什么程序，也和一堆系统图标糊在一起。

use std::ffi::c_void;
use std::io;
use std::path::Path;

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, GetDC, ReleaseDC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICONINFO,
};

/// 托盘 / `HICON` 用的边长。
///
/// 托盘实际只显示 16/20/24 像素，画 32 是为了系统降采样后线条还不糊。
pub const ICON_SIZE: u32 = 32;

/// 设置窗口标题栏 / Alt+Tab 用的边长（这两处经常被放大显示）。
pub const WINDOW_ICON_SIZE: u32 = 64;

/// 写进 `assets/app.ico` 的全部尺寸。
///
/// 16/20/24/32/40/48 覆盖 100%~250% 各档任务栏与列表视图，128/256 给
/// 「大图标」视图和文件属性对话框。
pub const ICO_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

// ---------------------------------------------------------------------------
// 图形
// ---------------------------------------------------------------------------

/// 底板颜色（深海军蓝，深浅任务栏上都压得住）。
const PLATE: (u8, u8, u8) = (0x14, 0x1A, 0x24);
/// 底板外圈一道亮一档的描边：暗色任务栏上靠它看出「这是块图标」。
const RIM: (u8, u8, u8) = (0x4E, 0x6C, 0x8A);
/// 准星本体颜色。
const INK: (u8, u8, u8) = (0x35, 0xE8, 0x79);

/// 圆角矩形路径（tiny-skia 没有现成的 round-rect）。
///
/// 圆角用三次贝塞尔近似四分之一圆，`k = 0.5523` 是标准的误差最小系数；
/// 图标最大 256px，这点误差远小于一个像素。
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.01 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(
            x, y, w, h,
        )?));
    }
    const K: f32 = 0.552_284_8;
    let c = r * K;
    let (x1, y1) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - r + c, y, x1, y + r - c, x1, y + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + c, x1 - r + c, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + r - c, y1, x, y1 - r + c, x, y1 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    pb.close();
    pb.finish()
}

/// 在 `pixmap` 上画出图标图形（底板 + 十字 + 圆点）。
///
/// 几何全按边长比例给出，所以同一套参数在 16px 和 256px 上都成立。
fn paint(pixmap: &mut Pixmap) {
    let s = pixmap.width() as f32;
    let center = s * 0.5;

    // ---- 底板：先铺外圈色，再盖上内缩一点的深色，露出一圈亮边 ----
    let rim_width = (s * 0.045).max(0.7);
    let mut rim = Paint::default();
    rim.set_color(tiny_skia::Color::from_rgba8(RIM.0, RIM.1, RIM.2, 0xFF));
    rim.anti_alias = true;
    if let Some(path) = rounded_rect(0.0, 0.0, s, s, s * 0.20) {
        pixmap.fill_path(&path, &rim, FillRule::Winding, Transform::identity(), None);
    }

    let inset = rim_width;
    let mut plate = Paint::default();
    plate.set_color(tiny_skia::Color::from_rgba8(
        PLATE.0, PLATE.1, PLATE.2, 0xFF,
    ));
    plate.anti_alias = true;
    if let Some(path) = rounded_rect(
        inset,
        inset,
        s - inset * 2.0,
        s - inset * 2.0,
        (s * 0.20 - inset).max(0.0),
    ) {
        pixmap.fill_path(
            &path,
            &plate,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // ---- 准星：四条臂 + 中心点 ----
    let mut ink = Paint::default();
    ink.set_color(tiny_skia::Color::from_rgba8(INK.0, INK.1, INK.2, 0xFF));
    ink.anti_alias = true;

    let gap = s * 0.085; // 中心缺口：一眼看出是准星，而不是一个加号
    let reach = s * 0.315;
    let mut pb = PathBuilder::new();
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        pb.move_to(center + dx * gap, center + dy * gap);
        pb.line_to(center + dx * reach, center + dy * reach);
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: (s * 0.088).max(1.3),
            miter_limit: 4.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Round,
            dash: None,
        };
        pixmap.stroke_path(&path, &ink, &stroke, Transform::identity(), None);
    }

    if let Some(dot) = PathBuilder::from_circle(center, center, (s * 0.075).max(1.0)) {
        pixmap.fill_path(&dot, &ink, FillRule::Winding, Transform::identity(), None);
    }
}

/// 图标的像素数据（**非预乘** RGBA，`size × size`）。
///
/// egui 的 `IconData` 要的正是这种格式；`HICON` 见 [`create_hicon`]，
/// `.ico` 见 [`ico_bytes`]。
pub fn app_icon_rgba(size: u32) -> Vec<u8> {
    let size = size.clamp(8, 512);
    let mut pixmap = Pixmap::new(size, size).expect("图标画布分配失败");
    paint(&mut pixmap);

    let mut data = pixmap.data().to_vec();
    crate::crosshair::unpremultiply(&mut data);
    data
}

// ---------------------------------------------------------------------------
// HICON（托盘）
// ---------------------------------------------------------------------------

/// 一个 `HICON` 的持有者：离开作用域就销毁。
///
/// 托盘把自己拿到的句柄存起来供「资源管理器重启后重新挂图标」复用，
/// 所以它的生命期必须比 [`crate::platform::tray::Tray`] 长——由 `App` 持有。
pub struct Icon(HICON);

impl Icon {
    /// 裸句柄。传给 Win32 时用；不要自行销毁。
    pub fn handle(&self) -> HICON {
        self.0
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.0);
        }
    }
}

/// 把 [`app_icon_rgba`] 包成 `HICON`。
///
/// 做法是 `CreateIconIndirect` + 两张 GDI 位图：颜色位图是 32bpp 顶朝下的
/// DIB（图标要 BGRA），掩码位图全 0（真正的透明交给 alpha 通道）。
/// 任何一步失败都返回 `None`，调用方退回系统默认图标，程序不该因此起不来。
pub fn create_hicon() -> Option<Icon> {
    let rgba = app_icon_rgba(ICON_SIZE);
    let size = ICON_SIZE as i32;

    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            // 负数 = 自上而下，正好等于渲染器的行序
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(
            screen,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            return None;
        }

        // RGBA -> BGRA（alpha 直接照抄，DIB 的 4 个字节布局就是 BGRA）
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for (to, from) in dst.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            to[0] = from[2];
            to[1] = from[1];
            to[2] = from[0];
            to[3] = from[3];
        }

        // 单色掩码：每行按 16 位对齐（`CreateBitmap` 的老规矩）。
        let stride = (size as usize).div_ceil(16) * 2;
        let mask_bits = vec![0u8; stride * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const c_void);

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let hicon = CreateIconIndirect(&info);

        // CreateIconIndirect 会把位图拷进图标对象，之后就可以释放了。
        DeleteObject(color);
        if !mask.is_null() {
            DeleteObject(mask);
        }
        ReleaseDC(std::ptr::null_mut(), screen);

        if hicon.is_null() {
            None
        } else {
            Some(Icon(hicon))
        }
    }
}

// ---------------------------------------------------------------------------
// .ico（文件图标）
// ---------------------------------------------------------------------------

/// 单个尺寸的 ICO 图像负载。
///
/// 小尺寸用未压缩的 BMP（兼容性最好，少数老组件对 32bpp PNG 支持不全），
/// 大尺寸用 PNG（256 的 BMP 要 256KB，PNG 只有几 KB）。
fn ico_entry_image(size: u32) -> Vec<u8> {
    let rgba = app_icon_rgba(size);
    if size >= 64 {
        png_payload(&rgba, size)
    } else {
        bmp_payload(&rgba, size)
    }
}

/// 直通 alpha 的 PNG：`Pixmap` 在这里只当字节容器，`encode_png` 原样写下数据，
/// 所以塞进去的必须是**非预乘** RGBA（否则边缘会有一圈偏黑）。
fn png_payload(rgba: &[u8], size: u32) -> Vec<u8> {
    let size = tiny_skia::IntSize::from_wh(size, size).expect("尺寸非法");
    let pixmap = Pixmap::from_vec(rgba.to_vec(), size).expect("像素数据长度不匹配");
    pixmap.encode_png().expect("PNG 编码失败")
}

/// ICO 里的 BMP 是「BITMAPINFOHEADER + 自下而上的 BGRA + 1bpp 掩码」，
/// 高度写两倍（XOR 图与 AND 掩码两张图叠在一起）。
fn bmp_payload(rgba: &[u8], size: u32) -> Vec<u8> {
    let w = size as usize;
    let xor_len = w * w * 4;
    let mask_stride = w.div_ceil(32) * 4;
    let and_len = mask_stride * w;

    let mut out = Vec::with_capacity(40 + xor_len + and_len);
    let push_u32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    push_u32(&mut out, 40); // biSize
    push_u32(&mut out, size); // biWidth
    push_u32(&mut out, size * 2); // biHeight = 图 + 掩码
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    push_u32(&mut out, 0); // BI_RGB
    push_u32(&mut out, (xor_len + and_len) as u32); // biSizeImage
    push_u32(&mut out, 0); // biXPelsPerMeter
    push_u32(&mut out, 0); // biYPelsPerMeter
    push_u32(&mut out, 0); // biClrUsed
    push_u32(&mut out, 0); // biClrImportant

    // 自下而上逐行写 BGRA
    for y in (0..w).rev() {
        for x in 0..w {
            let i = (y * w + x) * 4;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    // 掩码全 0：透明完全交给 alpha 通道
    out.resize(out.len() + and_len, 0);
    out
}

/// 完整的多尺寸 `.ico` 字节流。
pub fn ico_bytes() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = ICO_SIZES
        .iter()
        .map(|&size| (size, ico_entry_image(size)))
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // 保留字段
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 = 图标
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // 目录项里 256 及以上的边长写 0（一个字节放不下）
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0); // 调色板颜色数（真彩为 0）
        out.push(0); // 保留
        out.extend_from_slice(&1u16.to_le_bytes()); // 平面数
        out.extend_from_slice(&32u16.to_le_bytes()); // 位深
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// 把 [`ico_bytes`] 写成文件（`cargo run --example mkicon` 用）。
pub fn write_ico(path: &Path) -> io::Result<usize> {
    let bytes = ico_bytes();
    std::fs::write(path, &bytes)?;
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个尺寸都得真的画出东西，而且四角必须被圆角切掉
    /// （否则文件图标就是个实心方块）。
    #[test]
    fn every_size_has_transparent_corners() {
        for size in ICO_SIZES {
            let data = app_icon_rgba(size);
            assert_eq!(data.len(), (size * size * 4) as usize, "尺寸 {size}");

            let clear = data.chunks_exact(4).filter(|px| px[3] < 32).count();
            assert!(clear > 0, "尺寸 {size} 没有透明像素：四角没被圆角切掉");

            let opaque = data.chunks_exact(4).filter(|px| px[3] > 200).count();
            let total = (size * size) as usize;
            assert!(opaque > total / 2, "尺寸 {size} 底板太淡：{opaque}/{total}");
        }
    }

    /// 准星必须是**亮**的：深色底板之上要有明显更亮的像素，
    /// 否则缩小后就是一团深色。
    #[test]
    fn crosshair_is_brighter_than_plate() {
        let data = app_icon_rgba(64);
        let brightest = data
            .chunks_exact(4)
            .filter(|px| px[3] > 200)
            .map(|px| px[0] as u32 + px[1] as u32 + px[2] as u32)
            .max()
            .unwrap_or(0);
        // 亮绿 #35E879 三通道和 = 53 + 232 + 121 = 406
        assert!(brightest > 330, "最亮像素太暗（{brightest}），准星画丢了");
    }

    /// `.ico` 结构必须合法：ICONDIR 头、目录项数、偏移量都得对得上，
    /// 否则资源管理器会当成坏图标显示成空白。
    #[test]
    fn ico_container_is_well_formed() {
        let bytes = ico_bytes();
        assert_eq!(&bytes[0..4], &[0, 0, 1, 0], "ICONDIR 头不对");

        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        assert_eq!(count, ICO_SIZES.len());

        let mut entries = Vec::new();
        for (i, &expected) in ICO_SIZES.iter().enumerate() {
            let base = 6 + i * 16;
            let dim = bytes[base] as u32;
            assert_eq!(
                if dim == 0 { 256 } else { dim },
                expected,
                "第 {i} 项边长不对"
            );
            let len = u32::from_le_bytes(bytes[base + 8..base + 12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(bytes[base + 12..base + 16].try_into().unwrap()) as usize;
            assert!(off + len <= bytes.len(), "第 {i} 项越界");
            entries.push((off, len));
        }

        // 负载首尾相接、不重叠、正好铺满
        let mut sorted = entries.clone();
        sorted.sort();
        let mut cursor = 6 + 16 * count;
        for (off, len) in sorted {
            assert_eq!(off, cursor, "负载之间有空洞或重叠");
            cursor += len;
        }
        assert_eq!(cursor, bytes.len());

        // 小尺寸应是未压缩 BMP（以 40 字节的 BITMAPINFOHEADER 开头）
        let (off, _) = entries[0];
        assert_eq!(
            u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            40,
            "16px 的负载不是 BMP"
        );
        // 大尺寸应是 PNG 签名
        let (off, _) = entries[ICO_SIZES.len() - 1];
        assert_eq!(&bytes[off..off + 8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }
}
