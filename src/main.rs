//! 可执行入口。
//!
//! 目前支持这些不会创建窗口的子命令，方便在没有图形环境时自检：
//! * `--validate`        校验配置文件（默认配置路径）
//! * `--print`           打印“实际生效”的准星描述（JSON）
//! * `--list-presets`    列出所有预设
//! * `--preview <目录>`  把每个预设渲染成 PNG（仅调试用）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use screencrosshair::config::{self, Config};
use screencrosshair::crosshair;
use screencrosshair::presets;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut config_path: Option<PathBuf> = None;
    let mut action = Action::Run;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                match args.get(i) {
                    Some(p) => config_path = Some(PathBuf::from(p)),
                    None => {
                        eprintln!("错误：--config 后面需要跟一个路径");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--validate" => action = Action::Validate,
            "--print" => action = Action::Print,
            "--list-presets" => action = Action::ListPresets,
            "--preview" => {
                i += 1;
                match args.get(i) {
                    Some(p) => action = Action::Preview(PathBuf::from(p)),
                    None => {
                        eprintln!("错误：--preview 后面需要跟一个目录");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("screencrosshair {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数：{other}");
                print_help();
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let path = config::resolve_path(config_path.clone());

    match action {
        Action::Validate => validate(&path),
        Action::Print => print_effective(&path),
        Action::ListPresets => {
            list_presets();
            ExitCode::SUCCESS
        }
        Action::Preview(dir) => preview_all(&dir),
        Action::Run => run_gui(config_path),
    }
}

enum Action {
    Run,
    Validate,
    Print,
    ListPresets,
    Preview(PathBuf),
}

/// 启动覆盖层与托盘（Windows 专用）。
fn run_gui(config_path: Option<PathBuf>) -> ExitCode {
    #[cfg(windows)]
    {
        match screencrosshair::app::App::run(config_path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("启动失败：{err}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = config_path;
        eprintln!("屏幕覆盖层只支持 Windows。");
        ExitCode::FAILURE
    }
}

fn print_help() {
    println!(
        "ScreenCrosshair —— 屏幕中心准星\n\
         \n\
         用法: screencrosshair [选项]\n\
         \n\
         不带任何参数时启动准星覆盖层（常驻托盘）。\n\
         \n\
         选项:\n\
           -c, --config <路径>   使用指定的配置文件\n\
               --validate        校验配置文件后退出\n\
               --print           打印当前生效的准星 JSON\n\
               --list-presets    列出内置预设\n\
               --preview <目录>  把每个预设渲染成 PNG 写到该目录\n\
           -h, --help            显示帮助\n\
           -V, --version         显示版本\n\
         \n\
         全局热键（可在设置界面里改键，也可整体关闭）:\n\
           Ctrl+Alt+←/→      切换预设\n\
           Ctrl+Alt+H        显示/隐藏\n\
           Ctrl+Alt+↑/↓      不透明度 +/-\n\
           Ctrl+Alt+Shift+↑/↓  放大/缩小\n\
           Ctrl+Alt+O        设置界面\n\
           Ctrl+Alt+R        重新载入配置\n\
           Ctrl+Alt+Q        退出\n\
         \n\
         位置微调（单位像素）:\n\
           Ctrl+Shift+A/D/W/S        左/右/上/下 各移 1 像素\n\
           Ctrl+Alt+Shift+A/D/W/S    左/右/上/下 各移 10 像素\n\
           Ctrl+Alt+0                回到屏幕中心\n\
         \n\
         注：Ctrl+Alt+字母 的组合常被其它程序占用，注册失败的项会显示在\n\
         设置界面顶栏；改用 Ctrl+Shift+字母 通常能拿到。",
    );
}

fn load(path: &std::path::Path) -> Result<Config, String> {
    match config::load(path)? {
        Some(cfg) => Ok(cfg),
        None => Ok(Config::default()),
    }
}

fn validate(path: &std::path::Path) -> ExitCode {
    if let Err(e) = config::ensure_exists(path) {
        eprintln!("无法创建默认配置：{e}");
        return ExitCode::FAILURE;
    }
    match load(path) {
        Ok(mut cfg) => {
            cfg.normalize();
            println!("配置文件：{}", path.display());
            println!("版本：{}", cfg.version);
            println!("当前预设：{}", cfg.active);
            println!("可见：{}", cfg.visible);
            println!("显示器：{}", cfg.monitor.describe());
            println!(
                "内置预设 {} 个，可用预设 {} 个，自定义预设 {} 个",
                presets::BUILTINS.len(),
                cfg.preset_names().len(),
                cfg.custom_presets.len()
            );
            let spec = cfg.effective();
            println!(
                "生效颜色：{}  透明度：{:.2}  缩放：{:.2}  图元：{} 个",
                spec.color,
                spec.opacity,
                spec.scale,
                spec.shapes.len()
            );
            println!("校验通过。");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("配置有误：{e}");
            ExitCode::FAILURE
        }
    }
}

fn print_effective(path: &std::path::Path) -> ExitCode {
    match load(path) {
        Ok(mut cfg) => {
            cfg.normalize();
            match serde_json::to_string_pretty(&cfg.effective()) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("序列化失败：{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("配置有误：{e}");
            ExitCode::FAILURE
        }
    }
}

fn list_presets() {
    for (name, label) in presets::BUILTINS {
        println!("{name:<14} {label}");
    }
}

fn preview_all(dir: &std::path::Path) -> ExitCode {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("无法创建目录 {}：{e}", dir.display());
        return ExitCode::FAILURE;
    }
    let size = 256u32;
    let cols = 4u32;
    let count = presets::BUILTINS.len() as u32;
    let rows = count.div_ceil(cols);

    // 先把每张预览渲染出来
    let mut tiles: Vec<Vec<u8>> = Vec::new();
    for (name, _) in presets::BUILTINS.iter() {
        let Some(spec) = presets::builtin(name) else {
            eprintln!("预设 {name} 解析失败");
            return ExitCode::FAILURE;
        };
        let mut pixmap = match tiny_skia::Pixmap::new(size, size) {
            Some(p) => p,
            None => return ExitCode::FAILURE,
        };
        crosshair::draw(
            &mut pixmap,
            &spec,
            (size as f32 / 2.0, size as f32 / 2.0),
            1.0,
        );
        let out = dir.join(format!("{name}.png"));
        if let Err(e) = pixmap.save_png(&out) {
            eprintln!("写 {} 失败：{e}", out.display());
            return ExitCode::FAILURE;
        }
        tiles.push(pixmap.data().to_vec());
    }

    // 再拼成一张总览图（深灰底，方便肉眼查看）
    let sheet_w = size * cols;
    let sheet_h = size * rows;
    let mut sheet = match tiny_skia::Pixmap::new(sheet_w, sheet_h) {
        Some(p) => p,
        None => return ExitCode::FAILURE,
    };
    sheet.fill(tiny_skia::Color::from_rgba8(32, 32, 36, 255));
    let dst = sheet.data_mut();
    let dst_stride = (sheet_w * 4) as usize;
    for (idx, tile) in tiles.iter().enumerate() {
        let cx = (idx as u32 % cols) * size;
        let cy = (idx as u32 / cols) * size;
        for y in 0..size {
            let d0 = (cy + y) as usize * dst_stride + (cx * 4) as usize;
            let s0 = (y * size * 4) as usize;
            for x in 0..size as usize {
                let d = d0 + x * 4;
                let s = s0 + x * 4;
                let a = tile[s + 3] as u32;
                if a == 0 {
                    continue;
                }
                let inv = 255 - a;
                for c in 0..3 {
                    dst[d + c] = ((tile[s + c] as u32 * a + dst[d + c] as u32 * inv) / 255) as u8;
                }
            }
        }
    }
    let sheet_path = dir.join("all.png");
    if let Err(e) = sheet.save_png(&sheet_path) {
        eprintln!("写 {} 失败：{e}", sheet_path.display());
        return ExitCode::FAILURE;
    }
    println!("已输出 {count} 张预设图与拼图 {}", sheet_path.display());
    ExitCode::SUCCESS
}
