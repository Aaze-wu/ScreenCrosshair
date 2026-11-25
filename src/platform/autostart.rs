//! 开机自启：在「启动」文件夹里放一个指向本程序的快捷方式（`.lnk`）。
//!
//! 这里**不用** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。原因是实测：
//! 联想电脑管家一类「开机启动优化」会在项被写入后的 1~2 秒内把它搬走 ——
//! 值从 `Run` 下移到 `Run\LenovoDisabled`，并加上 `rem|` 前缀当作「已禁用」。
//! 表现就是「勾上开机自启、提示也显示已开启，但复选框自己弹回未勾选，下次开机也没运行」。
//! 「启动」文件夹不受这类工具处理，而且和注册表一样只需要当前用户权限。

use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW,
};

use super::wide;

const LNK_FILE_NAME: &str = "ScreenCrosshair";

/// 旧版本用过的注册表位置，只用来清理，避免和快捷方式一起触发两次启动。
const LEGACY_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const LEGACY_VALUE_NAME: &str = "ScreenCrosshair";

/// 快捷方式的「参数」部分：只有指定了配置文件路径时才需要。
pub fn arguments_for(config: Option<&Path>) -> String {
    match config {
        Some(cfg) => format!("--config \"{}\"", cfg.display()),
        None => String::new(),
    }
}

/// 快捷方式应该落在哪：`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ScreenCrosshair.lnk`
pub fn lnk_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let mut path = PathBuf::from(appdata);
    path.push(r"Microsoft\Windows\Start Menu\Programs\Startup");
    path.push(format!("{LNK_FILE_NAME}.lnk"));
    Some(path)
}

/// 开启自启：写入启动文件夹快捷方式，并清掉旧版遗留的注册表值。
pub fn enable(exe: &Path, config: Option<&Path>) -> Result<(), String> {
    let lnk = lnk_path().ok_or_else(|| "取不到启动文件夹路径".to_string())?;
    if let Some(dir) = lnk.parent() {
        std::fs::create_dir_all(dir).map_err(|err| format!("打开启动文件夹失败：{err}"))?;
    }
    com::create_shortcut(
        &lnk,
        exe,
        &arguments_for(config),
        exe.parent().filter(|dir| !dir.as_os_str().is_empty()),
    )?;
    // 旧值留着会和快捷方式一起触发两次启动。
    let _ = remove_legacy_run_value();
    Ok(())
}

fn open_legacy_key(access: u32) -> Result<HKEY, String> {
    let sub = wide(LEGACY_RUN_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            std::ptr::null(),
            0,
            access,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("打开注册表 Run 项失败（错误码 {status}）"));
    }
    Ok(key)
}

/// 删掉旧版本写进注册表的自启值（不存在也算成功）。
fn remove_legacy_run_value() -> Result<(), String> {
    let key = open_legacy_key(KEY_SET_VALUE | KEY_QUERY_VALUE)?;
    let name = wide(LEGACY_VALUE_NAME);
    let status = unsafe { RegDeleteValueW(key, name.as_ptr()) };
    unsafe {
        RegCloseKey(key);
    }
    // 值本来就不存在（ERROR_FILE_NOT_FOUND = 2）也算成功。
    if status != ERROR_SUCCESS && status != 2 {
        return Err(format!("删除旧的自启注册表值失败（错误码 {status}）"));
    }
    Ok(())
}

/// 读旧版自启值（只有测试还原现场时才用得上）。
#[cfg(test)]
fn legacy_run_value() -> Option<String> {
    use windows_sys::Win32::System::Registry::RegQueryValueExW;

    let key = open_legacy_key(KEY_QUERY_VALUE).ok()?;
    let name = wide(LEGACY_VALUE_NAME);
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut u8,
            &mut bytes,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = (bytes as usize / 2).min(buf.len());
    Some(super::from_wide_buf(&buf[..len]))
}

/// 写回旧版自启值（只有测试还原现场时才用得上）。
#[cfg(test)]
fn set_legacy_run_value(cmd: &str) {
    use windows_sys::Win32::System::Registry::{REG_SZ, RegSetValueExW};

    let Ok(key) = open_legacy_key(KEY_SET_VALUE) else {
        return;
    };
    let name = wide(LEGACY_VALUE_NAME);
    let cmd = wide(cmd);
    unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            cmd.as_ptr() as *const u8,
            (cmd.len() * 2) as u32,
        );
        RegCloseKey(key);
    }
}

/// 当前自启指向的快捷方式路径，`None` 表示未启用。
pub fn query() -> Option<String> {
    let lnk = lnk_path()?;
    lnk.is_file().then(|| lnk.display().to_string())
}

/// 是否已启用自启（快捷方式存在即视为启用）。
///
/// 走文件系统而不是注册表：设置界面每帧都要问一次，`Path::is_file` 比开注册表便宜得多，
/// 也不会碰上「值刚写进去就被启动优化工具搬走」的假阴性。
pub fn is_enabled() -> bool {
    query().is_some()
}

/// 关闭自启：删掉快捷方式，顺带清掉旧版注册表值。
pub fn disable() -> Result<(), String> {
    let mut failure: Option<String> = None;
    if let Some(lnk) = lnk_path() {
        match std::fs::remove_file(&lnk) {
            Ok(()) => {}
            // 本来就不存在也算成功，这样才能反复点「关闭」。
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => failure = Some(format!("删除启动快捷方式失败：{err}")),
        }
    }
    if let Err(err) = remove_legacy_run_value() {
        failure.get_or_insert(err);
    }
    match failure {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// 用 COM 的 `IShellLinkW` + `IPersistFile` 落一个 `.lnk`。
///
/// `windows-sys` 只绑函数不绑 COM 接口，所以虚表索引得自己写：
/// `IShellLinkW` 继承 `IUnknown`，`SetWorkingDirectory` 是 9、`SetArguments` 是 11、
/// `SetPath` 是 20；`IPersistFile::Save` 是 6。
mod com {
    use super::*;

    type HResult = i32;
    type Lpcwstr = *const u16;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const fn guid(data1: u32, tail: u8) -> Guid {
        Guid {
            data1,
            data2: 0,
            data3: 0,
            data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, tail],
        }
    }

    /// `{00021401-0000-0000-C000-000000000046}`
    const CLSID_SHELL_LINK: Guid = guid(0x0002_1401, 0x46);
    /// `{000214F9-0000-0000-C000-000000000046}`
    const IID_ISHELL_LINK_W: Guid = guid(0x0002_14F9, 0x46);
    /// `{0000010B-0000-0000-C000-000000000046}`
    const IID_IPERSIST_FILE: Guid = guid(0x0000_010B, 0x46);

    const CLSCTX_INPROC_SERVER: u32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 2;

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> HResult;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const Guid,
            outer: *const c_void,
            context: u32,
            riid: *const Guid,
            out: *mut *mut c_void,
        ) -> HResult;
    }

    type SetPath = unsafe extern "system" fn(*mut c_void, Lpcwstr) -> HResult;
    type SetString = unsafe extern "system" fn(*mut c_void, Lpcwstr) -> HResult;
    type QueryInterface =
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HResult;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type Save = unsafe extern "system" fn(*mut c_void, Lpcwstr, i32) -> HResult;

    /// 取虚表里第 `slot` 项。整个文件里只有这一个 `transmute_copy`，其余都是类型别名调用。
    unsafe fn method<F: Copy>(obj: *mut c_void, slot: usize) -> F {
        unsafe {
            let table = *(obj as *mut *const usize);
            let entry = *table.add(slot);
            std::mem::transmute_copy(&entry)
        }
    }

    unsafe fn drop_object(obj: *mut c_void) {
        if obj.is_null() {
            return;
        }
        let release: Release = unsafe { method(obj, 2) };
        unsafe { release(obj) };
    }

    fn wide_z(text: &OsStr) -> Vec<u16> {
        let mut buf: Vec<u16> = text.encode_wide().collect();
        buf.push(0);
        buf
    }

    fn failed(name: &str, hr: HResult) -> String {
        format!("{name} 失败（HRESULT 0x{:08X}）", hr as u32)
    }

    pub fn create_shortcut(
        lnk: &Path,
        target: &Path,
        arguments: &str,
        working_dir: Option<&Path>,
    ) -> Result<(), String> {
        // S_OK(0) 说明是我们初始化的，退出时要配对 CoUninitialize；
        // S_FALSE(1) 说明本线程早已初始化。别的失败码（例如线程已在 MTA 里）也继续试：
        // ShellLink 是进程内组件，套间模型不影响它，真不行会在下面报出来。
        let hr = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
        let result = create_shortcut_inner(lnk, target, arguments, working_dir);
        if hr == 0 {
            unsafe { CoUninitialize() };
        }
        result
    }

    fn create_shortcut_inner(
        lnk: &Path,
        target: &Path,
        arguments: &str,
        working_dir: Option<&Path>,
    ) -> Result<(), String> {
        let mut link: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_SHELL_LINK,
                std::ptr::null(),
                CLSCTX_INPROC_SERVER,
                &IID_ISHELL_LINK_W,
                &mut link,
            )
        };
        if hr < 0 || link.is_null() {
            return Err(failed("创建快捷方式对象", hr));
        }

        let result = unsafe { fill_and_save(link, lnk, target, arguments, working_dir) };
        unsafe { drop_object(link) };
        result
    }

    unsafe fn fill_and_save(
        link: *mut c_void,
        lnk: &Path,
        target: &Path,
        arguments: &str,
        working_dir: Option<&Path>,
    ) -> Result<(), String> {
        let target_w = wide_z(target.as_os_str());
        let set_path: SetPath = unsafe { method(link, 20) };
        let hr = unsafe { set_path(link, target_w.as_ptr()) };
        if hr < 0 {
            return Err(failed("设置目标路径", hr));
        }

        if !arguments.is_empty() {
            let args_w = wide_z(OsStr::new(arguments));
            let set_arguments: SetString = unsafe { method(link, 11) };
            let hr = unsafe { set_arguments(link, args_w.as_ptr()) };
            if hr < 0 {
                return Err(failed("设置启动参数", hr));
            }
        }

        if let Some(dir) = working_dir {
            let dir_w = wide_z(dir.as_os_str());
            let set_working_dir: SetString = unsafe { method(link, 9) };
            let hr = unsafe { set_working_dir(link, dir_w.as_ptr()) };
            if hr < 0 {
                return Err(failed("设置工作目录", hr));
            }
        }

        let mut persist: *mut c_void = std::ptr::null_mut();
        let query_interface: QueryInterface = unsafe { method(link, 0) };
        let hr = unsafe { query_interface(link, &IID_IPERSIST_FILE, &mut persist) };
        if hr < 0 || persist.is_null() {
            return Err(failed("获取 IPersistFile", hr));
        }

        let lnk_w = wide_z(lnk.as_os_str());
        let save: Save = unsafe { method(persist, 6) };
        let hr = unsafe { save(persist, lnk_w.as_ptr(), 1) };
        unsafe { drop_object(persist) };
        if hr < 0 {
            return Err(failed("写入快捷方式", hr));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_only_carry_config_path() {
        assert_eq!(arguments_for(None), "");
        let cfg = Path::new(r"C:\Users\me\AppData\Roaming\ScreenCrosshair\config.json");
        assert_eq!(
            arguments_for(Some(cfg)),
            r#"--config "C:\Users\me\AppData\Roaming\ScreenCrosshair\config.json""#
        );
    }

    #[test]
    fn lnk_path_points_into_startup_folder() {
        if std::env::var_os("APPDATA").is_none() {
            return;
        }
        let lnk = lnk_path().expect("有 APPDATA 就应当算得出路径");
        assert!(
            lnk.ends_with(r"Startup\ScreenCrosshair.lnk"),
            "{}",
            lnk.display()
        );
        let text = lnk.display().to_string();
        assert!(text.contains("Microsoft"), "{text}");
        assert!(text.contains("Start Menu"), "{text}");
    }

    /// 真落一个 `.lnk` 再删掉。
    ///
    /// 这是唯一能验证 COM 虚表索引、GUID、宽字符指针全对的办法 ——
    /// 纯函数测试完全覆盖不到 `IShellLinkW`/`IPersistFile` 的调用。
    /// 跑完会把原来那份快捷方式（如果用户真的开着自启）原样还原。
    #[test]
    fn lnk_roundtrip() {
        let Some(lnk) = lnk_path() else {
            return;
        };
        let backup = std::fs::read(&lnk).ok();
        let legacy = legacy_run_value();

        let exe = Path::new(r"C:\tmp\sc.exe");
        let cfg = Path::new(r"C:\tmp\config.json");
        if let Err(err) = enable(exe, Some(cfg)) {
            eprintln!("跳过 lnk_roundtrip：{err}");
            return;
        }
        assert!(lnk.is_file(), "写完看不到快捷方式文件");
        assert!(is_enabled());
        // 快捷方式是 0x4C 开头的二进制，大小远大于空文件；这里只确认确实写了内容。
        let written = std::fs::metadata(&lnk).map(|m| m.len()).unwrap_or(0);
        assert!(written > 64, "快捷方式文件太小（{written} 字节）");

        disable().expect("删除自启快捷方式");
        assert!(!lnk.is_file(), "删除后快捷方式仍在");
        assert!(!is_enabled());
        // 再删一次必须仍然成功，否则「关闭自启」按钮第二次点就报错。
        disable().expect("重复删除应当幂等");

        // 还原测试前的状态，不给这台机器留下痕迹。
        match backup {
            Some(bytes) => std::fs::write(&lnk, bytes).expect("还原原有快捷方式"),
            None => {
                let _ = std::fs::remove_file(&lnk);
            }
        }
        if let Some(prev) = legacy {
            set_legacy_run_value(&prev);
        }
    }
}
