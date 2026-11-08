//! 通道数值提取 —— 与 Python 版 `_extract_value_from_frame` / `sign_extend` 语义逐一对齐。
//!
//! 定稿语义（2026-11-05 与用户确认）：
//! - 索引任意 1~4 字节（实际不限），按书写顺序拼接：大端 = 顺序为高位→低位，小端 = 逆序；
//! - 任何一个索引越界 ⇒ **整条值返回 None**（不是丢弃越界字节拼剩余）；
//! - `sign_bit < 0` 表示无符号；否则按位扩展成 i64。

/// 从一帧中按索引提取数值。任何索引越界返回 `None`（整条无数据）。
pub fn extract_value(frame: &[u8], indices: &[usize], sign_bit: i32, big_endian: bool) -> Option<i64> {
    if indices.is_empty() {
        return None;
    }
    let mut value: u64 = 0;
    if big_endian {
        for &idx in indices {
            let b = *frame.get(idx)?;
            value = (value << 8) | b as u64;
        }
    } else {
        for &idx in indices.iter().rev() {
            let b = *frame.get(idx)?;
            value = (value << 8) | b as u64;
        }
    }
    Some(sign_extend(value, sign_bit))
}

/// 符号位扩展。`sign_bit_pos < 0` 时原样返回（无符号）。
///
/// Python 版：`value | ~((1 << (sb+1)) - 1)`，掩码为负（无限高位全 1），`|` 只置 1：
/// - 第 sb 位 = 1 → 高位全置 1 → 结果 = (value & mask) - (1 << (sb+1))（负数）；
/// - 第 sb 位 = 0 → `| 0` 不变 → **原值原样返回**（高位保留，不截断！）。
/// 三个语义陷阱中的第三个：真实流量（sign15+2字节，bit 恒为 1）不暴露，
/// sign 位低于位宽且 bit=0 的随机用例才暴露。
pub fn sign_extend(value: u64, sign_bit_pos: i32) -> i64 {
    if sign_bit_pos < 0 {
        return value as i64;
    }
    let sb = sign_bit_pos as u32;
    let mask = (1u64 << (sb + 1)) - 1;
    if value & (1u64 << sb) != 0 {
        ((value & mask) as i64) - (1i64 << (sb + 1))
    } else {
        value as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 探针同款帧：EB 90 01 02 03 04 05 06 FF FE 7F 00 00 00 2A 11
    const FRAME: [u8; 16] = [0xEB, 0x90, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                             0xFF, 0xFE, 0x7F, 0x00, 0x00, 0x00, 0x2A, 0x11];

    #[test]
    fn multi_width_big_endian() {
        assert_eq!(extract_value(&FRAME, &[2], -1, true), Some(0x1));
        assert_eq!(extract_value(&FRAME, &[2, 3], -1, true), Some(0x102));
        assert_eq!(extract_value(&FRAME, &[2, 3, 4], -1, true), Some(0x10203));
        assert_eq!(extract_value(&FRAME, &[11, 12, 13, 14], -1, true), Some(0x2A));
    }

    #[test]
    fn little_endian_reverses() {
        // f14 = 00 01 02 ... 0D（与探针一致）
        let f14: [u8; 14] = core::array::from_fn(|i| i as u8);
        assert_eq!(extract_value(&f14, &[11, 12, 13], -1, false), Some(0xD0C0B));
        assert_eq!(extract_value(&f14, &[11, 12], -1, false), Some(0xC0B));
        // 16 字节帧 [11,12,13] 三个字节都是 0x00 → 0
        assert_eq!(extract_value(&FRAME, &[11, 12, 13], -1, false), Some(0));
    }

    #[test]
    fn header_bytes_readable_but_constant() {
        // 手填 0,1：能取到帧头常量 0xEB90（合法，曲线为水平线）
        assert_eq!(extract_value(&FRAME, &[0, 1], -1, true), Some(0xEB90));
    }

    #[test]
    fn duplicate_indices_share_bytes() {
        assert_eq!(extract_value(&FRAME, &[2, 3], -1, true),
                   extract_value(&FRAME, &[2, 3], -1, true));
        // 与 CH1 共用同一对字节，结果一致（语义允许）
        let a = extract_value(&FRAME, &[2, 3], -1, true);
        let b = extract_value(&FRAME, &[2, 3], -1, true);
        assert_eq!(a, b);
    }

    #[test]
    fn out_of_range_returns_none() {
        // FRAME 16 字节：有效索引 0~15
        assert_eq!(extract_value(&FRAME, &[16, 17], -1, true), None);
        assert_eq!(extract_value(&FRAME, &[15, 16], -1, true), None);
        // 部分越界：整条无数据（2026-11-05 定稿）
        assert_eq!(extract_value(&FRAME, &[13, 15, 16], -1, true), None);
        assert_eq!(extract_value(&FRAME, &[11, 12, 13, 16], -1, true), None);
        assert_eq!(extract_value(&FRAME, &[], -1, true), None);
        // 边界内仍有效
        assert_eq!(extract_value(&FRAME, &[15], -1, true), Some(0x11));
    }

    #[test]
    fn sign_bit_variants() {
        // 0xFF FE，sign 15 → -2
        assert_eq!(extract_value(&FRAME, &[8, 9], 15, true), Some(-2));
        assert_eq!(extract_value(&FRAME, &[8, 9], -1, true), Some(0xFFFE));
        assert_eq!(sign_extend(0x8000, 15), -32768);
        assert_eq!(sign_extend(0xFFFF, 15), -1);
        assert_eq!(sign_extend(0x7FFF, 15), 32767);
        assert_eq!(sign_extend(12345, -1), 12345);
        // 4 字节符号位 31
        let f: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFE];
        assert_eq!(extract_value(&f, &[0, 1, 2, 3], 31, true), Some(-2));
        // 3 字节符号位 23
        let g: [u8; 3] = [0xFF, 0xFF, 0xFF];
        assert_eq!(extract_value(&g, &[0, 1, 2], 23, true), Some(-1));
    }

    #[test]
    fn sign_bit_zero_keeps_full_value() {
        // Python `|` 语义：符号位=0 时原值原样返回（高位保留，不截断）
        // 0xAD46A644 的 bit23 = 0 → 原值
        assert_eq!(sign_extend(0xAD46A644, 23), 0xAD46A644 as i64);
        // 0x16D9 的 bit7 = 1 → 截断扩展 → -39
        assert_eq!(sign_extend(0x16D9, 7), -39);
        // sign 位低于位宽且 bit=1 → 截断扩展
        assert_eq!(sign_extend(0x1FF, 7), -1); // 低8位 0xFF → -1
    }
}
