//! 显示器枚举与选择。

use windows_sys::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFOEXW,
    MonitorFromPoint,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use super::from_wide_buf;

/// `MONITORINFOF_PRIMARY` —— wingdi.h 里的宏，windows-sys 没有导出。
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// 一台显示器。
#[derive(Clone)]
pub struct MonitorInfo {
    /// 物理像素矩形（进程已开启 Per-Monitor V2 DPI 感知时 `GetMonitorInfoW` 返回真实像素）
    pub rect: RECT,
    pub primary: bool,
    /// 设备名，例如 `\\.\DISPLAY1`
    pub device: String,
    /// 有效 DPI（96 = 100%）
    pub dpi: u32,
}

impl MonitorInfo {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }

    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }

    /// 缩放系数（1.0 = 100%）。
    pub fn scale(&self) -> f32 {
        (self.dpi as f32 / 96.0).clamp(0.5, 4.0)
    }

    /// 便于日志与界面显示的一行描述。
    pub fn describe(&self) -> String {
        format!(
            "{}  {}x{}  {}%{}",
            self.device,
            self.width(),
            self.height(),
            (self.scale() * 100.0).round() as i32,
            if self.primary { "  (主显示器)" } else { "" }
        )
    }
}

impl std::fmt::Debug for MonitorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

unsafe extern "system" fn enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> windows_sys::core::BOOL {
    let out = unsafe { &mut *(data as *mut Vec<MonitorInfo>) };
    if let Some(info) = unsafe { query(hmonitor) } {
        out.push(info);
    }
    TRUE
}

unsafe fn query(hmonitor: HMONITOR) -> Option<MonitorInfo> {
    let mut mi: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(hmonitor, &mut mi.monitorInfo) } == 0 {
        return None;
    }

    let mut dpi = 96u32;
    let mut dpi_y = 96u32;
    // 失败（例如较老的系统）时保持 96，宁可略小也不报错。
    let _ = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) };

    Some(MonitorInfo {
        rect: mi.monitorInfo.rcMonitor,
        primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        device: from_wide_buf(&mi.szDevice),
        dpi,
    })
}

/// 枚举所有显示器（主显示器不保证排在第一位）。
pub fn enumerate() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(enum_proc),
            &mut list as *mut Vec<MonitorInfo> as LPARAM,
        );
    }
    list
}

/// 主显示器。
pub fn primary() -> Option<MonitorInfo> {
    let hmonitor = unsafe {
        MonitorFromPoint(
            windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        )
    };
    if hmonitor.is_null() {
        enumerate().into_iter().find(|m| m.primary)
    } else {
        unsafe { query(hmonitor) }
    }
}

/// 按配置里 0 基的下标取显示器；`None` 或越界时回退到主显示器。
///
/// 下标顺序按「主显示器优先，其余按设备名排序」，这样在多屏机器上
/// 下标不会因为系统枚举顺序变化而漂移。
pub fn pick(index: Option<usize>) -> Option<MonitorInfo> {
    let mut all = enumerate();
    if all.is_empty() {
        return None;
    }
    all.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then_with(|| a.device.cmp(&b.device))
    });
    match index {
        Some(i) if i < all.len() => Some(all[i].clone()),
        _ => Some(all[0].clone()),
    }
}

/// 按配置里的显示器选择取显示器。
pub fn pick_sel(sel: &crate::config::MonitorSel) -> Option<MonitorInfo> {
    pick(sel.index())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_finds_at_least_one() {
        let list = enumerate();
        assert!(!list.is_empty(), "至少要能枚举出一台显示器");
        for m in &list {
            assert!(m.width() > 0 && m.height() > 0);
            assert!((m.scale() - 1.0).abs() < 3.5);
        }
    }

    #[test]
    fn primary_is_primary() {
        let p = primary().expect("应有主显示器");
        assert!(p.primary || p.rect.left == 0 && p.rect.top == 0);
    }

    #[test]
    fn out_of_range_falls_back_to_primary() {
        let a = pick(Some(0)).expect("应有显示器");
        let b = pick(Some(9999)).expect("越界也应回退");
        assert_eq!(a.device, b.device);
    }
}
