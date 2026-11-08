//! 界面文案（中英对照）。key 用常量，避免拼写错误。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn toggle(&mut self) {
        *self = match self {
            Lang::Zh => Lang::En,
            Lang::En => Lang::Zh,
        };
    }
}

/// (key, 中文, English) —— 与 Python 15 版 tr() 词条对齐
const TABLE: &[(&str, &str, &str)] = &[
    ("app_title", "帧析助手", "Frame Analyzer"),
    ("connection", "数据连接", "Data Connection"),
    ("connect", "连接", "Connect"),
    ("connected", "已连接", "Connected"),
    ("disconnect", "断开", "Disconnect"),
    ("interface", "接口", "Interface"),
    ("serial", "Serial", "Serial"),
    ("udp", "UDP", "UDP"),
    ("port", "端口", "Port"),
    ("baud", "波特率", "Baud"),
    ("parity", "校验位", "Parity"),
    ("parity_tip", "硬件奇偶校验，修改后需重新连接生效", "Hardware parity, reconnect to apply"),
    ("checksum", "校验", "Check"),
    ("chk_none", "no", "no"),
    ("chk_sum8", "8", "8"),
    ("chk_sum16", "16", "16"),
    ("chk_err", "校验错", "Chk Err"),
    ("chk_sync", "等待校验同步", "Searching sync"),
    ("chk_synced", "校验已同步", "Synced"),
    ("signed", "有符号", "Signed"),
    ("unsigned", "无符号", "Unsigned"),
    ("mixed", "混合", "Mixed"),
    ("refresh", "刷新", "Refresh"),
    ("local_ip", "本机IP", "Local IP"),
    ("local_port", "本机端口", "Local Port"),
    ("remote_ip", "远端IP", "Remote IP"),
    ("remote_port", "远端端口", "Remote Port"),
    ("datalog", "数据记录", "Data Log"),
    ("save_extracted", "提取数据保存", "Save Extracted"),
    ("csv_file", "CSV文件...", "CSV File..."),
    ("save_rawchk", "原始数据保存", "Save Raw"),
    ("txt_file", "TXT文件...", "TXT File..."),
    ("no_file", "(no file)", "(no file)"),
    ("framecfg", "帧配置/离线绘图", "Frame Config / Offline"),
    ("header", "帧头", "Header"),
    ("length", "帧长", "Length"),
    ("byte_order", "字节序", "Endian"),
    ("cycle_code", "循环码", "Cycle"),
    ("cycle_width", "位宽", "Bits"),
    ("cycle_pos_tip", "帧内第几个字节，从 1 起算；留空 = 关闭丢帧判定", "Byte position in frame, 1-based; empty = disable"),
    ("cycle_stats", "循环码计数", "Cycle Stats"),
    ("cycle_cur", "当前值", "Current"),
    ("cycle_lost", "累计丢帧", "Lost"),
    ("cycle_jumps", "跳变次数", "Jumps"),
    ("cycle_jumps_short", "跳变", "Jumps"),
    ("cycle_dur", "连接时长", "Uptime"),
    ("cycle_last_jump", "上次跳变", "Last jump"),
    ("cycle_anom", "异常次数", "Anomalies"),
    ("cycle_anom_note", "（循环码不动）", "(code unchanged)"),
    ("cycle_reset", "清零", "Reset"),
    ("cycle_reset_tip", "设备重启或重新连接后点一下：计数归零，下一帧重新起算", "After device restart or reconnect: clear counters and restart baseline"),
    ("cycle_reset_done", "循环码计数已清零，下一帧重新起算", "Cycle counters reset; next frame starts a new baseline"),
    ("lost_frames", "丢帧", "Lost"),
    ("rec_saving", "保存中", "Saving"),
    ("rec_wait_conn", "待连接", "Waiting"),
    ("rec_choose_file", "待选择文件", "Choose a file"),
    ("rec_on", "记录中", "Recording"),
    ("rec_off", "未记录", "Not recording"),
    ("samples_pkt", "样本", "Sample"),
    ("max_points", "点数", "Points"),
    ("apply_frame", "应用帧配置", "Apply Frame"),
    ("load_file", "原始数据绘图", "Raw Data Plot"),
    ("analyze_clip", "剪贴板数据绘图", "Clipboard Plot"),
    ("chancfg", "通道配置", "Channels"),
    ("channels", "通道数", "Channels"),
    ("add_channel", "＋通道", "+ Ch"),
    ("del_channel", "－通道", "－ Ch"),
    ("apply", "应用", "Apply"),
    ("dif", "Dif", "Dif"),
    ("plot_title", "数据绘图", "Data Plot"),
    ("view_plot", "绘图", "Plot"),
    ("view_raw", "原始", "Raw"),
    ("help", "帮助", "Help"),
    ("send", "发送", "Send"),
    ("history", "命令发送", "Commands"),
    ("cmd_interval", "间隔(ms):", "Interval(ms):"),
    ("cmd_send_one", "发送", "Send"),
    ("cmd_add", "＋ 添加命令", "+ Add Command"),
    ("cmd_del", "删除此命令", "Delete"),
    ("cmd_start", "开始循环发送（按间隔轮发全部命令）", "Start loop sending"),
    ("cmd_stop", "停止循环发送", "Stop loop sending"),
    ("cmd_sending", "循环发送中", "Looping, next #"),
    ("cmd_empty", "命令列表为空，请先添加命令", "Command list is empty"),
    ("cmd_not_connected", "未连接", "Not connected"),
    ("err_title", "连接失败", "Connection Failed"),
    ("err_ok", "确定", "OK"),
    ("lang_btn", "EN", "中文"),
    ("style_panel", "界面微调", "UI Tuning"),
    ("frame_applied", "帧配置已应用", "Frame applied"),
    ("serial_lost", "串口连接已断开", "Serial connection lost"),
    ("udp_lost", "UDP 连接已断开", "UDP connection lost"),
    ("manual_lost", "已手动断开", "Disconnected"),
    ("not_connected", "未连接", "Disconnected"),
    ("config_error", "配置错误", "Config Error"),
    ("send_err", "发送失败", "Send failed"),
    ("no_ports", "未发现串口设备", "No serial ports found"),
    ("channel_applied", "通道配置已应用", "Channel applied"),
];

/// 查文案；缺 key 返回 key 本身（便于发现遗漏）。
pub fn tr(lang: Lang, key: &str) -> &'static str {
    for (k, zh, en) in TABLE {
        if *k == key {
            return match lang {
                Lang::Zh => zh,
                Lang::En => en,
            };
        }
    }
    ""
}
