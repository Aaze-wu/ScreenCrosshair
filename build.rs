//! 把 `assets/app.ico` 嵌进 exe，让资源管理器 / 任务栏 / Alt+Tab 显示准星图标。
//!
//! 为什么需要这一步：`cargo` 默认不给 exe 加任何资源，Windows 于是只能显示
//! 一张空白占位图。代码里设的窗口图标（`ViewportBuilder::with_icon`）只影响
//! **窗口**，管不到**文件本身**；文件图标只能来自 PE 的图标资源。
//!
//! 做法是调 Windows SDK 里的 `rc.exe` 把 `.rc` 编成 `.res`，再用
//! `cargo:rustc-link-arg-bins` 把它交给链接器（link.exe 直接接受 `.res`）。
//! 这样不需要额外的构建依赖 crate。
//!
//! 找不到 `rc.exe` 时只警告不失败：图标缺了顶多丑，不该让程序编译不过。

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    // 只处理 MSVC 目标：GNU 工具链要用 windres，那套不在这条路径上。
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }

    let rc = match find_rc() {
        Some(rc) => rc,
        None => {
            println!(
                "cargo:warning=找不到 rc.exe（Windows SDK），exe 将没有文件图标；\
                 窗口图标和托盘图标不受影响。装上「Windows SDK」或设置 RC 环境变量即可。"
            );
            return;
        }
    };

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out_dir.join("app.res");

    // 在 assets/ 里执行，这样 .rc 里的相对路径（app.ico）能直接解析。
    let status = Command::new(&rc)
        .current_dir(manifest_dir.join("assets"))
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("app.rc")
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
        Ok(s) => println!("cargo:warning=rc.exe 失败（{s}），exe 将没有文件图标"),
        Err(e) => println!(
            "cargo:warning=无法运行 {}：{e}，exe 将没有文件图标",
            rc.display()
        ),
    }
}

/// 找 `rc.exe`：先看 `RC` 环境变量，再扫 Windows SDK，最后试 PATH。
///
/// SDK 里通常有多份（每装一个版本留一份），取版本号最高的那个——和
/// `link.exe` 的选择习惯一致，避免用到只认老语法的那份。
fn find_rc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RC") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    let kits = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let bin = kits.join("Windows Kits").join("10").join("bin");

    let mut versions: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&bin) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let parts: Option<Vec<u32>> = name.split('.').map(|s| s.parse::<u32>().ok()).collect();
            if let Some(parts) = parts {
                versions.push((parts, entry.path()));
            }
        }
    }
    versions.sort_by(|a, b| a.0.cmp(&b.0));

    // 我们的目标永远是 x64；arm64 只作陪衬（跨架构编译时 SDK 只装了这个）。
    for arch in ["x64", "arm64", "x86"] {
        for (_, dir) in versions.iter().rev() {
            let candidate = dir.join(arch).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 最后碰运气：PATH 里有就直接用（个别环境会把它加进去）。
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return Some(PathBuf::from("rc.exe"));
    }
    None
}
