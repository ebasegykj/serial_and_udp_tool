//! Windows 资源嵌入：把 app.ico 编进 exe 文件图标（资源管理器/任务栏显示）。
//! 运行时窗口图标在 main.rs 里用 eframe ViewportBuilder::with_icon 单独设置。

use std::path::PathBuf;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        // build 脚本的工作目录是包根 app_rust\frameanalyzer\，
        // 共用图标在更上一级的项目根 serial_udp\app.ico → 上溯两级。
        res.set_icon("../../app.ico");
        // winresource 找不到 rc.exe 时（SDK 环境变量未注册）会静默跳过资源编译，
        // 导致 exe 没有文件图标 —— 这里主动扫描 Windows SDK 选最新版 x64 rc.exe。
        match find_rc_dir() {
            Some(bin) => {
                res.set_toolkit_path(bin.to_str().unwrap_or_default());
            }
            None => eprintln!("警告: 未找到 Windows SDK rc.exe，exe 文件图标将缺失"),
        }
        if let Err(e) = res.compile() {
            // 嵌入失败不阻断构建（仅影响 exe 文件图标），但要提示
            eprintln!("警告: app.ico 资源嵌入失败: {}", e);
        }
    }
}

/// 在 Windows SDK 安装目录里找含 x64\rc.exe 的最新版本 bin 目录。
fn find_rc_dir() -> Option<PathBuf> {
    const ROOT: &str = r"C:\Program Files (x86)\Windows Kits\10\bin";
    let mut best: Option<(String, PathBuf)> = None;
    let entries = std::fs::read_dir(ROOT).ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // 版本目录形如 10.0.26100.0
        if !name.starts_with("10.") {
            continue;
        }
        let dir = e.path().join("x64");
        if dir.join("rc.exe").is_file()
            && best
                .as_ref()
                .map(|(v, _)| name.as_str() > v.as_str())
                .unwrap_or(true)
        {
            best = Some((name, dir));
        }
    }
    best.map(|(_, p)| p)
}
