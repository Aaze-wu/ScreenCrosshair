//! 生成 `assets/app.ico`（文件图标），提交后由 `build.rs` 嵌进 exe。
//!
//! 图标是**代码画出来的**，不是手工绘制的二进制：
//! 想改形状就改 `src/platform/icon.rs`，然后重跑
//!
//! ```text
//! cargo run --example mkicon
//! ```
//!
//! 别用 `--release`，debug 跑一次就够快；产物是确定性字节，重跑不会产生 diff
//! （除非渲染器真的变了）。

fn main() -> std::io::Result<()> {
    let path = std::path::Path::new("assets/app.ico");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let len = screencrosshair::platform::icon::write_ico(path)?;
    println!(
        "wrote {} ({len} bytes, {} 个尺寸)",
        path.display(),
        screencrosshair::platform::icon::ICO_SIZES.len()
    );
    Ok(())
}
