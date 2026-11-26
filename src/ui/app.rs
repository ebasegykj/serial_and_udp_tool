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
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
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
/// 验算结果标题字号：「发现 N 处异常」与「未发现丢帧」共用，保证醒目程度一致。
const VERIFY_HEADING_SIZE: f32 = 14.0;
/// 左栏三个离线按钮字号（中英文均需放得下，宽度回归测试会逐一校验）。
const OFFLINE_BTN_FONT_SIZE: f32 = 11.0;
/// 验算绘图区最小边长（正方形）：截图「异常帧：第 3943 帧（图中红线）」红字约 287px，
/// 两个文字长度约 574px，取 580 留余量。窗口 min_size 保证宽度同样不小于它。
const VERIFY_PLOT_MIN_SIZE: f32 = 580.0;
/// 右栏顶部「曲线说明 + 红字异常帧号」一行文字及分隔的预留高度。
/// 两行并一行后由 48 降为 28，让图往上贴；绘图高度仍由整窗高度减去它得到，
/// 弹窗上下拖动时图高严格跟随，且图一直填到右栏底部。
const VERIFY_TOP_OVERHEAD: f32 = 28.0;
/// 验算弹窗分隔线默认位置：等于可拖动下限 VERIFY_SPLIT_MIN（用户实测：拖到最小时
/// 分隔线紧贴清单，才是对的对齐效果，故默认即该值）。打开后仍可左右拖动并持久化；
/// 主界面 style.left_panel_width 是独立常量，不受影响。
const VERIFY_SPLIT_DEFAULT: f32 = 160.0;
/// 验算弹窗最小/默认尺寸。min 由「左栏 300 + 分隔 6 + 绘图 580 + 内边距 34 = 920」，
/// 以及「顶部 48 + 绘图 580 + 标题栏/内边距 32 = 660」推出；default 必须 >= min。
const VERIFY_WINDOW_MIN_W: f32 = 920.0;
const VERIFY_WINDOW_MIN_H: f32 = 660.0;
const VERIFY_WINDOW_DEFAULT_W: f32 = 960.0;
const VERIFY_WINDOW_DEFAULT_H: f32 = 720.0;
/// 验算弹窗透明度百分比范围：100=完全不透明（与加 fill 前观感一致），20=最透
/// （再低整窗几乎消失，会被误认为坏了）；默认 80 一打开就是轻微透。
const VERIFY_OPACITY_MIN: u8 = 20;
const VERIFY_OPACITY_MAX: u8 = 100;
const VERIFY_OPACITY_DEFAULT: u8 = 80;
/// 验算弹窗可拖动分隔线：左栏最小 160，右栏绘图区最小 260；默认左栏也取最小值 160
/// （用户实测：拖到最小时分隔线紧贴清单才是对的对齐效果，故默认即该值）。
const VERIFY_SPLIT_MIN: f32 = 160.0;
const VERIFY_SPLIT_RIGHT_MIN: f32 = 260.0;
/// 验算快照抓取上限：前后各最多 500 帧（用户可在弹窗内调小到 1）。
/// 环形缓冲按 2*MAX+1 帧保留，快照内存上限 200 条 × 1001 帧 × 通道数。
const VERIFY_SNAP_MAX: usize = 500;
/// 状态栏空闲轮播：每 3 秒换一条，按 elapsed/3 取模循环；最近操作格宽度上限 260（超长截断+悬停全文）。
const STATUS_TIP_SECS: u64 = 3;
const RECENT_MAX_W: f32 = 260.0;
/// 轮播词条 key，顺序即轮播顺序（16 条，中英各一套在 i18n）。
const STATUS_TIP_KEYS: &[&str] = &[
    "tip_wheel_pan", "tip_wheel_zoom", "tip_reset_view", "tip_paused", "tip_clear",
    "tip_log_append", "tip_no_header", "tip_hex_source", "tip_no_cycle", "tip_verify_config",
    "tip_config_save", "tip_theme", "tip_samples", "tip_sign", "tip_loopback", "tip_verify",
];
/// 纯函数：经过秒数 → 轮播下标。测试可直接喂时间验证切换与回绕。
fn status_tip_index(elapsed_secs: u64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((elapsed_secs / STATUS_TIP_SECS) as usize) % n
}

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
    #[serde(default = "default_verify_split_w")]
    verify_split_w: f32,
}

/// 老配置缺 verify_split_w 时回退到 160（与拖动下限一致）。
fn default_verify_split_w() -> f32 {
    VERIFY_SPLIT_DEFAULT
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

/// 数据验算：一条循环码异常记录。
/// `snapshot` = 异常点前后各最多 VERIFY_SNAP_MAX 帧的通道值（按通道分组，每列最多 1001 点）；
/// UI 按「前/后 N 帧」截取显示；超过 200 条时只记清单不存快照（避免内存爆炸）。
#[derive(Clone, Debug)]
pub struct VerifyAnomaly {
    pub frame_no: u64,
    pub expected: u64,
    pub actual: u64,
    pub lost: u64,
    pub snapshot: Option<Vec<Vec<i64>>>,
    /// 异常点在 snapshot 各列中的下标（用来把它移到图中心 x=0）。
    /// 数据流开头/结尾快照不足 101 帧时该值 <50 或 >50，仍保证居中。
    pub snap_center: usize,
}

/// 数据验算弹窗状态（None = 无弹窗）。
#[derive(Clone, Debug)]
pub struct VerifyState {
    pub running: bool,
    pub read: u64,
    pub total: u64,
    pub anomalies: Vec<VerifyAnomaly>,
    pub selected: Option<usize>,
    /// 配置错误 / 读取失败提示（非空时优先显示）。
    pub msg: String,
    /// 弹窗内「前 N 帧 / 后 N 帧」输入（字符串便于编辑，取值时 clamp 到 1..=500）。
    pub before_text: String,
    pub after_text: String,
    /// 数据含非法/不完整字符：为真时不得显示「未发现丢帧」，改显示非法字符提示。
    pub has_invalid_char: bool,
    /// 下一帧强制把快照图坐标重置到数据真实范围。随弹窗状态一起存取，
    /// 避免放在 FrameApp 上被其它地方提前消费/丢失（换条目、改帧数、验算完成置位）。
    pub reset_bounds: bool,
    /// 弹窗底板透明度百分比 20..=100（100 不透明，默认 80）。滑块拖动即时生效。
    pub opacity_pct: u8,
}

/// 验算后台线程 → UI 的消息。
pub enum VerifyMsg {
    Progress { read: u64, total: u64 },
    Done { anomalies: Vec<VerifyAnomaly>, has_invalid_char: bool },
    Error(String),
}

/// 验算核心：流式喂 hex 文本字节 → 分帧 → 调用 Engine::process_buffer
/// （内部即 cycle_process_frame，保证与实时判定完全一致）→ 记录异常与前后快照。
/// 内存恒定：只保留最近 2*VERIFY_SNAP_MAX+1 帧的通道值环形缓冲，不保留全量帧/通道数据。
/// 快照按最大窗口抓取，UI 再按用户设置的「前/后 N 帧」截取，无需重跑验算。
pub struct VerifyRunner {
    /// 验算专用引擎（独立副本，不碰主绘图区）。
    engine: Engine,
    /// 未成帧的字节缓冲。
    byte_buf: Vec<u8>,
    /// 跨块边界的半个十六进制字符。
    hex_carry: Option<char>,
    /// UTF-8 BOM（EF BB BF）在文件开头的匹配进度（0/1/2 = 匹配中，3 = 已结束）。
    bom_state: u8,
    /// 发现非 hex、非空白（空格/制表/换行/回车/BOM）字符，或末尾出现落单的半字节。
    /// 为真时结果不可信，UI 不得显示「未发现丢帧」。
    pub has_invalid_char: bool,
    /// 最近 2*VERIFY_SNAP_MAX+1 帧：(帧号, 各通道值)。
    ring: VecDeque<(u64, Vec<i64>)>,
    /// 异常清单。
    pub anomalies: Vec<VerifyAnomaly>,
    /// 待补「后 MAX 帧」快照的异常下标。
    pending_snap: Vec<usize>,
    /// 最多存多少条快照。
    max_snapshots: usize,
}

impl VerifyRunner {
    pub fn new(mut engine: Engine, max_snapshots: usize) -> Self {
        // 验算路径不做通道全量提取/CSV 保存，关掉避免大文件内存增长。
        engine.extracted_save_enabled = false;
        // 无帧头时本 helper 已按帧长从字节 0 定长切帧（视为起点对齐），
        // 直接锁定同步，避免 process_buffer 为「连续两帧校验」再等下一帧。
        if engine.frame_header.is_empty() {
            engine.sync_locked = true;
        }
        Self {
            engine,
            byte_buf: Vec::new(),
            hex_carry: None,
            bom_state: 0,
            has_invalid_char: false,
            ring: VecDeque::with_capacity(2 * VERIFY_SNAP_MAX + 1),
            anomalies: Vec::new(),
            pending_snap: Vec::new(),
            max_snapshots,
        }
    }

    /// 从环形缓冲里取 [frame-MAX, frame+MAX] 的通道值，转置成「按通道分组」。
    /// 按最大窗口抓取，UI 再按用户设置的「前/后 N 帧」截取；异常帧下标一并返回。
    fn snapshot_at(&self, frame: u64) -> Option<(Vec<Vec<i64>>, usize)> {
        let lo = frame.saturating_sub(VERIFY_SNAP_MAX as u64);
        let hi = frame.saturating_add(VERIFY_SNAP_MAX as u64);
        let frames: Vec<&(u64, Vec<i64>)> = self
            .ring
            .iter()
            .filter(|(f, _)| *f >= lo && *f <= hi)
            .collect();
        if frames.is_empty() {
            return None;
        }
        // 异常帧若已不在环形缓冲里（极端截断），退化为取中点，仍不会 panic。
        let center = frames
            .iter()
            .position(|(f, _)| *f == frame)
            .unwrap_or_else(|| frames.len() / 2);
        let n = frames[0].1.len();
        let mut out = vec![Vec::with_capacity(frames.len()); n];
        for (_, v) in frames {
            for (i, &x) in v.iter().enumerate() {
                if i < n {
                    out[i].push(x);
                }
            }
        }
        Some((out, center))
    }

    /// 完成补快照：对已经等到「后 MAX 帧」的异常填 snapshot。
    fn finalize_snapshots(&mut self, current_frame: u64, force: bool) {
        let mut done = Vec::new();
        for (k, &idx) in self.pending_snap.iter().enumerate() {
            let f = self.anomalies[idx].frame_no;
            if force || current_frame >= f.saturating_add(VERIFY_SNAP_MAX as u64) {
                if let Some((snap, center)) = self.snapshot_at(f) {
                    self.anomalies[idx].snapshot = Some(snap);
                    self.anomalies[idx].snap_center = center;
                }
                done.push(k);
            }
        }
        for k in done.into_iter().rev() {
            self.pending_snap.remove(k);
        }
    }

    /// 喂入一块原始文本字节（可能把两字符的十六进制字节拆开，靠 hex_carry 衔接）。
    /// 只把 ASCII 空白与文件开头 BOM 当合法分隔；其余非 hex 字节一律记为非法，
    /// 不再静默跳过（否则乱码里的合法 hex 会被拼成帧，得出误导性的「未发现丢帧」）。
    pub fn feed(&mut self, chunk: &[u8]) {
        // 1) 十六进制字符流：跨块凑成对；遇非法字符清掉未配对的半字节，避免跨非法字符拼字节。
        const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
        for &b in chunk {
            // 文件开头允许一个 UTF-8 BOM，按字节匹配（跨块安全）；中途出现的 BOM 视为非法。
            if self.bom_state < 3 {
                if b == BOM[self.bom_state as usize] {
                    self.bom_state += 1;
                    continue;
                }
                if self.bom_state > 0 {
                    // 之前的部分匹配不是 BOM：那几个字节本身非 hex，记为非法。
                    self.has_invalid_char = true;
                }
                self.bom_state = 3;
            }
            if b.is_ascii_hexdigit() {
                let c = b as char;
                if let Some(first) = self.hex_carry.take() {
                    let s = format!("{}{}", first, c);
                    if let Ok(v) = u8::from_str_radix(&s, 16) {
                        self.byte_buf.push(v);
                    }
                } else {
                    self.hex_carry = Some(c);
                }
            } else if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C) {
                // 合法分隔符，保持 hex_carry（与原行为一致）。
            } else {
                // 乱码：如 fadfdsa / sa / "0x" 前缀的 x（现有格式是纯 hex 空格分隔）。
                self.has_invalid_char = true;
                self.hex_carry = None;
            }
        }
        // 2) 分帧：逐帧喂给独立引擎的 process_buffer（内部必调 cycle_process_frame）。
        let header = self.engine.frame_header.clone();
        let frame_len = self.engine.frame_length.max(1);
        loop {
            let found = if header.is_empty() {
                Some(0)
            } else {
                self.byte_buf
                    .windows(header.len())
                    .position(|w| w == header.as_slice())
            };
            match found {
                None => {
                    // 与原版一致：无帧头时保留末尾 header_len-1 字节等待下一块。
                    if header.len() > 1 && self.byte_buf.len() > header.len() - 1 {
                        let keep = header.len() - 1;
                        let len = self.byte_buf.len();
                        self.byte_buf.drain(..len - keep);
                    } else if header.len() == 1 {
                        self.byte_buf.clear();
                    }
                    break;
                }
                Some(idx) => {
                    if idx > 0 {
                        self.byte_buf.drain(..idx);
                    }
                }
            }
            if self.byte_buf.len() < frame_len {
                break;
            }
            let frame: Vec<u8> = self.byte_buf[..frame_len].to_vec();
            self.byte_buf.drain(..frame_len);

            let jumps_before = self.engine.cycle_jumps;
            let lost_before = self.engine.cycle_lost;
            let lens_before: Vec<usize> = self.engine.channel_values.iter().map(|v| v.len()).collect();
            self.engine.rx_buffer.clear();
            self.engine.rx_buffer.extend_from_slice(&frame);
            self.engine.process_buffer();
            // 缓冲区不随文件增长。
            self.engine.frame_log.clear();

            let frame_no = self.engine.frame_count;
            let mut vals = Vec::with_capacity(lens_before.len());
            for (i, before) in lens_before.iter().enumerate() {
                let last = self
                    .engine
                    .channel_values
                    .get(i)
                    .and_then(|v| v.get(*before..))
                    .and_then(|s| s.last())
                    .copied()
                    .unwrap_or(0);
                vals.push(last);
                if let Some(v) = self.engine.channel_values.get_mut(i) {
                    v.clear();
                }
            }
            self.ring.push_back((frame_no, vals));
            while self.ring.len() > 2 * VERIFY_SNAP_MAX + 1 {
                self.ring.pop_front();
            }
            self.finalize_snapshots(frame_no, false);

            if self.engine.cycle_jumps > jumps_before {
                let (expected, actual) = self.engine.cycle_last_jump.unwrap_or((0, 0));
                let store = self.anomalies.len() < self.max_snapshots;
                let idx = self.anomalies.len();
                self.anomalies.push(VerifyAnomaly {
                    frame_no,
                    expected,
                    actual,
                    lost: self.engine.cycle_lost.saturating_sub(lost_before),
                    snapshot: None,
                    snap_center: 0,
                });
                if store {
                    self.pending_snap.push(idx);
                }
            }
        }
    }

    /// 收尾：把还没等到后 MAX 帧的异常快照用现有环形缓冲补齐。
    /// 同时返回「数据是否含非法/不完整字符」供 UI 选择结论文案。
    pub fn finish_with_status(mut self) -> (Vec<VerifyAnomaly>, bool) {
        if self.hex_carry.is_some() {
            // 末尾落单的半字节（如单独一个 'f'）不是完整字节，数据不完整。
            self.has_invalid_char = true;
        }
        let invalid = self.has_invalid_char;
        let f = self.engine.frame_count;
        self.finalize_snapshots(f, true);
        (self.anomalies, invalid)
    }

    /// 仅取异常清单（测试与简单调用方用）。
    pub fn finish(self) -> Vec<VerifyAnomaly> {
        self.finish_with_status().0
    }
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

    // —— 回环测试（造帧循环发送，配合循环码丢帧统计） ——
    /// 是否启用回环测试：勾选后 ▶ 发「帧头+00填充+循环码」的造帧，不使用命令列表。
    pub loopback_enabled: bool,
    /// 回环帧末字节的循环码值（0..=255 回绕）。
    pub loopback_code: u8,

    // —— 状态栏（按功能区块分格，格内互斥、格间独立） ——
    /// 「最近操作」回执：离线分析 / 剪贴板 / 清零 / 主题 / 发送 / 命令等一次性提示。
    pub status_msg: String,
    /// status_msg 的设置时刻：一次性提示约 3 秒后自动消失。
    pub status_msg_at: Option<Instant>,
    /// 空闲轮播提示的起算时刻：无回执/非回环时每 3 秒换一条快捷提示。
    pub status_tip_epoch: Instant,
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
    /// 数据验算弹窗状态（None = 无弹窗）
    pub verify: Option<VerifyState>,
    /// 验算后台线程消息接收端
    pub verify_rx: Option<Receiver<VerifyMsg>>,
    /// 验算弹窗内快照图的缩放/复位计数（与主绘图区独立）
    pub verify_zoom: f32,
    pub verify_epoch: u64,
    /// 验算弹窗可拖动分隔线位置（左栏宽度），持久化到配置，默认 240。
    pub verify_split_w: f32,
    /// 上次同步到窗口标题栏的颜色（变化才调 DWM，避免每帧系统调用）
    pub last_titlebar: Option<egui::Color32>,
}

/// 文件对话框种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Csv,
    Txt,
    /// 数据验算：选原始数据 txt（打开对话框，不是保存）。
    Verify,
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
            loopback_enabled: false,
            loopback_code: 0,
            status_msg: String::new(),
            status_msg_at: None,
            status_tip_epoch: Instant::now(),
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
            verify: None,
            verify_rx: None,
            verify_zoom: 1.0,
            verify_epoch: 0,
            verify_split_w: VERIFY_SPLIT_DEFAULT,
            last_titlebar: None,
        };
        // 上次运行的 .config.json（语言/主题/帧配置/通道配置/连接参数/命令列表）
        app.load_config();
        // 启动时即按配置装配引擎（连接后直接收数）。
        // 走「静默」那条路：开机不该冒充用户点过「应用」而显示「帧格式已配置」。
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
    /// 只有引擎真的重建成功才显示「帧格式已配置」，否则保留 build_engine 给出的
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

    /// 🗑 清空绘图：重建引擎（同时清帧数/校验错/丢帧/跳变/波形）、清绘图快照，
    /// 并把 TX 计数一并归零。帧配置非法时 build_engine 失败、保留旧引擎，但快照与 TX
    /// 仍照旧清空（与原按钮行为一致：不论重建是否成功都清）。
    fn clear_plot(&mut self) {
        if let Ok(e) = self.build_engine() {
            self.engine = e;
        }
        // 快照一并清空：暂停时点清空也应是空图
        self.plot_snapshot_vals.clear();
        self.plot_snapshot_dif.clear();
        // 用户确认：TX 计数没有别的清零入口，随「清空绘图」一并归零。
        self.tx_count = 0;
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
    /// CSV 不是原始数据（里面没有循环码），识别到就给友好提示，绝不走 hex 解析报错。
    fn analyze_hex_string(&mut self, hex_str: &str) {
        if is_csv_text(hex_str) {
            self.set_status(tr(self.lang, "offline_csv_unsupported"));
            return;
        }
        // 配置错误（如循环码位置越界）时保留 frame_msg 里的明确提示，不静默用旧引擎继续
        if !self.apply_frame_config_silent() {
            return;
        }
        match parse_header_hex(hex_str) {
            Ok(data) => {
                if data.is_empty() {
                    self.set_status(tr(self.lang, "offline_empty"));
                    return;
                }
                self.engine.rx_buffer.extend_from_slice(&data);
                self.engine.process_buffer();
                self.trim_buffers();
                self.view_plot = true;
                self.set_status(fmt_nums(
                    tr(self.lang, "offline_done"),
                    &[self.engine.frame_count, self.engine.sample_count],
                ));
            }
            Err(_) => {
                // 只给固定短句，不把非法原文/底层细节塞进状态栏（曾撑爆 TX）。
                self.set_status(tr(self.lang, "offline_parse_err"));
            }
        }
    }

    fn load_file_offline(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Text/Hex", OFFLINE_TEXT_EXTS)
            .add_filter("All files", &["*"])
            .pick_file()
        {
            // 仍有 All files 入口：按扩展名再挡一次 CSV。
            let csv_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("csv"))
                .unwrap_or(false);
            if csv_ext {
                self.set_status(tr(self.lang, "offline_csv_unsupported"));
                return;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => self.analyze_hex_string(&content),
                Err(e) => self.set_status(fmt_str(tr(self.lang, "offline_read_err"), &e.to_string())),
            }
        }
    }

    fn analyze_clipboard(&mut self) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => {
                if text.trim().is_empty() {
                    self.set_status(tr(self.lang, "clipboard_empty"));
                } else {
                    self.analyze_hex_string(&text);
                }
            }
            Err(_) => self.set_status(tr(self.lang, "clipboard_unavailable")),
        }
    }

    // ============ 数据验算（原始数据 txt → 循环码丢帧定位） ============

    /// 起验算：用当前帧配置构造独立引擎，后台 BufReader 流式读（1MB 块），
    /// UI 只收进度消息，弹窗一直开着显示进度，不会像卡死。
    fn start_verify(&mut self, path: PathBuf) {
        // 没填循环码位置时引擎根本不判定，必须明确拒绝，不能显示「未发现丢帧」误导用户。
        if self.cycle_pos_text.trim().is_empty() {
            self.verify_rx = None;
            self.verify = Some(VerifyState {
                running: false,
                read: 0,
                total: 0,
                anomalies: Vec::new(),
                selected: None,
                msg: tr(self.lang, "verify_no_cycle").to_string(),
                before_text: "50".into(),
                after_text: "50".into(),
                has_invalid_char: false,
                reset_bounds: true,
                opacity_pct: VERIFY_OPACITY_DEFAULT,
            });
            self.verify_zoom = 1.0;
            self.verify_epoch = self.verify_epoch.wrapping_add(1);
            return;
        }
        let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let engine = match self.build_engine() {
            Ok(e) => e,
            Err(s) => {
                self.verify = Some(VerifyState {
                    running: false,
                    read: 0,
                    total,
                    anomalies: Vec::new(),
                    selected: None,
                    msg: format!("{}: {}", tr(self.lang, "config_error"), s),
                    before_text: "50".into(),
                    after_text: "50".into(),
                    has_invalid_char: false,
                    reset_bounds: true,
                    opacity_pct: VERIFY_OPACITY_DEFAULT,
                });
                return;
            }
        };
        self.verify = Some(VerifyState {
            running: true,
            read: 0,
            total,
            anomalies: Vec::new(),
            selected: None,
            msg: String::new(),
            before_text: "50".into(),
            after_text: "50".into(),
            has_invalid_char: false,
            reset_bounds: true,
            opacity_pct: VERIFY_OPACITY_DEFAULT,
        });
        self.verify_zoom = 1.0;
        self.verify_epoch = self.verify_epoch.wrapping_add(1);
        let (tx, rx) = mpsc::channel();
        self.verify_rx = Some(rx);
        std::thread::spawn(move || {
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.send(VerifyMsg::Error(e.to_string()));
                    return;
                }
            };
            let mut reader = BufReader::with_capacity(1 << 20, file);
            let mut runner = VerifyRunner::new(engine, 200);
            let mut buf = [0u8; 1 << 20];
            let mut read = 0u64;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        read += n as u64;
                        runner.feed(&buf[..n]);
                        let _ = tx.send(VerifyMsg::Progress { read, total });
                    }
                    Err(e) => {
                        let _ = tx.send(VerifyMsg::Error(e.to_string()));
                        return;
                    }
                }
            }
            let (anomalies, has_invalid_char) = runner.finish_with_status();
            let _ = tx.send(VerifyMsg::Done {
                anomalies,
                has_invalid_char,
            });
        });
    }

    /// 每帧收割验算进度 / 结果消息。
    fn poll_verify(&mut self) {
        if self.verify_rx.is_none() {
            return;
        }
        let mut msgs = Vec::new();
        if let Some(rx) = &self.verify_rx {
            let mut got_terminal = false;
            loop {
                match rx.try_recv() {
                    Ok(m) => {
                        if matches!(m, VerifyMsg::Done { .. } | VerifyMsg::Error(_)) {
                            got_terminal = true;
                        }
                        msgs.push(m);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !got_terminal {
                            msgs.push(VerifyMsg::Done {
                                anomalies: Vec::new(),
                                has_invalid_char: false,
                            });
                        }
                        break;
                    }
                }
            }
        }
        if msgs.is_empty() {
            return;
        }
        let mut close_rx = false;
        if let Some(st) = self.verify.as_mut() {
            for m in msgs {
                match m {
                    VerifyMsg::Progress { read, total } => {
                        st.read = read;
                        st.total = total;
                    }
                    VerifyMsg::Done { anomalies, has_invalid_char } => {
                        st.anomalies = anomalies;
                        st.has_invalid_char = has_invalid_char;
                        st.running = false;
                        // 验算完成 = 重置事件，标志随 state 一起保存，不再放在 FrameApp 上。
                        st.reset_bounds |= verify_reset_on_event(false, false, true);
                        close_rx = true;
                    }
                    VerifyMsg::Error(e) => {
                        st.msg = e;
                        st.running = false;
                        close_rx = true;
                    }
                }
            }
        }
        if close_rx {
            self.verify_rx = None;
        }
    }

    /// 验算弹窗：进行中显示进度 / msg 横跨整窗；完成后左栏异常清单（独立滚动）+ 右栏详情与快照图。
    fn verify_window(&mut self, ctx: &egui::Context) {
        if self.verify.is_none() {
            return;
        }
        let mut state = self.verify.take().unwrap();
        let mut open = true;
        // 底板颜色跟随当前主题（取 visuals.window_fill 的 rgb），只把 alpha 换成滑块值。
        // egui 0.29 的 Window 没有 .fill()，要用 .frame(Frame::window(...).fill(...))；
        // 100% 时 alpha=255、rgb 与默认 Frame::window 相同，观感与未设置前完全一致。
        let style = ctx.style();
        let win_fill = style.visuals.window_fill();
        let verify_fill = egui::Color32::from_rgba_unmultiplied(
            win_fill.r(),
            win_fill.g(),
            win_fill.b(),
            verify_opacity_alpha(state.opacity_pct),
        );
        egui::Window::new(tr(self.lang, "verify_title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .min_size([VERIFY_WINDOW_MIN_W, VERIFY_WINDOW_MIN_H])
            .default_size([VERIFY_WINDOW_DEFAULT_W, VERIFY_WINDOW_DEFAULT_H])
            .frame(egui::Frame::window(&style).fill(verify_fill))
            .show(ctx, |ui| {
                // 在整窗 Ui 上取一次内容高度：嵌套 Ui 的 available_height 在 vertical /
                // horizontal 中可能是受限值，绘图高度必须直接跟整窗尺寸走。
                let window_h = ui.available_height();
                if state.running {
                    ui.label(
                        egui::RichText::new(tr(self.lang, "verify_waiting"))
                            .color(self.style.accent_color()),
                    );
                    let frac = if state.total > 0 {
                        (state.read as f32 / state.total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(format!("{} / {}", state.read, state.total)),
                    );
                } else if !state.msg.is_empty() {
                    ui.label(egui::RichText::new(&state.msg).color(self.style.text_dim()));
                } else if state.anomalies.is_empty() {
                    // 非法字符优先于「未发现丢帧」：烂数据绝不能被说成正常。
                    let (hint, warning) = verify_result_hint(self.lang, state.has_invalid_char, 0)
                        .expect("无异常时必有结论提示");
                    let color = if warning {
                        egui::Color32::from_rgb(0xE0, 0x40, 0x40)
                    } else {
                        egui::Color32::from_rgb(0x2E, 0xA0, 0x55)
                    };
                    ui.label(
                        egui::RichText::new(hint)
                            .strong()
                            .size(VERIFY_HEADING_SIZE)
                            .color(color),
                    );
                    if warning {
                        ui.label(
                            egui::RichText::new(tr(self.lang, "verify_invalid_data_note"))
                                .color(self.style.text_dim()),
                        );
                    }
                } else {
                    // 左右分栏直接从弹窗内容区顶部开始：右栏第一行就是详情文字，顶部不放
                    // 整窗横条元素（结论标题、前后帧输入横条都移入左栏），否则它们内容靠左、
                    // 在右栏正上方留下整行空白，看起来像「右栏顶部一大块空白」。
                    // 常规 1/3 : 2/3；窄窗口优先保右栏 >= 260px。
                    ui.horizontal(|ui| {
                        let total_w = ui.available_width();
                        let list_w = verify_split_width(total_w, self.verify_split_w);
                        // 用整窗高度 window_h 定高，不能用 ui.available_height()：
                        // 后者在 horizontal 内会收缩到内容高度，导致红框只包住几条就停。
                        ui.allocate_ui_with_layout(
                            egui::vec2(list_w, window_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                // 结论标题 + 透明度滑块同一行：用户要求滑块紧跟「发现 N 处异常：」。
                                // 两个分支（正常 / 数据含非法字符）都要带滑块。
                                if state.has_invalid_char {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(tr(self.lang, "verify_invalid_data"))
                                                .strong()
                                                .size(VERIFY_HEADING_SIZE)
                                                .color(egui::Color32::from_rgb(0xE0, 0x40, 0x40)),
                                        );
                                        ui.add(
                                            egui::Slider::new(
                                                &mut state.opacity_pct,
                                                VERIFY_OPACITY_MIN..=VERIFY_OPACITY_MAX,
                                            )
                                            .show_value(false),
                                        );
                                    });
                                    ui.label(
                                        egui::RichText::new(tr(self.lang, "verify_invalid_data_note"))
                                            .color(self.style.text_dim()),
                                    );
                                } else {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(fmt_nums(
                                                tr(self.lang, "verify_found"),
                                                &[state.anomalies.len() as u64],
                                            ))
                                            .strong()
                                            .size(VERIFY_HEADING_SIZE),
                                        );
                                        ui.add(
                                            egui::Slider::new(
                                                &mut state.opacity_pct,
                                                VERIFY_OPACITY_MIN..=VERIFY_OPACITY_MAX,
                                            )
                                            .show_value(false),
                                        );
                                    });
                                }
                                // 前后帧输入：两个短输入框，回到加滑块前的原始写法。
                                let before_prev = state.before_text.clone();
                                let after_prev = state.after_text.clone();
                                ui.horizontal(|ui| {
                                    ui.label(tr(self.lang, "verify_before"));
                                    ui.add(egui::TextEdit::singleline(&mut state.before_text).desired_width(44.0));
                                    ui.label(tr(self.lang, "verify_after"));
                                    ui.add(egui::TextEdit::singleline(&mut state.after_text).desired_width(44.0));
                                });
                                // 前后帧数输入变化：下一帧强制重置到新窗口的真实边界。
                                if state.before_text != before_prev || state.after_text != after_prev {
                                    self.verify_epoch = self.verify_epoch.wrapping_add(1);
                                    self.verify_zoom = 1.0;
                                    state.reset_bounds |= verify_reset_on_event(false, true, false);
                                }
                                // 异常清单固定框：从清单标题下沿延伸到底部，条目少也保留整块
                                // 区域，条目多只在框内滚动。描边用软件统一配色，不用红色。
                                // 关键：左栏已用 window_h 定高，ui.available_height() 就是精确剩余；
                                // ScrollArea auto_shrink(false) 会吃掉它。切勿再 set_min_height——
                                // 那会让内容最小高度 > 窗口，拖动缩放松手后被 egui 顶回、框溢出底部。
                                let frame_stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                                egui::Frame::none()
                                    .stroke(frame_stroke)
                                    .rounding(3.0)
                                    .inner_margin(egui::Margin::same(4.0))
                                    .show(ui, |ui| {
                                        egui::ScrollArea::vertical()
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                for (i, a) in state.anomalies.iter().enumerate() {
                                                    let line = fmt_nums(
                                                        tr(self.lang, "verify_anomaly_line"),
                                                        &[a.frame_no, a.expected, a.actual, a.lost],
                                                    );
                                                    if ui.selectable_label(state.selected == Some(i), line).clicked() {
                                                        state.selected = Some(i);
                                                        self.verify_epoch = self.verify_epoch.wrapping_add(1);
                                                        self.verify_zoom = 1.0;
                                                        // 换异常条目：下一帧强制重置到该条快照的真实边界。
                                                        state.reset_bounds |= verify_reset_on_event(true, false, false);
                                                    }
                                                }
                                            });
                                    });
                            },
                        );
                        // 可拖动分隔条：紧贴左栏内容，hover 高亮，拖动改 verify_split_w，
                        // 松手时 save_config 持久化（不每帧写文件）。
                        let sep_w = 6.0_f32;
                        let (sep_rect, sep_resp) = ui.allocate_exact_size(
                            egui::vec2(sep_w, ui.available_height()),
                            egui::Sense::drag(),
                        );
                        let sep_color = if sep_resp.hovered() || sep_resp.dragged() {
                            self.style.accent_color()
                        } else {
                            ui.visuals().widgets.noninteractive.bg_stroke.color
                        };
                        ui.painter().line_segment(
                            [sep_rect.center_top(), sep_rect.center_bottom()],
                            egui::Stroke::new(2.0_f32, sep_color),
                        );
                        if sep_resp.dragged() {
                            self.verify_split_w = verify_split_width(
                                total_w,
                                self.verify_split_w + sep_resp.drag_delta().x,
                            );
                        }
                        if sep_resp.drag_stopped() {
                            self.save_config();
                        }
                        ui.vertical(|right| {
                            let before = verify_frames_from_text(&state.before_text);
                            let after = verify_frames_from_text(&state.after_text);
                            if let Some(sel) = state.selected {
                                if let Some(a) = state.anomalies.get(sel) {
                                    // 说明文字与红字异常帧号并排一行，靠 egui 默认 item_spacing 分隔。
                                    right.horizontal(|ui| {
                                        ui.label(tr(self.lang, "verify_detail"));
                                        // 明确写出异常帧真实帧号，避免相对坐标让人认不出是哪一帧。
                                        ui.label(
                                            egui::RichText::new(fmt_nums(
                                                tr(self.lang, "verify_center_frame"),
                                                &[a.frame_no],
                                            ))
                                            .strong()
                                            .color(egui::Color32::from_rgb(0xE0, 0x40, 0x40)),
                                        );
                                    });
                                    if let Some(snap) = &a.snapshot {
                                        // 按用户设置的前/后帧数从最大窗口快照中截取；新中心下标让
                                        // 异常点 x 仍等于真实帧号，x_bounds=[frame-前N, frame+后N]。
                                        let (snap_view, center_view) =
                                            verify_slice_snapshot(snap, a.snap_center, before, after);
                                        let x0 = verify_real_x_base(a.frame_no, center_view);
                                        let series: Vec<(Option<String>, Vec<f64>, usize, f64)> = snap_view
                                            .iter()
                                            .enumerate()
                                            .map(|(i, col)| {
                                                (
                                                    Some(format!("CH{}", i + 1)),
                                                    col.iter().map(|&v| v as f64).collect(),
                                                    i,
                                                    x0,
                                                )
                                            })
                                            .collect();
                                        // 大红圈标出异常帧各通道的实际值（x=真实帧号）；空心不遮数据点。
                                        let marker_ys: Vec<f64> = snap_view
                                            .iter()
                                            .map(|col| {
                                                col.get(center_view)
                                                    .copied()
                                                    .unwrap_or(0)
                                                    as f64
                                            })
                                            .collect();
                                        let mut zoom = self.verify_zoom;
                                        // 重置标志随 state 传递：本帧取用、绘图后写回，
                                        // 不放在 FrameApp 上，避免链路中途丢失。
                                        let mut reset = state.reset_bounds;
                                        crate::ui::plot::draw_plot_with_id(
                                            "verify_plot",
                                            right,
                                            1.5,
                                            &series,
                                            // 高度直接由整窗高度算，弹窗上下拖多大图就长多大。
                                            verify_plot_height_from_window(window_h),
                                            self.verify_epoch,
                                            &mut zoom,
                                            true, // 像主图暂停那样叠加实心数据点
                                            Some((a.frame_no as f64, marker_ys)),
                                            Some(verify_x_bounds(a.frame_no, before, after)),
                                            &mut reset,
                                            self.lang,
                                        );
                                        self.verify_zoom = zoom;
                                        state.reset_bounds = reset;
                                    } else {
                                        right.label(tr(self.lang, "verify_no_snapshot"));
                                    }
                                }
                            }
                        });
                    });
                }
            });
        if open {
            self.verify = Some(state);
        } else {
            self.verify = None;
            self.verify_rx = None;
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
            if kind == PickKind::Verify {
                let picked = rfd::FileDialog::new()
                    .add_filter("Text", &["txt", "log", "hex"])
                    .pick_file();
                let _ = tx.send(picked);
                return;
            }
            let dialog = match kind {
                PickKind::Csv => rfd::FileDialog::new()
                    .set_file_name("extracted.csv")
                    .add_filter("CSV", &["csv"]),
                PickKind::Txt => rfd::FileDialog::new()
                    .set_file_name("raw.txt")
                    .add_filter("Text", &["txt"]),
                PickKind::Verify => unreachable!(),
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
                (PickKind::Verify, Some(p)) => self.start_verify(p),
                (PickKind::Verify, None) => {}
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
            self.set_status(format!("主题: {}", self.style.theme().name));
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
        // 原始数据绘图 | 剪贴板绘图 | 数据验算（三等分；字号适中，中英文都不撑宽左栏）
        let lang = self.lang;
        let mut clicked = [false; 3];
        ui.columns(3, |cols| {
            let keys = ["load_file", "analyze_clip", "verify_btn"];
            for (i, col) in cols.iter_mut().enumerate() {
                let key = keys[i];
                let t = tr(lang, key);
                #[cfg(test)]
                {
                    let galley = col.fonts(|f| {
                        f.layout_no_wrap(
                            t.to_owned(),
                            egui::FontId::proportional(OFFLINE_BTN_FONT_SIZE),
                            egui::Color32::WHITE,
                        )
                    });
                    assert!(
                        galley.rect.width() + 6.0 <= col.available_width(),
                        "三按钮文案应放得下（不撑宽左栏）: {} w={} slot={}",
                        t,
                        galley.rect.width(),
                        col.available_width()
                    );
                }
                // 三按钮完全同款：普通暗色按钮，不做 accent/白字特殊处理。
                let btn = egui::Button::new(egui::RichText::new(t).size(OFFLINE_BTN_FONT_SIZE));
                if col.add_sized([col.available_width(), 24.0], btn).clicked() {
                    clicked[i] = true;
                }
            }
        });
        if clicked[0] {
            self.load_file_offline();
        }
        if clicked[1] {
            self.analyze_clipboard();
        }
        if clicked[2] {
            self.spawn_pick(PickKind::Verify);
        }
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
            self.set_status(reset_done);
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
                    self.set_status(format!("{}: {}", tr(self.lang, "send_err"), e));
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
                self.set_status(format!("{}: 远端IP/端口未设置", tr(self.lang, "send_err")));
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
            Err(e) => self.set_status(format!("{}: {}", tr(self.lang, "send_err"), e)),
        }
    }

    /// 造一帧回环测试数据：帧头（header_text）+ 若干 00 填充 + 末字节循环码（0→255→0）。
    /// 只看帧头与帧长：不生成校验字节、不看 HEX/ASCII 开关，直接发原始字节。
    /// 帧长必须 > 帧头长度 + 1（至少要留一个 00 和一个循环码字节），否则明确报错不 panic。
    fn next_loopback_frame(&mut self) -> Result<Vec<u8>, String> {
        let header = parse_header_hex(&self.header_text)
            .map_err(|e| format!("{}: {}", tr(self.lang, "loopback_hdr_err"), e))?;
        let length: usize = self
            .length_text
            .trim()
            .parse()
            .map_err(|_| format!("{}: {}", tr(self.lang, "loopback_len_err"), self.length_text))?;
        if length <= header.len() + 1 {
            return Err(format!(
                "{}: {} <= {} + 1",
                tr(self.lang, "loopback_bad_len"),
                length,
                header.len()
            ));
        }
        let mut data = header;
        data.resize(length - 1, 0x00);
        data.push(self.loopback_code);
        self.loopback_code = self.loopback_code.wrapping_add(1);
        Ok(data)
    }

    /// 直接发原始字节（回环造帧专用）：不编码、不记历史，仅按链路目标发送并计 TX。
    fn send_raw_bytes(&mut self, data: &[u8]) {
        let target = if self.link_kind == LinkKind::Udp {
            let ip = self.udp_remote_ip.trim();
            let port = self.udp_remote_port.trim();
            if ip.is_empty() || port.is_empty() {
                self.set_status(format!("{}: 远端IP/端口未设置", tr(self.lang, "send_err")));
                return;
            }
            Some(format!("{}:{}", ip, port))
        } else {
            None
        };
        match send_bytes(&self.handle, data, target.as_deref()) {
            Ok(_) => self.tx_count += 1,
            Err(e) => self.set_status(format!("{}: {}", tr(self.lang, "send_err"), e)),
        }
    }

    /// 命令列表 / 回环测试循环发送计时：到点发下一条，未连接自动停。
    /// 回环模式忽略空命令列表、发 next_loopback_frame() 造的帧；普通模式轮发命令列表。
    /// 在 ui() 每帧调用（egui 即时模式，无需额外线程）。
    fn poll_cmd_loop(&mut self) {
        if !self.cmd_loop {
            return;
        }
        if !self.connected {
            self.cmd_loop = false;
            return;
        }
        // 回环模式允许空命令列表；普通模式空列表自动停。
        if !self.loopback_enabled && self.cmd_list.is_empty() {
            self.cmd_loop = false;
            return;
        }
        let due = self
            .cmd_next_send
            .map(|t| Instant::now() >= t)
            .unwrap_or(true);
        if due {
            if self.loopback_enabled {
                match self.next_loopback_frame() {
                    Ok(data) => self.send_raw_bytes(&data),
                    Err(msg) => {
                        // 帧头/帧长配错：停循环并把明确错误放进「最近操作」格。
                        self.set_status(msg);
                        self.cmd_loop = false;
                    }
                }
            } else {
                let idx = self.cmd_loop_idx.min(self.cmd_list.len() - 1);
                let text = self.cmd_list[idx].clone();
                self.send_payload(&text, false);
                self.cmd_loop_idx = (idx + 1) % self.cmd_list.len();
            }
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
            verify_split_w: self.verify_split_w,
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
        // 可拖动分隔线位置：老配置缺省已由 serde 填 160，这里再 clamp 防坏值。
        self.verify_split_w = verify_split_width(920.0, cfg.verify_split_w);
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
    /// none_text 由调用方按格传入：帧配置格与通道配置格未应用时显示不同文案。
    fn config_receipt_text(&self, r: &ConfigReceipt, none_text: &str) -> String {
        match r {
            ConfigReceipt::None => none_text.to_string(),
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
        self.config_receipt_text(&self.frame_msg, tr(self.lang, "frame_unconfigured"))
    }
    fn channel_cell_text(&self) -> String {
        self.config_receipt_text(&self.channel_msg, tr(self.lang, "channel_unconfigured"))
    }

    /// 状态栏从左到右 11 格文本（最后一格是 TX）。
    /// 按功能区块分格：格内互斥（后一条换掉前一条是合理的），格间独立、互不覆盖。
    /// 抽成纯数据便于单测断言顺序与口径。
    /// 记录中指示灯是独立 widget（不进本数组），便于保持本数组单测的简单断言。
    /// 统一设置「最近操作」回执并打时间戳（约 3 秒后自动消失）。
    /// 所有一次性提示都走这里，避免遗漏时间戳导致常驻。
    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
        self.status_msg_at = Some(Instant::now());
    }

    /// 一次性回执是否已过期（约 3 秒）。未打时间戳的（如测试直接赋值）视为未过期。
    fn status_msg_expired(&self) -> bool {
        self.status_msg_at
            .map(|t| t.elapsed() >= Duration::from_secs(3))
            .unwrap_or(false)
    }

    fn status_cells(&self) -> Vec<String> {
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
        // ⑤校验格：显示选中的校验模式（no / 校验8位 / 校验16位），让人一眼看出配置生效。
        // 无帧头 + 开校验时才附同步后缀（·待同步 / ·已同步）：同步状态只在无帧头时有意义，
        // 有帧头靠帧头切帧，不显示同步。后缀两态互斥，共用这一格，不会同时出现。
        let sync = match self.engine.checksum_mode {
            ChecksumMode::None => tr(self.lang, "chk_no").to_string(),
            ChecksumMode::Sum8 | ChecksumMode::Sum16 => {
                let is8 = self.engine.checksum_mode == ChecksumMode::Sum8;
                let mode = tr(self.lang, if is8 { "chk_mode8" } else { "chk_mode16" });
                if self.engine.frame_header.is_empty() {
                    let tail = tr(self.lang, if self.engine.sync_locked { "chk_synced" } else { "chk_sync" });
                    format!("{}·{}", mode, tail)
                } else {
                    mode.to_string()
                }
            }
        };
        // ⑥⑦⑧计数常驻显示（没有也显示 0，不再留空）
        let chk = format!("{} {}", tr(self.lang, "chk_err"), self.engine.checksum_errors);
        let lost = format!("{} {}", tr(self.lang, "lost_frames"), self.engine.lost_frames());
        let frames = if self.lang == Lang::Zh {
            format!("帧 {}", self.engine.frame_count)
        } else {
            format!("Frames: {}", self.engine.frame_count)
        };
        // ⑧跳变（循环码跳号累计）：常驻显示，0 也显示。按用户要求放在帧数前面。
        let jumps = format!("{} {}", tr(self.lang, "cycle_jumps_short"), self.engine.cycle_jumps);
        // ⑪最近操作：回环测试中固定显示；一次性回执约 3 秒后消失；空闲则轮播快捷提示。
        let recent = if self.loopback_enabled && self.cmd_loop {
            tr(self.lang, "loopback_running").to_string()
        } else if self.status_msg.is_empty() || self.status_msg_expired() {
            let idx = status_tip_index(
                self.status_tip_epoch.elapsed().as_secs(),
                STATUS_TIP_KEYS.len(),
            );
            tr(self.lang, STATUS_TIP_KEYS[idx]).to_string()
        } else {
            self.status_msg.clone()
        };
        let tx = format!("TX: {}", self.tx_count);
        // 顺序按用户定稿：连接 | 帧配置 | 校验同步 | 通道配置 | 通道信息 | 校验错 | 丢帧 | 跳变 | 帧数 | 最近操作 | TX
        // 校验紧跟帧配置（校验本就是帧配置的一部分），通道配置紧跟通道信息（位宽/符号是通道的结果）。
        vec![conn, frame_cfg, sync, channel_cfg, chan_info, chk, lost, jumps, frames, recent, tx]
    }

    /// 通道信息格：位宽按已应用的 Idx 字节数 × 8，正负按 ± 是否为 -1。
    /// 位宽与符号紧贴无空格：中文「8位有符号」，英文缩写「8sig / 8unsig」。
    /// 位宽混 → 「8/16位混合」；位宽一致但符号混 → 「16位混合」。
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
            if zh { format!("{}位", uniq[0]) } else { uniq[0].to_string() }
        } else {
            let list = uniq.iter().map(|b| b.to_string()).collect::<Vec<_>>().join("/");
            if zh { format!("{}位", list) } else { list }
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
        // 中英都紧贴无空格：中文「8位有符号」，英文前缀已含 bit 缩写（8bit→8sig）。
        format!("{}{}", bit_word, sign_word)
    }

    /// 状态行：按 status_cells() 的顺序铺开，每格贴合当前文字、不做均摊。
    /// 空间不够时按优先级从低到高压缩：最近操作 → 通道信息 → 配置回执
    /// → 校验/数字 → 连接；数字格保底不压缩，长文字截断加省略号、悬停看全文。
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let dim = self.style.text_dim();
        let accent = self.style.accent_color();
        let cells = self.status_cells(); // 11 格：[0连接][1帧配置][2校验同步][3通道配置][4通道信息][5校验错][6丢帧][7跳变][8帧数][9最近操作][10TX]
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
        // 下限：数字格（5 校验错 / 6 丢帧 / 7 跳变 / 8 帧数）保底不压；
        // 空格子留最小占位；其余给适度下限，空间不够时优先让位。
        // 换序后：1帧配置、2校验同步、3通道配置、4通道信息；校验同步本无下限，不套 56。
        let mut lower: Vec<f32> = vec![
            conn_min.min(w[0]),
            w[1].min(56.0),
            w[2],
            w[3].min(56.0),
            w[4].min(48.0),
            w[5], w[6], w[7], w[8],
        ];
        for i in 0..9 {
            if left_texts[i].is_empty() || left_texts[i] == "—" {
                w[i] = w[i].max(18.0);
                lower[i] = lower[i].min(w[i]);
            }
        }
        // 记录灯固定宽度（不随状态跳动）
        let light_w: f32 = 76.0;
        // 绘图状态格固定宽：「实时绘图 / 绘图已暂停」取较宽者，切换时不跳动。
        let plot_state_w = labels_width(
            ui,
            &[tr(self.lang, "plot_state_live"), tr(self.lang, "plot_state_paused")],
        ) + 4.0;
        // 右排：最近操作（轮播提示/一次性回执）+ TX。最近操作设宽度上限，
        // 超长回执截断 + 悬停看全文，绝不再把 TX 挤出状态栏。
        let recent_w = labels_width(ui, &[
            recent_text.as_str(),
            tr(self.lang, "rec_wait_conn"),
            tr(self.lang, "rec_saving"),
        ]).min(RECENT_MAX_W) + 2.0;
        let tx_w = labels_width(ui, &[tx_text.as_str(), "TX: 0000000"]) + 4.0;

        // 分隔线：左 9 格间 8 条 + 绘图状态前 1 条 + 记录灯前 1 条 = 10 条
        let seps: f32 = sep_w * 10.0;
        let gaps: f32 = sp * 12.0 + 4.0;
        let left_avail = (ui.available_width()
            - plot_state_w - light_w - recent_w - tx_w - seps - gaps)
            .max(0.0);
        let mut need: f32 = w.iter().sum();
        // 压缩优先级（从最不重要开始）：通道信息 → 帧配置 → 通道配置 → 同步 → 连接；
        // 换序后索引：通道信息 4、帧配置 1、通道配置 3、同步 2、连接 0。
        // 数字格 5/6/7/8 保底不压；最近操作已挪到右排可整体让位。
        let order: [usize; 5] = [4, 1, 3, 2, 0];
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
                let color = if i == 7 { accent } else { dim }; // 跳变格保持主色强调（换位置后索引 7）
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
            // 绘图状态：常驻两态，暂停时红色呼吸提醒（独立 widget，不进 status_cells 文本数组）
            ui.add_sized([sep_w, h], egui::Separator::default().vertical().spacing(sep_w));
            self.draw_plot_state(ui, plot_state_w, h);
            // 记录指示灯（独立 widget，不进 status_cells 文本数组）
            ui.add_sized([sep_w, h], egui::Separator::default().vertical().spacing(sep_w));
            self.draw_record_light(ui, light_w, h);
            // 弹性 spacer：纯空白，不绘制文字/线/背景，吃掉剩余宽度
            let remain = ui.available_width() - recent_w - tx_w - sp;
            if remain > 0.0 {
                ui.add_space(remain);
            }
            // 右排：最近操作（靠右）。空闲轮播用更淡的灰，与真正的一次性回执区分。
            let tip_now = !self.loopback_enabled || !self.cmd_loop;
            let idle_tip = tip_now
                && (self.status_msg.is_empty() || self.status_msg_expired());
            let recent_color = if idle_tip {
                egui::Color32::from_gray(120)
            } else {
                dim
            };
            let r_resp = ui.add_sized(
                [recent_w, h],
                egui::Label::new(egui::RichText::new(recent_text.as_str()).color(recent_color))
                    .truncate()
                    .halign(egui::Align::RIGHT),
            );
            if !recent_text.is_empty() {
                let _ = r_resp.on_hover_text(recent_text.as_str());
            }
            // 空闲轮播需要持续重绘，否则窗口无事件时文字不换。
            if idle_tip {
                ui.ctx().request_repaint_after(Duration::from_millis(500));
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

    /// 状态栏「绘图状态」：常驻两态。实时绘图=暗色小字、无任何特殊效果；绘图已暂停=
    /// 红色呼吸闪烁（持续请求重绘），避免误点暂停后不知道为什么不画图。
    fn draw_plot_state(&self, ui: &mut egui::Ui, w: f32, h: f32) {
        let dim = self.style.text_dim();
        let paused = self.plot_paused;
        let text = tr(
            self.lang,
            if paused { "plot_state_paused" } else { "plot_state_live" },
        );
        let rich = if paused {
            // 呼吸幅度约 0.47Hz：红色在深红与亮红之间来回，一眼能看到在动。
            let t = ui.input(|i| i.time);
            let pulse = (((t * 3.0).sin() + 1.0) * 0.5) as f32; // 0..1
            let r = 0xC0 + (0x3F as f32 * pulse) as u8;
            let g = 0x30 + (0x40 as f32 * pulse) as u8;
            let b = 0x30 + (0x20 as f32 * pulse) as u8;
            ui.ctx().request_repaint();
            egui::RichText::new(text).strong().color(egui::Color32::from_rgb(r, g, b))
        } else {
            egui::RichText::new(text).color(dim)
        };
        ui.add_sized([w, h], egui::Label::new(rich).truncate().halign(egui::Align::LEFT));
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
                ui.label("   ★ 无帧头 + 启用校验时：收到的数据可能是任意字节开头，软件会用「连续两帧校验都正确」来确认分帧起点（单帧通过可能是巧合），对齐前不画图，状态栏显示「校验N位·待同步」；对齐后正常解析，连续 3 帧校验失败会自动重新搜索对齐。");
                ui.label("3. 通道配置：填通道数点「应用」，每行 Idx 填字节索引（支持 , 空格 中文逗号 顿号），± 为符号位(-1=无，Idx 填完自动算)，Dif 勾选=原始线+差分虚线叠加；");
                ui.label("4. 离线绘图：帧配置好后点「载入文件」或「分析剪贴板」，把 hex 文本直接绘图；");
                ui.label("5. 数据记录：勾选提取数据保存(CSV)/原始数据保存(TXT)，未选文件时自动弹出保存对话框；路径标签悬停可看完整路径；");
                ui.label("6. 绘图工具栏：🏠 恢复原视图、➕/➖ 缩放、🗑 清空（顺带把 TX 计数归零）、⏸ 暂停绘图（数据照常解析，暂停时显示数据点圆点），▶ 恢复实时；");
                ui.label("7. 批量发送（原命令发送）：手动填若干条命令，设间隔(ms)后 ▶ 自动循环轮发，或逐条点「发送」；HEX/ASCII 跟随发送行格式开关；");
                ui.label(tr(self.lang, "loopback_help"));
                ui.label("8. 配置自动保存：帧配置/通道配置/语言/主题/波特率/校验位/命令列表存 exe 同目录 .config.json，退出和应用时写回，重启自动恢复；");
                ui.label("9. 🌙 循环切换 8 套主题（深色→原始→淡绿→淡蓝→米黄→粉色→Mac→Win），窗口标题栏跟随（Win11）；");
                ui.label("10. F12 界面微调：左栏宽度，保存到 ui_style.json。");
                ui.label("12. 数据验算：帧配置区点「数据验算」选原始数据 txt，软件流式读取并按当前帧配置分帧、算循环码，找出丢帧发生在哪一段；弹窗列出每条异常（帧号/期望→实际/丢帧数），点一条在下方画出该点前后各 N 帧的通道曲线（默认 50，可在弹窗内改 1~500，不覆盖主图）；大文件也不卡界面。验算前请确认帧配置与当时采数一致：校验=no、循环码位置=帧长（1 起算末字节）、位宽=8；循环码位置留空则无法验算。");
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
                    ui.checkbox(&mut self.loopback_enabled, tr(self.lang, "loopback_test"))
                        .on_hover_text(tr(self.lang, "loopback_tip"));
                    if self.cmd_loop {
                        if ui.add(egui::Button::new("⏸")).on_hover_text(tr(self.lang, "cmd_stop")).clicked() {
                            self.cmd_loop = false;
                        }
                        let sending = if self.loopback_enabled {
                            tr(self.lang, "loopback_running").to_string()
                        } else {
                            format!("{} #{}", tr(self.lang, "cmd_sending"), self.cmd_loop_idx + 1)
                        };
                        ui.label(egui::RichText::new(sending).color(self.style.accent_color()));
                    } else if ui
                        .add(egui::Button::new("▶"))
                        .on_hover_text(tr(self.lang, "cmd_start"))
                        .clicked()
                    {
                        if !self.loopback_enabled && self.cmd_list.is_empty() {
                            self.set_status(tr(self.lang, "cmd_empty"));
                        } else if !self.connected {
                            self.set_status(tr(self.lang, "cmd_not_connected"));
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
                if self.loopback_enabled {
                    ui.label(
                        egui::RichText::new(tr(self.lang, "loopback_setup_tip"))
                            .color(self.style.text_dim())
                            .small(),
                    );
                }
                ui.separator();
                // 命令行列表：编辑 + 逐条发送 + 删除（命令多时可滚动）
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if self.loopback_enabled {
                        ui.label(
                            egui::RichText::new(tr(self.lang, "loopback_hint"))
                                .color(self.style.text_dim())
                                .small(),
                        );
                    } else if self.cmd_list.is_empty() {
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
            if !self.loopback_enabled && self.cmd_list.is_empty() {
                self.cmd_loop = false;
            } else if !self.cmd_list.is_empty() {
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
        self.poll_verify();
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
                                        self.lang,
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
                                self.clear_plot();
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
        self.verify_window(ctx);
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
/// 把 i18n 模板里的 `{}` 依次换成数字（format! 要求字面量模板，不能直接用 tr 结果）。
fn fmt_nums(tpl: &str, nums: &[u64]) -> String {
    let mut s = tpl.to_string();
    for n in nums {
        s = s.replacen("{}", &n.to_string(), 1);
    }
    s
}

/// 把 i18n 模板里的第一个 `{}` 换成字符串（底层错误细节 / 路径等）。
fn fmt_str(tpl: &str, s: &str) -> String {
    tpl.replacen("{}", s, 1)
}

/// 验算快照图的 x 基准：用真实帧号，异常点（快照下标 center）落在 x=frame_no。
/// 第 i 个点的 x = (frame_no - center) + i，前后两点都是真实帧号，鼠标读数即真实帧号。
fn verify_real_x_base(frame_no: u64, center: usize) -> f64 {
    frame_no as f64 - center as f64
}

/// 验算「前/后 N 帧」输入框取值：非法/空回退 50，clamp 到 1..=VERIFY_SNAP_MAX。
fn verify_frames_from_text(text: &str) -> usize {
    text.trim()
        .parse::<usize>()
        .ok()
        .filter(|&n| n >= 1)
        .unwrap_or(50)
        .min(VERIFY_SNAP_MAX)
}

/// 验算快照图的 x 范围：以真实帧号 frame_no 为中心，前 before / 后 after 帧。
/// 早期帧下界可能为负（代表第 1 帧之前的虚拟位置），保留负数不做 clamp，
/// 保证异常点 x=frame_no 始终严格居中；透明辅助线用首末值平延撑开该范围。
fn verify_x_bounds(frame_no: u64, before: usize, after: usize) -> [f64; 2] {
    let f = frame_no as f64;
    [f - before as f64, f + after as f64]
}

/// 按用户设置的「前/后 N 帧」从最大窗口快照中截取一段，返回（按通道分组的片段, 新中心下标）。
/// 新中心下标用于复用 verify_real_x_base：异常点 x 仍等于真实帧号。
fn verify_slice_snapshot(
    snap: &[Vec<i64>],
    center: usize,
    before: usize,
    after: usize,
) -> (Vec<Vec<i64>>, usize) {
    let len = snap.first().map_or(0, |c| c.len());
    if len == 0 {
        return (Vec::new(), 0);
    }
    let start = center.min(len).saturating_sub(before);
    let end = center.saturating_add(after).saturating_add(1).min(len);
    let new_center = center.saturating_sub(start);
    (snap.iter().map(|c| c[start..end].to_vec()).collect(), new_center)
}

/// 验算结论文案（配置错误/无循环码由 state.msg 优先处理，不进这里）。
/// 返回 (文案, 是否警告)：None = 有异常，走「发现 N 处异常」标题。
/// 关键：数据含非法字符时优先于「未发现丢帧」，烂数据不能被当成正常数据。
fn verify_result_hint(
    lang: Lang,
    has_invalid_char: bool,
    anomaly_count: usize,
) -> Option<(String, bool)> {
    if has_invalid_char {
        Some((tr(lang, "verify_invalid_data").to_string(), true))
    } else if anomaly_count == 0 {
        Some((tr(lang, "verify_no_loss").to_string(), false))
    } else {
        None
    }
}

/// 验算快照图高度底层计算：不低于绘图正方形下限 VERIFY_PLOT_MIN_SIZE。
/// 由 verify_plot_height_from_window 用整窗高度调用；主绘图区高度逻辑不动。
fn verify_plot_height(available: f32) -> f32 {
    available.max(VERIFY_PLOT_MIN_SIZE)
}

/// 验算弹窗绘图高度：直接由整窗内容高度计算（减顶部固定开销），不依赖嵌套 Ui 的
/// available_height（在 ui.vertical / columns 中可能是受限值，导致图高卡住）。
/// 不额外留底部余量：让绘图底边（帧号轴）正好落在弹窗下边界上。
/// 没有上限：弹窗拖多高图就跟着长多高；下限 VERIFY_PLOT_MIN_SIZE 的正方形。
fn verify_plot_height_from_window(window_h: f32) -> f32 {
    verify_plot_height(window_h - VERIFY_TOP_OVERHEAD)
}

/// 可拖动分隔线位置：把期望左栏宽 desired clamp 到 [160, total-260]，
/// 保证绘图区至少 260px。拖动增量与配置恢复都走这一个纯函数。
fn verify_split_width(total: f32, desired: f32) -> f32 {
    let max_w = (total - VERIFY_SPLIT_RIGHT_MIN).max(VERIFY_SPLIT_MIN);
    desired.clamp(VERIFY_SPLIT_MIN, max_w)
}

/// 透明度百分比 → Color32 的 alpha（unpremultiplied，0..255）。
/// 先 clamp 到 20..=100 再四舍五入：80→204、100→255、20→51。
/// 100% 时 alpha=255，配合原 window_fill 的 rgb，观感与加 fill 前完全一致。
fn verify_opacity_alpha(pct: u8) -> u8 {
    let p = pct.clamp(VERIFY_OPACITY_MIN, VERIFY_OPACITY_MAX) as u32;
    ((p * 255 + 50) / 100) as u8
}

/// 纯函数：三类重置事件（换异常条目 / 改前后帧数 / 验算完成）任一发生即要求下一帧重置坐标。
fn verify_reset_on_event(select_changed: bool, frames_changed: bool, done: bool) -> bool {
    select_changed || frames_changed || done
}

/// 「原始数据绘图」文件对话框的扩展名白名单：CSV 不是原始数据，必须排除。
const OFFLINE_TEXT_EXTS: &[&str] = &["txt", "hex", "log"];

/// 判断一段文本是不是 CSV（导出表头）：首行以 `Frame,` 开头，或含逗号表头。
fn is_csv_text(text: &str) -> bool {
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .trim_start_matches('\u{feff}');
    if first.starts_with("Frame,") {
        return true;
    }
    first.contains(',') && first.to_ascii_lowercase().contains("frame")
}

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
        // 11 格：连接/帧配置/校验同步/通道配置/通道信息/校验错/丢帧/跳变/帧数/最近操作/TX
        assert_eq!(c.len(), 11);
        assert_eq!(c[0], tr(app.lang, "not_connected"));
        // 开机时两格不该出现「已应用」回执：启动只是悄悄装配配置，
        // 不算用户点过「应用」。用「不是回执状态」断言，避免依赖磁盘上
        // 是否有残留配置（配置损坏时开机显示错误提示是合理的，也允许）。
        assert!(
            !matches!(app.frame_msg, ConfigReceipt::FrameApplied),
            "开机不该显示「帧格式已配置」: {:?}",
            app.frame_msg
        );
        assert!(
            !matches!(app.channel_msg, ConfigReceipt::ChannelApplied(_)),
            "开机不该显示「通道已配置」: {:?}",
            app.channel_msg
        );
        // 未点应用时两格给出明确文案，不再是留白「—」
        if app.frame_msg == ConfigReceipt::None {
            assert_eq!(c[1], tr(app.lang, "frame_unconfigured"));
        }
        if app.channel_msg == ConfigReceipt::None {
            assert_eq!(c[3], tr(app.lang, "channel_unconfigured"));
        }
        // 校验同步已挪到索引 2，紧跟帧配置
        assert_eq!(c[2], tr(app.lang, "chk_no"), "未开校验时明确写「无校验」，不留空白");
        assert!(c[5].ends_with('0'), "校验错常驻 0: {:?}", c[5]);
        assert!(c[6].ends_with('0'), "丢帧常驻 0: {:?}", c[6]);
        // 第 8 格（索引 7）= 跳变 N，0 也常驻显示（按用户要求与帧数换位）
        assert!(
            c[7].contains(&tr(app.lang, "cycle_jumps_short")) && c[7].ends_with('0'),
            "跳变常驻 0: {:?}",
            c[7]
        );
        // 第 9 格（索引 8）= 帧数
        assert!(c[8].contains('0'), "帧数常驻: {:?}", c[8]);
        // 第 10 格（索引 9）= 最近操作，空闲时轮播快捷提示（不再是 —）
        assert!(
            STATUS_TIP_KEYS.iter().any(|k| tr(app.lang, k) == c[9]),
            "空闲时应显示轮播提示之一: {:?}",
            c[9]
        );
        // 第 11 格（索引 10）= TX: 0
        assert_eq!(c[10], "TX: 0");
        // 校验格：显示所选模式；无帧头时才附互斥的同步后缀，有帧头只显示模式。
        app.engine.checksum_mode = ChecksumMode::Sum8;
        app.engine.frame_header = Vec::new(); // 无帧头 → 附同步后缀
        app.engine.sync_locked = false;
        assert_eq!(
            app.status_cells()[2],
            format!("{}·{}", tr(app.lang, "chk_mode8"), tr(app.lang, "chk_sync"))
        );
        app.engine.sync_locked = true;
        assert_eq!(
            app.status_cells()[2],
            format!("{}·{}", tr(app.lang, "chk_mode8"), tr(app.lang, "chk_synced"))
        );
        // 有帧头：靠帧头切帧，不显示同步后缀
        app.engine.frame_header = vec![0xEB, 0x90];
        assert_eq!(app.status_cells()[2], tr(app.lang, "chk_mode8"));
        // Sum16 带 16 位前缀
        app.engine.checksum_mode = ChecksumMode::Sum16;
        app.engine.frame_header = Vec::new();
        assert_eq!(
            app.status_cells()[2],
            format!("{}·{}", tr(app.lang, "chk_mode16"), tr(app.lang, "chk_synced"))
        );
        // no → 无校验，不带任何前缀/后缀
        app.engine.checksum_mode = ChecksumMode::None;
        assert_eq!(app.status_cells()[2], tr(app.lang, "chk_no"));
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
        // 通道配置从索引 2 挪到索引 3
        assert_eq!(c[3], format!("{} (4)", tr(app.lang, "channel_applied")));
        // 最近操作在第 10 格（索引 9）
        assert_eq!(c[9], "主题: 深色");
        app.status_msg.clear();
        let c2 = app.status_cells();
        assert_eq!(c2[1], tr(app.lang, "frame_applied"), "清最近操作不应顶掉帧配置回执");
        assert_eq!(c2[3], format!("{} (4)", tr(app.lang, "channel_applied")));
    }

    /// 回归：开机不该冒充用户点过「应用」而显示「帧格式已配置」。
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
            "开机不该显示「帧格式已配置」: {:?}",
            app.frame_msg
        );
        assert!(
            !matches!(app.channel_msg, ConfigReceipt::ChannelApplied(_)),
            "开机不该显示「通道已配置」: {:?}",
            app.channel_msg
        );
        // 无回执时按格给出明确文案（纯渲染口径，不依赖磁盘状态）
        assert_eq!(app.frame_cell_text(), tr(app.lang, "frame_unconfigured"));
        assert_eq!(app.channel_cell_text(), tr(app.lang, "channel_unconfigured"));

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
        assert_eq!(zh[3], format!("{} ({})", tr(Lang::Zh, "channel_applied"), n));

        // 切到英文：两格立即变英文，且不需要重新点「应用」
        app.lang = Lang::En;
        let en = app.status_cells();
        assert_eq!(en[1], tr(Lang::En, "frame_applied"));
        assert_eq!(en[3], format!("{} ({})", tr(Lang::En, "channel_applied"), n));
        assert_ne!(en[1], zh[1], "帧配置格应随语言变化");
        assert_ne!(en[3], zh[3], "通道配置格应随语言变化");

        // 切回中文：还原
        app.lang = Lang::Zh;
        let back = app.status_cells();
        assert_eq!(back[1], zh[1]);
        assert_eq!(back[3], zh[3]);

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
        assert_eq!(app.channel_info_text(), "16位有符号");
        app.channels[1].sig_text = "-1".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "16位混合");
        app.channels[0].idx_text = "2".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "8/16位混合");
        app.channels[0].idx_text = "2,3".into();
        app.channels[0].sig_text = "-1".into();
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "16位无符号");
        // 设成 8 位有符号并落盘（此时 lang 仍是中文，写盘安全）
        for c in app.channels.iter_mut() {
            c.idx_text = "2".into();
            c.sig_text = "15".into();
        }
        app.apply_channel_config();
        assert_eq!(app.channel_info_text(), "8位有符号");
        // 下面只在内存里切语言读文本，绝不 apply——apply 会把 En 写进共享
        // .config.json，并行测试读到后全乱（曾致 2 个无关用例失败）。
        app.lang = Lang::En;
        assert_eq!(app.channel_info_text(), "8sig");
        app.engine.channels[0].sign_bit = -1;
        app.engine.channels[1].sign_bit = -1;
        assert_eq!(app.channel_info_text(), "8unsig");
        app.lang = Lang::Zh; // 纯内存复位，不写盘
    }

    /// 回环造帧：帧头拼接 + 00 填充 + 末字节循环码递增。
    #[test]
    fn loopback_frame_header_fill_and_increment() {
        let mut app = FrameApp::new();
        app.header_text = "EB 90".into();
        app.length_text = "4".into();
        app.loopback_code = 0;
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xEB, 0x90, 0x00, 0x00]);
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xEB, 0x90, 0x00, 0x01]);
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xEB, 0x90, 0x00, 0x02]);
        // 一字节帧头也不特殊：帧长 3 → 头 + 00 填充? 头长1，length3>1+1，resize(2) 补1个00
        app.header_text = "AB".into();
        app.length_text = "3".into();
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xAB, 0x00, 0x03]);
    }

    /// 回环循环码 0→255→0 回绕（u8 wrapping，不 panic）。
    #[test]
    fn loopback_code_wraps_ff_to_00() {
        let mut app = FrameApp::new();
        app.header_text = "EB 90".into();
        app.length_text = "4".into();
        app.loopback_code = 255;
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xEB, 0x90, 0x00, 0xFF]);
        assert_eq!(app.loopback_code, 0);
        assert_eq!(app.next_loopback_frame().unwrap(), vec![0xEB, 0x90, 0x00, 0x00]);
    }

    /// 非法帧长（<= 帧头长度 + 1、非数字）明确报错，不 panic。
    #[test]
    fn loopback_bad_length_reports_error() {
        let mut app = FrameApp::new();
        app.header_text = "EB 90".into(); // 帧头 2 字节，帧长须 >= 4
        app.length_text = "3".into();
        assert!(app.next_loopback_frame().is_err(), "帧长 3 <= 2+1 应报错");
        app.length_text = "2".into();
        assert!(app.next_loopback_frame().is_err(), "帧长 2 应报错");
        app.length_text = "abc".into();
        assert!(app.next_loopback_frame().is_err(), "非数字帧长应报错");
        // 帧长正好 = 帧头长度+1（3）仍报错；留够一个填充后才合法。
        app.length_text = "4".into();
        assert!(app.next_loopback_frame().is_ok());
    }

    /// 最近操作：回环运行中固定显示「回环测试中」；一次性回执约 3 秒后消失。
    #[test]
    fn status_cell_loopback_running_and_expiry() {
        let mut app = FrameApp::new();
        // 回环运行中：固定提示，且不受 status_msg 影响
        app.loopback_enabled = true;
        app.cmd_loop = true;
        app.set_status(tr(app.lang, "cycle_reset_done"));
        assert_eq!(app.status_cells()[9], tr(app.lang, "loopback_running"));
        // 停止后：回落到普通回执
        app.cmd_loop = false;
        assert_eq!(app.status_cells()[9], tr(app.lang, "cycle_reset_done"));
        // 回执过期（手动把时间戳拨到 4 秒前）：最近操作格回落到空闲轮播提示
        app.status_msg_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(4));
        let r = app.status_cells()[9].clone();
        assert!(
            STATUS_TIP_KEYS.iter().any(|k| tr(app.lang, k) == r),
            "过期后应回落轮播提示: {:?}",
            r
        );
    }

    /// 🗑 清空绘图：TX 与帧计数一起归 0（防止只清一半），快照也清空。
    #[test]
    fn clear_plot_resets_tx_and_frame_count() {
        let mut app = FrameApp::new();
        // 并行测试会共享 .config.json，显式设一套合法帧配置，避免 clear_plot 内
        // build_engine() 因并发写出的临时错误配置而失败（测试隔离）。
        app.header_text = String::new();
        app.length_text = "4".into();
        app.samples_text = "1".into();
        app.max_points_text = "500".into();
        app.cycle_pos_text = String::new();
        app.tx_count = 42;
        app.engine.frame_count = 7;
        app.engine.checksum_errors = 3;
        app.plot_snapshot_vals.push(vec![1, 2, 3]);
        app.plot_snapshot_dif.push(vec![4, 5, 6]);
        app.clear_plot();
        assert_eq!(app.tx_count, 0, "清空绘图应把 TX 计数归零");
        assert_eq!(app.engine.frame_count, 0, "清空绘图应把帧计数归零");
        assert_eq!(app.engine.checksum_errors, 0, "清空绘图应把校验错归零");
        assert!(
            app.plot_snapshot_vals.is_empty() && app.plot_snapshot_dif.is_empty(),
            "清空绘图应清掉暂停快照"
        );
    }

    // ===== 数据验算 =====

    /// 验算用引擎：帧头 EB 90、帧长 4、循环码在最后一字节（0-based 3）、8 位无校验；
    /// CH1 读字节 2..3 = 帧序号本身（大端 0x00XX），便于断言快照数值。
    fn verify_engine() -> Engine {
        let chans = vec![ChannelCfg {
            indices: vec![2, 3],
            sign_bit: -1,
            active: true,
        }];
        Engine::with_cycle(
            vec![0xEB, 0x90],
            4,
            1,
            true,
            chans,
            false,
            ChecksumMode::None,
            Some(3),
            8,
        )
    }

    fn codes_hex(codes: &[u8]) -> String {
        let mut s = String::new();
        for &c in codes {
            s.push_str(&format!("EB 90 00 {:02X} ", c));
        }
        s
    }

    fn run_verify(codes: &[u8]) -> Vec<VerifyAnomaly> {
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(codes_hex(codes).as_bytes());
        r.finish()
    }

    #[test]
    fn verify_normal_and_wrap_no_anomaly() {
        // 0..=255 再回 0：255→0 是自然回绕，不得误报。
        let mut codes: Vec<u8> = (0..=255).collect();
        codes.extend_from_slice(&[0, 1, 2]);
        let a = run_verify(&codes);
        assert!(a.is_empty(), "正常递增+回绕不应报异常: {:?}", a);
    }

    #[test]
    fn verify_jump_records_expected_actual_lost() {
        // 0,1,2 后跳到 5：期望 3、实际 5、丢 2 帧。
        let a = run_verify(&[0, 1, 2, 5, 6]);
        assert_eq!(a.len(), 1, "应恰好一条异常");
        assert_eq!(a[0].frame_no, 4);
        assert_eq!(a[0].expected, 3);
        assert_eq!(a[0].actual, 5);
        assert_eq!(a[0].lost, 2);
    }

    #[test]
    fn verify_chunk_boundary_keeps_bytes() {
        // 按 1 字节喂入，故意把每个两字符 hex 拆到不同块；结果必须与整块一致。
        let codes = [0u8, 1, 2, 5, 6];
        let text = codes_hex(&codes);
        let mut r = VerifyRunner::new(verify_engine(), 200);
        for &b in text.as_bytes() {
            r.feed(&[b]);
        }
        let a = r.finish();
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].frame_no, a[0].expected, a[0].actual, a[0].lost), (4, 3, 5, 2));
    }

    #[test]
    fn verify_snapshot_captures_max_window() {
        // 帧 1..100 正常 0..99；帧 101 跳到 103；之后继续 104..（共 197 帧）。
        let mut codes: Vec<u8> = (0..100).collect();
        codes.push(103);
        codes.extend((104..200u8).collect::<Vec<_>>());
        let a = run_verify(&codes);
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].frame_no, a[0].expected, a[0].actual, a[0].lost), (101, 100, 103, 3));
        let snap = a[0].snapshot.as_ref().expect("应存下快照");
        assert_eq!(snap.len(), 1, "只有 1 个通道");
        // 数据总长 197 < 2*MAX+1，快照保留全部可用帧（UI 再按前/后 N 帧截取）。
        assert_eq!(snap[0].len(), 197);
        assert_eq!(a[0].snap_center, 100, "异常帧 101 在快照中的下标为 100");
        // 中心（index 100）= 异常帧 101；前一帧 100 = 99；后一帧 102 = 104。
        assert_eq!(snap[0][99], 99);
        assert_eq!(snap[0][100], 103);
        assert_eq!(snap[0][101], 104);
    }

    #[test]
    fn verify_three_buttons_fit_left_panel() {
        let ctx = egui::Context::default();
        let mut app = FrameApp::new();
        app.style.left_panel_width = 300.0;
        // 三按钮行内 #[cfg(test)] 断言会在两语言下逐一检查文案宽度不超载。
        for lang in [Lang::Zh, Lang::En] {
            app.lang = lang;
            run_frames(&mut app, &ctx, 4);
        }
        assert!(
            app.last_panel_width <= 302.0,
            "三按钮不应撑宽左栏: width={}",
            app.last_panel_width
        );
    }

    /// 「原始数据绘图」过滤器不得含 csv（CSV 不是原始数据.txt）。
    #[test]
    fn offline_filter_excludes_csv() {
        assert!(
            !OFFLINE_TEXT_EXTS.contains(&"csv"),
            "原始数据绘图过滤器不应含 csv: {:?}",
            OFFLINE_TEXT_EXTS
        );
        assert!(OFFLINE_TEXT_EXTS.contains(&"txt"));
        assert!(OFFLINE_TEXT_EXTS.contains(&"hex"));
        assert!(OFFLINE_TEXT_EXTS.contains(&"log"));
    }

    /// CSV 文本走分析入口时给友好提示，不暴露底层 hex 解析报错；中英都要对。
    #[test]
    fn offline_csv_text_friendly_status() {
        let mut app = FrameApp::new();
        let csv = "Frame,CH1,CH2\n1,10,20\n2,11,21\n";
        for lang in [Lang::Zh, Lang::En] {
            app.lang = lang;
            app.analyze_hex_string(csv);
            assert_eq!(
                app.status_msg,
                tr(lang, "offline_csv_unsupported"),
                "CSV 应给友好提示（{:?}）",
                lang
            );
            assert!(
                !app.status_msg.contains("invalid digit"),
                "不能暴露底层 hex 报错: {}",
                app.status_msg
            );
        }
        // 带 BOM 的表头同样识别。
        app.analyze_hex_string("\u{feff}Frame,CH1\n1,2\n");
        assert_eq!(app.status_msg, tr(app.lang, "offline_csv_unsupported"));
    }

    /// 真实帧号坐标：series x 基准 = frame_no - snap_center，异常点 x = 真实帧号；
    /// x_bounds 以 frame_no 为中心（左右各 50），marker_x = frame_no。
    #[test]
    fn verify_real_frame_number_coordinates() {
        // 第 41 帧、前面 40 帧 → center=40 → x 基准=1，第 i 点 x=1+i，异常点 x=41。
        let (frame_no, center) = (41u64, 40usize);
        let base = verify_real_x_base(frame_no, center);
        assert_eq!(base, 1.0, "x 基准应为 frame_no - snap_center");
        assert_eq!(base + center as f64, frame_no as f64, "异常点应落在真实帧号 x=41");
        assert_eq!(base + (center - 1) as f64, 40.0, "异常点前一点应是真实帧号 40");
        assert_eq!(base + (center + 1) as f64, 42.0, "异常点后一点应是真实帧号 42");
        // x_bounds=[-9,91]，中心恰为 41，异常点仍严格居中。
        let bounds = verify_x_bounds(frame_no, 50, 50);
        assert_eq!(bounds, [-9.0, 91.0]);
        assert_eq!((bounds[0] + bounds[1]) * 0.5, frame_no as f64, "x 范围应以帧号居中");
        // marker_x 就是真实帧号，且与 x 范围中心一致（红圈与居中不冲突）。
        let marker_x = frame_no as f64;
        assert_eq!(marker_x, 41.0, "marker_x 应为真实帧号 41");
        assert_eq!(marker_x, (bounds[0] + bounds[1]) * 0.5, "marker_x 与 x 范围中心一致");

        // 边界帧：第 5 帧、center=4 → x 基准=1，点 x=1..5，异常点 x=5。
        let base5 = verify_real_x_base(5, 4);
        assert_eq!(base5, 1.0);
        assert_eq!(base5 + 4.0, 5.0, "边界帧异常点落在真实帧号 5");
        let bounds5 = verify_x_bounds(5, 50, 50);
        assert_eq!(bounds5, [-45.0, 55.0], "下界可负（虚拟位置），不 clamp 以保持居中");
        assert_eq!((bounds5[0] + bounds5[1]) * 0.5, 5.0);

        // 首帧：第 1 帧、center=0 → x 基准=1，异常点 x = base1 + 0 = 1。
        let base1 = verify_real_x_base(1, 0);
        assert_eq!(base1, 1.0, "首帧 x 基准 = 1 - 0 = 1");
        assert_eq!(base1 + 0.0, 1.0, "首帧异常点 x=1");
        assert_eq!(verify_x_bounds(1, 50, 50), [-49.0, 51.0]);

        // 满 101 帧窗口：frame=100、center=50 → x 基准=50，点 x=50..150。
        let base100 = verify_real_x_base(100, 50);
        assert_eq!(base100, 50.0);
        assert_eq!(base100 + 50.0, 100.0, "满窗口异常点 x=真实帧号");
        assert_eq!(base100 + 100.0, 150.0);
        assert_eq!(verify_x_bounds(100, 50, 50), [50.0, 150.0], "满窗口无负坐标");
    }

    /// 快照中立异常点下标：开头/中段不足 101 帧时都取到真实位置，不是写死 50。
    #[test]
    fn verify_snapshot_center_index_at_edges() {
        // 第 2 帧就跳变：前面只有 1 帧 → center=1。
        let a = run_verify(&[0, 5]);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].frame_no, 2);
        assert_eq!(a[0].snap_center, 1);
        // 第 5 帧跳变：前面 4 帧 → center=4。
        let a = run_verify(&[0, 1, 2, 3, 8]);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].frame_no, 5);
        assert_eq!(a[0].snap_center, 4);
        let snap = a[0].snapshot.as_ref().unwrap();
        assert_eq!(snap[0].len(), a[0].snap_center + 1, "开头处快照到异常点为止");
        assert_eq!(snap[0][a[0].snap_center], 8, "异常点应是跳变后的实际值");
        // 中段 41 帧跳变：前面 40 帧 → center=40。
        let mut codes: Vec<u8> = (0..40).collect();
        codes.push(45);
        let a = run_verify(&codes);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].frame_no, 41);
        assert_eq!(a[0].snap_center, 40);
    }

    /// 三按钮同款：源码静态检查，按钮行不得有 accent / 白字特殊分支。
    #[test]
    fn three_buttons_have_no_accent_branch() {
        let src = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let start = src
            .find("原始数据绘图 | 剪贴板绘图 | 数据验算")
            .expect("应找到三按钮行注释");
        let end = src[start..]
            .find("/// 左栏组 4")
            .expect("应找到下一组注释")
            + start;
        let row = &src[start..end];
        assert!(!row.contains("if i == 2"), "三按钮不应有 i==2 特殊分支");
        assert!(!row.contains(".fill(accent)"), "三按钮不应有 accent 填充");
        // 注意：#[cfg(test)] 的宽度断言里也用 Color32::WHITE 量文本，不能据此判断按钮样式。
    }

    /// 验算快照图：首尾补真实帧号边界点后自动边界覆盖全窗口，异常点仍在真实帧号且居中。
    #[test]
    fn verify_plot_points_padded_and_center_kept() {
        use crate::ui::plot::pad_points_to_x_range;
        // 模拟第 5 帧、center=4：x0=1，5 个点 x=1..5（异常点 x=5，y=8）。
        let frame_no = 5u64;
        let base = verify_real_x_base(frame_no, 4);
        let pts: Vec<[f64; 2]> = (0..5)
            .map(|i| [base + i as f64, (4 + i) as f64])
            .collect();
        assert_eq!(pts.first().unwrap()[0], 1.0);
        assert_eq!(pts.last().unwrap(), &[5.0, 8.0]);
        let bounds = verify_x_bounds(frame_no, 50, 50); // [-45, 55]
        let padded = pad_points_to_x_range(&pts, bounds);
        assert_eq!(padded.first().unwrap()[0], -45.0, "首点应补到 frame-50=-45");
        assert_eq!(padded.last().unwrap()[0], 55.0, "末点应补到 frame+50=55");
        assert_eq!(padded.first().unwrap()[1], 4.0, "首补点 y 取通道首值");
        assert_eq!(padded.last().unwrap()[1], 8.0, "末补点 y 取通道末值");
        // 异常点（真实帧号 x=5，y=8）仍在，且是 x 范围的中心。
        let center_pt = padded.iter().find(|p| p[0] == frame_no as f64).unwrap();
        assert_eq!(*center_pt, [5.0, 8.0], "异常点必须在真实帧号 x=5 且 y 不变");
        assert_eq!((bounds[0] + bounds[1]) * 0.5, frame_no as f64, "异常点严格居中");
        assert!(padded.len() >= pts.len() + 2);
        // 已经铺满 [frame-50, frame+50] 时不再重复补点。
        let full: Vec<[f64; 2]> = (50..=150).map(|x| [x as f64, 1.0]).collect();
        assert_eq!(pad_points_to_x_range(&full, [50.0, 150.0]).len(), full.len());
    }

    /// 静态回归：验算图不得在闭包内 set_plot_bounds（会锁死 Y 并抢走缩放），
    /// 且必须传 show_points=true、真实帧号 marker_x、以帧号居中的 x_bounds。
    #[test]
    fn verify_plot_no_forced_bounds_and_shows_points() {
        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        assert!(
            !plot.contains("[lo, b.min()[1]]"),
            "不得在闭包内 set_plot_bounds 锁住默认 Y=-1~0"
        );
        assert!(
            plot.contains("pad_points_to_x_range"),
            "应改用首尾补边界点让自动边界覆盖窗口"
        );
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        assert!(
            app.contains("true, // 像主图暂停那样叠加实心数据点"),
            "验算快照图必须 show_points=true"
        );
        assert!(
            app.contains("Some((a.frame_no as f64, marker_ys))"),
            "验算快照图必须在真实帧号标出异常点大红圈"
        );
        assert!(
            app.contains("Some(verify_x_bounds(a.frame_no, before, after))"),
            "x_bounds 必须以真实帧号为中心且按用户设置的前/后帧数变化"
        );
        // 只检查生产代码区（测试源码里也写了这个字面量用于断言，不能算进来）。
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        assert!(
            !prod.contains("Some([-50.0, 50.0])"),
            "不得再使用相对坐标的固定 [-50,50] 范围"
        );
    }

    /// 结论提示：「未发现丢帧」与「发现 N 处异常」同样加粗 + 同字号；
    /// 非法字符提示优先于「未发现丢帧」，中英文都一致。
    #[test]
    fn verify_no_loss_heading_is_prominent() {
        let app_src = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app_src[..app_src.find("mod layout_tests").expect("应找到测试模块")];
        assert!(prod.contains(".strong()"), "结论提示必须加粗");
        assert!(prod.contains(".size(VERIFY_HEADING_SIZE)"), "结论提示必须大字号");

        for lang in [Lang::Zh, Lang::En] {
            assert_eq!(
                verify_result_hint(lang, false, 0),
                Some((tr(lang, "verify_no_loss").to_string(), false)),
                "干净数据无异常才显示未发现丢帧"
            );
            let invalid = Some((tr(lang, "verify_invalid_data").to_string(), true));
            assert_eq!(
                verify_result_hint(lang, true, 0),
                invalid,
                "乱码无异常时必须显示非法字符，不能说未发现丢帧"
            );
            assert_eq!(
                verify_result_hint(lang, true, 3),
                invalid,
                "乱码即使切出异常也应以非法字符提示为主"
            );
            assert_eq!(
                verify_result_hint(lang, false, 3),
                None,
                "干净数据有异常走「发现 N 处异常」标题"
            );
        }
    }

    /// 非法字符检测：乱码（含非 hex 字母）必须置标志，干净/含空白/BOM 数据不得误报。
    #[test]
    fn verify_invalid_char_detection() {
        // 乱码：fadfdsa / sa 里有非 hex 的 s；不能静默跳过。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(b"eb 90 fadfdsafdsafdsa\nfdsa\nsa\n");
        assert!(r.has_invalid_char, "含非 hex 字符的乱码必须记录非法标志");

        // 干净纯 hex（带帧头，codes_hex 生成空格分隔）不误报。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(codes_hex(&[0, 1, 2, 5, 6]).as_bytes());
        assert!(!r.has_invalid_char, "纯 hex 数据不得误报");

        // 空格/制表/回车/换行都是合法分隔，不算非法。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(b"\tEB\r\n90 00 01\t02\n");
        assert!(!r.has_invalid_char, "空白分隔不得误报");

        // 文件头 UTF-8 BOM 合法。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(b"\xef\xbb\xbfEB 90 00 01");
        assert!(!r.has_invalid_char, "BOM 不得误报");

        // 0x 前缀：x 非 hex，现有格式是纯 hex 空格分隔，按非法处理。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(b"0xEB 0x90");
        assert!(r.has_invalid_char, "0x 前缀的 x 按非法字符处理");

        // 末尾落单的半字节（单独一个 f）数据不完整，finish 时也要记非法。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(b"EB 90 0");
        let (_, invalid) = r.finish_with_status();
        assert!(invalid, "末尾落单半字节应视为数据不完整");

        // 干净数据 finish 后仍为 false。
        let mut r = VerifyRunner::new(verify_engine(), 200);
        r.feed(codes_hex(&[0, 1]).as_bytes());
        let (_, invalid) = r.finish_with_status();
        assert!(!invalid);
    }

    /// 按钮改名 + 字号：analyze_clip 改为「剪贴板绘图」，三按钮字号 11pt 且渲染/断言共用同一常量。
    #[test]
    fn analyze_clip_label_and_button_font_size() {
        let i18n = std::fs::read_to_string("src/ui/i18n.rs").expect("应能读到 i18n.rs");
        assert!(
            i18n.contains("(\"analyze_clip\", \"剪贴板绘图\", \"Clipboard Plot\")"),
            "剪贴板按钮文案应改为「剪贴板绘图」"
        );
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        assert!(prod.contains("const OFFLINE_BTN_FONT_SIZE: f32 = 11.0;"));
        assert!(prod.contains(".size(OFFLINE_BTN_FONT_SIZE)"), "按钮渲染应使用该字号");
        assert!(
            prod.contains("FontId::proportional(OFFLINE_BTN_FONT_SIZE)"),
            "宽度断言必须与渲染同字号"
        );
    }

    /// 补点不可见：边界点只进全透明辅助线撑开自动边界，可见曲线只用真实点；
    /// 不用 NaN 断线（egui_plot 0.29 的 Line::shapes 不过滤 NaN 顶点）。
    #[test]
    fn verify_padding_points_are_invisible() {
        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        assert!(plot.contains("Color32::TRANSPARENT"), "补点辅助线必须全透明");
        assert!(
            plot.contains("let pts = raw;"),
            "可见曲线/数据点必须只用真实点，不画补点"
        );
        assert!(
            plot.contains("pad_points_to_x_range(&raw, range)"),
            "辅助线仍用补点撑开坐标范围"
        );
    }

    /// 前后帧数：默认 50，clamp 到 1..=500。
    #[test]
    fn verify_frames_from_text_default_and_clamp() {
        assert_eq!(verify_frames_from_text("50"), 50);
        assert_eq!(verify_frames_from_text(""), 50, "空值回退默认 50");
        assert_eq!(verify_frames_from_text("abc"), 50, "非法回退默认 50");
        assert_eq!(verify_frames_from_text("0"), 50, "0 非法回退默认 50");
        assert_eq!(verify_frames_from_text("1"), 1, "下限 1");
        assert_eq!(verify_frames_from_text("20"), 20);
        assert_eq!(verify_frames_from_text("500"), 500, "上限 500");
        assert_eq!(verify_frames_from_text("501"), 500, "超过上限 clamp 到 500");
        assert_eq!(verify_frames_from_text("99999"), 500);
    }

    /// 可变窗口：x_bounds 按前/后帧数变化，异常点 x 仍是真实帧号；快照截取与边界帧正确。
    #[test]
    fn verify_variable_window_bounds_and_slice() {
        let b = verify_x_bounds(41, 20, 30);
        assert_eq!(b, [21.0, 71.0], "前 20 后 30 → [frame-20, frame+30]");
        // 前后不等时异常点不必是 x 范围绝对正中，但 x 必须等于真实帧号。
        assert!(b[0] < 41.0 && b[1] > 41.0);
        let d = verify_x_bounds(41, 50, 50);
        assert_eq!(d, [-9.0, 91.0]);
        assert_eq!((d[0] + d[1]) * 0.5, 41.0, "默认 50/50 时严格居中");

        // 快照按最大窗口存 100 点、center=50；前 10 后 20 → 片段 40..70，新中心 10。
        let snap: Vec<Vec<i64>> = vec![(0..100).map(|i| i as i64).collect()];
        let (view, center) = verify_slice_snapshot(&snap, 50, 10, 20);
        assert_eq!(view[0].len(), 31);
        assert_eq!(view[0].first(), Some(&40));
        assert_eq!(view[0].last(), Some(&70));
        assert_eq!(center, 10);
        assert_eq!(
            verify_real_x_base(41, center) + center as f64,
            41.0,
            "截取后异常点仍在真实帧号"
        );
        // 边界帧：前 20 帧不足时从 0 截起，新中心随之调整。
        let (view2, center2) = verify_slice_snapshot(&snap, 5, 20, 20);
        assert_eq!(view2[0].first(), Some(&0));
        assert_eq!(center2, 5);
        // 空快照不 panic。
        let (empty, c) = verify_slice_snapshot(&[], 0, 50, 50);
        assert!(empty.is_empty());
        assert_eq!(c, 0);
    }

    /// 未配置循环码时必须提示去帧配置填写，不能显示「未发现丢帧」。
    #[test]
    fn verify_no_cycle_config_shows_guide_not_no_loss() {
        let mut app = FrameApp::new();
        app.cycle_pos_text.clear();
        app.start_verify(std::path::PathBuf::from("no-such-file.txt"));
        let st = app.verify.as_ref().expect("应弹出验算窗口");
        assert!(!st.running, "不应启动后台验算");
        assert!(st.anomalies.is_empty());
        assert_eq!(st.msg, tr(app.lang, "verify_no_cycle"));
        assert_ne!(
            st.msg,
            tr(app.lang, "verify_no_loss"),
            "无循环码配置时绝不能说「未发现丢帧」"
        );
        assert_eq!(verify_frames_from_text(&st.before_text), 50);
        assert_eq!(verify_frames_from_text(&st.after_text), 50);
    }

    /// 验算图高度：由整窗高度直接计算（右栏顶部无横条占位），下限为绘图正方形 580，无上限。
    #[test]
    fn verify_plot_height_adapts_to_window() {
        assert_eq!(verify_plot_height(1000.0), 1000.0, "底层高度不留余量，绘图底边贴弹窗底边");
        assert_eq!(verify_plot_height(10.0), VERIFY_PLOT_MIN_SIZE, "下限为绘图正方形边长");
        // 从整窗内容高度算：拖到 1000 图高 972（减 28px 顶部开销）；小窗守住 580。
        assert_eq!(verify_plot_height_from_window(1000.0), 972.0, "弹窗拖大图同步变高，无上限");
        assert_eq!(verify_plot_height_from_window(400.0), VERIFY_PLOT_MIN_SIZE, "小窗守住正方形下限");
        assert_eq!(verify_plot_height_from_window(100.0), VERIFY_PLOT_MIN_SIZE);
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let call = prod.find("\"verify_plot\"").expect("应找到验算图调用");
        let call_tail = &prod[call..call + 700];
        assert!(
            call_tail.contains("verify_plot_height_from_window(window_h)"),
            "验算图高度必须由整窗高度算，不受嵌套 Ui 受限的 available_height 影响"
        );
        assert!(
            !call_tail.contains("right.available_height()"),
            "不得再从 right.available_height() 取高度（会卡在极限）"
        );
        assert!(
            !call_tail.contains("260.0"),
            "验算图高度不得再写死 260.0"
        );
    }

    /// 绘图区最小边长 >= 两个「异常帧：第 3943 帧（图中红线）」文字长度（截图约 287px × 2）。
    #[test]
    fn verify_plot_min_size_covers_two_frame_labels() {
        const FRAME_LABEL_W: f32 = 287.0;
        assert!(
            VERIFY_PLOT_MIN_SIZE >= 2.0 * FRAME_LABEL_W,
            "绘图下限须 >= 两个异常帧红字长度，否则红字会竖排/图被挤成竖线"
        );
        assert!(
            (560.0..=580.0).contains(&VERIFY_PLOT_MIN_SIZE),
            "最小边长应落在 560~580 区间"
        );
    }

    /// 验算图关掉实心背景（透出半透明窗口底板），主图保留背景；函数签名不变。
    #[test]
    fn verify_plot_background_only_off_for_verify() {
        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        assert!(
            plot.contains(".show_background(id_base != \"verify_plot\")"),
            "验算图须关极端底色实心矩形，主图 data_plot 须保留背景"
        );
        assert!(
            plot.contains("fn draw_plot_with_id(") && plot.contains("id_base: &str"),
            "不得改函数签名（主图调用与 &mut reset 静态断言依赖它）"
        );
    }

    /// 坐标轴与异常图例中文化：从 i18n 取词条，主图与验算图都跟随界面语言。
    #[test]
    fn verify_plot_labels_follow_language() {
        use crate::ui::i18n::{tr, Lang};
        assert_eq!(tr(Lang::Zh, "plot_x_label"), "帧号");
        assert_eq!(tr(Lang::En, "plot_x_label"), "Frame Number");
        assert_eq!(tr(Lang::Zh, "plot_y_label"), "数值");
        assert_eq!(tr(Lang::En, "plot_y_label"), "Value");
        assert_eq!(tr(Lang::Zh, "plot_anomaly"), "异常点");
        assert_eq!(tr(Lang::En, "plot_anomaly"), "Anomaly");

        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        assert!(!plot.contains(".x_axis_label(\"Frame Number\")"), "横轴不得写死英文");
        assert!(!plot.contains(".y_axis_label(\"Value\")"), "纵轴不得写死英文");
        assert!(!plot.contains(".name(\"Anomaly\")"), "图例名不得写死英文");
        for key in ["plot_x_label", "plot_y_label", "plot_anomaly"] {
            assert!(plot.contains(&format!("tr(lang, \"{}\")", key)), "须用 lang 取 {}", key);
        }
        assert!(plot.contains("lang: Lang"), "两个绘图函数都要带 lang 参数");

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        for needle in [
            "crate::ui::plot::draw_plot_with_id(",
            "crate::ui::plot::draw_plot(",
        ] {
            let pos = app.find(needle).expect("应找到绘图调用");
            let tail: String = app[pos..].lines().take(16).collect::<Vec<_>>().join("\n");
            assert!(tail.contains("self.lang"), "{} 调用须传 self.lang", needle);
        }
    }

    /// 标题行滑块是短控件（不撑满）、无字样无数字；前后帧短框和清单保持原样。
    #[test]
    fn verify_left_column_layout_reverted_with_slider_in_title_row() {
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let start = prod.find("fn verify_window(").expect("应找到 verify_window");
        let end = prod.find("// ============ 面板绘制").expect("应找到面板绘制分界");
        let win = &prod[start..end];
        // 滑块是普通 ui.add 短控件，不再 add_sized 撑满。
        assert_eq!(
            win.matches(".show_value(false)").count(),
            2,
            "两处滑块都不显示 100% 数字"
        );
        assert_eq!(
            win.matches("tr(self.lang, \"verify_opacity\")").count(),
            0,
            "界面不再渲染「透明度/Opacity」字样"
        );
        assert_eq!(win.matches("egui::Slider::new(").count(), 2, "滑块须只在两处标题行");
        for key in ["verify_invalid_data", "verify_found"] {
            let needle = format!("tr(self.lang, \"{}\")", key);
            let pos = win.find(&needle).expect("应找到标题词条");
            let tail: String = win[pos..].lines().take(16).collect::<Vec<_>>().join("\n");
            assert!(tail.contains("egui::Slider::new("), "{} 标题行须带滑块", key);
            assert!(tail.contains("show_value(false)"), "{} 标题行滑块须隐藏数字", key);
            assert!(!tail.contains("add_sized("), "{} 标题行滑块不得撑满", key);
            assert!(!tail.contains("verify_opacity"), "{} 标题行不得有字样", key);
        }
        // 前后帧行仍是两个 44.0 短框，清单仍按文字宽，二者都不撑满。
        assert_eq!(
            win.matches("TextEdit::singleline(&mut state.before_text).desired_width(44.0)").count(),
            1
        );
        assert_eq!(
            win.matches("TextEdit::singleline(&mut state.after_text).desired_width(44.0)").count(),
            1
        );
        assert!(
            win.contains("ui.selectable_label(state.selected == Some(i), line).clicked()"),
            "清单须保持按文字宽的 selectable_label"
        );
    }

    /// 可拖动分隔线：默认 160（= 拖动下限）、clamp 到 [160, total-260]、配置缺省回退 160、松手持久化。
    #[test]
    fn verify_split_width_default_clamp_and_persist() {
        assert_eq!(VERIFY_SPLIT_DEFAULT, 160.0, "默认左栏取拖动下限 160");
        assert_eq!(VERIFY_SPLIT_DEFAULT, VERIFY_SPLIT_MIN, "默认即最小极限");
        assert_eq!(default_verify_split_w(), 160.0, "老配置缺字段时回退 160");
        assert_eq!(verify_split_width(904.0, 160.0), 160.0);
        assert_eq!(verify_split_width(904.0, 1000.0), 644.0, "上限 total-260，保住绘图区");
        assert_eq!(verify_split_width(904.0, 20.0), 160.0, "下限 160");
        assert_eq!(verify_split_width(300.0, 160.0), 160.0, "窗口过窄时下限优先");

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let win = &prod[prod.find("fn verify_window(").unwrap()..];
        assert!(win.contains("egui::Sense::drag()"), "分隔条须可拖动");
        assert!(win.contains("sep_resp.drag_delta().x"), "拖动增量改左栏宽度");
        assert!(win.contains("self.verify_split_w = verify_split_width("), "拖动后须 clamp");
        assert!(
            win.contains("sep_resp.drag_stopped()") && win.contains("self.save_config()"),
            "松手须持久化，不能每帧写文件"
        );
        assert!(
            prod.contains("verify_split_w: self.verify_split_w,"),
            "save_config 须写入分隔线位置"
        );
        assert!(
            prod.contains("self.verify_split_w = verify_split_width(920.0, cfg.verify_split_w)"),
            "load_config 须恢复分隔线位置"
        );
        assert!(
            prod.contains("#[serde(default = \"default_verify_split_w\")]"),
            "PersistConfig 缺字段时须回退 160"
        );
    }

    /// 窗口 min_size 推导约束：min 高 >= 顶部开销 + 绘图下限 + 余量；min 宽 >= 默认左栏
    /// + 绘图下限 + 分隔 + 内边距；且 min <= default（否则一打开就超限）。
    #[test]
    fn verify_window_min_size_constraints() {
        assert!(
            VERIFY_WINDOW_MIN_H >= VERIFY_TOP_OVERHEAD + VERIFY_PLOT_MIN_SIZE + 24.0,
            "min 高须容纳顶部开销 + 绘图下限 + 标题栏/内边距余量"
        );
        assert!(
            VERIFY_WINDOW_MIN_W >= VERIFY_SPLIT_DEFAULT + VERIFY_PLOT_MIN_SIZE + 6.0 + 16.0,
            "min 宽须容纳默认左栏 + 绘图下限 + 分隔 + 内边距"
        );
        assert!(VERIFY_WINDOW_MIN_W <= VERIFY_WINDOW_DEFAULT_W, "min 宽不得大于 default");
        assert!(VERIFY_WINDOW_MIN_H <= VERIFY_WINDOW_DEFAULT_H, "min 高不得大于 default");

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let win = &prod[prod.find("fn verify_window(").expect("应找到 verify_window")..];
        assert!(win.contains(".min_size(["), "验算弹窗必须设最小尺寸");
        assert!(
            win[..win.find(".show(ctx").unwrap()].contains(".min_size([VERIFY_WINDOW_MIN_W, VERIFY_WINDOW_MIN_H])"),
            "min_size 必须使用推导常量"
        );
    }

    /// 透明度默认值与范围：默认 80，滑块 20..=100，越界值由纯函数 clamp。
    #[test]
    fn verify_opacity_default_and_range() {
        assert_eq!(VERIFY_OPACITY_DEFAULT, 80, "默认 80 一打开就是轻微透");
        assert_eq!(VERIFY_OPACITY_MIN, 20);
        assert_eq!(VERIFY_OPACITY_MAX, 100);
        assert!(
            VERIFY_OPACITY_DEFAULT > VERIFY_OPACITY_MIN && VERIFY_OPACITY_DEFAULT < VERIFY_OPACITY_MAX
        );
        // 越界输入 clamp 到边界：不会让窗口消失，也不会超过 u8 alpha。
        assert_eq!(verify_opacity_alpha(0), 51, "0 被 clamp 到 20 → 51");
        assert_eq!(verify_opacity_alpha(255), 255, "255 被 clamp 到 100 → 255");
    }

    /// 百分比 → alpha 换算正确，且窗口链静态检查：.fill + unpremultiplied + 跟随主题 + 滑块词条。
    #[test]
    fn verify_opacity_alpha_mapping_and_window_wiring() {
        assert_eq!(verify_opacity_alpha(100), 255, "100% 完全不透明，与原观感一致");
        assert_eq!(verify_opacity_alpha(80), 204, "80% → 204");
        assert_eq!(verify_opacity_alpha(20), 51, "20% → 51");
        assert_eq!(verify_opacity_alpha(50), 128, "50% 四舍五入 127.5 → 128");
        for pct in VERIFY_OPACITY_MIN..=VERIFY_OPACITY_MAX {
            let a = verify_opacity_alpha(pct);
            assert!(a >= 51, "alpha 须 >= 51（u8 天然 <= 255），got {}", a);
        }

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let win = &prod[prod.find("fn verify_window(").expect("应找到 verify_window")..];
        assert!(
            win.contains(".frame(egui::Frame::window(&style).fill(verify_fill))"),
            "egui 0.29 须用 Frame::window(...).fill(...) 设置底板透明度"
        );
        assert!(
            win.contains("egui::Color32::from_rgba_unmultiplied("),
            "fill 颜色须用 unpremultiplied alpha"
        );
        assert!(
            win.contains("style.visuals.window_fill()"),
            "底板颜色须跟随当前主题 visuals，不能写死 rgb"
        );
        assert!(
            win.contains("egui::Slider::new(") && win.contains(".show_value(false)"),
            "透明度滑块须隐藏数字，界面不再显示「透明度」字样"
        );
        let i18n = std::fs::read_to_string("src/ui/i18n.rs").expect("应能读到 i18n.rs");
        assert!(
            i18n.contains("(\"verify_opacity\", \"透明度\", \"Opacity\")"),
            "中英词条须齐备，中文不动其它词条"
        );
    }

    /// 重置链路全链路：事件置位 → X=[frame-前N, frame+后N] → plot.rs 先设边界再消费标志。
    #[test]
    fn verify_reset_chain_full_path() {
        // 1) 改帧数置位；无事件时保持，不覆盖用户缩放。
        let mut flag = false;
        flag |= verify_reset_on_event(false, true, false);
        assert!(flag, "改前后帧数必须置位");
        let mut idle = false;
        idle |= verify_reset_on_event(false, false, false);
        assert!(!idle, "无事件不得重置，保留用户缩放/拖动");

        // 2) X 边界由帧号与前后帧纯函数决定，换条目/改帧数都走同一函数。
        assert_eq!(verify_x_bounds(41, 50, 50), [-9.0, 91.0]);
        assert_eq!(verify_x_bounds(41, 3, 7), [38.0, 48.0]);

        // 3) plot.rs：只有真正算出真实边界才消费标志，空帧不吞掉重置。
        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        let reset_at = plot.rfind("if *reset_bounds {").expect("重置分支必须存在");
        let tail: String = plot[reset_at..]
            .lines()
            .take(12)
            .collect::<Vec<_>>()
            .join("\n");
        let set_at = tail.find("pui.set_plot_bounds(").expect("重置分支必须设边界");
        let consume_at = tail.find("*reset_bounds = false;").expect("设边界后须消费标志");
        assert!(set_at < consume_at, "必须先设边界再消费，空帧不吞重置");
    }

    /// 三类事件（换异常条目 / 改前后帧数 / 验算完成）都必须置位坐标重置标志，
    /// 且标志随 VerifyState 存取形成闭环（不再放 FrameApp 上造成链路断点）。
    #[test]
    fn verify_reset_bounds_flag_set_on_events() {
        // 纯函数全组合：任一事件为真即重置，全假才保留缩放。
        assert!(!verify_reset_on_event(false, false, false));
        assert!(verify_reset_on_event(true, false, false));
        assert!(verify_reset_on_event(false, true, false));
        assert!(verify_reset_on_event(false, false, true));
        assert!(verify_reset_on_event(true, true, true));

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        // 换异常条目
        let sel = prod
            .find("if ui.selectable_label(state.selected == Some(i), line).clicked()")
            .expect("应找到条目点击分支");
        // 按行取片段，避免固定字节窗口踩到中文注释的 UTF-8 多字节边界。
        let sel_tail: String = prod[sel..].lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(
            sel_tail.contains("state.reset_bounds |= verify_reset_on_event(true, false, false)"),
            "换异常条目必须置位 state.reset_bounds"
        );
        // 改前后帧数输入框
        let inp = prod
            .find("if state.before_text != before_prev || state.after_text != after_prev")
            .expect("应找到前后帧数变化分支");
        let inp_tail: String = prod[inp..].lines().take(8).collect::<Vec<_>>().join("\n");
        assert!(
            inp_tail.contains("state.reset_bounds |= verify_reset_on_event(false, true, false)"),
            "改前后帧数必须置位 state.reset_bounds"
        );
        // 验算完成（poll_verify 的 Done 分支）
        assert!(
            prod.contains("st.reset_bounds |= verify_reset_on_event(false, false, true)"),
            "验算完成时必须置位 state.reset_bounds"
        );
        // 绘图调用：本帧取用 → 传入 → 写回，闭环不丢。
        assert!(prod.contains("let mut reset = state.reset_bounds;"), "须从 state 取用标志");
        assert!(prod.contains("&mut reset,"), "标志须传入 draw_plot_with_id");
        assert!(prod.contains("state.reset_bounds = reset;"), "绘图后须写回 state");
        assert!(
            !prod.contains("self.verify_reset_bounds"),
            "标志不得留在 FrameApp 上，否则 take()/写回会形成链路断点"
        );
    }

    /// 重置帧算出真实边界：截图2 的第 41 帧、前/后各 500 → X=[-459,541]，Y 为数据实际范围。
    #[test]
    fn verify_reset_uses_real_data_bounds() {
        use crate::ui::plot::{bounds_with_y_margin, compute_series_bounds};
        let frame_no = 41u64;
        let before = 500usize;
        let after = 500usize;
        let bounds = verify_x_bounds(frame_no, before, after);
        assert_eq!(bounds, [-459.0, 541.0], "截图2 的正确 X 范围");

        let x0 = verify_real_x_base(frame_no, before); // 异常点在快照中的下标为 500
        let series: Vec<(Option<String>, Vec<f64>, usize, f64)> = vec![(
            Some("CH1".into()),
            (0..1001).map(|i| i as f64).collect(),
            0,
            x0,
        )];
        let real = compute_series_bounds(&series, Some(bounds), Some((frame_no as f64, vec![500.0])));
        assert_eq!(real, Some([-459.0, 541.0, 0.0, 1000.0]), "真实范围须含辅助点 x 与全部真实 y");
        let [min, max] = bounds_with_y_margin(real.unwrap());
        assert_eq!([min[0], max[0]], [-459.0, 541.0], "X 必须严格是 [frame-前N, frame+后N]");
        assert!(min[1] < 0.0 && max[1] > 1000.0, "Y 才留边距");

        let plot = std::fs::read_to_string("src/ui/plot.rs").expect("应能读到 plot.rs");
        // 有两个 if *reset_bounds（缩放分支短路 + 末尾真正设边界），取最后一个。
        let reset_at = plot
            .rfind("if *reset_bounds {")
            .expect("重置分支必须存在");
        let set_at = plot[reset_at..]
            .find("pui.set_plot_bounds(")
            .expect("重置分支必须调用 set_plot_bounds");
        assert!(
            plot[reset_at..reset_at + set_at].contains("compute_series_bounds"),
            "重置必须用数据真实范围，不能用 plot_bounds() 默认 Y=-1~0"
        );
        // 非重置帧不得设边界：全文件只有两处 set_plot_bounds（缩放复位 + 重置帧）。
        assert_eq!(
            plot.matches("pui.set_plot_bounds(").count(),
            2,
            "除一次性缩放复位外，只允许重置帧设边界"
        );
    }

    /// 结构性回归：验算完成后是「左清单 / 右绘图」；结论/输入横条移入左栏，右栏从弹窗
    /// 顶部直接开始（顶部无空白），绘图高度由整窗高度算填满右栏；
    /// running 进度与 msg 提示仍在分栏之外横跨整窗。
    #[test]
    fn verify_window_uses_left_list_right_plot_layout() {
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let win = &prod[prod
            .find("fn verify_window(")
            .expect("应找到 verify_window")..];
        assert!(win.contains("ui.horizontal(|ui| {"), "完成后的内容区必须左右两栏");
        assert!(
            win.contains("verify_split_width(total_w, self.verify_split_w)"),
            "分栏宽度须用可拖动的 verify_split_w 经纯函数 clamp 计算"
        );
        assert!(
            win.contains("allocate_ui_with_layout"),
            "左栏须分配固定宽度，剩余给右栏"
        );

        let cols = win.find("ui.horizontal(|ui| {").unwrap();
        let rv = win.find("ui.vertical(|right| {").expect("应找到右栏 vertical");
        assert!(rv > cols, "右栏 vertical 必须在分栏内");
        let cols_tail = &win[cols..];
        assert!(
            cols_tail.contains("ScrollArea::vertical()"),
            "左栏必须放可滚动的异常清单"
        );
        assert!(
            cols_tail.contains("auto_shrink([false, false])"),
            "清单须填满左栏可用高度，条数多少都不影响右栏"
        );
        assert!(
            !cols_tail.contains("max_height(240.0)"),
            "不得再用固定 240 高度限制清单/绘图"
        );
        assert!(
            cols_tail.contains("verify_plot_height_from_window(window_h)"),
            "绘图高度须由整窗高度算，填满右栏到底部"
        );
        assert!(
            cols_tail.contains("ui.selectable_label(state.selected == Some(i), line).clicked()"),
            "点条目更新右栏图的逻辑须保留"
        );

        // 右栏顶部无空白：整窗结论/输入横条必须在分栏起点与右栏 vertical 之间（即左栏内）。
        let left_region = &win[cols..rv];
        for key in ["verify_found", "verify_before", "verify_after"] {
            assert!(
                left_region.contains(key),
                "结论/前后帧输入须移入左栏，否则右栏顶部留大块空白"
            );
        }
        let right_region = &win[rv..];
        let detail_rel = right_region
            .find("right.horizontal(|ui| {")
            .expect("右栏首元素须是 horizontal（说明+红字帧号并一行）");
        let right_head = &right_region[..detail_rel];
        assert!(!right_head.contains("TextEdit"), "右栏顶部不得有输入框占位");
        assert!(!right_head.contains("ui.label"), "右栏顶部不得有多余 label 占位");
        assert!(
            right_head.contains("let before = verify_frames_from_text"),
            "右栏顶部紧接帧数读取与详情，不留空白"
        );
        // 并一行结构：这个 horizontal 内须同时含 verify_detail 与 verify_center_frame，
        // 且红字帧号仍 strong + 红色；防止回退成上下两行。
        let head_line = &right_region[detail_rel..];
        let close_rel = head_line.find("});").expect("并一行的 horizontal 须闭合");
        let head_line = &head_line[..close_rel];
        assert!(head_line.contains("verify_detail"), "说明文字须在并一行内");
        assert!(head_line.contains("verify_center_frame"), "红字帧号须在并一行内");
        assert!(head_line.contains(".strong()"), "帧号须保持 strong");
        assert!(head_line.contains("0xE0, 0x40, 0x40"), "帧号须保持红色");

        // running / msg 分支在 columns 之前，保持整窗横跨。
        assert!(win.find("if state.running").unwrap() < cols);
        assert!(win.find("!state.msg.is_empty()").unwrap() < cols);
    }

    /// 异常清单：固定描边框包住可滚动列表，条数少时框仍在，条数多时框内滚动；
    /// 只包清单，不碰上方标题/前后帧输入与右栏绘图。
    #[test]
    fn verify_anomaly_list_has_fixed_frame() {
        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let win = &prod[prod.find("fn verify_window(").unwrap()..];
        let frame_at = win.find("egui::Frame::none()").expect("清单须有固定框");
        let scroll_at = win[frame_at..]
            .find("ScrollArea::vertical()")
            .expect("红框内须是可滚动清单");
        let close_at = win[frame_at..]
            .find("ui.selectable_label(state.selected == Some(i), line).clicked()")
            .expect("清单点击逻辑须在框内");
        assert!(close_at > scroll_at, "点选须发生在红框内的滚动列表里");
        let frame_part = &win[frame_at..frame_at + scroll_at];
        assert!(
            frame_part.contains(".stroke(frame_stroke)"),
            "框须用描边变量，不再写死红色（红框曾导致缩窗反弹）"
        );
        assert!(
            win[..frame_at].contains("noninteractive.bg_stroke"),
            "描边色须取自软件统一配色，不再写死红色"
        );
        assert!(frame_part.contains(".stroke("), "框须有描边而非仅背景");
        assert!(
            frame_part.contains("inner_margin"),
            "框内留边距，条目文字不能贴着边框"
        );
        assert!(
            win[frame_at..].contains("auto_shrink([false, false])"),
            "框内清单仍须填满左栏可用高度"
        );
        assert!(
            !win[frame_at..].contains("set_min_height(list_left_h"),
            "不得再 set_min_height——它会把内容最小高度锁死，导致拖小弹窗松手后反弹"
        );
        assert!(
            win.contains("egui::vec2(list_w, window_h)"),
            "左栏须用整窗高度定高，不能用会收缩到内容的 ui.available_height()"
        );
        // 前后帧输入在框之前，右栏 vertical 在框之后：框不会挤压上方与右栏。
        let before_at = win.find("TextEdit::singleline(&mut state.before_text)").unwrap();
        let right_at = win.find("ui.vertical(|right| {").unwrap();
        assert!(before_at < frame_at && right_at > frame_at, "红框只能夹在输入与右栏之间");
    }

    /// 状态栏：绘图状态常驻两态且暂停时才醒目动态；空闲轮播快捷提示、不再常驻操作格。
    #[test]
    fn status_bar_plot_state_and_idle_tip_rotation() {
        for lang in [Lang::Zh, Lang::En] {
            assert!(!tr(lang, "plot_state_live").is_empty(), "实时绘图文案缺失");
            assert!(!tr(lang, "plot_state_paused").is_empty(), "绘图已暂停文案缺失");
            assert!(!tr(lang, "chk_no").is_empty(), "无校验文案缺失");
            // 16 条轮播提示每种语言都须有文案，且两两不同（不是占位符）
            let mut seen = std::collections::HashSet::new();
            for k in STATUS_TIP_KEYS {
                let t = tr(lang, k);
                assert!(!t.is_empty(), "轮播提示 {} 缺失", k);
                assert!(seen.insert(t), "轮播提示 {} 文案重复: {}", k, t);
            }
        }
        assert_eq!(STATUS_TIP_KEYS.len(), 16, "用户敲定 16 条轮播提示");

        // 纯函数切换：0/1/2 秒同一条，3 秒换下一条，3*len 回绕。
        let n = STATUS_TIP_KEYS.len();
        assert_eq!(status_tip_index(0, n), 0);
        assert_eq!(status_tip_index(2, n), 0);
        assert_eq!(status_tip_index(3, n), 1);
        assert_eq!(status_tip_index(6, n), 2);
        assert_eq!(status_tip_index((STATUS_TIP_SECS * n as u64) as u64, n), 0, "轮播须回绕");
        assert_eq!(status_tip_index(7, 0), 0, "空列表不 panic");

        let app = std::fs::read_to_string("src/ui/app.rs").expect("应能读到 app.rs");
        let prod = &app[..app.find("mod layout_tests").expect("应找到测试模块")];
        let line = &prod[prod.find("fn status_line(").unwrap()..prod.find("fn draw_plot_state(").unwrap()];
        let state_at = line.find("self.draw_plot_state(ui, plot_state_w, h)").expect("须有绘图状态格");
        let light_at = line.find("self.draw_record_light(ui, light_w, h)").expect("须有记录灯");
        assert!(state_at < light_at, "绘图状态须在「未记录」灯前面");
        assert!(!line.contains("plot_op_hint"), "TX 前的常驻操作提示格须去掉，改为最近操作格空闲轮播");
        assert!(line.contains("RECENT_MAX_W"), "最近操作格须设宽度上限，超长回执不撑爆 TX");
        assert!(line.contains("request_repaint_after"), "空闲轮播须定时请求重绘才能切换");
        assert!(line.contains("egui::Color32::from_gray(120)"), "轮播提示须用更淡的灰");

        let state_fn = &prod[prod.find("fn draw_plot_state(").unwrap()..];
        assert!(state_fn.contains("self.plot_paused"), "状态须取自真实暂停标志");
        assert!(state_fn.contains("request_repaint()"), "暂停呼吸须持续请求重绘");
        assert!(state_fn.contains("ui.input(|i| i.time)"), "呼吸须由时间驱动");
    }

    /// 用户实测：非法数据触发的解析错误回执曾把整串原文塞进状态栏撑爆 TX；
    /// 现在只显示固定短句，且不带 `{}`（不会再嵌入原始输入）。
    #[test]
    fn parse_error_receipt_is_short_and_drops_raw_input() {
        for lang in [Lang::Zh, Lang::En] {
            let t = tr(lang, "offline_parse_err");
            assert!(!t.contains("{}"), "错误短句不得再带占位符: {}", t);
            assert!(t.len() <= 60, "错误短句须足够短: {}", t);
        }
        // fmt_str 对无固定槽的模板应原样返回，不追加原文。
        let short = tr(Lang::Zh, "offline_parse_err");
        assert_eq!(fmt_str(short, "的萨芬沙发上放大发v\nnaaaaaaaaaaaa"), short);
    }

}
