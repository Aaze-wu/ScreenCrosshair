//! 全屏透明覆盖层窗口。
//!
//! 用 `WS_EX_LAYERED` + `UpdateLayeredWindow(ULW_ALPHA)`，也就是给窗口提供一张
//! 32 位预乘 BGRA 位图，系统负责合成到桌面，因此能画出真正的逐像素半透明，
//! 且不需要 `WS_EX_NOREDIRECTIONBITMAP` 之类的取巧手段。
//!
//! `WS_EX_TRANSPARENT` 让鼠标点击穿透到下面的窗口，`WS_EX_NOACTIVATE` +
//! `WS_EX_TOOLWINDOW` 保证它不抢焦点、不出现在 Alt-Tab 列表里。

use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HGDIOBJ, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, RegisterClassW, SM_CXSCREEN,
    SM_CYSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
    WM_DESTROY, WM_NCCREATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::wide;

const CLASS_NAME: &str = "ScreenCrosshairOverlay";

/// `AC_SRC_OVER` / `AC_SRC_ALPHA`（wingdi.h 宏，windows-sys 未导出为常量）。
const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;

/// 供 WndProc 转发事件的回调。
pub type WndCallback = Box<dyn Fn(u32, usize, isize) -> Option<isize>>;

/// 与 `HWND` 关联、需要跨线程访问的数据。
struct OverlayContext {
    callback: WndCallback,
}

/// 一块可以上传像素的分层窗口。
pub struct Overlay {
    hwnd: HWND,
    mem_dc: windows_sys::Win32::Graphics::Gdi::HDC,
    old_bitmap: HGDIOBJ,
    dib: HBITMAP,
    /// DIB 的像素起始地址（32bpp 预乘 BGRA，自上而下）
    bits: *mut u8,
    width: u32,
    height: u32,
    screen_x: i32,
    screen_y: i32,
}

// 像素指针只在创建它的线程里用（消息循环所在线程），这里只是为了让 Overlay
// 能放进同一个结构体里使用。
unsafe impl Send for Overlay {}

impl Overlay {
    /// 创建覆盖层窗口。
    ///
    /// `x/y/width/height` 是**物理像素**的桌面坐标与尺寸（一般来自
    /// [`super::monitor::MonitorInfo::rect`]）。
    pub fn new(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        callback: WndCallback,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("覆盖层尺寸不能为 0".to_string());
        }

        unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());
            if hinstance.is_null() {
                return Err("GetModuleHandleW 失败".to_string());
            }

            let class_name = wide(CLASS_NAME);
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            // 重复注册（例如同一进程里重建窗口）会失败，忽略即可。
            RegisterClassW(&wc);

            let ex_style = WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;

            // 宽高先给 0，随后由 resize 统一设置。
            let hwnd = CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                wide("ScreenCrosshair").as_ptr(),
                WS_POPUP,
                x,
                y,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW 失败".to_string());
            }

            // 把上下文挂到窗口上，WndProc 里再取出来。
            let ctx = Box::new(OverlayContext { callback });
            set_context(hwnd, Box::into_raw(ctx) as isize);

            let screen_dc = windows_sys::Win32::Graphics::Gdi::GetDC(std::ptr::null_mut());
            if screen_dc.is_null() {
                DestroyWindow(hwnd);
                return Err("GetDC(screen) 失败".to_string());
            }
            let mem_dc = CreateCompatibleDC(screen_dc);
            windows_sys::Win32::Graphics::Gdi::ReleaseDC(std::ptr::null_mut(), screen_dc);
            if mem_dc.is_null() {
                DestroyWindow(hwnd);
                return Err("CreateCompatibleDC 失败".to_string());
            }

            let mut overlay = Overlay {
                hwnd,
                mem_dc,
                old_bitmap: std::ptr::null_mut(),
                dib: std::ptr::null_mut(),
                bits: std::ptr::null_mut(),
                width: 0,
                height: 0,
                screen_x: x,
                screen_y: y,
            };
            overlay.resize(x, y, width, height)?;
            Ok(overlay)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 重新分配 DIB 并移动窗口（换显示器、分辨率变化时调用）。
    pub fn resize(&mut self, x: i32, y: i32, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("覆盖层尺寸不能为 0".to_string());
        }
        unsafe {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // 负高度 = 自上而下的 DIB，和 tiny-skia 的行序一致
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(
                self.mem_dc,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                return Err("CreateDIBSection 失败".to_string());
            }

            let old = SelectObject(self.mem_dc, dib as HGDIOBJ);
            if self.dib.is_null() {
                self.old_bitmap = old;
            } else {
                // 释放上一张位图
                SelectObject(self.mem_dc, self.old_bitmap);
                DeleteObject(self.dib as HGDIOBJ);
                SelectObject(self.mem_dc, dib as HGDIOBJ);
                self.old_bitmap = old;
            }

            self.dib = dib;
            self.bits = bits as *mut u8;
            self.width = width;
            self.height = height;
            self.screen_x = x;
            self.screen_y = y;
        }
        Ok(())
    }

    /// 上传一张预乘 BGRA 位图（长度必须等于 `width * height * 4`）。
    pub fn upload(&mut self, bgra_premultiplied: &[u8]) -> Result<(), String> {
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if bgra_premultiplied.len() != expected {
            return Err(format!(
                "像素长度不匹配：期望 {expected}，实际 {}",
                bgra_premultiplied.len()
            ));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                bgra_premultiplied.as_ptr(),
                self.bits,
                bgra_premultiplied.len(),
            );

            let dst = POINT {
                x: self.screen_x,
                y: self.screen_y,
            };
            let size = SIZE {
                cx: self.width as i32,
                cy: self.height as i32,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA,
            };

            let screen_dc = windows_sys::Win32::Graphics::Gdi::GetDC(std::ptr::null_mut());
            let ok = UpdateLayeredWindow(
                self.hwnd,
                screen_dc,
                &dst,
                &size,
                self.mem_dc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
            windows_sys::Win32::Graphics::Gdi::ReleaseDC(std::ptr::null_mut(), screen_dc);
            if ok == 0 {
                return Err("UpdateLayeredWindow 失败".to_string());
            }
        }
        Ok(())
    }

    /// 清空为全透明（不显示准星时用，避免直接隐藏窗口引起闪烁）。
    pub fn clear(&mut self) -> Result<(), String> {
        let len = (self.width as usize) * (self.height as usize) * 4;
        let zeros = vec![0u8; len];
        self.upload(&zeros)
    }

    pub fn show(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// 保持置顶（有些游戏/全屏程序会改 Z 序）。
    pub fn raise(&self) {
        self.set_topmost(true);
    }

    /// 切换「始终置顶」。
    ///
    /// 设置界面也是本进程的普通窗口，而覆盖层是 `WS_EX_TOPMOST`：
    /// 不主动让位的话，准星会直接画在设置窗口上面（看上去像界面画错了）。
    /// 打开设置界面时传 `false`，关掉后再传 `true`。
    pub fn set_topmost(&self, on: bool) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        };
        let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            SetWindowPos(
                self.hwnd,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 桌面总尺寸（多屏拼起来的包围盒的对角），供日志使用。
    pub fn virtual_screen() -> (i32, i32) {
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                let ptr = get_context(self.hwnd);
                if ptr != 0 {
                    drop(Box::from_raw(ptr as *mut OverlayContext));
                    set_context(self.hwnd, 0);
                }
                DestroyWindow(self.hwnd);
            }
            if !self.dib.is_null() {
                if !self.old_bitmap.is_null() {
                    SelectObject(self.mem_dc, self.old_bitmap);
                }
                DeleteObject(self.dib as HGDIOBJ);
            }
            if !self.mem_dc.is_null() {
                DeleteDC(self.mem_dc);
            }
        }
    }
}

const GWLP_USERDATA: i32 = -21;

unsafe fn set_context(hwnd: HWND, value: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, value);
    }
}

unsafe fn get_context(hwnd: HWND) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    if msg == WM_NCCREATE {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    if msg == WM_DESTROY {
        return 0;
    }
    unsafe {
        let ptr = get_context(hwnd);
        if ptr != 0 {
            let ctx = &*(ptr as *const OverlayContext);
            if let Some(result) = (ctx.callback)(msg, wparam, lparam) {
                return result;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// 消息窗口的窗口类名。
///
/// 提成常量是因为单实例检测要靠 `FindWindowW` 找到已在运行的那个实例
/// （见 [`crate::platform::single`]），两边写成同一个字符串才不会悄悄失联。
pub const MSG_CLASS_NAME: &str = "ScreenCrosshairMsg";

/// 生成一个纯占位的窗口，用来接收热键 / 托盘回调（覆盖层窗口本身是
/// `WS_EX_TRANSPARENT` 的，虽然也能收消息，但单独一个隐藏窗口更清晰）。
pub struct MessageWindow {
    hwnd: HWND,
}

impl MessageWindow {
    pub fn new(callback: WndCallback) -> Result<Self, String> {
        unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());
            let class_name = wide(MSG_CLASS_NAME);
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            // 刻意不用 HWND_MESSAGE（消息专用窗口）：那不是真正的顶层窗口，
            // 托盘菜单弹出前的 `SetForegroundWindow` 会失败，菜单就不会在
            // 点击别处时自动关闭。这里建一个「永不显示」的顶层工具窗口
            // （WS_EX_TOOLWINDOW 让它不进任务栏和 Alt+Tab）。
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                wide(MSG_CLASS_NAME).as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("创建消息窗口失败".to_string());
            }
            let ctx = Box::new(OverlayContext { callback });
            set_context(hwnd, Box::into_raw(ctx) as isize);
            Ok(MessageWindow { hwnd })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        unsafe {
            let ptr = get_context(self.hwnd);
            if ptr != 0 {
                drop(Box::from_raw(ptr as *mut OverlayContext));
                set_context(self.hwnd, 0);
            }
            DestroyWindow(self.hwnd);
        }
    }
}
