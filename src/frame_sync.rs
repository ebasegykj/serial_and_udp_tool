//! 帧同步与样本流处理 —— 与 Python 版 `_process_buffer` / `_append_channel_value` 语义逐一对齐。
//!
//! 定稿语义（2026-11-05/06）：
//! - 帧头可选：空 = 定长切帧（收到的第一个字节即索引 0）；
//! - 有帧头：`find` 对齐，帧头落在帧内第 0 字节；找不到时**删除末尾 header_len-1 字节**（原版行为）；
//! - 大包模式（spp>1）：步长 = 全部激活通道索引的全局 max-min+1，样本 s 平移 s*步长；
//! - 越界通道该样本为 None，**不写入**值队列；
//! - CSV 缓冲：只有全部激活通道都取到值的样本才入档（整行丢弃语义，原版行为）。

use crate::extract::extract_value;

/// 帧校验模式（协议层 SUM 累加和，校验字节在帧尾，从帧头算起）。
/// - Sum8：帧头到倒数第 2 字节累加和取低 8 位 == 帧最后 1 字节；
/// - Sum16：累加和取低 16 位 == 帧最后 2 字节（按字节序设置排列：big=高字节在前）。
/// 校验失败的帧整帧丢弃：不进 frame_log、不计 frame_count、不提取通道值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChecksumMode {
    #[default]
    None,
    Sum8,
    Sum16,
}

impl ChecksumMode {
    /// 校验尾部字节数（None=0，Sum8=1，Sum16=2）。
    pub fn trailer_len(self) -> usize {
        match self {
            ChecksumMode::None => 0,
            ChecksumMode::Sum8 => 1,
            ChecksumMode::Sum16 => 2,
        }
    }

    /// 持久化字符串（.config.json）。
    pub fn as_str(self) -> &'static str {
        match self {
            ChecksumMode::None => "none",
            ChecksumMode::Sum8 => "sum8",
            ChecksumMode::Sum16 => "sum16",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "sum8" => ChecksumMode::Sum8,
            "sum16" => ChecksumMode::Sum16,
            _ => ChecksumMode::None,
        }
    }
}

/// 单通道解析配置（对应 Python 的 `active_channel_configs` 快照）。
#[derive(Debug, Clone)]
pub struct ChannelCfg {
    pub indices: Vec<usize>,
    pub sign_bit: i32,
    pub active: bool,
}

/// 解析引擎。字段与 Python 版实例属性一一对应。
pub struct Engine {
    pub rx_buffer: Vec<u8>,
    pub frame_header: Vec<u8>,
    pub frame_length: usize,
    pub samples_per_packet: usize,
    pub byte_order_big: bool,
    /// 帧校验模式（协议层 SUM，校验失败丢帧并计数）
    pub checksum_mode: ChecksumMode,
    /// 校验失败帧计数（状态栏显示；清空绘图时随引擎重建归零）
    pub checksum_errors: u64,
    pub channel_count: usize,
    pub channels: Vec<ChannelCfg>,
    pub channel_values: Vec<Vec<i64>>,
    pub channel_diff_values: Vec<Vec<i64>>,
    pub frame_count: u64,
    pub sample_count: u64,
    pub extracted_save_enabled: bool,
    pub extracted_save_buffer: Vec<(u64, Vec<i64>)>,
    /// 原始视图帧计数（对拍用；Python 版存 (frame_seq, frame_bytes)）
    pub raw_frame_count: u64,
    /// 校验同步锁：无帧头（定长切帧）模式下，流起点未必落在帧边界。
    /// 只有**连续两帧校验都通过**才认定分帧起点正确并置位；置位后校验失败才按坏帧处理。
    pub sync_locked: bool,
    /// 连续校验失败次数（无帧头模式：连续 ≥3 次判定同步丢失，重新搜索对齐）
    pub consecutive_errors: usize,
    /// 对拍调试：记录切出的每帧内容
    pub frame_log: Vec<Vec<u8>>,

    // ---- 循环码丢帧判定 ----
    /// 循环码在帧内的字节位置（0-based）；None = 关闭，完全不做判定、不计数。
    pub cycle_pos: Option<usize>,
    /// 循环码位宽（只允许 8 / 16 / 24 / 32），决定读几个字节与回绕上限。
    pub cycle_width_bits: u8,
    /// 回绕上限 = 1u64 << 位宽（8→256、16→65536、24→16777216、32→4294967296）。
    /// 由位宽自动推导，不需要用户再配回绕规则。
    pub cycle_limit: u64,
    /// 上一帧读到的循环码值（None = 还没读到第一帧，第一帧只做基准不判定）。
    pub cycle_last: Option<u64>,
    /// 当前帧读到的循环码值（界面「当前值」用）。
    pub cycle_current: Option<u64>,
    /// 循环码判定出来的累计丢帧数（状态栏「丢帧 N」在启用循环码时取此值）。
    pub cycle_lost: u64,
    /// 跳变次数：实际值与期望值不一致的帧数（含重复/回退）。
    pub cycle_jumps: u64,
    /// 异常次数：重复码（delta==0）或回退（计数器重置/乱序），不计入丢帧。
    pub cycle_anomalies: u64,
    /// 上次跳变：(期望值, 实际值)，界面显示「期望 → 实际」。
    pub cycle_last_jump: Option<(u64, u64)>,
    /// 帧头对齐 / 对齐搜索时悄悄丢弃的字节数。
    /// 说明：这些是【字节数】，折算成帧数时用 `/ frame_length`（不整除向下取整），
    /// 属于估算值；校验失败帧已由 checksum_errors 单独计数，不在这里重复累加。
    pub skipped_bytes: u64,
}

impl Engine {
    pub fn new(
        frame_header: Vec<u8>,
        frame_length: usize,
        samples_per_packet: usize,
        byte_order_big: bool,
        channels: Vec<ChannelCfg>,
        extracted_save_enabled: bool,
    ) -> Self {
        // 旧签名保持等价：无校验、无循环码（对拍工具 parity.rs 走这里，schema 不变）
        Self::with_cycle(
            frame_header,
            frame_length,
            samples_per_packet,
            byte_order_big,
            channels,
            extracted_save_enabled,
            ChecksumMode::None,
            None,
            8,
        )
    }

    /// 带帧校验模式的构造（校验功能新增；原 `new` 保持旧签名等价于无校验）。
    /// 循环码默认关闭，供旧调用方与单元测试使用。
    pub fn with_checksum(
        frame_header: Vec<u8>,
        frame_length: usize,
        samples_per_packet: usize,
        byte_order_big: bool,
        channels: Vec<ChannelCfg>,
        extracted_save_enabled: bool,
        checksum_mode: ChecksumMode,
    ) -> Self {
        Self::with_cycle(
            frame_header,
            frame_length,
            samples_per_packet,
            byte_order_big,
            channels,
            extracted_save_enabled,
            checksum_mode,
            None,
            8,
        )
    }

    /// 完整构造：帧校验 + 循环码丢帧判定。
    /// - `cycle_pos`：`None` = 关闭（位置输入框留空），完全不判定、不计数；
    /// - `cycle_width_bits`：只允许 8/16/24/32，非法值回退 8；
    /// - 回绕上限由位宽自动推导，不需要用户配置。
    /// 所有计数器在这里初始化为 0 —— 应用帧配置重建引擎时自然归零
    /// （与 checksum_errors 一致）。
    pub fn with_cycle(
        frame_header: Vec<u8>,
        frame_length: usize,
        samples_per_packet: usize,
        byte_order_big: bool,
        channels: Vec<ChannelCfg>,
        extracted_save_enabled: bool,
        checksum_mode: ChecksumMode,
        cycle_pos: Option<usize>,
        cycle_width_bits: u8,
    ) -> Self {
        let bits: u8 = match cycle_width_bits {
            8 | 16 | 24 | 32 => cycle_width_bits,
            _ => 8,
        };
        let n = channels.len();
        Engine {
            rx_buffer: Vec::new(),
            frame_header,
            frame_length,
            samples_per_packet,
            byte_order_big,
            checksum_mode,
            checksum_errors: 0,
            sync_locked: false,
            consecutive_errors: 0,
            channel_count: n,
            channels,
            channel_values: vec![Vec::new(); n],
            channel_diff_values: vec![Vec::new(); n],
            frame_count: 0,
            sample_count: 0,
            extracted_save_enabled,
            extracted_save_buffer: Vec::new(),
            raw_frame_count: 0,
            frame_log: Vec::new(),
            cycle_pos,
            cycle_width_bits: bits,
            cycle_limit: 1u64 << bits,
            cycle_last: None,
            cycle_current: None,
            cycle_lost: 0,
            cycle_jumps: 0,
            cycle_anomalies: 0,
            cycle_last_jump: None,
            skipped_bytes: 0,
        }
    }

    /// 追加新样本并做一阶差分增量（`diffs[i] = values[i] - values[i-1]`）。
    /// 与 Python `_append_channel_value` 一致：仅在已有前值时写入差分。
    fn append_value(&mut self, ch: usize, val: i64) {
        let last = self.channel_values[ch].last().copied();
        self.channel_values[ch].push(val);
        if let Some(last) = last {
            self.channel_diff_values[ch].push(val - last);
        }
    }

    /// 协议层 SUM 校验：从帧头（第 0 字节）累加到校验字节前。
    /// - Sum8：和的低 8 位 == 帧最后 1 字节；
    /// - Sum16：和的低 16 位 == 帧最后 2 字节（按字节序设置排列）；
    /// - None：恒通过。帧长不足以容纳校验尾部时放行（不误杀）。
    fn verify_checksum(&self, frame: &[u8]) -> bool {
        let n = self.checksum_mode.trailer_len();
        if n == 0 || frame.len() <= n {
            return true;
        }
        let sum: u32 = frame[..frame.len() - n].iter().fold(0u32, |a, &b| a.wrapping_add(b as u32));
        match self.checksum_mode {
            ChecksumMode::None => true,
            ChecksumMode::Sum8 => (sum & 0xFF) as u8 == frame[frame.len() - 1],
            ChecksumMode::Sum16 => {
                let expect = if self.byte_order_big {
                    ((frame[frame.len() - 2] as u16) << 8) | frame[frame.len() - 1] as u16
                } else {
                    ((frame[frame.len() - 1] as u16) << 8) | frame[frame.len() - 2] as u16
                };
                (sum & 0xFFFF) as u16 == expect
            }
        }
    }

    /// 循环码占用的字节数（8/16/24/32 → 1/2/3/4）。
    pub fn cycle_bytes(&self) -> usize {
        (self.cycle_width_bits / 8) as usize
    }

    /// 按「位置 + 位宽 + 当前字节序」从一帧里读出循环码值，用 u64 承载。
    /// - big：高字节在前；little：低字节在前；
    /// - 越界返回 None（应用帧配置时已做校验，这里是防御性兜底，绝不 panic）。
    pub fn cycle_read(&self, frame: &[u8], pos: usize) -> Option<u64> {
        let n = self.cycle_bytes();
        let end = pos.checked_add(n)?;
        if end > frame.len() {
            return None;
        }
        let mut value: u64 = 0;
        for i in 0..n {
            let b = frame[pos + i] as u64;
            let shift = if self.byte_order_big {
                8 * (n - 1 - i)
            } else {
                8 * i
            };
            value |= b << shift;
        }
        Some(value)
    }

    /// 每成功切出【一帧】后调用一次（不是每个样本；samples_per_packet>1 时仍只取一次）。
    /// 判定规则（回绕公式按团队定稿）：
    ///   delta = (cur - last + limit) % limit
    ///   - delta == 1：正常（含自然回绕 255 → 0，不得误报）
    ///   - delta == 0：重复码 → 记异常，不计丢帧
    ///   - delta > 1 ：丢帧 += delta - 1
    /// 号码往回退（设备重启 / 重新连接导致计数器从 0 重来）**不在这里猜**：
    /// 由人在界面「循环码计数」框点「清零」，cycle_reset() 会把上一帧基准一并作废，
    /// 下一帧重新起算。软件只按差值数号，不猜设备状态。
    pub fn cycle_process_frame(&mut self, frame: &[u8]) {
        let Some(pos) = self.cycle_pos else {
            return; // 位置为空 = 开关关闭，完全不判定、不计数
        };
        let Some(value) = self.cycle_read(frame, pos) else {
            return; // 越界：应用帧配置时已给出明确错误，这里静默跳过（不 panic）
        };
        self.cycle_current = Some(value);
        let limit = self.cycle_limit;
        let Some(last) = self.cycle_last else {
            // 第一帧只作为比较基准，不判定
            self.cycle_last = Some(value);
            return;
        };
        let expected = (last + 1) % limit;
        // (cur - last + limit) % limit，用加法规避 u64 下溢
        let delta = (value + limit - last) % limit;
        if delta == 1 {
            // 正常递增（含跨回绕点的 255 → 0）
            self.cycle_last = Some(value);
            return;
        }
        // 走到这里都是「跳变」：记录期望 → 实际
        self.cycle_jumps += 1;
        self.cycle_last_jump = Some((expected, value));
        if delta == 0 {
            self.cycle_anomalies += 1; // 重复码
        } else {
            self.cycle_lost += delta - 1; // 跳号：中间缺了 delta-1 帧
        }
        self.cycle_last = Some(value);
    }

    /// 界面「清零」：计数全部归零，并把「上一帧基准」作废。
    /// 基准必须一起清 —— 否则设备重启后第一帧（号很小）仍会跟重启前的旧号比较，
    /// 按差值算出一大笔假丢帧。清完后下一帧自动重新当基准，不判定、不计数。
    /// 状态栏「丢帧 N」在启用循环码时直接取 cycle_lost，因此会同步回到 0。
    pub fn cycle_reset(&mut self) {
        self.cycle_last = None;
        self.cycle_current = None;
        self.cycle_lost = 0;
        self.cycle_jumps = 0;
        self.cycle_anomalies = 0;
        self.cycle_last_jump = None;
    }

    /// 状态栏「丢帧 N」的唯一主数字：
    /// - 启用循环码 → 用循环码判定出来的丢帧数；
    /// - 未启用循环码 → 校验错计数 + 重新对齐时悄悄跳过的帧数（估算值）。
    /// 校验失败帧已经有 checksum_errors，skipped_bytes 只统计对齐丢弃，不重复计。
    /// 注释：skipped_bytes 是字节数，折算帧数用 `/ frame_length`（向下取整），属估算值。
    pub fn lost_frames(&self) -> u64 {
        if self.cycle_pos.is_some() {
            return self.cycle_lost;
        }
        let estimated: u64 = if self.frame_length > 0 {
            self.skipped_bytes / self.frame_length as u64
        } else {
            0
        };
        self.checksum_errors.saturating_add(estimated)
    }

    /// 无帧头模式的分帧对齐搜索（工业二进制协议常见做法）：
    /// 接收流可能从任意字节开始，单帧校验通过可能只是巧合（8 位校验误判概率 1/256），
    /// 因此要求**连续两帧校验都通过**（误判概率 1/65536）才认定分帧起点正确。
    /// 未对齐期间不切帧、不画图；已搜索区域无匹配时丢弃，仅保留尾部 2L-1 字节。
    fn locate_frame_start(&mut self) {
        let l = self.frame_length;
        let need = l.saturating_mul(2);
        if l < 2 || self.rx_buffer.len() < need {
            return; // 不够两帧，等更多数据
        }
        let mut found: Option<usize> = None;
        for off in 0..=(self.rx_buffer.len() - need) {
            if self.verify_checksum(&self.rx_buffer[off..off + l])
                && self.verify_checksum(&self.rx_buffer[off + l..off + need])
            {
                found = Some(off);
                break;
            }
        }
        match found {
            Some(off) => {
                if off > 0 {
                    // 对齐前悄悄跳过的字节（估算丢帧用，折算见 lost_frames）
                    self.skipped_bytes += off as u64;
                    self.rx_buffer.drain(..off);
                }
                self.sync_locked = true;
                self.consecutive_errors = 0;
            }
            None => {
                // 已搜索区域找不到连续两帧匹配：丢弃，仅保留尾部 2L-1 字节
                // （下一个候选起点可能就在尾部里）
                let keep = need - 1;
                let drop_n = self.rx_buffer.len() - keep;
                // 对齐搜索阶段悄悄跳过的字节（估算丢帧用，折算见 lost_frames）
                self.skipped_bytes += drop_n as u64;
                self.rx_buffer.drain(..drop_n);
            }
        }
    }

    /// 帧同步与数值提取（移植自 Python `_process_buffer`，逻辑逐行对齐）。
    pub fn process_buffer(&mut self) {
        let header_len = self.frame_header.len();
        // 无帧头 + 启用校验 + 尚未对齐：先确认分帧起点（连续两帧校验通过）才切帧。
        // 未对齐期间不切帧、不画图——收到的第一个字节不一定是真实帧的第一字节。
        if header_len == 0 && self.checksum_mode != ChecksumMode::None && !self.sync_locked {
            self.locate_frame_start();
            if !self.sync_locked {
                return;
            }
        }
        // Python 版每轮从快照读配置；Rust 侧直接 clone 等价
        let configs = self.channels.clone();
        let active_channels: Vec<usize> = configs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.active)
            .map(|(i, _)| i)
            .collect();

        let mut sample_size: usize = 0;
        if self.samples_per_packet > 1 && !active_channels.is_empty() {
            let mut all_indices: Vec<usize> = Vec::new();
            for &i in &active_channels {
                all_indices.extend(configs[i].indices.iter().cloned());
            }
            if !all_indices.is_empty() {
                let min_idx = *all_indices.iter().min().unwrap();
                let max_idx = *all_indices.iter().max().unwrap();
                sample_size = max_idx - min_idx + 1;
            }
        }

        while self.rx_buffer.len() >= self.frame_length {
            // 无帧头：定长切帧；有帧头：find 对齐
            let found: Option<usize> = if header_len == 0 {
                Some(0)
            } else {
                find_sub(&self.rx_buffer, &self.frame_header)
            };
            match found {
                None => {
                    // 与 Python 原版逐行对齐：`del rx[-(header_len-1):]`
                    // 是【删除末尾 h-1 字节、保留前缀】（不是保留末尾！）。
                    // 陷阱：header_len==1 时 h-1=0，Python 的 `del rx[-0:]` == `del rx[0:]`
                    //       即【清空整个缓冲区】（-0 就是 0），必须特判。
                    // 注：真实流分片到达时该行为可能丢字节，属原版固有语义；
                    // M2 阶段可提请用户讨论是否修正。
                    if header_len > 0 && self.rx_buffer.len() > header_len - 1 {
                        if header_len == 1 {
                            self.rx_buffer.clear();
                        } else {
                            let new_len = self.rx_buffer.len() - (header_len - 1);
                            self.rx_buffer.truncate(new_len);
                        }
                    }
                    break;
                }
                Some(idx) => {
                    if idx > 0 {
                        // 帧头对齐时悄悄跳过的字节（估算丢帧用，折算见 lost_frames）
                        self.skipped_bytes += idx as u64;
                        self.rx_buffer.drain(..idx);
                    }
                }
            }
            if self.rx_buffer.len() < self.frame_length {
                break;
            }

            let frame: Vec<u8> = self.rx_buffer[..self.frame_length].to_vec();
            // 协议层 SUM 校验（对齐已由帧头或 locate_frame_start 保证）
            if !self.verify_checksum(&frame) {
                self.checksum_errors += 1;
                self.consecutive_errors += 1;
                // 坏帧整帧丢弃：不进 frame_log、不计任何帧数、不提取通道
                self.rx_buffer.drain(..self.frame_length);
                if header_len == 0 && self.consecutive_errors >= 3 {
                    // 无帧头模式连续 3 次失败 → 同步丢失（设备重启/流错位），
                    // 失锁交回搜索逻辑重新对齐（下一轮 process_buffer 先 locate）
                    self.sync_locked = false;
                    self.consecutive_errors = 0;
                    break;
                }
                continue;
            }
            self.consecutive_errors = 0;
            self.sync_locked = true;
            self.rx_buffer.drain(..self.frame_length);
            self.raw_frame_count += 1;
            self.frame_count += 1;
            self.frame_log.push(frame.clone());
            // 循环码判定：每帧只做一次（不随 samples_per_packet 重复），
            // 离线绘图 / 剪贴板导入都走 process_buffer，自动生效。
            self.cycle_process_frame(&frame);

            let num_samples = if self.samples_per_packet > 1 { self.samples_per_packet } else { 1 };
            for s in 0..num_samples {
                let mut sample_values: Vec<i64> = Vec::new();
                let mut sample_ok = true;

                for (i, cfg) in configs.iter().enumerate() {
                    if cfg.active && i < self.channel_count {
                        let adjusted: Vec<usize> = if self.samples_per_packet > 1 {
                            cfg.indices.iter().map(|&x| x + s * sample_size).collect()
                        } else {
                            cfg.indices.clone()
                        };
                        match extract_value(&frame, &adjusted, cfg.sign_bit, self.byte_order_big) {
                            Some(v) => {
                                self.append_value(i, v);
                                sample_values.push(v);
                            }
                            None => {
                                sample_ok = false;
                            }
                        }
                    }
                }

                if sample_ok
                    && self.extracted_save_enabled
                    && sample_values.len() == active_channels.len()
                {
                    let cnt = self.sample_count;
                    self.extracted_save_buffer.push((cnt, sample_values.clone()));
                }

                self.sample_count += 1;
            }
        }
    }
}

/// 字节序列查找（语义同 `bytes.find`：needle 为空返回 0）。
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(header: &[u8], len: usize, spp: usize, big: bool, chans: &[(Vec<usize>, i32, bool)], save: bool) -> Engine {
        Engine::new(
            header.to_vec(),
            len,
            spp,
            big,
            chans.iter().map(|(idx, sb, act)| ChannelCfg { indices: idx.clone(), sign_bit: *sb, active: *act }).collect(),
            save,
        )
    }

    fn hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..clean.len()).step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn fixed_length_split_no_header() {
        // 22 字节 / 帧长 11 → 2 帧，剩 2 字节；CH1=[0x0102,0x1112] CH2=[0x0304,0x1314]
        let mut e = engine(b"", 11, 1, true,
            &[(vec![2, 3], 15, true), (vec![4, 5], 15, true)], false);
        e.rx_buffer = hex("1122 010203040506070809 3344 111213141516171819 AA BB");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.rx_buffer.len(), 2);
        assert_eq!(e.channel_values[0], vec![0x0102, 0x1112]);
        assert_eq!(e.channel_values[1], vec![0x0304, 0x1314]);
    }

    #[test]
    fn header_alignment_and_garbage_prefix() {
        let mut e = engine(&hex("EB90"), 11, 1, true,
            &[(vec![2, 3], 15, true), (vec![4, 5], 15, true)], false);
        e.rx_buffer = hex("0000 EB90 010203040506070809 0000 EB90 111213141516171819");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.channel_values[0][0], 0x0102);
    }

    #[test]
    fn partial_header_drops_tail() {
        // 无完整帧头：find=-1 → 删除末尾 h-1 字节（原版行为，非保留）
        let mut e = engine(&hex("EB90"), 2, 1, true, &[(vec![0, 1], -1, true)], false);
        e.rx_buffer = hex("AA BB EB");
        e.process_buffer();
        assert_eq!(e.frame_count, 0);
        assert_eq!(e.rx_buffer, vec![0xAA, 0xBB]);
    }

    #[test]
    fn one_byte_header_miss_clears_buffer() {
        // Python 陷阱：h=1 时 del rx[-0:] == del rx[0:] → find 失败清空整个缓冲区
        let mut e = engine(&hex("5A"), 3, 1, true, &[(vec![0], -1, true)], false);
        e.rx_buffer = hex("AA BB CC DD");
        e.process_buffer();
        assert_eq!(e.frame_count, 0);
        assert!(e.rx_buffer.is_empty());
    }

    #[test]
    fn out_of_range_channel_silent() {
        // 帧长 14、CH2 索引 14,15 越界 → 无数据不崩溃（原版语义）
        let mut e = engine(b"", 14, 1, true,
            &[(vec![2, 3], 15, true), (vec![14, 15], 15, true)], false);
        e.rx_buffer = (0u8..14).collect();
        e.process_buffer();
        assert_eq!(e.frame_count, 1);
        assert!(e.channel_values[1].is_empty());
    }

    #[test]
    fn packet_mode_step_and_csv_drop() {
        // 大包 Smp=2、帧长 14、索引 11,12：样本1=[11,12]=0xB0C 有效；样本2=[13,14] 越界
        let mut e = engine(b"", 14, 2, true, &[(vec![11, 12], 15, true)], true);
        e.rx_buffer = (0u8..14).collect();
        e.process_buffer();
        assert_eq!(e.frame_count, 1);
        assert_eq!(e.sample_count, 2);
        assert_eq!(e.channel_values[0], vec![0xB0C]);
        // 样本1 成功 → 入 CSV；样本2 越界 → 整行丢弃（原版行为）
        assert_eq!(e.extracted_save_buffer, vec![(0, vec![0xB0C])]);
    }

    #[test]
    fn csv_all_ok_saves_row() {
        let mut e = engine(b"", 11, 1, true,
            &[(vec![2, 3], 15, true), (vec![4, 5], 15, true)], true);
        e.rx_buffer = hex("EB90 010203040506070809");
        e.process_buffer();
        assert_eq!(e.extracted_save_buffer.len(), 1);
        assert_eq!(e.extracted_save_buffer[0], (0, vec![0x0102, 0x0304]));
    }

    // ---- 帧校验（SUM）----

    fn engine_chk(header: &[u8], len: usize, big: bool, mode: ChecksumMode, chans: &[(Vec<usize>, i32, bool)]) -> Engine {
        Engine::with_checksum(
            header.to_vec(), len, 1, big,
            chans.iter().map(|(idx, sb, act)| ChannelCfg { indices: idx.clone(), sign_bit: *sb, active: *act }).collect(),
            false, mode,
        )
    }

    #[test]
    fn sum8_valid_frame_passes() {
        // 帧：11 字节，前 10 字节和 = 0x2D(01..09) + 0x33 = 0x60；尾字节 0x60
        // 无帧头需连续两帧才锁定 → 送两帧相同数据
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 60");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 0);
        assert_eq!(e.channel_values[0], vec![0x0304, 0x0304]);
    }

    #[test]
    fn sum8_bad_frame_dropped() {
        // 有帧头：帧头已对齐仍校验失败 = 坏帧 → 整帧丢弃、计错、不提取
        let mut e = engine_chk(&hex("EB90"), 13, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("EB90 01 02 03 04 05 06 07 08 09 33 FF");
        e.process_buffer();
        assert_eq!(e.frame_count, 0);
        assert_eq!(e.raw_frame_count, 0);
        assert_eq!(e.checksum_errors, 1);
        assert!(e.frame_log.is_empty());
        assert!(e.channel_values[0].is_empty());
        assert!(e.rx_buffer.is_empty()); // 坏帧也消费掉，不堵塞
        assert!(!e.sync_locked);
    }

    #[test]
    fn sum8_resync_slides_to_alignment() {
        // 无帧头 + 流起点错位（前置 3 字节垃圾）：连续两帧校验通过才锁定分帧起点，
        // 锁定后正常解析；搜索过程不计坏帧（errors=0）
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("AA BB CC 01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 60");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 0);
        assert_eq!(e.channel_values[0], vec![0x0304, 0x0304]);
        assert!(e.rx_buffer.is_empty());
        assert!(e.sync_locked);
    }

    #[test]
    fn sum8_unsynced_never_draws_garbage() {
        // 无帧头 + 未对齐（不足两帧 / 无连续匹配）：不切帧、不画图。
        // 不足两帧时原样保留缓冲区，等更多数据
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 10 11 12 13 14");
        e.process_buffer();
        assert_eq!(e.frame_count, 0);
        assert_eq!(e.checksum_errors, 0);
        assert!(e.channel_values[0].is_empty());
        assert_eq!(e.rx_buffer.len(), 14); // 不足 2 帧，原样保留
        assert!(!e.sync_locked);
    }

    #[test]
    fn sum8_single_frame_not_enough_to_lock() {
        // 单帧校验通过不足信（可能巧合）：不够两帧时不锁定、不切帧
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 60");
        e.process_buffer();
        assert_eq!(e.frame_count, 0);
        assert!(!e.sync_locked);
    }

    #[test]
    fn sum8_locked_bad_frame_counted() {
        // 锁定后（连续两帧通过）第三帧校验失败 → 按坏帧计数丢弃
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 00");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 1);
        assert!(e.sync_locked);
    }

    #[test]
    fn sum8_consecutive_failures_unlock() {
        // 锁定后连续 3 次失败 → 判定同步丢失，失锁等待重新对齐
        let mut e = engine_chk(b"", 11, true, ChecksumMode::Sum8,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 60 01 02 03 04 05 06 07 08 09 33 00 01 02 03 04 05 06 07 08 09 33 00 01 02 03 04 05 06 07 08 09 33 00");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 3);
        assert!(!e.sync_locked);
    }

    #[test]
    fn sum16_big_endian_trailer() {
        // 前 10 字节和 = 0x60；16 位校验 big → 尾 2 字节 00 60；送两帧以锁定同步
        let mut e = engine_chk(b"", 12, true, ChecksumMode::Sum16,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 00 60 01 02 03 04 05 06 07 08 09 33 00 60");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 0);
    }

    #[test]
    fn sum16_little_endian_trailer() {
        // 同上 0x60，little → 尾 2 字节 60 00；送两帧
        let mut e = engine_chk(b"", 12, false, ChecksumMode::Sum16,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 60 00 01 02 03 04 05 06 07 08 09 33 60 00");
        e.process_buffer();
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.checksum_errors, 0);
    }

    #[test]
    fn checksum_none_never_drops() {
        // 无校验：坏尾字节也照常切帧（回归原语义）
        let mut e = engine_chk(b"", 11, true, ChecksumMode::None,
            &[(vec![2, 3], 15, true)]);
        e.rx_buffer = hex("01 02 03 04 05 06 07 08 09 33 FF");
        e.process_buffer();
        assert_eq!(e.frame_count, 1);
        assert_eq!(e.checksum_errors, 0);
    }

    // ---- 循环码丢帧判定 ----

    /// 构造带循环码的引擎（无帧头、无校验，定长切帧，便于精确喂帧）。
    fn engine_cycle(len: usize, spp: usize, big: bool, pos: Option<usize>, bits: u8) -> Engine {
        Engine::with_cycle(
            Vec::new(),
            len,
            spp,
            big,
            vec![ChannelCfg { indices: vec![2, 3], sign_bit: 15, active: true }],
            false,
            ChecksumMode::None,
            pos,
            bits,
        )
    }

    /// 造一帧：把循环码按 位置/位宽/字节序 写进指定字节位置，其余补 0。
    fn frame_with_code(len: usize, pos: usize, bits: u8, code: u64, big: bool) -> Vec<u8> {
        let n = (bits / 8) as usize;
        let mut frame = vec![0u8; len];
        for i in 0..n {
            let shift = if big { 8 * (n - 1 - i) } else { 8 * i };
            frame[pos + i] = ((code >> shift) & 0xFF) as u8;
        }
        frame
    }

    /// 依次喂入若干循环码值，每帧一次 process_buffer。
    fn feed_codes(e: &mut Engine, len: usize, pos: usize, bits: u8, big: bool, codes: &[u64]) {
        for &c in codes {
            e.rx_buffer.extend_from_slice(&frame_with_code(len, pos, bits, c, big));
            e.process_buffer();
        }
    }

    #[test]
    fn cycle_disabled_does_nothing() {
        // 位置为空 = 开关关闭：完全不判定、不计数，当前值也不写
        let mut e = engine_cycle(4, 1, true, None, 8);
        feed_codes(&mut e, 4, 0, 8, true, &[1, 5, 9]);
        assert_eq!(e.frame_count, 3);
        assert_eq!(e.cycle_current, None);
        assert_eq!(e.cycle_lost, 0);
        assert_eq!(e.cycle_jumps, 0);
        assert_eq!(e.cycle_anomalies, 0);
        assert_eq!(e.cycle_last_jump, None);
        // 未启用时状态栏总数走「校验错 + 对齐估算」，此处两者皆 0
        assert_eq!(e.lost_frames(), 0);
    }

    #[test]
    fn cycle_delta_one_and_wrap_255_to_0() {
        // 正常递增；并且 8 位计数器自然回绕 255 → 0 不得误报丢帧/异常
        let mut e = engine_cycle(4, 1, true, Some(0), 8);
        assert_eq!(e.cycle_limit, 256);
        feed_codes(&mut e, 4, 0, 8, true, &[253, 254, 255, 0, 1, 2]);
        assert_eq!(e.frame_count, 6);
        assert_eq!(e.cycle_lost, 0);
        assert_eq!(e.cycle_jumps, 0);
        assert_eq!(e.cycle_anomalies, 0);
        assert_eq!(e.cycle_current, Some(2));
        assert_eq!(e.lost_frames(), 0);
    }

    #[test]
    fn cycle_jump_counts_lost_frames() {
        // 10 → 13：delta=3，丢帧 += 2；记录跳变 (期望 11 → 实际 13)
        let mut e = engine_cycle(4, 1, true, Some(0), 8);
        feed_codes(&mut e, 4, 0, 8, true, &[10, 13]);
        assert_eq!(e.cycle_lost, 2);
        assert_eq!(e.cycle_jumps, 1);
        assert_eq!(e.cycle_anomalies, 0);
        assert_eq!(e.cycle_last_jump, Some((11, 13)));
        assert_eq!(e.lost_frames(), 2);
    }

    #[test]
    fn cycle_no_backward_guess_strict_delta() {
        // 软件不猜「是不是设备重启」：重复码 5→5 记异常；
        // 号码倒着走 5→3 严格按差值算（delta=254，丢帧 += 253）。
        // 调试时重启/断连由人在界面点「清零」，见下一个测试。
        let mut e = engine_cycle(4, 1, true, Some(0), 8);
        feed_codes(&mut e, 4, 0, 8, true, &[5, 5, 3]);
        assert_eq!(e.cycle_anomalies, 1);
        assert_eq!(e.cycle_lost, 253);
        assert_eq!(e.cycle_jumps, 2);
        assert_eq!(e.cycle_last_jump, Some((6, 3)));
        assert_eq!(e.lost_frames(), 253);
    }

    #[test]
    fn cycle_reset_clears_counters_and_baseline() {
        // 清零：全部计数归零 + 上一帧基准作废；状态栏总数同步回 0。
        let mut e = engine_cycle(4, 1, true, Some(0), 8);
        feed_codes(&mut e, 4, 0, 8, true, &[10, 13]);
        assert_eq!(e.cycle_lost, 2);
        e.cycle_reset();
        assert_eq!(e.cycle_lost, 0);
        assert_eq!(e.cycle_jumps, 0);
        assert_eq!(e.cycle_anomalies, 0);
        assert_eq!(e.cycle_last, None);
        assert_eq!(e.cycle_current, None);
        assert_eq!(e.cycle_last_jump, None);
        assert_eq!(e.lost_frames(), 0);

        // 模拟设备重启后号码从 0 重来：人点了清零，下一帧当基准，1、2 正常递增，
        // 不会再拿重启前的 13 去比、也不会冒出一大笔假丢帧。
        feed_codes(&mut e, 4, 0, 8, true, &[0, 1, 2]);
        assert_eq!(e.cycle_lost, 0);
        assert_eq!(e.cycle_jumps, 0);
        assert_eq!(e.cycle_anomalies, 0);
        assert_eq!(e.cycle_current, Some(2));
        assert_eq!(e.lost_frames(), 0);
    }

    #[test]
    fn cycle_width_endian_and_auto_limit() {
        // 16 位 big/little 读取正确、delta==1 正常；24 位回绕上限自动推导
        let mut big = engine_cycle(6, 1, true, Some(0), 16);
        assert_eq!(big.cycle_limit, 65536);
        feed_codes(&mut big, 6, 0, 16, true, &[0x0102, 0x0103]);
        assert_eq!(big.cycle_current, Some(0x0103));
        assert_eq!(big.cycle_lost, 0);

        let mut little = engine_cycle(6, 1, false, Some(0), 16);
        feed_codes(&mut little, 6, 0, 16, false, &[0x0102, 0x0103]);
        assert_eq!(little.cycle_current, Some(0x0103));
        assert_eq!(little.cycle_lost, 0);

        // 24 位：limit = 1<<24，最大值 → 0 属正常回绕
        let mut e24 = engine_cycle(8, 1, true, Some(0), 24);
        assert_eq!(e24.cycle_limit, 16777216);
        feed_codes(&mut e24, 8, 0, 24, true, &[16777215, 0]);
        assert_eq!(e24.cycle_lost, 0);
        assert_eq!(e24.cycle_jumps, 0);

        // 32 位：limit = 1<<32
        let e32 = engine_cycle(8, 1, true, Some(0), 32);
        assert_eq!(e32.cycle_limit, 4294967296);
        assert_eq!(e32.cycle_bytes(), 4);
    }

    #[test]
    fn cycle_once_per_frame_even_with_spp() {
        // 大包模式 samples_per_packet=2：一帧仍只取一次循环码，不能按样本重复判定
        let mut e = engine_cycle(6, 2, true, Some(0), 8);
        feed_codes(&mut e, 6, 0, 8, true, &[1, 3]);
        assert_eq!(e.frame_count, 2);
        assert_eq!(e.sample_count, 4);
        assert_eq!(e.cycle_lost, 1); // 只算 1 次跳号丢 1 帧，不是 2 帧
        assert_eq!(e.cycle_jumps, 1);
    }

    #[test]
    fn skipped_bytes_estimate_only_when_cycle_off() {
        // 帧头对齐 drain(..idx) 丢 3 字节：折算 3/4=0 帧（向下取整，估算值）
        let mut e = Engine::with_cycle(
            hex("EB90"), 4, 1, true,
            vec![ChannelCfg { indices: vec![0], sign_bit: -1, active: true }],
            false, ChecksumMode::None, None, 8,
        );
        e.rx_buffer = hex("AA BB CC EB90 01 02");
        e.process_buffer();
        assert_eq!(e.skipped_bytes, 3);
        assert_eq!(e.lost_frames(), 0); // 3/4 向下取整 = 0

        // 丢 8 字节 → 估算 2 帧；与 checksum_errors 相加得到状态栏总数
        e.rx_buffer = hex("00 00 00 00 00 00 00 00 EB90 01 02");
        e.process_buffer();
        assert_eq!(e.skipped_bytes, 11);
        assert_eq!(e.lost_frames(), 2); // 11/4 = 2

        // 启用循环码后，状态栏总数只取循环码判定结果，不再叠加估算
        e.cycle_pos = Some(0);
        e.cycle_lost = 7;
        assert_eq!(e.lost_frames(), 7);
    }
}
