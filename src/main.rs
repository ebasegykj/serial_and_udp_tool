//! 帧析助手 UI 入口（eframe / egui）。

// Release 构建隐藏 Windows 控制台窗口；debug 构建保留终端便于看 eprintln 调试输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use frameanalyzer::ui::{FrameApp, style};

/// 从内嵌的 app.ico 解码窗口图标（任务栏 / 标题栏）。
/// 解码失败时返回 None，窗口退回系统默认图标（不阻断启动）。
fn load_app_icon() -> Option<eframe::egui::IconData> {
    // 共用图标放在项目根 serial_udp\app.ico（本文件在 app_rust\frameanalyzer\src\ 下，上溯三级）。
    // Python 版打包脚本也引用同一份，改图标只需改根目录那一个文件。
    let bytes = include_bytes!("../../../app.ico");
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    Some(eframe::egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn main() -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Binary Frame Analyzer by gykjwrq@163.com")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([900.0, 600.0]);
    // 窗口/任务栏图标（app.ico 解码失败时保持系统默认，不阻断启动）
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Binary Frame Analyzer by gykjwrq@163.com",
        options,
        Box::new(|cc| {
            style::install_fonts(&cc.egui_ctx);
            Ok(Box::new(FrameApp::new()))
        }),
    )
}
