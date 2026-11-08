//! .config.json 配置结构 —— 与 Python 版 JSON 字段兼容（值为字符串、帧头可为空）。

use serde::Deserialize;

/// Python 版保存的顶层结构。
#[derive(Debug, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub frame: FrameCfg,
    #[serde(default)]
    pub channels: Vec<ChannelSaved>,
}

/// 帧配置。Python 版把输入框原样存成字符串（如 length: "11"）。
#[derive(Debug, Deserialize, Default)]
pub struct FrameCfg {
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub length: String,
    #[serde(default)]
    pub order: String,
    #[serde(default)]
    pub samples: String,
    #[serde(default)]
    pub max_points: String,
    #[serde(default)]
    pub channel_count: String,
    /// 循环码在帧内的字节位置（界面 1 起算，输入框原样字符串；空 = 关闭不记）。
    /// `#[serde(default)]` 兼容旧配置（缺字段时为空串）。
    #[serde(default)]
    pub cycle_pos: String,
    /// 循环码位宽（"8"/"16"/"24"/"32"；空串或非法值回退 8）。
    #[serde(default)]
    pub cycle_width: String,
}

/// indices 兼容两种历史格式：数组（现行）或字符串（旧版）。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum IndicesField {
    List(Vec<i64>),
    Str(String),
}

impl IndicesField {
    pub fn to_indices(&self) -> Vec<i64> {
        match self {
            IndicesField::List(v) => v.clone(),
            IndicesField::Str(s) => parse_indices_str(s),
        }
    }
}

/// 单通道持久化结构。
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelSaved {
    #[serde(default)]
    pub indices: Option<IndicesField>,
    #[serde(default)]
    pub sign_bit: i64,
    #[serde(default)]
    pub diff: bool,
    #[serde(default)]
    pub visible: bool,
}

impl ChannelSaved {
    pub fn indices(&self) -> Vec<i64> {
        self.indices.as_ref().map(|f| f.to_indices()).unwrap_or_default()
    }
}

/// 解析帧头 hex（Python 版语义：去空格换行后 `bytes.fromhex`）。
/// 空串 → 空 Vec（无帧头 / 定长切帧，2026-11-05 定稿）。
pub fn parse_header_hex(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    if clean.len() % 2 != 0 {
        return Err(format!("非奇数长度十六进制串: {:?}", s));
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).map_err(|e| format!("{:?}: {}", &clean[i..i + 2], e)))
        .collect()
}

/// 解析索引串 —— 与 Python `_parse_indices` 一致：
/// 逗号 / 空格 / 中文逗号（，）/ 顿号（、）都可作为分隔符。
pub fn parse_indices_str(s: &str) -> Vec<i64> {
    s.split([',', '，', '、', ' ', '\t', '\r', '\n'])
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_parsing() {
        assert_eq!(parse_header_hex("EB 90").unwrap(), vec![0xEB, 0x90]);
        assert_eq!(parse_header_hex("").unwrap(), Vec::<u8>::new()); // 空帧头合法
        assert_eq!(parse_header_hex("eb90").unwrap(), vec![0xEB, 0x90]);
        assert!(parse_header_hex("GG").is_err());
    }

    #[test]
    fn indices_separators() {
        assert_eq!(parse_indices_str("2,3"), vec![2, 3]);
        assert_eq!(parse_indices_str("2 3"), vec![2, 3]);
        assert_eq!(parse_indices_str("2，3"), vec![2, 3]);
        assert_eq!(parse_indices_str("2、3"), vec![2, 3]);
        assert_eq!(parse_indices_str("2,3,4,5"), vec![2, 3, 4, 5]);
        assert_eq!(parse_indices_str("11,12,13,14"), vec![11, 12, 13, 14]);
    }

    #[test]
    fn config_json_compat() {
        // 与真实 .config.json 同构（含空 indices 与字符串 indices 两种历史格式）
        let j = r#"{
          "language": "zh", "theme": "深色",
          "frame": {"header": "", "length": "11", "order": "big",
                    "samples": "1", "max_points": "500", "channel_count": "2"},
          "channels": [
            {"indices": [2, 3], "sign_bit": 15, "diff": false, "visible": true},
            {"indices": "4,5", "sign_bit": 15, "diff": false, "visible": true}
          ]
        }"#;
        let cfg: AppConfig = serde_json::from_str(j).unwrap();
        assert_eq!(cfg.frame.header, "");
        assert_eq!(cfg.frame.length, "11");
        assert_eq!(cfg.channels[0].indices(), vec![2, 3]);
        assert_eq!(cfg.channels[1].indices(), vec![4, 5]);
        let hdr = parse_header_hex(&cfg.frame.header).unwrap();
        assert!(hdr.is_empty());
    }

    #[test]
    fn frame_cfg_cycle_fields_default_for_old_json() {
        // 旧 .config.json 没有循环码字段：serde(default) 应给出空串，不破坏老配置读取
        let j = r#"{"frame": {"header": "", "length": "11", "order": "big",
                    "samples": "1", "max_points": "500", "channel_count": "2"},
                   "channels": []}"#;
        let cfg: AppConfig = serde_json::from_str(j).unwrap();
        assert_eq!(cfg.frame.cycle_pos, "");
        assert_eq!(cfg.frame.cycle_width, "");

        // 新配置能读回循环码位置与位宽
        let j2 = r#"{"frame": {"header": "", "length": "11", "order": "big",
                     "samples": "1", "max_points": "500", "channel_count": "2",
                     "cycle_pos": "4", "cycle_width": "16"},
                    "channels": []}"#;
        let cfg2: AppConfig = serde_json::from_str(j2).unwrap();
        assert_eq!(cfg2.frame.cycle_pos, "4");
        assert_eq!(cfg2.frame.cycle_width, "16");
    }
}
