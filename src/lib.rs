//! 帧析助手 core —— 与 Python 版（12端口扫描.py）语义逐一对齐（对拍 339 项零差异）。
//!
//! 模块划分：
//! - [`extract`]    通道数值提取（任意 1~4 字节索引、大小端、符号位扩展）
//! - [`frame_sync`] 帧同步与样本流处理（帧头可选、大包模式、CSV 缓冲）
//! - [`config`]     .config.json 配置结构（与 Python 版 JSON 字段兼容）
//! - [`link`]       串口 / UDP 链路层（只读线程 + mpsc 回传）
//! - [`ui`]         egui 界面（方案 B 样式系统：ui_style.json + F12 微调面板）

pub mod config;
pub mod extract;
pub mod frame_sync;
pub mod link;
pub mod ui;
