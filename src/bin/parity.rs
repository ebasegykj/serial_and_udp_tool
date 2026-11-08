//! 对拍工具：从 stdin 读 JSON 用例，输出 Rust 侧解析结果 JSON。
//! 由 `_parity_check.py` 驱动（Python 版结果为参照，逐值比对）。

use frameanalyzer::extract::extract_value;
use frameanalyzer::frame_sync::{ChannelCfg, Engine};
use serde::Deserialize;
use serde_json::json;
use std::io::Read;

#[derive(Deserialize)]
struct ExtractCase {
    frame_hex: String,
    big: bool,
    cases: Vec<IdxCase>,
}

#[derive(Deserialize)]
struct IdxCase {
    indices: Vec<i64>,
    sign: i64,
}

#[derive(Deserialize)]
struct StreamCase {
    stream_hex: String,
    header_hex: String,
    frame_length: u64,
    spp: u64,
    big: bool,
    save_enabled: bool,
    channels: Vec<ChCase>,
}

#[derive(Deserialize)]
struct ChCase {
    indices: Vec<i64>,
    sign: i64,
    active: bool,
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
        .collect()
}

fn to_usize(v: &[i64]) -> Vec<usize> {
    v.iter().map(|&x| x as usize).collect()
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let root: serde_json::Value = serde_json::from_str(&input).unwrap();

    // ---- extract 对拍 ----
    let mut extract_out: Vec<Vec<Option<i64>>> = Vec::new();
    for case in root["extract"].as_array().unwrap() {
        let ec: ExtractCase = serde_json::from_value(case.clone()).unwrap();
        let frame = hex_to_bytes(&ec.frame_hex);
        let mut vals = Vec::new();
        for c in &ec.cases {
            vals.push(extract_value(&frame, &to_usize(&c.indices), c.sign as i32, ec.big));
        }
        extract_out.push(vals);
    }

    // ---- stream 对拍 ----
    let mut stream_out: Vec<serde_json::Value> = Vec::new();
    for case in root["stream"].as_array().unwrap() {
        let sc: StreamCase = serde_json::from_value(case.clone()).unwrap();
        let mut e = Engine::new(
            hex_to_bytes(&sc.header_hex),
            sc.frame_length as usize,
            sc.spp as usize,
            sc.big,
            sc.channels
                .iter()
                .map(|ch| ChannelCfg {
                    indices: to_usize(&ch.indices),
                    sign_bit: ch.sign as i32,
                    active: ch.active,
                })
                .collect(),
            sc.save_enabled,
        );
        e.rx_buffer = hex_to_bytes(&sc.stream_hex);
        e.process_buffer();
        stream_out.push(json!({
            "frames": e.frame_count,
            "remain": e.rx_buffer.len(),
            "remain_hex": e.rx_buffer.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "),
            "sample_count": e.sample_count,
            "values": e.channel_values,
            "diffs": e.channel_diff_values,
            "csv": e.extracted_save_buffer,
            "frames_hex": e.frame_log.iter().map(|f| f.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>(),
        }));
    }

    println!("{}", json!({ "extract": extract_out, "stream": stream_out }));
}
