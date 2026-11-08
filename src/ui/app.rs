//! 界面主状态机 —— 布局 1:1 复刻 Python 15 版（15布局修改.py）。
//!
//! 左栏（可折叠组对齐）：
//! 1. 连接：[🌙][EN] 接口:下拉 → 串口(端口/⟳/波特率) 或 UDP(本机IP/本机端口/远端IP/远端端口) → 连接按钮
//! 2. 数据记录：☑提取数据保存 [CSV文件...] (路径) / ☑原始数据保存 [TXT文件...] (路径)
//! 3. 帧配置/离线绘图：帧头/帧长、字节序/样本帧、最大点数 → 应用帧配置 → 载入文件/分析剪贴板
//! 4. 通道配置：通道数 [n] [应用] → 色标图例行 → 通道行 [✓]N Idx:[..] ±:[..] Dif☑
//!
//! 右侧：数据绘图组（绘图/原始 切换 + 🗑/帮助）→ 底部 Data 发送行 → 状态栏（左状态 + 右 TX:n）

use crate::config::{AppConfig, parse_header_hex, parse_indices_str};
use crate::frame_sync::{ChannelCfg, ChecksumMode, Engine};
use crate::link::{
    LinkHandle, LinkWorker, list_serial_ports, open_serial, open_udp, send_bytes,
};
use crate::ui::i18n::{Lang, tr};
use crate::ui::style::StyleConfig;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 波特率预置档：常用档（≤1000000）+ 1000000 + 5000000（用户要求精简）
const BAUD_PRESETS: &[&str] = &[
    "9600", "19200", "38400", "57600", "115200", "230400", "460800", "921600", "1000000",
    "5000000",
];

/// 卡片内边距（四边一致）。原先走 F12，现已固定，避免旧 ui_style.json 把值锁在旧数值。
const CARD_PAD: f32 = 6.0;
/// 中央三张卡片（绘图 / Data 发送行 / 状态行）的统一间隙。
const CARD_GAP_V: f32 = 8.0;

// ============ 配置持久化（.config.json，字段对齐 Python 版） ============

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PersistFrame {
    #[serde(default)]
    header: String,
    #[serde(default)]
    length: String,
    #[serde(default)]
    order_big: bool,
    #[serde(default)]
    checksum: String,
    #[serde(default)]
    samples: String,
    #[serde(default)]
    max_points: String,
    /// 循环码位置（界面 1 起算，输入框原样字符串；空串 = 关闭）。
    #[serde(default)]
    cycle_pos: String,
    /// 循环码位宽（"8"/"16"/"24"/"32"）。
    #[serde(default)]
    cycle_width: String,
    /// 位置口径标记："1" = 界面 1 起算；旧配置缺此字段 = 按 0 起算存的值，读取时自动 +1。
    #[serde(default)]
    cycle_pos_base: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PersistChannel {
    #[serde(default)]
    idx: String,
    #[serde(default)]
    sig: String,
    #[serde(default)]
    dif: bool,
    #[serde(default)]
    visible: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PersistConfig {
    #[serde(default)]
    language: String,
    #[serde(default)]
    theme: usize,
    #[serde(default)]
    frame: PersistFrame,
    #[serde(default)]
    channels: Vec<PersistChannel>,
    #[serde(default)]
    baud: String,
    #[serde(default)]
    parity: String,
    #[serde(default)]
    udp_local_ip: String,
    #[serde(default)]
    udp_local_port: String,
    #[serde(default)]
    udp_remote_ip: String,
    #[serde(default)]
    udp_remote_port: String,
    #[serde(default)]
    send_text: String,
    #[serde(default)]
    cmd_list: Vec<String>,
    #[serde(default)]
    cmd_interval_ms: u32,
}

/// 单通道编辑状态（输入框原样保存，应用时才解析 —— 与 Python 版一致）。
#[derive(Debug, Clone)]
pub struct ChannelUi {
    pub idx_text: String,
    pub sig_text: String,
    pub dif: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Serial,
    Udp,
}

/// 硬件奇偶校验位（UART Parity，连接参数，修改后需重新连接生效）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParityOpt {
    #[default]
    None,
    Even,
    Odd,
}

impl ParityOpt {
    /// 下拉显示文本（协议术语，中英一致）。
    pub fn label(self) -> &'static str {
        match self {
            ParityOpt::None => "None",
            ParityOpt::Even => "Even",
            ParityOpt::Odd => "Odd",
        }
    }

    pub fn to_serialport(self) -> serialport::Parity {
        match self {
            ParityOpt::None => serialport::Parity::None,
            ParityOpt::Even => serialport::Parity::Even,
            ParityOpt::Odd => serialport::Parity::Odd,
        }
    }

    /// 持久化字符串（.config.json）。
    pub fn as_str(self) -> &'static str {
        match self {
            ParityOpt::None => "none",
            ParityOpt::Even => "even",
            ParityOpt::Odd => "odd",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "even" => ParityOpt::Even,
            "odd" => ParityOpt::Odd,
            _ => ParityOpt::None,
        }
    }
}

/// 状态栏「帧配置 / 通道配置」两格的回执：只记录「发生了什么 + 参数」，
/// 显示时才按当前语言翻译成文字。这样中英切换会立即跟随，
/// 不会出现「切到英文还留着一句存好的中文回执」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigReceipt {
    None,
    FrameApplied,
    ChannelApplied(usize),
    Error(String),
}

pub struct FrameApp {
    pub lang: Lang,
    pub style: StyleConfig,
    pub style_dirty: bool,
    pub show_style_panel: bool,
    /// 上一帧左栏实际渲染宽度（布局回归测试用）
    pub last_panel_width: f32,
    pub show_help: bool,
    pub show_history: bool,
    pub view_plot: bool,

    // —— 连接 ——
    pub link_kind: LinkKind,
    pub serial_ports: Vec<String>,
    pub serial_port_sel: usize,
    pub baud_text: String,
    /// 硬件奇偶校验位（None/Even/Odd，仅 Serial 显示）
    pub parity_opt: ParityOpt,
    pub udp_local_ip: String,
    pub udp_local_port: String,
    pub udp_remote_ip: String,
    pub udp_remote_port: String,
    pub connected: bool,
    pub tx_count: u64,
    pub handle: Option<LinkHandle>,
    pub worker: Option<LinkWorker>,

    // —— 数据记录（Python 版勾选 + 文件按钮语义） ——
    pub extracted_enabled: bool,
    pub csv_path: Option<PathBuf>,
    pub csv_saved_rows: usize,
    pub raw_enabled: bool,
    pub txt_path: Option<PathBuf>,
    pub raw_pending: Vec<u8>,
    /// 连接建立时刻（用于连接时长显示）；未连接为 None。
    pub connected_at: Option<Instant>,

    // —— 帧配置（字符串原样，应用时解析） ——
    pub header_text: String,
    pub length_text: String,
    pub order_big: bool,
    /// 帧校验模式（无校验 / SUM8 / SUM16，校验字节在帧尾，应用帧配置生效）
    pub checksum_mode: ChecksumMode,
    pub samples_text: String,
    pub max_points_text: String,
    /// 循环码在帧内的字节位置（界面 1 起算，输入框原样字符串；空 = 关闭不记）。
    pub cycle_pos_text: String,
    /// 循环码位宽（8 / 16 / 24 / 32），决定读几个字节与回绕上限。
    pub cycle_width_bits: u8,

    // —— 通道 ——
    pub channel_count_text: String,
    pub channels: Vec<ChannelUi>,

    // —— 数据引擎 ——
    pub engine: Engine,
    pub max_points: usize,

    // —— 发送 ——
    pub send_text: String,
    pub send_hex: bool,
    pub send_history: Vec<String>,

    // —— 命令列表（原历史发送改版：手动填命令 + 间隔 + 循环/点选发送） ——
    /// 用户手动填写的命令（HEX/ASCII 跟随主发送行的格式开关）
    pub cmd_list: Vec<String>,
    /// 循环发送间隔（毫秒）
    pub cmd_interval_ms: u32,
    /// 自动循环发送开关
    pub cmd_loop: bool,
    /// 循环发送的当前序号（轮转）
    pub cmd_loop_idx: usize,
    /// 下次循环发送的时刻
    pub cmd_next_send: Option<Instant>,

    // —— 状态栏（按功能区块分格，格内互斥、格间独立） ——
    /// 「最近操作」回执：离线分析 / 剪贴板 / 清零 / 主题 / 发送 / 命令等一次性提示。
    pub status_msg: String,
    /// 连接格：非空时取代常驻「已连接/未连接」（串口错误、掉线原因）；状态变化时清空。
    pub link_msg: String,
    /// 帧配置格：应用成功或配置错误，一直留到下次应用（存状态，显示时翻译）。
    pub frame_msg: ConfigReceipt,
    /// 通道配置格：应用成功或配置错误，一直留到下次应用（存状态，显示时翻译）。
    pub channel_msg: ConfigReceipt,
    /// 连接失败弹窗内容（Some = 显示弹窗，点确定关闭）
    pub err_dialog: Option<String>,
    /// 绘图复位计数（+1 = 恢复自适应视图，等价 matplotlib Home）
    pub plot_epoch: u64,
    /// 一次性缩放因子（<1 放大，>1 缩小）
    pub plot_zoom: f32,
    /// 绘图暂停：true 时数据照常解析，绘图区冻结在快照上
    pub plot_paused: bool,
    /// 暂停瞬间的通道值快照（原始值 / 一阶差分，类型与引擎一致 i64）
    pub plot_snapshot_vals: Vec<Vec<i64>>,
    pub plot_snapshot_dif: Vec<Vec<i64>>,
    /// 暂停瞬间的总帧数（真实帧号基准，冻结坐标轴）
    pub plot_snapshot_base: u64,
    /// 后台文件对话框结果接收端（避免阻塞 UI 线程导致"无响应"）
    pub pick: Option<(PickKind, std::sync::mpsc::Receiver<Option<PathBuf>>)>,
    /// 上次同步到窗口标题栏的颜色（变化才调 DWM，避免每帧系统调用）
    pub last_titlebar: Option<egui::Color32>,
}

/// 文件对话框种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Csv,
    Txt,
}

impl FrameApp {
    pub fn new() -> Self {
        let style = StyleConfig::load();
        // 预填 .config.json（若有），结构与 Python 版兼容
        let cfg: Option<AppConfig> = std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        let (header, length, samples, max_points, saved_channels, count_text, cfg_lang, cycle_pos, cycle_width) =
            match &cfg {
                Some(c) => (
                    c.frame.header.clone(),
                    c.frame.length.clone(),
                    c.frame.samples.clone(),
                    c.frame.max_points.clone(),
                    Some(c.channels.clone()),
                    c.frame.channel_count.clone(),
                    c.language.clone(),
                    c.frame.cycle_pos.clone(),
                    c.frame.cycle_width.clone(),
                ),
                None => (
                    String::new(),
                    "11".into(),
                    "1".into(),
                    "500".into(),
                    None,
                    "2".into(),
                    "zh".into(),
                    String::new(),
                    "8".into(),
                ),
            };
        let count: usize = count_text.trim().parse().unwrap_or(2).clamp(1, 8);
        let channels: Vec<ChannelUi> = match saved_channels {
            Some(list) => list
                .iter()
                .take(count)
                .map(|c| ChannelUi {
                    idx_text: c
                        .indices()
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    sig_text: c.sign_bit.to_string(),
                    dif: c.diff,
                    visible: c.visible,
                })
                .collect(),
            None => (0..count)
                .map(|i| {
                    let s = 2 + i * 2;
                    ChannelUi {
                        idx_text: format!("{},{}", s, s + 1),
                        sig_text: "-1".into(),
                        dif: false,
                        visible: true,
                    }
                })
                .collect(),
        };
        let length = if length.is_empty() { "11".into() } else { length };
        let samples = if samples.is_empty() { "1".into() } else { samples };
        let max_points = if max_points.is_empty() { "500".into() } else { max_points };
        let cycle_width_bits = parse_cycle_width(&cycle_width);
        let mut app = FrameApp {
            lang: if cfg_lang == "en" { Lang::En } else { Lang::Zh },
            style,
            style_dirty: false,
            show_style_panel: false,
            last_panel_width: 0.0,
            show_help: false,
            show_history: false,
            view_plot: true,
            link_kind: LinkKind::Serial,
            serial_ports: list_serial_ports(),
            serial_port_sel: 0,
            baud_text: "1000000".into(),
            parity_opt: ParityOpt::None,
            udp_local_ip: "0.0.0.0".into(),
            udp_local_port: "8080".into(),
            udp_remote_ip: String::new(),
            udp_remote_port: String::new(),
            connected: false,
            tx_count: 0,
            handle: None,
            worker: None,
            extracted_enabled: false,
            csv_path: None,
            csv_saved_rows: 0,
            raw_enabled: false,
            txt_path: None,
            raw_pending: Vec::new(),
            connected_at: None,
            header_text: header,
            length_text: length,
            order_big: true,
            checksum_mode: ChecksumMode::None,
            samples_text: samples,
            max_points_text: max_points,
            cycle_pos_text: cycle_pos,
            cycle_width_bits,
            channel_count_text: count.to_string(),
            channels,
            engine: Engine::with_checksum(Vec::new(), 11, 1, true, Vec::new(), false, ChecksumMode::None),
            max_points: 500,
            send_text: "EB 90 01 02 03 04 05 06 07 08 00".into(),
            send_hex: true,
            send_history: Vec::new(),
            cmd_list: Vec::new(),
            cmd_interval_ms: 1000,
            cmd_loop: false,
            cmd_loop_idx: 0,
            cmd_next_send: None,
            status_msg: String::new(),
            link_msg: String::new(),
            frame_msg: ConfigReceipt::None,
            channel_msg: ConfigReceipt::None,
            err_dialog: None,
            plot_epoch: 0,
            plot_zoom: 1.0,
            plot_paused: false,
            plot_snapshot_vals: Vec::new(),
            plot_snapshot_dif: Vec::new(),
            plot_snapshot_base: 0,
            pick: None,
            last_titlebar: None,
        };
        // 上次运行的 .config.json（语言/主题/帧配置/通道配置/连接参数/命令列表）
        app.load_config();
        // 启动时即按配置装配引擎（连接后直接收数）。
        // 走「静默」那条路：开机不该冒充用户点过「应用」而显示「帧配置已应用」。
        // 配置本身有错时仍会显示明确错误提示，这个保留。
        app.apply_frame_config_silent();
        app.save_config(); // 保留原有的启动时写回（旧配置迁移结果落盘），只是不留回执
        app
    }

    // ============ 数据流 ============

    /// 轮询链路事件 → 喂引擎 → 自动落盘 → 修剪显示缓冲。
    fn poll_link(&mut self) {
        let mut incoming: Vec<Vec<u8>> = Vec::new();
        let mut lost: Option<String> = None;
        if let Some(w) = &self.worker {
            for ev in w.events.try_iter() {
                match ev {
                    crate::link::LinkEvent::Data(d) => incoming.push(d),
                    crate::link::LinkEvent::Lost(r) => lost = Some(r),
                }
            }
        }
        if let Some(reason) = lost {
            self.connected = false;
            self.connected_at = None;
            self.handle = None;
            self.worker = None; // Drop → 停接收线程
            self.link_msg = reason;
        }
        if !incoming.is_empty() {
            let mut new_bytes = 0usize;
            for d in incoming {
                new_bytes += d.len();
                self.engine.rx_buffer.extend_from_slice(&d);
                self.raw_pending.extend_from_slice(&d);
            }
            // 原始数据待写缓冲上限 20MB，丢最旧
            let excess = self.raw_pending.len().saturating_sub(20_000_000);
            if excess > 0 {
                self.raw_pending.drain(..excess);
            }
            self.engine.process_buffer();
            let _ = new_bytes;
            self.autosave_tick();
            self.trim_buffers();
        }
    }

    /// 数据记录自动落盘：提取数据 → CSV 追加；原始数据 → TXT 追加（hex 空格分隔）。
    fn autosave_tick(&mut self) {
        // 提取数据 CSV
        if self.extracted_enabled {
            if let Some(path) = self.csv_path.clone() {
                let rows = &self.engine.extracted_save_buffer;
                if self.csv_saved_rows < rows.len() {
                    let need_header = !path.exists();
                    let mut out = String::new();
                    if need_header {
                        let n = self.engine.channel_count;
                        out.push_str("Frame,");
                        out.push_str(
                            &(0..n).map(|i| format!("CH{}", i + 1)).collect::<Vec<_>>().join(","),
                        );
                        out.push('\n');
                    }
                    for (cnt, vals) in &rows[self.csv_saved_rows..] {
                        out.push_str(&cnt.to_string());
                        out.push(',');
                        out.push_str(
                            &vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                        );
                        out.push('\n');
                    }
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        use std::io::Write;
                        let _ = f.write_all(out.as_bytes());
                    }
                    self.csv_saved_rows = rows.len();
                }
            }
        }
        // 原始数据 TXT
        if self.raw_enabled {
            if let Some(path) = self.txt_path.clone() {
                if !self.raw_pending.is_empty() {
                    let hex: Vec<String> =
                        self.raw_pending.iter().map(|b| format!("{:02X}", b)).collect();
                    let mut out = hex.join(" ");
                    out.push(' ');
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        use std::io::Write;
                        let _ = f.write_all(out.as_bytes());
                    }
                    self.raw_pending.clear();
                }
            }
        }
    }

    /// 显示缓冲修剪：值队列最多 max_points（Python deque maxlen 展示语义）。
    fn trim_buffers(&mut self) {
        for v in &mut self.engine.channel_values {
            let excess = v.len().saturating_sub(self.max_points);
            if excess > 0 {
                v.drain(..excess);
            }
        }
        for v in &mut self.engine.channel_diff_values {
            let excess = v.len().saturating_sub(self.max_points);
            if excess > 0 {
                v.drain(..excess);
            }
        }
        let excess = self.engine.frame_log.len().saturating_sub(1000);
        if excess > 0 {
            self.engine.frame_log.drain(..excess);
        }
        let excess = self
            .engine
            .extracted_save_buffer
            .len()
            .saturating_sub(100_000);
        if excess > 0 {
            self.engine.extracted_save_buffer.drain(..excess);
            self.csv_saved_rows = self.csv_saved_rows.min(self.engine.extracted_save_buffer.len());
        }
    }

    // ============ 配置应用 ============

    /// 应用帧配置：解析输入框 → 重建引擎（清空历史数据，同 Python 应用语义）。
    /// 只有引擎真的重建成功才显示「帧配置已应用」，否则保留 build_engine 给出的
    /// 明确错误提示（循环码位置越界 / 解析失败不能静默）。
    pub fn apply_frame_config(&mut self) {
        let ok = self.apply_frame_config_silent();
        if ok {
            self.frame_msg = ConfigReceipt::FrameApplied;
        }
        self.save_config(); // 帧配置立即持久化
    }

    /// 返回 true = 引擎已重建；false = 配置错误（错误信息已写入 frame_msg）。
    fn apply_frame_config_silent(&mut self) -> bool {
        match self.build_engine() {
            Ok(e) => {
                self.engine = e;
                self.csv_saved_rows = 0;
                true
            }
            Err(s) => {
                // 只存错误明细，语言前缀在显示时按当前语言补上
                self.frame_msg = ConfigReceipt::Error(s);
                false
            }
        }
    }

    fn build_engine(&mut self) -> Result<Engine, String> {
        let header = parse_header_hex(&self.header_text)?;
        let length: usize = self
            .length_text
            .trim()
            .parse()
            .map_err(|_| format!("帧长 {:?}", self.length_text))?;
        if length == 0 {
            return Err("帧长不能为 0".into());
        }
        let samples: usize = self
            .samples_text
            .trim()
            .parse()
            .map_err(|_| format!("样本/帧 {:?}", self.samples_text))?;
        if samples == 0 {
            return Err("样本/帧不能为 0".into());
        }
        let mp: usize = self
            .max_points_text
            .trim()
            .parse()
            .map_err(|_| format!("最大点数 {:?}", self.max_points_text))?;
        self.max_points = mp;

        // 循环码：位置留空 = 关闭（完全不判定、不计数）；非空为「第几个字节」，从 1 起算
        // （与帧长口径一致），应用时转成引擎内部 0-based。填 0 明确报错，不静默。
        let cycle_pos: Option<usize> = if self.cycle_pos_text.trim().is_empty() {
            None
        } else {
            let p1: usize = self
                .cycle_pos_text
                .trim()
                .parse()
                .map_err(|_| format!(
                    "循环码位置 {:?} 无法解析（须填 1 起算的字节序号，留空=关闭）",
                    self.cycle_pos_text
                ))?;
            if p1 == 0 {
                return Err("循环码位置从 1 起算：第 1 个字节请填 1，不能填 0；留空=关闭".into());
            }
            Some(p1 - 1)
        };
        // 位宽只允许 8/16/24/32（UI 下拉固定），防御性兜底非法值回退 8。
        let cycle_width_bits: u8 = match self.cycle_width_bits {
            8 | 16 | 24 | 32 => self.cycle_width_bits,
            _ => 8,
        };
        let cycle_bytes: usize = (cycle_width_bits / 8) as usize;
        if let Some(p) = cycle_pos {
            // saturating_add 防止超大位置导致加法溢出（debug 下 panic）
            if p.saturating_add(cycle_bytes) > length {
                return Err(format!(
                    "循环码位置 {} + {} 字节（位宽 {}）超出帧长 {}：请改小位置/位宽，或加大帧长",
                    p + 1, cycle_bytes, cycle_width_bits, length
                ));
            }
        }

        let channels: Vec<ChannelCfg> = self
            .channels
            .iter()
            .map(|c| {
                let indices: Vec<usize> = parse_indices_str(&c.idx_text)
                    .into_iter()
                    .filter(|&x| x >= 0)
                    .map(|x| x as usize)
                    .collect();
                let sign_bit: i32 = c.sig_text.trim().parse().unwrap_or(-1);
                ChannelCfg { indices, sign_bit, active: true }
            })
            .collect();
        Ok(Engine::with_cycle(
            header,
            length,
            samples,
            self.order_big,
            channels,
            true,
            self.checksum_mode,
            cycle_pos,
            cycle_width_bits,
        ))
    }

    /// 生成绘图 series。
    /// - 暂停时数据源为快照（plot_snapshot_*，含冻结的帧号基准），否则为引擎实时值。
    /// - 每个可见通道先推原始线 `Some("CHn")`（进图例）；
    ///   勾选 Dif 时再叠加差分虚线 `None`（不进图例，Python `_nolegend_` 等价）。
    /// - 元组第 4 项 = x 基准（真实帧序号）：缓冲只存最近 max_points 个点，
    ///   x = 总帧数 − 缓冲长度 + i + 1，使坐标轴/鼠标读数都是真实帧号（最后一点 = 总帧数）。
    pub fn build_plot_series(&self) -> Vec<(Option<String>, Vec<f64>, usize, f64)> {
        let (vals, dif, total) = if self.plot_paused {
            (
                &self.plot_snapshot_vals,
                &self.plot_snapshot_dif,
                self.plot_snapshot_base as f64,
            )
        } else {
            (
                &self.engine.channel_values,
                &self.engine.channel_diff_values,
                self.engine.frame_count as f64,
            )
        };
        let mut out = Vec::new();
        for (i, c) in self.channels.iter().enumerate() {
            if !c.visible || i >= vals.len() {
                continue;
            }
            let raw: Vec<f64> = vals[i].iter().map(|&v| v as f64).collect();
            let x0_raw = total - raw.len() as f64 + 1.0;
            out.push((Some(format!("CH{}", i + 1)), raw, i, x0_raw));
            if c.dif && i < dif.len() {
                let d: Vec<f64> = dif[i].iter().map(|&v| v as f64).collect();
                let x0_dif = total - d.len() as f64 + 1.0;
                out.push((None, d, i, x0_dif));
            }
        }
        out
    }

    /// 应用通道配置（Python `apply_channel_config` 语义）：
    /// 调整通道数量、保留旧行配置、新行按 2+i*2 预填；
    /// 值队列清空，但 rx_buffer / 计数器保留（不断流）。
    fn apply_channel_config(&mut self) {
        let count: usize = self
            .channel_count_text
            .trim()
            .parse::<usize>()
            .unwrap_or(1)
            .clamp(1, 8);
        self.channel_count_text = count.to_string();
        let mut new_chans: Vec<ChannelUi> = Vec::with_capacity(count);
        for i in 0..count {
            if let Some(old) = self.channels.get(i) {
                if !parse_indices_str(&old.idx_text).is_empty() {
                    new_chans.push(old.clone());
                    continue;
                }
            }
            let s = 2 + i * 2;
            new_chans.push(ChannelUi {
                idx_text: format!("{},{}", s, s + 1),
                sig_text: "-1".into(),
                dif: false,
                visible: true,
            });
        }
        self.channels = new_chans;
        // 引擎：替换通道快照 + 清空值队列，保留 rx_buffer / 计数器
        let cfgs: Vec<ChannelCfg> = self
            .channels
            .iter()
            .map(|c| ChannelCfg {
                indices: parse_indices_str(&c.idx_text)
                    .into_iter()
                    .filter(|&x| x >= 0)
                    .map(|x| x as usize)
                    .collect(),
                sign_bit: c.sig_text.trim().parse().unwrap_or(-1),
                active: true,
            })
            .collect();
        let n = cfgs.len();
        self.engine.channels = cfgs;
        self.engine.channel_count = n;
        self.engine.channel_values = vec![Vec::new(); n];
        self.engine.channel_diff_values = vec![Vec::new(); n];
        self.csv_saved_rows = 0;
        self.channel_msg = ConfigReceipt::ChannelApplied(n);
        self.save_config(); // 通道配置立即持久化
    }

    // ============ 离线分析 ============

    /// 载入文件 / 分析剪贴板 共用：解析 hex → 按当前帧配置喂引擎绘图。
    fn analyze_hex_string(&mut self, hex_str: &str) {
        // 配置错误（如循环码位置越界）时保留 frame_msg 里的明确提示，不静默用旧引擎继续
        if !self.apply_frame_config_silent() {
            return;
        }
        match parse_header_hex(hex_str) {
            Ok(data) => {
                if data.is_empty() {
                    self.status_msg = "离线分析: 数据为空".into();
                    return;
                }
                self.engine.rx_buffer.extend_from_slice(&data);
                self.engine.process_buffer();
                self.trim_buffers();
                self.view_plot = true;
                self.status_msg = format!(
                    "离线分析完成: 帧 {} 样本 {}",
                    self.engine.frame_count, self.engine.sample_count
                );
            }
            Err(e) => {
                self.status_msg = format!("解析错误: {}", e);
            }
        }
    }

    fn load_file_offline(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Text/Hex", &["txt", "hex", "csv", "log"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => self.analyze_hex_string(&content),
                Err(e) => self.status_msg = format!("读取失败: {}", e),
            }
        }
    }

    fn analyze_clipboard(&mut self) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => {
                if text.trim().is_empty() {
                    self.status_msg = "剪贴板为空".into();
                } else {
                    self.analyze_hex_string(&text);
                }
            }
            Err(_) => self.status_msg = "剪贴板不可用或为空".into(),
        }
    }

    // ============ 面板绘制 ============

    /// 左栏组 1：连接（Python port_frame 结构 1:1）。
    fn connection_body(&mut self, ui: &mut egui::Ui) {
        // 顶行：接口: 下拉 靠左；EN / 🌙 靠右，永远同一行不换行。
        // 做法：先实测右侧按钮总宽，下拉宽度 = 剩余宽度，任何语言都放得下。
        let t_if = format!("{}:", tr(self.lang, "interface"));
        let sp = ui.spacing().item_spacing.x;
        let bpad = ui.spacing().button_padding.x * 2.0;
        let label_w = labels_width(ui, &[&t_if]);
        let btns_w = 30.0 // 🌙（表情按钮固定宽）
            + bpad
            + labels_width(ui, &[tr(self.lang, "lang_btn")])
            + bpad
            + sp * 2.0;
        let combo_w = (ui.available_width() - label_w - btns_w - sp * 2.0).max(50.0);
        ui.horizontal(|ui| {
            field_label(ui, &t_if, label_w, self.style.text_dim());
            egui::ComboBox::from_id_salt("interface_combo")
                .selected_text(match self.link_kind {
                    LinkKind::Serial => tr(self.lang, "serial"),
                    LinkKind::Udp => tr(self.lang, "udp"),
                })
                .width(combo_w)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.link_kind, LinkKind::Serial, tr(self.lang, "serial"));
                    ui.selectable_value(&mut self.link_kind, LinkKind::Udp, tr(self.lang, "udp"));
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.conn_extra_buttons(ui);
            });
        });

        let dim = self.style.text_dim();
        match self.link_kind {
            LinkKind::Serial => {
                let t_port = format!("{}:", tr(self.lang, "port"));
                let t_baud = format!("{}:", tr(self.lang, "baud"));
                let t_parity = format!("{}:", tr(self.lang, "parity"));
                let lw = labels_width(ui, &[&t_port, &t_baud, &t_parity]);
                ui.horizontal(|ui| {
                    field_label(ui, &t_port, lw, dim);
                    // 端口下拉在前、刷新在后；下拉里列出刷新出来的端口号。
                    // 刷新按钮宽度必须实测（英文 "Refresh" 比中文"刷新"宽），
                    // 否则估算值偏小 → 行内容超出面板 → egui 会把左栏撑宽（切英文变宽的根因）。
                    let sp = ui.spacing().item_spacing.x;
                    let btn_pad = ui.spacing().button_padding.x * 2.0;
                    let refresh_w = labels_width(ui, &[tr(self.lang, "refresh")]) + btn_pad;
                    let combo_w = (ui.available_width() - refresh_w - sp).max(60.0);
                    let sel_text = if self.serial_ports.is_empty() {
                        String::new()
                    } else {
                        // 框内只显示短名 COMxx（全名把界面撑爆）；下拉列表里才是全名
                        self.serial_ports
                            .get(self.serial_port_sel.min(self.serial_ports.len() - 1))
                            .map(|s| port_short_name(s).to_string())
                            .unwrap_or_default()
                    };
                    egui::ComboBox::from_id_salt("serial_port")
                        .selected_text(sel_text)
                        .width(combo_w)
                        .show_ui(ui, |ui| {
                            if self.serial_ports.is_empty() {
                                ui.weak(tr(self.lang, "no_ports"));
                            } else {
                                for (i, name) in self.serial_ports.iter().enumerate() {
                                    ui.selectable_value(&mut self.serial_port_sel, i, name);
                                }
                            }
                        });
                    if ui.button(tr(self.lang, "refresh")).clicked() {
                        self.serial_ports = list_serial_ports();
                        self.serial_port_sel = 0;
                    }
                });
                ui.horizontal(|ui| {
                    field_label(ui, &t_baud, lw, dim);
                    // 可手动输入任意波特率。外观与其余下拉一致：
                    // Frame 描边(2px) + 内容区固定 18 高 = 总高 20（= 标准 ComboBox 高度）；
                    // 内部 TextEdit 无边框（16.1 高）垂直居中，下拉三角画在框内右端。
                    let total_w = (ui.available_width() - 10.0).max(80.0);
                    egui::Frame::none()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().window_stroke())
                        .rounding(3.0)
                        .inner_margin(egui::Margin::symmetric(4.0, 0.0))
                        .show(ui, |ui| {
                            ui.set_min_width(total_w - 14.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2((total_w - 14.0).min(ui.available_width()), 18.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);
                                    let id = ui.make_persistent_id("baud_arrow");
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(14.0, 16.0),
                                        egui::Sense::click(),
                                    );
                                    if resp.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            3.0,
                                            ui.visuals().widgets.hovered.weak_bg_fill,
                                        );
                                    }
                                    let c = rect.center();
                                    ui.painter().add(egui::Shape::convex_polygon(
                                        vec![
                                            c + egui::vec2(-4.0, -2.0),
                                            c + egui::vec2(4.0, -2.0),
                                            c + egui::vec2(0.0, 3.0),
                                        ],
                                        ui.visuals().text_color(),
                                        egui::Stroke::NONE,
                                    ));
                                    if resp.clicked() {
                                        ui.memory_mut(|m| m.toggle_popup(id));
                                    }
                                    egui::popup::popup_below_widget(
                                        ui,
                                        id,
                                        &resp,
                                        egui::PopupCloseBehavior::CloseOnClick,
                                        |ui| {
                                            ui.set_min_width(96.0);
                                            for it in BAUD_PRESETS {
                                                if ui.selectable_label(self.baud_text == *it, *it).clicked() {
                                                    self.baud_text = (*it).to_string();
                                                }
                                            }
                                        },
                                    );
                                    let tw = ui.available_width();
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.baud_text)
                                            .desired_width(tw)
                                            .frame(false),
                                    );
                                },
                            );
                        });
                });
                // 校验位下拉：None/Even/Odd（标准 ComboBox，与波特率框等宽等高），修改后需重新连接生效
                ui.horizontal(|ui| {
                    field_label(ui, &t_parity, lw, dim);
                    // ComboBox 实际渲染宽比 width 参数多出箭头区 ≈4-6px，宽度预算里扣除以对齐波特率框
                    let w = (ui.available_width() - 10.0 - 6.0).max(80.0);
                    let resp = egui::ComboBox::from_id_salt("parity_combo")
                        .selected_text(self.parity_opt.label())
                        .width(w)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.parity_opt, ParityOpt::None, ParityOpt::None.label());
                            ui.selectable_value(&mut self.parity_opt, ParityOpt::Even, ParityOpt::Even.label());
                            ui.selectable_value(&mut self.parity_opt, ParityOpt::Odd, ParityOpt::Odd.label());
                        });
                    resp.response.on_hover_text(tr(self.lang, "parity_tip"));
                });
            }
            LinkKind::Udp => {
                let t_li = format!("{}:", tr(self.lang, "local_ip"));
                let t_lp = format!("{}:", tr(self.lang, "local_port"));
                let t_ri = format!("{}:", tr(self.lang, "remote_ip"));
                let t_rp = format!("{}:", tr(self.lang, "remote_port"));
                let lw = labels_width(ui, &[&t_li, &t_lp, &t_ri, &t_rp]);
                // 四行逐行排：标签定宽右对齐 → 冒号成一条线，输入框等长且不越界
                let row = |ui: &mut egui::Ui, text: &str, buf: &mut String| {
                    ui.horizontal(|ui| {
                        field_label(ui, text, lw, dim);
                        let w = ui.available_width().max(80.0);
                        ui.add(egui::TextEdit::singleline(buf).desired_width(w));
                    });
                };
                row(ui, &t_li, &mut self.udp_local_ip);
                row(ui, &t_lp, &mut self.udp_local_port);
                row(ui, &t_ri, &mut self.udp_remote_ip);
                row(ui, &t_rp, &mut self.udp_remote_port);
            }
        }
        // 连接 / 断开按钮（通栏）
        let (label, bg) = if self.connected {
            (tr(self.lang, "disconnect"), egui::Color32::from_rgb(0x8A, 0x3A, 0x3A))
        } else {
            (tr(self.lang, "connect"), self.style.accent_color())
        };
        if ui
            .add_sized(
                [ui.available_width(), 28.0],
                egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
                    .fill(bg),
            )
            .clicked()
        {
            if self.connected {
                self.worker = None;
                self.handle = None;
                self.connected = false;
                self.connected_at = None;
                self.link_msg.clear(); // 断开就是「未连接」，不再单独显示「已手动断开」
            } else {
                self.do_connect();
            }
        }
    }

    fn do_connect(&mut self) {
        match self.link_kind {
            LinkKind::Serial => {
                if self.serial_ports.is_empty() {
                    self.link_msg = tr(self.lang, "no_ports").to_string();
                    return;
                }
                let sel = self.serial_port_sel.min(self.serial_ports.len() - 1);
                let display = self.serial_ports[sel].clone();
                let port_name = display.split_whitespace().next().unwrap_or("").to_string();
                let baud: u32 = match self.baud_text.trim().parse() {
                    Ok(b) => b,
                    Err(_) => {
                        self.link_msg =
                            format!("{}: 波特率 {:?}", tr(self.lang, "config_error"), self.baud_text);
                        return;
                    }
                };
                match open_serial(&port_name, baud, self.parity_opt.to_serialport()) {
                    Ok(h) => {
                        let inner = match &h {
                            LinkHandle::Serial(a) => a.clone(),
                            LinkHandle::Udp(_) => unreachable!(),
                        };
                        self.worker = Some(LinkWorker::spawn_serial(inner));
                        self.handle = Some(h);
                        self.connected = true;
                        self.connected_at = Some(Instant::now());
                        self.link_msg.clear();
                    }
                    Err(e) => {
                        self.link_msg = e.clone();
                        self.err_dialog = Some(format!("{} {}\n{}", port_name, baud, e));
                    }
                }
            }
            LinkKind::Udp => {
                let bind_ip = if self.udp_local_ip.trim().is_empty() {
                    "0.0.0.0".to_string()
                } else {
                    self.udp_local_ip.trim().to_string()
                };
                let port: u16 = match self.udp_local_port.trim().parse() {
                    Ok(p) => p,
                    Err(_) => {
                        self.link_msg = format!(
                            "{}: 本机端口 {:?}",
                            tr(self.lang, "config_error"),
                            self.udp_local_port
                        );
                        return;
                    }
                };
                match open_udp(&bind_ip, port) {
                    Ok(h) => {
                        let inner = match &h {
                            LinkHandle::Udp(s) => s.clone(),
                            LinkHandle::Serial(_) => unreachable!(),
                        };
                        self.worker = Some(LinkWorker::spawn_udp(inner));
                        self.handle = Some(h);
                        self.connected = true;
                        self.connected_at = Some(Instant::now());
                        self.link_msg.clear();
                    }
                    Err(e) => {
                        self.link_msg = e.clone();
                        self.err_dialog = Some(format!("{}:{}\n{}", bind_ip, port, e));
                    }
                }
            }
        }
    }

    /// 连接组右上角的小按钮（🌙 主题 / EN 语言）。
    /// 在后台线程弹文件对话框（UI 线程完全不阻塞，修复"数据记录软件无响应"）。
    fn spawn_pick(&mut self, kind: PickKind) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let dialog = match kind {
                PickKind::Csv => rfd::FileDialog::new()
                    .set_file_name("extracted.csv")
                    .add_filter("CSV", &["csv"]),
                PickKind::Txt => rfd::FileDialog::new()
                    .set_file_name("raw.txt")
                    .add_filter("Text", &["txt"]),
            };
            let _ = tx.send(dialog.save_file());
        });
        self.pick = Some((kind, rx));
    }

    /// 每帧收割后台对话框结果：选了文件启用记录，取消则复位勾选。
    fn poll_pick(&mut self) {
        if self.pick.is_none() {
            return;
        }
        let result = match &self.pick {
            Some((_, rx)) => rx.try_recv().ok(),
            None => None,
        };
        if let Some(res) = result {
            let kind = self.pick.as_ref().unwrap().0;
            self.pick = None;
            match (kind, res) {
                (PickKind::Csv, Some(p)) => {
                    self.csv_path = Some(p);
                    self.csv_saved_rows = 0;
                    self.extracted_enabled = true;
                }
                (PickKind::Csv, None) => self.extracted_enabled = false,
                (PickKind::Txt, Some(p)) => {
                    self.txt_path = Some(p);
                    self.raw_enabled = true;
                }
                (PickKind::Txt, None) => self.raw_enabled = false,
            }
        }
    }

    fn conn_extra_buttons(&mut self, ui: &mut egui::Ui) {
        if ui
            .small_button("🌙")
            .on_hover_text("切换主题（循环：深色→原始→淡绿→淡蓝→米黄→粉色→Mac→Win）")
            .clicked()
        {
            self.style.cycle_theme();
            self.status_msg = format!("主题: {}", self.style.theme().name);
            let _ = self.style.save(); // 立即记住，下次启动沿用
            self.save_config(); // 主题也写进 .config.json
        }
        if ui.small_button(tr(self.lang, "lang_btn")).clicked() {
            self.lang.toggle();
            self.save_config(); // 语言切换立即持久化
        }
    }

    /// 左栏组 2：数据记录（勾选启用 + 文件按钮，Python 语义）。
    /// 对话框在后台线程弹出（修复同步调用导致的软件无响应），结果每帧收割。
    fn data_record_body(&mut self, ui: &mut egui::Ui) {
        // 提取数据保存（CSV）
        ui.horizontal_wrapped(|ui| {
            let csv_rec = self.csv_recording();
            let accent = self.style.accent_color();
            let dim = self.style.text_dim();
            let changed = Self::record_checkbox(
                ui,
                &mut self.extracted_enabled,
                tr(self.lang, "save_extracted"),
                csv_rec,
                self.lang,
                accent,
                dim,
            );
            if ui.button(tr(self.lang, "csv_file")).clicked() {
                self.spawn_pick(PickKind::Csv);
            } else if changed && self.extracted_enabled && self.csv_path.is_none() {
                // 勾选但没选文件 → 先弹对话框；取消则复位勾选
                self.extracted_enabled = false;
                self.spawn_pick(PickKind::Csv);
            }
            // 后面：始终显示路径/文件名（旧功能，悬停看全路径）
            self.csv_path_label(ui);
        });
        // 原始数据保存（TXT）
        ui.horizontal_wrapped(|ui| {
            let txt_rec = self.txt_recording();
            let accent = self.style.accent_color();
            let dim = self.style.text_dim();
            let changed = Self::record_checkbox(
                ui,
                &mut self.raw_enabled,
                tr(self.lang, "save_rawchk"),
                txt_rec,
                self.lang,
                accent,
                dim,
            );
            if ui.button(tr(self.lang, "txt_file")).clicked() {
                self.spawn_pick(PickKind::Txt);
            } else if changed && self.raw_enabled && self.txt_path.is_none() {
                self.raw_enabled = false;
                self.spawn_pick(PickKind::Txt);
            }
            self.txt_path_label(ui);
        });
    }

    /// 数据记录勾选框：标签随状态变化（即用户要的"状态替换标签"，不再另起状态字）。
    /// - 未勾选           → 原标签（提取数据保存 / 原始数据保存），默认色
    /// - 已勾选未在写      → 「待连接」，灰
    /// - 已勾选正在写      → 「数据保存中」，主色（蓝，亮=正在写）
    fn record_checkbox(
        ui: &mut egui::Ui,
        enabled: &mut bool,
        base_label: &'static str,
        recording: bool,
        lang: Lang,
        accent: egui::Color32,
        dim: egui::Color32,
    ) -> bool {
        let label = Self::record_box_label(enabled, base_label, recording, lang);
        let mut rt = egui::RichText::new(label);
        if recording {
            rt = rt.color(accent);
        } else if *enabled {
            rt = rt.color(dim);
        }
        ui.add(egui::Checkbox::new(enabled, rt)).changed()
    }

    /// 纯函数：勾选框显示文字（状态替换标签）。
    fn record_box_label(
        enabled: &bool,
        base_label: &'static str,
        recording: bool,
        lang: Lang,
    ) -> &'static str {
        if !*enabled {
            base_label
        } else if recording {
            tr(lang, "rec_saving")
        } else {
            tr(lang, "rec_wait_conn")
        }
    }

    /// 数据记录组：勾选框后、按钮后的路径/文件名（始终显示，旧功能）。
    /// 文件名 + 悬停全路径；未选文件显示「(no file)」。
    /// 颜色默认（状态变化体现在前面的勾选框/状态文字，不在这里）。
    fn record_follow(
        &self,
        path: &Option<std::path::PathBuf>,
    ) -> (String, Option<egui::Color32>, String) {
        let full = path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let text = path
            .as_ref()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .unwrap_or_else(|| tr(self.lang, "no_file").to_string());
        (text, None, full)
    }

    /// 是否正在写 CSV：已连接 && 勾选提取保存 && 已选好文件。
    fn csv_recording(&self) -> bool {
        self.connected && self.extracted_enabled && self.csv_path.is_some()
    }
    /// 是否正在写 TXT：已连接 && 勾选原始保存 && 已选好文件。
    fn txt_recording(&self) -> bool {
        self.connected && self.raw_enabled && self.txt_path.is_some()
    }
    /// 记录指示灯：任一保存正在写即亮（与数据记录组状态文字共用判定，避免一个亮一个不亮）。
    fn recording_any(&self) -> bool {
        self.csv_recording() || self.txt_recording()
    }

    fn csv_path_label(&mut self, ui: &mut egui::Ui) {
        let (text, color, hover) = self.record_follow(&self.csv_path);
        let mut rt = egui::RichText::new(text).small();
        if let Some(c) = color {
            rt = rt.color(c);
        }
        let lbl = ui.label(rt);
        if !hover.is_empty() {
            lbl.on_hover_text(hover);
        }
    }

    fn txt_path_label(&mut self, ui: &mut egui::Ui) {
        let (text, color, hover) = self.record_follow(&self.txt_path);
        let mut rt = egui::RichText::new(text).small();
        if let Some(c) = color {
            rt = rt.color(c);
        }
        let lbl = ui.label(rt);
        if !hover.is_empty() {
            lbl.on_hover_text(hover);
        }
    }

    /// 左栏组 3：帧配置/离线绘图。
    /// 自研简洁布局：单列逐行，标签定宽右对齐（冒号对齐），输入框定宽/自填满，
    /// 任何面板宽度都不会重叠遮挡。
    fn frame_cfg_body(&mut self, ui: &mut egui::Ui) {
        let accent = self.style.accent_color();
        let dim = self.style.text_dim();
        // 组内标签统一宽度（取最宽者，冒号右对齐且不留多余空白）
        let t_header = format!("{}:", tr(self.lang, "header"));
        let t_chk = format!("{}:", tr(self.lang, "checksum"));
        let t_length = format!("{}:", tr(self.lang, "length"));
        let t_order = format!("{}:", tr(self.lang, "byte_order"));
        let t_samples = format!("{}:", tr(self.lang, "samples_pkt"));
        let t_points = format!("{}:", tr(self.lang, "max_points"));
        // 循环码新增一行的两个标签：必须一起进 label_w，否则冒号对齐会坏。
        let t_cycle = format!("{}:", tr(self.lang, "cycle_code"));
        let t_cycle_w = format!("{}:", tr(self.lang, "cycle_width"));
        // 组内标签统一宽度（取最宽者，冒号右对齐且不留多余空白）。
        // "Check:"/"校验:" 都短于组内最宽标签，进 label_w 不影响对齐也不会撑宽。
        let label_w = labels_width(
            ui,
            &[
                &t_header, &t_chk, &t_length, &t_order, &t_samples, &t_points,
                &t_cycle, &t_cycle_w,
            ],
        );
        let lbl = |ui: &mut egui::Ui, text: &str| field_label(ui, text, label_w, dim);
        // 帧头行与参数区用同一套两列分栏：左列 帧头，右列 校验。
        // 这样"校验:"标签与下方"样本:"标签起点完全一致（对齐），下拉宽度也与输入框等宽。
        // 撑宽防护：左列 available ≈ 65-69px ≥ TextEdit 最小宽 64（回归测试把关）。
        #[cfg(test)]
        let mut chk_col_x = f32::NAN;
        ui.columns(2, |cols| {
            let c0 = &mut cols[0];
            c0.horizontal(|ui| {
                lbl(ui, &t_header);
                let w = ui.available_width().max(40.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.header_text)
                        .desired_width(w)
                        .hint_text("EB 90"),
                );
            });
            let c1 = &mut cols[1];
            c1.horizontal(|ui| {
                lbl(ui, &t_chk);
                let w = ui.available_width().max(40.0);
                let chk_text = match self.checksum_mode {
                    ChecksumMode::None => tr(self.lang, "chk_none"),
                    ChecksumMode::Sum8 => tr(self.lang, "chk_sum8"),
                    ChecksumMode::Sum16 => tr(self.lang, "chk_sum16"),
                };
                egui::ComboBox::from_id_salt("checksum_combo")
                    .selected_text(chk_text)
                    .width(w)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.checksum_mode, ChecksumMode::None, tr(self.lang, "chk_none"));
                        ui.selectable_value(&mut self.checksum_mode, ChecksumMode::Sum8, tr(self.lang, "chk_sum8"));
                        ui.selectable_value(&mut self.checksum_mode, ChecksumMode::Sum16, tr(self.lang, "chk_sum16"));
                    });
            });
            #[cfg(test)]
            {
                chk_col_x = c1.min_rect().min.x;
            }
        });
        // 参数区两列紧凑排布（三行，仍然两列不新增列）：
        // 左列 帧长 / 字节序 / 循环码，右列 样本 / 点数 / 位宽。
        // SUM 校验：从帧头算起，校验字节在帧尾（8=和低8位=最后1字节；16=和低16位=最后2字节），
        // 应用帧配置生效，校验失败整帧丢弃并在状态栏计数。
        ui.columns(2, |cols| {
            // ---- 左列 ----
            let c0 = &mut cols[0];
            c0.horizontal(|ui| {
                lbl(ui, &t_length);
                let w = ui.available_width().max(28.0);
                ui.add(egui::TextEdit::singleline(&mut self.length_text).desired_width(w));
            });
            c0.horizontal(|ui| {
                lbl(ui, &t_order);
                let w = ui.available_width().max(28.0);
                egui::ComboBox::from_id_salt("order_combo")
                    .selected_text(if self.order_big { "big" } else { "little" })
                    .width(w)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.order_big, true, "big");
                        ui.selectable_value(&mut self.order_big, false, "little");
                    });
            });
            // 新增第三行（仍然两列，绝不新增列）：循环码位置，界面 1 起算，留空 = 关闭
            c0.horizontal(|ui| {
                lbl(ui, &t_cycle);
                let w = ui.available_width().max(28.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.cycle_pos_text)
                        .desired_width(w)
                        .hint_text("0"),
                )
                .on_hover_text(tr(self.lang, "cycle_pos_tip"));
            });
            // ---- 右列 ----
            let c1 = &mut cols[1];
            c1.horizontal(|ui| {
                lbl(ui, &t_samples);
                let w = ui.available_width().max(28.0);
                ui.add(egui::TextEdit::singleline(&mut self.samples_text).desired_width(w));
            });
            #[cfg(test)]
            {
                // 对齐断言：帧头行的"校验"列起点必须与这里"样本"列起点一致
                let sample_col_x = c1.min_rect().min.x;
                assert!(
                    (chk_col_x - sample_col_x).abs() < 0.5,
                    "校验列与样本列未对齐: chk={chk_col_x:.1} sample={sample_col_x:.1}"
                );
            }
            c1.horizontal(|ui| {
                lbl(ui, &t_points);
                let w = ui.available_width().max(28.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.max_points_text).desired_width(w),
                );
            });
            // 新增第三行（右列）：位宽下拉，固定 8 / 16 / 24 / 32，默认 8
            c1.horizontal(|ui| {
                lbl(ui, &t_cycle_w);
                let w = ui.available_width().max(28.0);
                egui::ComboBox::from_id_salt("cycle_width_combo")
                    .selected_text(self.cycle_width_bits.to_string())
                    .width(w)
                    .show_ui(ui, |ui| {
                        for bits in [8u8, 16, 24, 32] {
                            ui.selectable_value(
                                &mut self.cycle_width_bits,
                                bits,
                                bits.to_string(),
                            );
                        }
                    });
            });
        });
        // 应用帧配置（通栏）
        let row_ap = ui.add_sized(
            [ui.available_width(), 24.0],
            egui::Button::new(
                egui::RichText::new(tr(self.lang, "apply_frame"))
                    .color(egui::Color32::WHITE),
            )
            .fill(accent),
        );
        if row_ap.clicked() {
            self.apply_frame_config();
        }
        // 原始数据绘图 | 剪贴板数据绘图（各占一半）
        ui.horizontal(|ui| {
            let half = (ui.available_width() - 8.0) / 2.0;
            if ui
                .add_sized([half, 24.0], egui::Button::new(tr(self.lang, "load_file")))
                .clicked()
            {
                self.load_file_offline();
            }
            if ui
                .add_sized([half, 24.0], egui::Button::new(tr(self.lang, "analyze_clip")))
                .clicked()
            {
                self.analyze_clipboard();
            }
        });
    }

    /// 左栏组 4：通道配置（通道数+应用 → 色标图例 → 通道行）。
    fn channel_body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("{}:", tr(self.lang, "channels")));
            ui.add(
                egui::TextEdit::singleline(&mut self.channel_count_text)
                    .desired_width(90.0),
            );
            // 应用按钮与"应用帧配置"同款主色
            if ui
                .add_sized(
                    [72.0, 24.0],
                    egui::Button::new(
                        egui::RichText::new(tr(self.lang, "apply"))
                            .color(egui::Color32::WHITE),
                    )
                    .fill(self.style.accent_color()),
                )
                .clicked()
            {
                self.apply_channel_config();
            }
        });
        // 色标图例行（Python _update_color_legend）
        let count = self.channels.len().min(8);
        if count > 0 {
            ui.horizontal(|ui| {
                for i in 0..count {
                    let c = crate::ui::style::CHANNEL_COLORS[i % 8];
                    ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!(" {} ", i + 1))
                                .color(egui::Color32::WHITE)
                                .small(),
                        )
                        .fill(c),
                    );
                }
            });
        }
        // 通道行：[✓] N Idx:[........] ±:[..] Dif☑
        // 只有 Idx 随左栏宽度伸缩；± 和 Dif 固定宽度（对齐 Python：sign_bit 框 width=3）
        let dim = self.style.text_dim();
        let dif_text = tr(self.lang, "dif");
        let lw = labels_width(ui, &["Idx:", "±:"]);
        let dif_w = labels_width(ui, &[dif_text]) + 26.0; // 勾选框 18 + 间距
        let sp = ui.spacing().item_spacing.x;
        const SIG_W: f32 = 46.0; // ± 框固定宽（-1 / 15 / 23 都放得下）
        // 固定开销：勾选框 + 序号 + 两个标签 + ± 框 + Dif + 6 个间距
        let fixed = 18.0 + 10.0 + lw * 2.0 + SIG_W + dif_w + sp * 6.0;
        let idx_w = (ui.available_width() - fixed).max(40.0);
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let color = crate::ui::style::CHANNEL_COLORS[i % 8];
            ui.horizontal(|ui| {
                ui.checkbox(&mut ch.visible, "");
                ui.label(
                    egui::RichText::new(format!("{}", i + 1))
                        .color(color)
                        .strong(),
                );
                field_label(ui, "Idx:", lw, dim);
                let idx_resp = ui.add_sized(
                    [idx_w, 20.0],
                    egui::TextEdit::singleline(&mut ch.idx_text),
                );
                // Python 版行为：离开 Idx 框（FocusOut）或按回车（Return）后，
                // 自动把符号位设为 字节数*8-1（1 字节→7，2 字节→15，3 字节→23）
                if idx_resp.lost_focus() {
                    let n = parse_indices_str(&ch.idx_text).len();
                    if n > 0 {
                        ch.sig_text = (n * 8 - 1).to_string();
                    }
                }
                field_label(ui, "±:", lw, dim);
                ui.add_sized(
                    [SIG_W, 20.0],
                    egui::TextEdit::singleline(&mut ch.sig_text),
                );
                ui.checkbox(&mut ch.dif, dif_text);
            });
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        // 锁死内容宽度 = 面板宽度：切换中/英文时标签变长也不会把左栏顶宽，
        // 否则 egui 会把 SidePanel 撑到内容宽度（表现为"切语言左栏自己变宽"）。
        let full_w = ui.available_width();
        ui.set_min_width(full_w);
        ui.set_max_width(full_w);
        ui.add_space(2.0);
        ui.style_mut().spacing.item_spacing.y = 4.0;
        // 左栏字体比默认大，TextEdit 实际高 ≈20；ComboBox/按钮高度取 interact_size(18)，
        // 不随字体变 → 端口/波特率/校验位下拉永远差一截。统一提到 20 对齐（所有下拉/按钮等高）。
        ui.style_mut().spacing.interact_size.y = 20.0;
        // 组标题（大标题）在卡片外面，卡片只包内容 —— 对齐 Python LabelFrame 观感
        let accent = self.style.accent_color();
        let pad = CARD_PAD;
        let _g1 = egui::CollapsingHeader::new(
            egui::RichText::new(tr(self.lang, "connection")).strong().color(accent),
        )
        .default_open(true)
        .show(ui, |ui| {
            card(ui, egui::Margin::same(pad), |ui| self.connection_body(ui));
        });
        let _g2 = egui::CollapsingHeader::new(
            egui::RichText::new(tr(self.lang, "datalog")).strong().color(accent),
        )
        .default_open(true)
        .show(ui, |ui| {
            card(ui, egui::Margin::same(pad), |ui| self.data_record_body(ui));
        });
        let _g3 = egui::CollapsingHeader::new(
            egui::RichText::new(tr(self.lang, "framecfg")).strong().color(accent),
        )
        .default_open(true)
        .show(ui, |ui| {
            card(ui, egui::Margin::same(pad), |ui| self.frame_cfg_body(ui));
        });
        let _g4 = egui::CollapsingHeader::new(
            egui::RichText::new(tr(self.lang, "chancfg")).strong().color(accent),
        )
        .default_open(true)
        .show(ui, |ui| {
            card(ui, egui::Margin::same(pad), |ui| self.channel_body(ui));
        });
        // 第 5 组：循环码计数（独立折叠组 + 卡片，不塞进通道卡片内部）。
        // 只在循环码位置非空时显示；留空时这一片完全不出现。
        // 跟在通道卡片后面即可：外层是 ScrollArea，8 通道时仍可滚动到，
        // 卡片内全部用会自动换行的 label，不会把左栏顶宽。
        if self.cycle_stats_visible() {
            let _g5 = egui::CollapsingHeader::new(
                egui::RichText::new(tr(self.lang, "cycle_stats")).strong().color(accent),
            )
            .default_open(true)
            .show(ui, |ui| {
                card(ui, egui::Margin::same(pad), |ui| self.cycle_stats_body(ui));
            });
        }
        ui.add_space(4.0);
    }

    /// 循环码计数框的展示开关：仅在有循环位置（cycle_pos_text 非空）时显示。
    fn cycle_stats_visible(&self) -> bool {
        !self.cycle_pos_text.trim().is_empty()
    }

    /// 连接时长文案：已连接显示 HH:MM:SS，未连接显示 —。
    fn connection_duration_text(&self) -> String {
        match self.connected_at {
            Some(t0) => {
                let s = t0.elapsed().as_secs();
                let (hh, mm, ss) = (s / 3600, (s % 3600) / 60, s % 60);
                format!("{:02}:{:02}:{:02}", hh, mm, ss)
            }
            None => "—".to_string(),
        }
    }

    /// 第 5 组内容：循环码判定明细（不重复状态栏那个「丢帧 N」总数的大字）。
    /// 全部用 small label，文字会自动换行，中英切换都放得下、不撑宽左栏。
    fn cycle_stats_body(&mut self, ui: &mut egui::Ui) {
        let dim = self.style.text_dim();
        let cur: String = self
            .engine
            .cycle_current
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".to_string());
        let last_jump: String = self
            .engine
            .cycle_last_jump
            .map(|(expected, actual)| format!("{} → {}", expected, actual))
            .unwrap_or_else(|| "—".to_string());
        // 连接时长：已连接显示 HH:MM:SS，未连接显示 —
        let dur_text = self.connection_duration_text();
        // 清零按钮文案 / 提示 / 点击后状态文案；点击只置标志，出网格后再清零。
        let reset_label = tr(self.lang, "cycle_reset");
        let reset_tip = tr(self.lang, "cycle_reset_tip");
        let reset_done = tr(self.lang, "cycle_reset_done");
        let reset_pressed = std::cell::Cell::new(false);
        // 顶部：连接时长（带细分隔线）
        ui.separator();
        // 两列网格：标签列由 Grid 自动取最长标签定宽、数值列自适应，避免冒号错位。
        egui::Grid::new("cycle_stats_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                let lbl = |ui: &mut egui::Ui, t: &str| {
                    ui.add(egui::Label::new(egui::RichText::new(t).small().color(dim)));
                };
                // 连接时长
                lbl(ui, tr(self.lang, "cycle_dur"));
                ui.label(egui::RichText::new(dur_text.clone()).small());
                ui.end_row();
                // 当前循环码
                lbl(ui, tr(self.lang, "cycle_cur"));
                ui.label(egui::RichText::new(cur).small());
                ui.end_row();
                // 累计丢帧
                lbl(ui, tr(self.lang, "cycle_lost"));
                ui.label(egui::RichText::new(self.engine.cycle_lost.to_string()).small());
                ui.end_row();
                // 跳变次数
                lbl(ui, tr(self.lang, "cycle_jumps"));
                ui.label(egui::RichText::new(self.engine.cycle_jumps.to_string()).small());
                ui.end_row();
                // 上次跳变
                lbl(ui, tr(self.lang, "cycle_last_jump"));
                ui.label(egui::RichText::new(last_jump).small());
                ui.end_row();
                        // 异常次数：数值后跟弱提示「循环码不动」，清零按钮同行靠右（点击只置标志，出网格后再清零）
                lbl(ui, tr(self.lang, "cycle_anom"));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(self.engine.cycle_anomalies.to_string()).small());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(tr(self.lang, "cycle_anom_note"))
                                .small()
                                .weak(),
                        ),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(reset_label)
                            .on_hover_text(reset_tip)
                            .clicked()
                        {
                            reset_pressed.set(true);
                        }
                    });
                });
                ui.end_row();
            });
        // 清零：网格内按钮只置标志（避免在网格闭包里改 self），出网格后再真正清零，
        // 清掉计数 + 上一帧基准，下一帧重新起算；软件不猜回退，由人主动清零。
        if reset_pressed.get() {
            self.engine.cycle_reset();
            self.status_msg = reset_done.to_string();
        }
    }

    /// 底部发送行（Python tx_frame 1:1）。
    /// Data 行与状态行的垂直间隔在 F12 里调（send_status_gap）。
    fn send_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Data:");
            // 右侧 HEX/ASCII/发送/命令发送 的宽度实测（英文按钮更宽，写死会撑宽卡片）
            let sp = ui.spacing().item_spacing.x;
            let bpad = ui.spacing().button_padding.x * 2.0;
            let right_w = ("HEX ASCII".split_whitespace())
                .map(|t| labels_width(ui, &[t]) + bpad)
                .chain([
                    labels_width(ui, &[tr(self.lang, "send")]) + bpad,
                    labels_width(ui, &[tr(self.lang, "history")]) + bpad,
                ])
                .sum::<f32>();
            let entry_w = (ui.available_width() - right_w - sp * 5.0).max(60.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.send_text)
                    .desired_width(entry_w),
            );
            ui.selectable_value(&mut self.send_hex, true, "HEX");
            ui.selectable_value(&mut self.send_hex, false, "ASCII");
            if ui.button(tr(self.lang, "send")).clicked() {
                self.do_send();
            }
            if ui.button(tr(self.lang, "history")).clicked() {
                // 弹出历史窗口由 update 层处理（show_history 标志）
                self.show_history = !self.show_history;
            }
        });
    }

    fn do_send(&mut self) {
        let text = self.send_text.clone();
        self.send_payload(&text, true);
    }

    /// 统一发送通道：编码（HEX/ASCII）→ 目标（串口/UDP 远端）→ send_bytes。
    /// `record_history`：主发送行记录历史，命令列表/循环发送不记录。
    fn send_payload(&mut self, text: &str, record_history: bool) {
        if text.trim().is_empty() {
            return;
        }
        let data: Vec<u8> = if self.send_hex {
            match parse_header_hex(text) {
                Ok(b) => b,
                Err(e) => {
                    self.status_msg = format!("{}: {}", tr(self.lang, "send_err"), e);
                    return;
                }
            }
        } else {
            text.as_bytes().to_vec()
        };
        let target = if self.link_kind == LinkKind::Udp {
            let ip = self.udp_remote_ip.trim();
            let port = self.udp_remote_port.trim();
            if ip.is_empty() || port.is_empty() {
                self.status_msg = format!("{}: 远端IP/端口未设置", tr(self.lang, "send_err"));
                return;
            }
            Some(format!("{}:{}", ip, port))
        } else {
            None
        };
        match send_bytes(&self.handle, &data, target.as_deref()) {
            Ok(_) => {
                self.tx_count += 1;
                if record_history {
                    self.send_history.push(text.to_string());
                    if self.send_history.len() > 50 {
                        self.send_history.remove(0);
                    }
                }
            }
            Err(e) => self.status_msg = format!("{}: {}", tr(self.lang, "send_err"), e),
        }
    }

    /// 命令列表循环发送计时：到点发下一条（轮转），未连接自动停。
    /// 在 ui() 每帧调用（egui 即时模式，无需额外线程）。
    fn poll_cmd_loop(&mut self) {
        if !self.cmd_loop {
            return;
        }
        if !self.connected || self.cmd_list.is_empty() {
            self.cmd_loop = false;
            return;
        }
        let due = self
            .cmd_next_send
            .map(|t| Instant::now() >= t)
            .unwrap_or(true);
        if due {
            let idx = self.cmd_loop_idx.min(self.cmd_list.len() - 1);
            let text = self.cmd_list[idx].clone();
            self.send_payload(&text, false);
            self.cmd_loop_idx = (idx + 1) % self.cmd_list.len();
            self.cmd_next_send =
                Some(Instant::now() + Duration::from_millis(self.cmd_interval_ms.max(10) as u64));
        }
    }

    // ============ 配置持久化（.config.json） ============

    /// 配置文件位置：exe 同目录 `.config.json`（对齐 Python 版；双击运行即 exe 目录）。
    fn config_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join(".config.json");
            }
        }
        PathBuf::from(".config.json")
    }

    /// 写回配置：退出 / 语言切换 / 主题切换 / 应用帧配置 / 应用通道配置时调用。
    /// 写失败静默（exe 在只读目录时跳过，不影响运行）。
    pub fn save_config(&self) {
        let cfg = PersistConfig {
            language: match self.lang {
                Lang::Zh => "zh".into(),
                Lang::En => "en".into(),
            },
            theme: self.style.theme_index,
            frame: PersistFrame {
                header: self.header_text.clone(),
                length: self.length_text.clone(),
                order_big: self.order_big,
                checksum: self.checksum_mode.as_str().to_string(),
                samples: self.samples_text.clone(),
                max_points: self.max_points_text.clone(),
                cycle_pos: self.cycle_pos_text.clone(),
                cycle_width: self.cycle_width_bits.to_string(),
                cycle_pos_base: "1".into(),
            },
            channels: self
                .channels
                .iter()
                .map(|c| PersistChannel {
                    idx: c.idx_text.clone(),
                    sig: c.sig_text.clone(),
                    dif: c.dif,
                    visible: c.visible,
                })
                .collect(),
            baud: self.baud_text.clone(),
            parity: self.parity_opt.as_str().to_string(),
            udp_local_ip: self.udp_local_ip.clone(),
            udp_local_port: self.udp_local_port.clone(),
            udp_remote_ip: self.udp_remote_ip.clone(),
            udp_remote_port: self.udp_remote_port.clone(),
            send_text: self.send_text.clone(),
            cmd_list: self.cmd_list.clone(),
            cmd_interval_ms: self.cmd_interval_ms,
        };
        if let Ok(s) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(Self::config_path(), s);
        }
    }

    /// 启动时恢复上次配置（无文件/解析失败 → 保持默认值，静默跳过）。
    fn load_config(&mut self) {
        let Ok(s) = std::fs::read_to_string(Self::config_path()) else {
            return;
        };
        let Ok(cfg) = serde_json::from_str::<PersistConfig>(&s) else {
            return;
        };
        self.lang = match cfg.language.as_str() {
            "en" => Lang::En,
            _ => Lang::Zh,
        };
        self.style.theme_index = cfg.theme;
        let f = cfg.frame;
        if !f.header.is_empty() {
            self.header_text = f.header;
        }
        if !f.length.is_empty() {
            self.length_text = f.length;
        }
        self.order_big = f.order_big;
        self.checksum_mode = ChecksumMode::from_str(&f.checksum);
        if !f.samples.is_empty() {
            self.samples_text = f.samples;
        }
        if !f.max_points.is_empty() {
            self.max_points_text = f.max_points;
        }
        // 循环码位置：空串 = 关闭，必须如实恢复（不能以 is_empty 判断跳过）。
        // 旧配置按 0 起算存；新版界面改成 1 起算，缺标记时自动 +1，避免升级后读错字节。
        self.cycle_pos_text = migrate_cycle_pos(&f.cycle_pos_base, f.cycle_pos);
        self.cycle_width_bits = parse_cycle_width(&f.cycle_width);
        if !cfg.channels.is_empty() {
            self.channel_count_text = cfg.channels.len().to_string();
            self.channels = cfg
                .channels
                .iter()
                .map(|c| ChannelUi {
                    idx_text: c.idx.clone(),
                    sig_text: c.sig.clone(),
                    dif: c.dif,
                    visible: c.visible,
                })
                .collect();
        }
        if !cfg.baud.is_empty() {
            self.baud_text = cfg.baud;
        }
        self.parity_opt = ParityOpt::from_str(&cfg.parity);
        if !cfg.udp_local_ip.is_empty() {
            self.udp_local_ip = cfg.udp_local_ip;
        }
        if !cfg.udp_local_port.is_empty() {
            self.udp_local_port = cfg.udp_local_port;
        }
        self.udp_remote_ip = cfg.udp_remote_ip;
        self.udp_remote_port = cfg.udp_remote_port;
        if !cfg.send_text.is_empty() {
            self.send_text = cfg.send_text;
        }
        self.cmd_list = cfg.cmd_list;
        if cfg.cmd_interval_ms > 0 {
            self.cmd_interval_ms = cfg.cmd_interval_ms;
        }
    }

    /// 原始视图：`序号: XX XX XX` 每帧一行（Python raw_text 语义，最多 1000 行）。
    fn raw_view(&mut self, ui: &mut egui::Ui, height: f32) {
        egui::Frame::none()
            .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x3A, 0x3E, 0x4E)))
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let frames = &self.engine.frame_log;
                            if frames.is_empty() {
                                ui.weak("(暂无数据，连接后在此显示原始帧)");
                                return;
                            }
                            let start_seq = self.engine.raw_frame_count.saturating_sub(frames.len() as u64);
                            for (i, frame) in frames.iter().enumerate() {
                                let seq = start_seq + i as u64;
                                let hex: Vec<String> =
                                    frame.iter().map(|b| format!("{:02X}", b)).collect();
                                ui.monospace(format!("{}: {}", seq, hex.join(" ")));
                            }
                        });
                });
            });
    }

    /// 把配置回执按「当前语言」渲染成状态栏文字。每帧重算，
    /// 所以中英切换后无需重新点「应用」，两格会立刻跟着变。
    /// 注：错误明细文本来自配置解析，目前本身是中文（全项目错误信息统一为中文硬编码），
    /// 这里只让「配置错误 / Config Error」前缀跟随语言。
    fn config_receipt_text(&self, r: &ConfigReceipt) -> String {
        match r {
            ConfigReceipt::None => "—".to_string(),
            ConfigReceipt::FrameApplied => tr(self.lang, "frame_applied").to_string(),
            ConfigReceipt::ChannelApplied(n) => {
                format!("{} ({})", tr(self.lang, "channel_applied"), n)
            }
            ConfigReceipt::Error(detail) => {
                format!("{}: {}", tr(self.lang, "config_error"), detail)
            }
        }
    }

    fn frame_cell_text(&self) -> String {
        self.config_receipt_text(&self.frame_msg)
    }
    fn channel_cell_text(&self) -> String {
        self.config_receipt_text(&self.channel_msg)
    }

    /// 状态栏从左到右 11 格文本（最后一格是 TX）。
    /// 按功能区块分格：格内互斥（后一条换掉前一条是合理的），格间独立、互不覆盖。
    /// 抽成纯数据便于单测断言顺序与口径。
    /// 记录中指示灯是独立 widget（不进本数组），便于保持本数组单测的简单断言。
    fn status_cells(&self) -> Vec<String> {
        let dash = "—".to_string();
        // ①连接格：一次性连接错误优先，否则回落到常驻「已连接/未连接」
        let conn = if self.link_msg.is_empty() {
            tr(self.lang, if self.connected { "connected" } else { "not_connected" }).to_string()
        } else {
            self.link_msg.clone()
        };
        // ②③配置回执：一直留着，直到下次应用才更新；文字按当前语言实时渲染
        let frame_cfg = self.frame_cell_text();
        let channel_cfg = self.channel_cell_text();
        // ④通道信息：已应用配置的位宽与正负
        let chan_info = self.channel_info_text();
        // ⑤校验同步状态：只在「无帧头 + 开校验」时有意义
        let sync = if self.engine.checksum_mode != ChecksumMode::None
            && self.engine.frame_header.is_empty()
        {
            tr(self.lang, if self.engine.sync_locked { "chk_synced" } else { "chk_sync" }).to_string()
        } else {
            String::new()
        };
        // ⑥⑦⑧计数常驻显示（没有也显示 0，不再留空）
        let chk = format!("{} {}", tr(self.lang, "chk_err"), self.engine.checksum_errors);
        let lost = format!("{} {}", tr(self.lang, "lost_frames"), self.engine.lost_frames());
        let frames = if self.lang == Lang::Zh {
            format!("帧 {}", self.engine.frame_count)
        } else {
            format!("Frames: {}", self.engine.frame_count)
        };
        // ⑩跳变（循环码跳号累计）：常驻显示，0 也显示
        let jumps = format!("{} {}", tr(self.lang, "cycle_jumps_short"), self.engine.cycle_jumps);
        // ⑪最近操作：离线/剪贴板/清零/主题/发送/命令等一次性回执
        let recent = if self.status_msg.is_empty() { dash } else { self.status_msg.clone() };
        let tx = format!("TX: {}", self.tx_count);
        vec![conn, frame_cfg, channel_cfg, chan_info, sync, chk, lost, frames, jumps, recent, tx]
    }

    /// 通道信息格：位宽按已应用的 Idx 字节数 × 8，正负按 ± 是否为 -1。
    /// 位宽混 → 「8/16位 混合」；位宽一致但符号混 → 「16位 混合」。
    fn channel_info_text(&self) -> String {
        let mut bits: Vec<u32> = Vec::new();
        let mut signed: Vec<bool> = Vec::new();
        for cfg in self.engine.channels.iter().take(self.engine.channel_count) {
            if cfg.active && !cfg.indices.is_empty() {
                bits.push((cfg.indices.len() * 8) as u32);
                signed.push(cfg.sign_bit != -1);
            }
        }
        if bits.is_empty() {
            return "—".to_string();
        }
        let mut uniq = bits.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let zh = self.lang == Lang::Zh;
        let bit_word = if uniq.len() == 1 {
            if zh { format!("{}位", uniq[0]) } else { format!("{}bit", uniq[0]) }
        } else {
            let list = uniq.iter().map(|b| b.to_string()).collect::<Vec<_>>().join("/");
            if zh { format!("{}位", list) } else { format!("{}bit", list) }
        };
        let all_signed = signed.iter().all(|&s| s);
        let all_unsigned = signed.iter().all(|&s| !s);
        let sign_word = if uniq.len() > 1 || (!all_signed && !all_unsigned) {
            tr(self.lang, "mixed")
        } else if all_signed {
            tr(self.lang, "signed")
        } else {
            tr(self.lang, "unsigned")
        };
        format!("{} {}", bit_word, sign_word)
    }

    /// 状态行：按 status_cells() 的顺序铺开，每格贴合当前文字、不做均摊。
    /// 空间不够时按优先级从低到高压缩：最近操作 → 通道信息 → 配置回执
    /// → 校验/数字 → 连接；数字格保底不压缩，长文字截断加省略号、悬停看全文。
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let dim = self.style.text_dim();
        let accent = self.style.accent_color();
        let cells = self.status_cells(); // 11 格：[0连接][1帧配置][2通道配置][3通道信息][4校验同步][5校验错][6丢帧][7帧数][8跳变][9最近操作][10TX]
        let left_texts: Vec<&str> = cells[..9].iter().map(|s| s.as_str()).collect();
        let recent_text = cells[9].clone();
        let tx_text = cells[10].clone();

        let h: f32 = 20.0;
        let sep_w: f32 = 8.0;
        let sp: f32 = ui.spacing().item_spacing.x;

        // 左排每格贴合当前文字宽 + 2，不均摊、不居中、文字显式左对齐
        let mut w: Vec<f32> = left_texts.iter().map(|t| labels_width(ui, &[*t]) + 2.0).collect();
        let conn_min = labels_width(ui, &[
            tr(self.lang, "connected"),
            tr(self.lang, "not_connected"),
        ]) + 2.0;
        // 下限：数字格（5 校验错 / 6 丢帧 / 7 帧数 / 8 跳变）保底不压；
        // 空格子留最小占位；其余给适度下限，空间不够时优先让位。
        let mut lower: Vec<f32> = vec![
            conn_min.min(w[0]),
            w[1].min(56.0),
            w[2].min(56.0),
            w[3].min(48.0),
            w[4], w[5], w[6], w[7], w[8],
        ];
        for i in 0..9 {
            if left_texts[i].is_empty() || left_texts[i] == "—" {
                w[i] = w[i].max(18.0);
                lower[i] = lower[i].min(w[i]);
            }
        }
        // 记录灯固定宽度（不随状态跳动）
        let light_w: f32 = 76.0;
        // 右排：最近操作 + TX，各自固定宽、文字靠右，TX 最右且不额外吃剩余宽度
        let recent_w = labels_width(ui, &[recent_text.as_str(), tr(self.lang, "rec_wait_conn"), tr(self.lang, "rec_saving")]) + 2.0;
        let tx_w = labels_width(ui, &[tx_text.as_str(), "TX: 0000000"]) + 4.0;

        // 分隔线：左 9 格间 8 条 + 灯前 1 条 + 右排前 1 条 = 10 条
        let seps: f32 = sep_w * 10.0;
        let gaps: f32 = sp * 11.0 + 4.0;
        let left_avail = (ui.available_width() - light_w - recent_w - tx_w - seps - gaps).max(0.0);
        let mut need: f32 = w.iter().sum();
        // 压缩优先级（从最不重要开始）：通道信息 → 帧配置 → 通道配置 → 同步 → 连接；
        // 数字格 5/6/7/8 保底不压；最近操作已挪到右排可整体让位。
        let order: [usize; 5] = [3, 1, 2, 4, 0];
        if need > left_avail {
            for &i in &order {
                if need <= left_avail {
                    break;
                }
                let cut = (need - left_avail).min((w[i] - lower[i]).max(0.0));
                w[i] -= cut;
                need -= cut;
            }
        }

        ui.horizontal(|ui| {
            for (i, text) in left_texts.iter().enumerate() {
                if i > 0 {
                    ui.add_sized([sep_w, h], egui::Separator::default().vertical().spacing(sep_w));
                }
                let color = if i == 8 { accent } else { dim };
                let resp = ui.add_sized(
                    [w[i], h],
                    egui::Label::new(egui::RichText::new(*text).color(color))
                        .truncate()
                        .halign(egui::Align::LEFT),
                );
                if !text.is_empty() && *text != "—" {
                    let _ = resp.on_hover_text(*text);
                }
            }
            // 记录指示灯（独立 widget，不进 status_cells 文本数组）
            ui.add_sized([sep_w, h], egui::Separator::default().vertical().spacing(sep_w));
            self.draw_record_light(ui, light_w, h);
            // 弹性 spacer：纯空白，不绘制文字/线/背景，吃掉剩余宽度
            let remain = ui.available_width() - recent_w - tx_w - sp;
            if remain > 0.0 {
                ui.add_space(remain);
            }
            // 右排：最近操作（靠右）
            let r_resp = ui.add_sized(
                [recent_w, h],
                egui::Label::new(egui::RichText::new(recent_text.as_str()).color(dim))
                    .truncate()
                    .halign(egui::Align::RIGHT),
            );
            if !recent_text.is_empty() && recent_text != "—" {
                let _ = r_resp.on_hover_text(recent_text.as_str());
            }
            // TX 最右
            ui.add_sized(
                [tx_w, h],
                egui::Label::new(egui::RichText::new(tx_text.as_str()).color(dim))
                    .truncate()
                    .halign(egui::Align::RIGHT),
            );
        });
    }

    /// 状态栏「记录中」指示灯：亮=任一保存正在写（csv_recording||txt_recording），灰=未记录。
    /// 独立 widget（不进 status_cells 文本数组），固定宽度不跳动；悬停提示正在写的文件与行数。
    fn draw_record_light(&self, ui: &mut egui::Ui, w: f32, h: f32) {
        let on = self.recording_any();
        let dot_color = if on {
            self.style.accent_color()
        } else {
            egui::Color32::from_gray(140)
        };
        let label = if on {
            tr(self.lang, "rec_on")
        } else {
            tr(self.lang, "rec_off")
        };
        let mut hover = String::new();
        if self.csv_recording() {
            if let Some(p) = &self.csv_path {
                hover.push_str(&format!("{}: {}\n", tr(self.lang, "csv_file"), p.display()));
            }
            hover.push_str(&if self.lang == Lang::Zh {
                format!("已写 {} 行\n", self.csv_saved_rows)
            } else {
                format!("{} rows written\n", self.csv_saved_rows)
            });
        }
        if self.txt_recording() {
            if let Some(p) = &self.txt_path {
                hover.push_str(&format!("{}: {}\n", tr(self.lang, "txt_file"), p.display()));
            }
        }
        let resp = ui.allocate_ui(egui::vec2(w, h), |ui| {
            ui.horizontal(|ui| {
                let (r, painter) =
                    ui.allocate_painter(egui::vec2(10.0, 10.0), egui::Sense::hover());
                painter.circle_filled(r.rect.center(), 4.0, dot_color);
                ui.label(egui::RichText::new(label).color(dot_color));
            });
        });
        if !hover.is_empty() {
            let _ = resp.response.on_hover_text(hover);
        }
    }

    /// 连接失败弹窗（Some(错误信息) 时显示，点确定 / 右上角 × 关闭）。
    fn err_dialog_window(&mut self, ctx: &egui::Context) {
        let mut show = self.err_dialog.is_some();
        if !show {
            return;
        }
        let msg = self.err_dialog.clone().unwrap_or_default();
        // msg 由 do_connect 构造为两行：
        //   串口：「<端口名> <波特率>\n<错误原因>」
        //   UDP ：「<绑定IP>:<端口>\n<错误原因>」
        // 这里拆成「错误原因（strong/大，主）」与「端口/波特行（weak/小，次）」。
        // 文案与关闭逻辑不变，仅调整主次呈现与窗口尺寸。
        let (port_line, reason_line) = match msg.split_once('\n') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (String::new(), msg.clone()),
        };
        let mut open = true;
        egui::Window::new(egui::RichText::new(tr(self.lang, "err_title")).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            // 按内容收紧、去空白：锁定宽度，并给 max_height 兜底，
            // 防止旧窗口尺寸记忆把弹窗撑成大片空白。
            .min_width(340.0)
            .max_width(420.0)
            .max_height(260.0)
            .show(ctx, |ui| {
                // 错误原因：主，strong 且字号略大
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(reason_line).strong().size(14.0),
                    )
                    .wrap(),
                );
                // 端口/波特行：次，weak 且小
                if !port_line.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(port_line).weak().small(),
                        )
                        .wrap(),
                    );
                }
                ui.separator();
                // 确定按钮水平居中、紧贴文字下方：top_down + 居中，窗口按内容收紧不撑高，
                // 不会出现大段空白，按钮也不会悬空。
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if ui.button(tr(self.lang, "err_ok")).clicked() {
                        show = false;
                    }
                });
            });
        if !show || !open {
            self.err_dialog = None;
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new(tr(self.lang, "help"))
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label("1. 连接区选择 Serial/UDP，串口选端口+波特率（下拉选择）+校验位(None/Even/Odd，改后需重连)，UDP 填本机IP:端口监听；连接失败会弹窗提示具体原因。");
                ui.label("2. 帧配置：帧头可空（定长切帧）、校验下拉（no=无校验，8=SUM 8位，16=SUM 16位）：从帧头第 1 字节累加到校验字节前，8=和取低 8 位与帧最后 1 字节比对，16=和取低 16 位与帧最后 2 字节比对（高低字节跟随字节序设置）；校验失败的帧整帧丢弃，状态栏显示「校验错 N」计数；帧长、字节序、样本/帧、最大点数，点「应用帧配置」生效；循环码：填帧内字节序号（从 1 起算，第一字节填 1，与帧长口径一致）+ 位宽（8/16/24/32），软件每帧读一次循环码，跳号就按差值累计丢帧、号码重复记异常，状态栏只显示一个「丢帧 N」总数（未启用循环码时按校验错 + 对齐时悄悄跳过的帧数估算），通道配置下方多了「循环码计数」明细框；留空位置=关闭，位置加字节数超出帧长会明确报错；调试时设备重启或断开重连会让循环码从 0 重来，在「循环码计数」框点「清零」即可把计数和比较基准一起归零、下一帧重新起算（软件不猜回退，号码倒着走一律按差值计，忘了清零会看到偏大的丢帧数）；");
                ui.label("   ★ 无帧头 + 启用校验时：收到的数据可能是任意字节开头，软件会用「连续两帧校验都正确」来确认分帧起点（单帧通过可能是巧合），对齐前不画图，状态栏显示「等待校验同步」；对齐后正常解析，连续 3 帧校验失败会自动重新搜索对齐。");
                ui.label("3. 通道配置：填通道数点「应用」，每行 Idx 填字节索引（支持 , 空格 中文逗号 顿号），± 为符号位(-1=无，Idx 填完自动算)，Dif 勾选=原始线+差分虚线叠加；");
                ui.label("4. 离线绘图：帧配置好后点「载入文件」或「分析剪贴板」，把 hex 文本直接绘图；");
                ui.label("5. 数据记录：勾选提取数据保存(CSV)/原始数据保存(TXT)，未选文件时自动弹出保存对话框；路径标签悬停可看完整路径；");
                ui.label("6. 绘图工具栏：🏠 恢复原视图、➕/➖ 缩放、🗑 清空、⏸ 暂停绘图（数据照常解析，暂停时显示数据点圆点），▶ 恢复实时；");
                ui.label("7. 命令发送：手动填若干条命令，设间隔(ms)后 ▶ 自动循环轮发，或逐条点「发送」；HEX/ASCII 跟随发送行格式开关；");
                ui.label("8. 配置自动保存：帧配置/通道配置/语言/主题/波特率/校验位/命令列表存 exe 同目录 .config.json，退出和应用时写回，重启自动恢复；");
                ui.label("9. 🌙 循环切换 8 套主题（深色→原始→淡绿→淡蓝→米黄→粉色→Mac→Win），窗口标题栏跟随（Win11）；");
                ui.label("10. F12 界面微调：左栏宽度，保存到 ui_style.json。");
            });
        self.show_help = open;
    }

    /// 命令列表窗口（原历史发送改版，用户要求）：
    /// 手动填写若干条命令 → 设置发送间隔 → ▶ 自动循环轮发 或 每条手动点"发送"。
    /// HEX/ASCII 编码跟随主发送行的格式开关。
    fn history_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_history;
        let mut del: Option<usize> = None;
        egui::Window::new(tr(self.lang, "history"))
            .open(&mut open)
            .default_width(480.0)
            .show(ctx, |ui| {
                // 顶部：发送间隔 + 循环开关
                ui.horizontal(|ui| {
                    ui.label(tr(self.lang, "cmd_interval"));
                    ui.add(
                        egui::DragValue::new(&mut self.cmd_interval_ms)
                            .speed(100)
                            .range(10..=600000)
                            .suffix(" ms"),
                    );
                    if self.cmd_loop {
                        if ui.add(egui::Button::new("⏸")).on_hover_text(tr(self.lang, "cmd_stop")).clicked() {
                            self.cmd_loop = false;
                        }
                        ui.label(
                            egui::RichText::new(format!(
                                "{} #{}",
                                tr(self.lang, "cmd_sending"),
                                self.cmd_loop_idx + 1
                            ))
                            .color(self.style.accent_color()),
                        );
                    } else if ui
                        .add(egui::Button::new("▶"))
                        .on_hover_text(tr(self.lang, "cmd_start"))
                        .clicked()
                    {
                        if self.cmd_list.is_empty() {
                            self.status_msg = tr(self.lang, "cmd_empty").to_string();
                        } else if !self.connected {
                            self.status_msg = tr(self.lang, "cmd_not_connected").to_string();
                        } else {
                            self.cmd_loop = true;
                            self.cmd_loop_idx = 0;
                            self.cmd_next_send = Some(Instant::now());
                        }
                    }
                    if !self.connected {
                        ui.label(
                            egui::RichText::new(tr(self.lang, "cmd_not_connected"))
                                .color(self.style.text_dim())
                                .small(),
                        );
                    }
                });
                ui.separator();
                // 命令行列表：编辑 + 逐条发送 + 删除（命令多时可滚动）
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if self.cmd_list.is_empty() {
                        ui.label(egui::RichText::new(tr(self.lang, "cmd_empty")).weak());
                    }
                    for i in 0..self.cmd_list.len() {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("{:>2}.", i + 1));
                            let row_w = ui.available_width()
                                - ui.spacing().item_spacing.x * 2.0
                                - 96.0; // 发送 + × 两个按钮
                            ui.add(
                                egui::TextEdit::singleline(&mut self.cmd_list[i])
                                    .desired_width(row_w.max(60.0))
                                    .font(egui::TextStyle::Monospace),
                            );
                            if ui.small_button(tr(self.lang, "cmd_send_one")).clicked() {
                                let text = self.cmd_list[i].clone();
                                self.send_payload(&text, false);
                            }
                            // × (U+00D7)：✕ 字形中文字体缺失会显示方块
                            if ui
                                .small_button("×")
                                .on_hover_text(tr(self.lang, "cmd_del"))
                                .clicked()
                            {
                                del = Some(i);
                            }
                        });
                    }
                });
                if ui.button(tr(self.lang, "cmd_add")).clicked() {
                    self.cmd_list.push(String::new());
                }
            });
        if let Some(i) = del {
            self.cmd_list.remove(i);
            if self.cmd_list.is_empty() {
                self.cmd_loop = false;
            } else {
                self.cmd_loop_idx %= self.cmd_list.len();
            }
        }
        self.show_history = open;
    }
}

impl FrameApp {
    /// 全部界面绘制（从 eframe::App::update 抽出来，便于无窗口环境下做布局回归测试）。
    pub fn ui(&mut self, ctx: &egui::Context) {
        // F12 → 样式调试面板
        let f12 = ctx.input(|i| i.key_pressed(egui::Key::F12));
        if f12 {
            self.show_style_panel = !self.show_style_panel;
        }
        self.style.apply_theme(ctx);
        self.style.apply_text_styles(ctx);
        self.poll_link();
        self.poll_pick();
        self.poll_cmd_loop();

        // 底部状态行：横贯整个窗口（含左栏下方），标准状态栏形态
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                card(
                    ui,
                    egui::Margin { left: CARD_PAD, right: CARD_PAD, top: 3.0, bottom: 3.0 },
                    |ui| {
                        self.status_line(ui);
                    },
                );
            });

        // 左栏
        let panel = egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(self.style.left_panel_width)
            .width_range(300.0..=600.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.left_panel(ui);
                });
            });
        // 记录实际面板宽度（用于回归测试：切语言后不得自行变化）
        self.last_panel_width = panel.response.rect.width();

        // 中央主区（自底向上：Data 发送行 → 绘图卡片）。
        // 复刻 CentralPanel 默认边距(Margin::same(8))，仅把底边距改为
        // F12 可调的 send_status_gap —— 即 Data 行与状态栏的间隔。
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(hex_or(self.style.theme().bg, egui::Color32::GRAY))
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 8.0,
                        bottom: 12.0, // Data 行与状态栏间隔（用户调定，固化 12）
                    }),
            )
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr(self.lang, "plot_title"))
                        .strong()
                        .color(self.style.accent_color()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(tr(self.lang, "help")).clicked() {
                        self.show_help = !self.show_help;
                    }
                });
            });
            let mut hover_coord: Option<[f64; 2]> = None;
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                // 两张卡片：绘图↔Data 固定 CARD_GAP_V；Data↔状态栏间隔走中央面板底边距
                ui.style_mut().spacing.item_spacing.y = CARD_GAP_V;
                let pad = CARD_PAD;
                // 最底：发送行（上下零内边距）
                card(
                    ui,
                    egui::Margin { left: pad, right: pad, top: 0.0, bottom: 0.0 },
                    |ui| {
                        self.send_bar(ui);
                    },
                );
                // 绘图卡片（底部零内边距 → 工具栏到 Data 行间隙与 Data→状态行一致）
                card(
                    ui,
                    egui::Margin { left: pad, right: pad, top: pad, bottom: 0.0 },
                    |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        // 绘图/原始 切换（大按钮，选中主色填充，对齐 Python）
                        ui.horizontal(|ui| {
                            // 未选中按钮文字用主题正文色（WHITE 在浅色主题下不可见）
                            let dim_text = ui.visuals().text_color();
                            let sel = egui::Button::new(
                                egui::RichText::new(tr(self.lang, "view_plot")).color(
                                    if self.view_plot {
                                        egui::Color32::WHITE
                                    } else {
                                        dim_text
                                    },
                                ),
                            )
                            .fill(if self.view_plot {
                                self.style.accent_color()
                            } else {
                                ui.visuals().extreme_bg_color
                            });
                            if ui.add(sel).clicked() {
                                self.view_plot = true;
                            }
                            let sel2 = egui::Button::new(
                                egui::RichText::new(tr(self.lang, "view_raw")).color(
                                    if !self.view_plot {
                                        egui::Color32::WHITE
                                    } else {
                                        dim_text
                                    },
                                ),
                            )
                            .fill(if !self.view_plot {
                                self.style.accent_color()
                            } else {
                                ui.visuals().extreme_bg_color
                            });
                            if ui.add(sel2).clicked() {
                                self.view_plot = false;
                            }
                        });
                        let inner_h = (ui.available_height() - 44.0).max(160.0);
                        if self.view_plot {
                            let series = self.build_plot_series();
                            let lw = self.style.line_width;
                            let plot_bg =
                                hex_or(&self.style.bg_plot, egui::Color32::BLACK);
                            hover_coord = ui
                                .scope(|ui| {
                                    // 绘图区底色独立于输入框底色（对齐 Python plot_bg）
                                    ui.style_mut().visuals.extreme_bg_color = plot_bg;
                                    crate::ui::plot::draw_plot(
                                        ui,
                                        lw,
                                        &series,
                                        inner_h,
                                        self.plot_epoch,
                                        &mut self.plot_zoom,
                                        self.plot_paused,
                                    )
                                })
                                .inner;
                        } else {
                            self.raw_view(ui, inner_h);
                        }
                        // 工具栏（在绘图卡片内，紧贴绘图区下沿，对齐 Python）
                        ui.horizontal(|ui| {
                            if ui.button("🏠").on_hover_text("恢复原视图 (Home)").clicked() {
                                self.plot_epoch = self.plot_epoch.wrapping_add(1);
                                self.plot_zoom = 1.0;
                            }
                            if ui.button("➕").on_hover_text("放大").clicked() {
                                self.plot_zoom = 0.7;
                            }
                            if ui.button("➖").on_hover_text("缩小").clicked() {
                                self.plot_zoom = 1.4;
                            }
                            if ui.button("🗑").on_hover_text("清空绘图").clicked() {
                                if let Ok(e) = self.build_engine() {
                                    self.engine = e;
                                }
                                // 快照一并清空：暂停时点清空也应是空图
                                self.plot_snapshot_vals.clear();
                                self.plot_snapshot_dif.clear();
                            }
                            // 绘图暂停/继续：暂停时数据照常解析，绘图区冻结在快照上
                            if self.plot_paused {
                                if ui.button("▶").on_hover_text("继续绘图").clicked() {
                                    self.plot_paused = false;
                                    self.plot_snapshot_vals.clear();
                                    self.plot_snapshot_dif.clear();
                                    self.plot_epoch = self.plot_epoch.wrapping_add(1);
                                }
                            } else if ui
                                .button("⏸")
                                .on_hover_text("暂停绘图（数据照常解析，不断开连接）")
                                .clicked()
                            {
                                self.plot_paused = true;
                                self.plot_snapshot_vals = self.engine.channel_values.clone();
                                self.plot_snapshot_dif = self.engine.channel_diff_values.clone();
                                self.plot_snapshot_base = self.engine.frame_count;
                                self.plot_epoch = self.plot_epoch.wrapping_add(1);
                            }
                            // 坐标提示在按钮后面（对齐 Python matplotlib 工具栏 (x, y) 位置）
                            if let Some(c) = hover_coord {
                                ui.monospace(format!("(x, y) = ({:.4}, {:.4})", c[0], c[1]));
                            }
                        });
                    });
                });
            });
        });

        // 帮助 / 命令发送 / 连接失败弹窗 / F12 样式面板
        self.help_window(ctx);
        self.history_window(ctx);
        self.err_dialog_window(ctx);
        self.style
            .debug_panel(ctx, &mut self.show_style_panel, &mut self.style_dirty);
        self.style.apply_text_styles(ctx);

        // 窗口标题栏颜色随主题（Win11 DWM；Win10 无此 API 时静默跳过）
        let tb = self.style.theme().panel;
        let tb_color = egui::Color32::from_hex(tb).unwrap_or(egui::Color32::GRAY);
        if self.last_titlebar != Some(tb_color) {
            set_titlebar_color(tb_color);
            self.last_titlebar = Some(tb_color);
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn set_titlebar_color(color: egui::Color32) {
    // COLORREF = 0x00BBGGRR
    let cref: u32 =
        ((color.b() as u32) << 16) | ((color.g() as u32) << 8) | (color.r() as u32);
    extern "system" {
        fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> isize;
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attr: u32,
            pv: *const core::ffi::c_void,
            cb: u32,
        ) -> i32;
    }
    let title: Vec<u16> = "Binary Frame Analyzer by gykjwrq@163.com"
        .encode_utf16()
        .chain([0])
        .collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd == 0 {
        return;
    }
    const DWMWA_CAPTION_COLOR: u32 = 35;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &cref as *const u32 as _, 4);
    }
}

#[cfg(not(windows))]
fn set_titlebar_color(_color: egui::Color32) {}

impl eframe::App for FrameApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    /// 退出前保存 .config.json（帧配置/通道配置/语言/主题/连接参数/命令列表）。
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }
}

/// .config.json 路径（exe 同目录，兜底当前目录）。
fn config_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(".config.json")))
        .unwrap_or_else(|| std::path::PathBuf::from(".config.json"))
}

/// 循环码位宽持久化字符串 → u8（只允许 8/16/24/32，缺字段/非法值回退 8）。
/// 旧配置的循环码位置按 0 起算存；新版界面改成 1 起算，缺标记时自动 +1。
/// 有标记（"1"）或位置留空时原样返回；非数字不瞎改。
fn migrate_cycle_pos(base: &str, pos: String) -> String {
    if base == "1" || pos.trim().is_empty() {
        return pos;
    }
    match pos.trim().parse::<i64>() {
        Ok(v) => (v + 1).to_string(),
        Err(_) => pos,
    }
}

fn parse_cycle_width(s: &str) -> u8 {
    match s.trim().parse::<u8>() {
        Ok(b) if matches!(b, 8 | 16 | 24 | 32) => b,
        _ => 8,
    }
}

/// 十六进制色快捷解析（非法值回退）
fn hex_or(s: &str, fallback: egui::Color32) -> egui::Color32 {
    egui::Color32::from_hex(s).unwrap_or(fallback)
}

/// 从端口显示名提取短名：`COM12 (USB: ...)` → `COM12`；纯 `COM7` 原样返回。
fn port_short_name(label: &str) -> &str {
    // 显示名格式 `COM7 #NPort Communication Port 1 (COM7)` → 取 " #" 前短名
    label.split(" #").next().unwrap_or(label)
}

/// 定宽字段标签：宽度固定、文字左对齐，使名字左边缘成一条线。
fn field_label(ui: &mut egui::Ui, text: &str, w: f32, color: egui::Color32) {
    ui.allocate_ui(egui::vec2(w, 22.0), |ui| {
        // 左对齐：名字左边缘成一条线（英文标签长短差异大，右对齐会显得参差）
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(text).color(color));
        });
    });
}

/// 量出一组标签里最宽的那个，作为该组的统一标签宽度（对齐冒号又不留多余空白）。
fn labels_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    labels
        .iter()
        .map(|s| {
            ui.fonts(|f| {
                f.layout_no_wrap(
                    (*s).to_owned(),
                    font.clone(),
                    egui::Color32::WHITE,
                )
            })
            .size()
            .x
        })
        .fold(0.0_f32, f32::max)
        .ceil()
}

/// 分组卡片（对齐 Python LabelFrame：panel 底色 + border 边框 + 圆角，铺满可用宽度）
fn card<R>(ui: &mut egui::Ui, margin: egui::Margin, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(ui.visuals().window_fill)
        .stroke(ui.visuals().window_stroke())
        .rounding(4.0)
        .inner_margin(margin)
        .show(ui, |ui| {
            // Frame 默认收缩到内容宽，强制铺满所在面板宽度
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// 无窗口跑若干帧（模拟真实渲染），用于验证"左栏宽度不受语言切换影响"。
    fn run_frames(app: &mut FrameApp, ctx: &egui::Context, n: usize) {
        for _ in 0..n {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| app.ui(ctx));
        }
    }

    #[test]
    fn left_panel_width_stable_udp_mode() {
        let ctx = egui::Context::default();
        let mut app = FrameApp::new();
        app.style.left_panel_width = 300.0;
        app.link_kind = LinkKind::Udp;
        app.lang = Lang::Zh;
        run_frames(&mut app, &ctx, 4);
        let zh = app.last_panel_width;
        app.lang = Lang::En;
        run_frames(&mut app, &ctx, 4);
        let en = app.last_panel_width;
        assert!((zh - en).abs() < 0.5, "UDP 模式切英文左栏变宽: zh={zh} en={en}");
    }

    #[test]
    fn left_panel_width_stable_across_language() {
        let ctx = egui::Context::default();
        let mut app = FrameApp::new();
        app.style.left_panel_width = 300.0;
        app.lang = Lang::Zh;
        run_frames(&mut app, &ctx, 4);
        let zh = app.last_panel_width;
        app.lang = Lang::En;
        run_frames(&mut app, &ctx, 4);
        let en = app.last_panel_width;
        app.lang = Lang::Zh;
        run_frames(&mut app, &ctx, 4);
        let back = app.last_panel_width;
        assert!(
            (zh - en).abs() < 0.5,
            "切到英文后左栏宽度变了: zh={zh} en={en}"
        );
        assert!(
            (zh - back).abs() < 0.5,
            "切回中文后左栏宽度没恢复: zh={zh} back={back}"
        );
    }

    /// 回归：用户反馈"选 7 通道绘图只显示 6 个图示、7/8 路不出来"。
    /// 通道数 N 应用后，series 必须有 N 条带名字的原始线（图例条目一一对应）。
    #[test]
    fn seven_and_eight_channels_produce_all_series() {
        for n in [7usize, 8] {
            let mut app = FrameApp::new();
            app.channel_count_text = n.to_string();
            app.apply_channel_config();
            assert_eq!(app.channels.len(), n, "通道 UI 行数应为 {n}");
            assert_eq!(app.engine.channel_values.len(), n, "引擎通道数应为 {n}");
            // 塞一点数据使线非空
            for v in app.engine.channel_values.iter_mut() {
                v.push(1);
            }
            let series = app.build_plot_series();
            let named = series
                .iter()
                .filter(|(nm, _, _, _)| nm.is_some())
                .count();
            assert_eq!(
                series.len(),
                n,
                "通道数 {n} 时 series 应有 {n} 条原始线"
            );
            assert_eq!(named, n, "通道数 {n} 时图例项（带名字的线）应为 {n}");
        }
    }

    /// 回归：Dif 勾选 = 原始线 + 差分虚线叠加显示（Python 语义），
    /// 原始线带名字进图例、差分线无名字不进图例。
    #[test]
    fn dif_checked_overlays_diff_line_without_legend() {
        let mut app = FrameApp::new();
        app.channel_count_text = "2".into();
        app.apply_channel_config();
        app.channels[0].dif = true;
        app.channels[1].dif = true;
        for v in app.engine.channel_values.iter_mut() {
            v.push(1);
        }
        for v in app.engine.channel_diff_values.iter_mut() {
            v.push(-1);
        }
        let series = app.build_plot_series();
        assert_eq!(series.len(), 4, "2 通道全勾 Dif 应有 2 原始 + 2 差分 = 4 条");
        assert_eq!(
            series.iter().filter(|(nm, _, _, _)| nm.is_some()).count(),
            2,
            "图例项应为 2（只有原始线带名字）"
        );
        assert_eq!(
            series.iter().filter(|(nm, _, _, _)| nm.is_none()).count(),
            2,
            "差分虚线应为 2（不带名字）"
        );
    }

    #[test]
    fn cycle_group_does_not_widen_left_panel() {
        // 8 通道 + 启用循环码（第 5 组出现）：中英切换仍不得把左栏顶宽。
        // 同时会跑到 frame_cfg_body 里 #[cfg(test)] 的「校验列 == 样本列」对齐断言。
        let ctx = egui::Context::default();
        let mut app = FrameApp::new();
        app.style.left_panel_width = 300.0;
        app.channel_count_text = "8".into();
        app.apply_channel_config();
        app.cycle_pos_text = "1".into();
        app.cycle_width_bits = 16;
        app.apply_frame_config();
        assert_eq!(app.engine.cycle_pos, Some(0));
        app.lang = Lang::Zh;
        run_frames(&mut app, &ctx, 4);
        let zh = app.last_panel_width;
        app.lang = Lang::En;
        run_frames(&mut app, &ctx, 4);
        let en = app.last_panel_width;
        assert!(
            (zh - en).abs() < 0.5,
            "启用循环码 + 8 通道时切英文左栏变宽: zh={zh} en={en}"
        );
    }

    #[test]
    fn cycle_out_of_range_position_reports_error() {
        // 位置 + 字节数 > 帧长：应用帧配置必须明确报错，不 panic、不静默
        let mut app = FrameApp::new();
        app.length_text = "4".into();
        app.cycle_pos_text = "4".into();
        app.cycle_width_bits = 16; // 第 4 字节起 2 字节 → 内部 3+2=5 > 帧长 4
        app.apply_frame_config();
        assert!(
            app.frame_cell_text().contains("超出帧长"),
            "越界应有明确错误提示，实际: {:?}",
            app.frame_cell_text()
        );
        // 位置留空 = 关闭
        app.cycle_pos_text = String::new();
        app.apply_frame_config();
        assert_eq!(app.engine.cycle_pos, None);
    }

    /// 回归：绘图暂停时显示快照，引擎继续解析不受影响。
    #[test]
    fn paused_plot_uses_snapshot() {
        let mut app = FrameApp::new();
        app.channel_count_text = "1".into();
        app.apply_channel_config();
        app.engine.channel_values[0].push(42);
        // 暂停：记录快照
        app.plot_paused = true;
        app.plot_snapshot_vals = app.engine.channel_values.clone();
        app.plot_snapshot_dif = app.engine.channel_diff_values.clone();
        // 引擎继续收新数据（模拟不断流）
        app.engine.channel_values[0].push(99);
        let series = app.build_plot_series();
        match &series[0].1[..] {
            [v] => assert_eq!(*v, 42.0, "暂停时绘图应取快照值 42，而不是新值 99"),
            other => panic!("快照应只有 1 个点，实际 {:?}", other),
        }
    }
    #[test]
    fn cycle_position_is_one_based_and_zero_rejected() {
        let mut app = FrameApp::new();
        app.length_text = "4".into();
        app.cycle_pos_text = "1".into();
        app.cycle_width_bits = 8;
        app.apply_frame_config();
        assert_eq!(app.engine.cycle_pos, Some(0), "界面第 1 字节应转成引擎内部 0");
        assert_eq!(app.frame_cell_text(), tr(app.lang, "frame_applied"));
        app.cycle_pos_text = "0".into();
        app.apply_frame_config();
        assert!(app.frame_cell_text().contains("从 1 起算"), "填 0 应明确报错: {:?}", app.frame_cell_text());
    }

    #[test]
    fn old_config_cycle_pos_migrates_to_one_based() {
        assert_eq!(migrate_cycle_pos("", "4".into()), "5");
        assert_eq!(migrate_cycle_pos("1", "4".into()), "4");
        assert_eq!(migrate_cycle_pos("", String::new()), "");
        assert_eq!(migrate_cycle_pos("", "abc".into()), "abc");
    }

    #[test]
    fn status_bar_cells_order_and_always_zero() {
        let mut app = FrameApp::new();
        let c = app.status_cells();
        // 11 格：连接/帧配置/通道配置/通道信息/校验同步/校验错/丢帧/帧数/跳变/最近操作/TX
        assert_eq!(c.len(), 11);
        assert_eq!(c[0], tr(app.lang, "not_connected"));
        // 开机时两格不该出现「已应用」回执：启动只是悄悄装配配置，
        // 不算用户点过「应用」。用「不是回执状态」断言，避免依赖磁盘上
        // 是否有残留配置（配置损坏时开机显示错误提示是合理的，也允许）。
        assert!(
            !matches!(app.frame_msg, ConfigReceipt::FrameApplied),
            "开机不该显示「帧配置已应用」: {:?}",
            app.frame_msg
        );
        assert!(
            !matches!(app.channel_msg, ConfigReceipt::ChannelApplied(_)),
            "开机不该显示「通道配置已应用」: {:?}",
            app.channel_msg
        );
        // 配置正常时，两格留白显示「—」
        if app.frame_msg == ConfigReceipt::None {
            assert_eq!(c[1], "—");
        }
        if app.channel_msg == ConfigReceipt::None {
            assert_eq!(c[2], "—");
        }
        assert!(c[4].is_empty(), "未开校验时校验状态留白");
        assert!(c[5].ends_with('0'), "校验错常驻 0: {:?}", c[5]);
        assert!(c[6].ends_with('0'), "丢帧常驻 0: {:?}", c[6]);
        assert!(c[7].contains('0'), "帧数常驻: {:?}", c[7]);
        // 第 9 格（索引 8）= 跳变 N，0 也常驻显示
        assert!(
            c[8].contains(&tr(app.lang, "cycle_jumps_short")) && c[8].ends_with('0'),
            "跳变常驻 0: {:?}",
            c[8]
        );
        // 第 10 格（索引 9）= 最近操作，空时显示 —
        assert_eq!(c[9], "—", "最近操作空时显示 —");
        // 第 11 格（索引 10）= TX: 0
        assert_eq!(c[10], "TX: 0");
        app.engine.checksum_mode = ChecksumMode::Sum8;
        app.engine.sync_locked = false;
        assert_eq!(app.status_cells()[4], tr(app.lang, "chk_sync"));
        app.engine.sync_locked = true;
        assert_eq!(app.status_cells()[4], tr(app.lang, "chk_synced"));
    }

    #[test]
    fn status_hints_are_independent() {
        let mut app = FrameApp::new();
        app.frame_msg = ConfigReceipt::FrameApplied;
        app.channel_msg = ConfigReceipt::ChannelApplied(4);
        app.link_msg = "未发现串口设备".into();
        app.status_msg = "主题: 深色".into();
        let c = app.status_cells();
        assert_eq!(c[0], "未发现串口设备");
        assert_eq!(c[1], tr(app.lang, "frame_applied"));
        assert_eq!(c[2], format!("{} (4)", tr(app.lang, "channel_applied")));
        // 最近操作在第 10 格（索引 9）
        assert_eq!(c[9], "主题: 深色");
        app.status_msg.clear();
        let c2 = app.status_cells();
        assert_eq!(c2[1], tr(app.lang, "frame_applied"), "清最近操作不应顶掉帧配置回执");
        assert_eq!(c2[2], format!("{} (4)", tr(app.lang, "channel_applied")));
    }

    /// 回归：开机不该冒充用户点过「应用」而显示「帧配置已应用」。
    /// 启动只装配引擎不留回执；只有真正点「应用帧配置」后才出现，
    /// 且出现后仍要跟随中英切换（见 config_cells_follow_language_switch）。
    #[test]
    fn startup_has_no_config_receipt_but_button_click_does() {
        // 开机：绝不显示「已应用」回执。这里不要求「完全空白」——
        // 磁盘上若有残留的错误配置，开机显示错误提示是合理且必须保留的。
        // 测试进程共用一份 .config.json，故断言只看「不是已应用」。
        let mut app = FrameApp::new();
        assert!(
            !matches!(app.frame_msg, ConfigReceipt::FrameApplied),
            "开机不该显示「帧配置已应用」: {:?}",
            app.frame_msg
        );
        assert!(
            !matches!(app.channel_msg, ConfigReceipt::ChannelApplied(_)),
            "开机不该显示「通道配置已应用」: {:?}",
            app.channel_msg
        );
        // 无回执时渲染成留白「—」（纯渲染口径，不依赖磁盘状态）
        assert_eq!(app.config_receipt_text(&ConfigReceipt::None), "—");

        // 复位成合法配置，用户真正点「应用帧配置」后：回执才出现
        app.length_text = "11".into();
        app.cycle_pos_text = String::new();
        app.cycle_width_bits = 8;
        app.apply_frame_config();
        assert_eq!(app.frame_msg, ConfigReceipt::FrameApplied);
        assert_eq!(app.frame_cell_text(), tr(Lang::Zh, "frame_applied"));
        // 应用通道配置后：通道格才出现
        app.apply_channel_config();
        let n = app.engine.channel_count;
        assert_eq!(app.channel_msg, ConfigReceipt::ChannelApplied(n));
        assert_eq!(
            app.channel_cell_text(),
            format!("{} ({})", tr(Lang::Zh, "channel_applied"), n)
        );

        // 出现后切英文仍要跟随
        app.lang = Lang::En;
        assert_eq!(app.frame_cell_text(), tr(Lang::En, "frame_applied"));
        assert_eq!(
            app.channel_cell_text(),
            format!("{} ({})", tr(Lang::En, "channel_applied"), n)
        );
    }

    /// 回归：状态栏「帧配置 / 通道配置」两格必须跟随中英切换。
    /// 旧实现把「已应用」这句话在点击时按当时语言存成固定字符串，
    /// 切语言后不重算 → 一直是中文。现在存状态、显示时才翻译。
    #[test]
    fn config_cells_follow_language_switch() {
        let mut app = FrameApp::new();
        // 复位成合法配置：测试进程共用一份 .config.json，别的用例可能留下
        // 越界/填 0 等坏配置，会让「应用」失败而显示错误提示。
        app.length_text = "11".into();
        app.cycle_pos_text = String::new();
        app.cycle_width_bits = 8;
        app.apply_frame_config();
        app.apply_channel_config();
        let n = app.engine.channel_count; // 默认通道数由配置决定，不写死

        // 中文态
        app.lang = Lang::Zh;
        let zh = app.status_cells();
        assert_eq!(zh[1], tr(Lang::Zh, "frame_applied"));
        assert_eq!(zh[2], format!("{} ({})", tr(Lang::Zh, "channel_applied"), n));

        // 切到英文：两格立即变英文，且不需要重新点「应用」
        app.lang = Lang::En;
        let en = app.status_cells();
        assert_eq!(en[1], tr(Lang::En, "frame_applied"));
        assert_eq!(en[2], format!("{} ({})", tr(Lang::En, "channel_applied"), n));
        assert_ne!(en[1], zh[1], "帧配置格应随语言变化");
        assert_ne!(en[2], zh[2], "通道配置格应随语言变化");

        // 切回中文：还原
        app.lang = Lang::Zh;
        let back = app.status_cells();
        assert_eq!(back[1], zh[1]);
        assert_eq!(back[2], zh[2]);

        // 错误回执的前缀也跟随语言
        app.frame_msg = ConfigReceipt::Error("帧长不能为 0".into());
        assert!(app.frame_cell_text().starts_with(tr(Lang::Zh, "config_error")));
        app.lang = Lang::En;
        assert!(app.frame_cell_text().starts_with(tr(Lang::En, "config_error")));
    }

    /// 记录指示灯三态：仅「已连接 && 勾选保存 && 已选文件」时亮，其余不亮。
    #[test]
    fn record_light_three_conditions() {
        let mut app = FrameApp::new();
        app.csv_path = Some(std::path::PathBuf::from("a.csv"));
        app.txt_path = Some(std::path::PathBuf::from("b.txt"));
        // 未连接 → 不亮
        app.connected = false;
        app.extracted_enabled = true;
        app.raw_enabled = true;
        assert!(!app.recording_any(), "未连接不应写入");
        // 已连接但都没勾选 → 不亮
        app.connected = true;
        app.extracted_enabled = false;
        app.raw_enabled = false;
        assert!(!app.recording_any(), "都没勾选不应写入");
        // 已连接 + 勾选 CSV + 有文件 → 亮
        app.extracted_enabled = true;
        assert!(app.csv_recording() && app.recording_any(), "CSV 应写入");
        // 已连接 + 勾选 TXT + 有文件 → 亮
        app.extracted_enabled = false;
        app.raw_enabled = true;
        assert!(app.txt_recording() && app.recording_any(), "TXT 应写入");
        // 已连接 + 勾选但没选文件 → 不亮
        app.csv_path = None;
        app.extracted_enabled = true;
        assert!(!app.csv_recording(), "没选文件不应写入");
    }

    /// 数据记录组：勾选框标签随状态替换；后面路径/文件名始终显示（旧功能）。
    #[test]
    fn data_record_state_machine() {
        let app = FrameApp::new();
        let base = tr(app.lang, "save_extracted");
        let path = Some(std::path::PathBuf::from("/tmp/rec.csv"));
        // 后面路径/文件名：任何状态都显示文件名 + 悬停全路径，默认色（旧功能）
        let (t, c, h) = app.record_follow(&path);
        assert_eq!(t, "rec.csv");
        assert!(c.is_none(), "路径标签应默认色");
        assert_eq!(h, "/tmp/rec.csv");

        // 未勾选 → 显示原标签（提取数据保存），不替换成状态字
        assert_eq!(
            FrameApp::record_box_label(&false, base, false, app.lang),
            base
        );

        // 勾选 + 未在写（未连接 / 连上但没选文件）→ 待连接（灰）
        assert_eq!(
            FrameApp::record_box_label(&true, base, false, app.lang),
            tr(app.lang, "rec_wait_conn")
        );

        // 勾选 + 已在写（已连 + 已选文件）→ 数据保存中（蓝，由 record_checkbox 上色）
        assert_eq!(
            FrameApp::record_box_label(&true, base, true, app.lang),
            tr(app.lang, "rec_saving")
        );
    }

    /// 循环码计数框：仅在有循环位置时显示；连接时长格式 HH:MM:SS，未连接为 —。
    #[test]
    fn cycle_stats_visible_and_duration_format() {
        let mut app = FrameApp::new();
        // 无循环位置 → 不显示
        app.cycle_pos_text = String::new();
        assert!(!app.cycle_stats_visible());
        // 有循环位置 → 显示
        app.cycle_pos_text = "1".into();
        assert!(app.cycle_stats_visible());
        // 未连接 → 时长 —
        app.connected_at = None;
        assert_eq!(app.connection_duration_text(), "—");
        // 已连接 3h2m1s → 03:02:01
        app.connected_at = Some(
            std::time::Instant::now() - std::time::Duration::from_secs(3 * 3600 + 2 * 60 + 1),
        );
        assert_eq!(app.connection_duration_text(), "03:02:01");
        // 进位：1h0m5s → 01:00:05
        app.connected_at = Some(
            std::time::Instant::now() - std::time::Duration::from_secs(3600 + 5),
        );
        assert_eq!(app.connection_duration_text(), "01:00:05");
    }

    #[test]
    fn channel_info_bits_and_sign() {
        let mut app = FrameApp::new();
        app.channel_count_text = "2".into();
        app.apply_channel_config();
        for c in app.channels.iter_mut() {
            c.idx_text = "2,3".into();
            c.sig_text = "15".into();
        }
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "16位 有符号");
        app.channels[1].sig_text = "-1".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "16位 混合");
        app.channels[0].idx_text = "2".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "8/16位 混合");
        app.channels[0].idx_text = "2,3".into();
        app.channels[0].sig_text = "-1".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "16位 无符号");
    }

}
