//! Motorola S-record parser and writer.
//!
//! S0 is ignored. S1/S2/S3 are data. S5/S6, when present, must match the
//! data-record count. S7/S8/S9 end the file. The checksum is the one's
//! complement of the sum of the count, address, and data bytes.

use crate::error::{err, Result};

use super::image::{coalesce, Region};

/// Parse `text` into coalesced, ascending, non-adjacent regions.
pub fn parse(text: &str) -> Result<Vec<Region>> {
    let mut runs: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut data_records = 0u32;
    let mut counted: Option<u32> = None;
    let mut saw_end = false;
    for (index, raw) in text.split('\n').enumerate() {
        let line_no = index + 1;
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if saw_end {
            return Err(err(format!("SREC line {line_no}: data after termination")));
        }
        if !line.starts_with('S') && !line.starts_with('s') {
            return Err(err(format!("SREC line {line_no}: not an S-record")));
        }
        let kind = line.as_bytes().get(1).map(|b| b.to_ascii_uppercase());
        let Some(kind) = kind else {
            return Err(err(format!("SREC line {line_no}: truncated")));
        };
        if kind == b'0' {
            continue;
        }
        let bytes = decode_hex(line_no, &line[2..])?;
        if bytes.is_empty() {
            return Err(err(format!("SREC line {line_no}: missing count")));
        }
        let count = bytes[0] as usize;
        if bytes.len() != count + 1 {
            return Err(err(format!(
                "SREC line {line_no}: count {count} does not match the record length"
            )));
        }
        let body = &bytes[1..bytes.len() - 1];
        let sum = bytes[..bytes.len() - 1]
            .iter()
            .fold(0u8, |acc, b| acc.wrapping_add(*b));
        if !sum != bytes[bytes.len() - 1] {
            return Err(err(format!("SREC line {line_no}: checksum mismatch")));
        }
        let addr_len = match kind {
            b'1' | b'5' | b'9' => 2,
            b'2' | b'6' | b'8' => 3,
            b'3' | b'7' => 4,
            _ => {
                return Err(err(format!(
                    "SREC line {line_no}: unsupported type S{}",
                    kind as char
                )))
            }
        };
        if body.len() < addr_len {
            return Err(err(format!(
                "SREC line {line_no}: record shorter than its address"
            )));
        }
        let addr = read_addr(&body[..addr_len]);
        let data = &body[addr_len..];
        match kind {
            b'1' | b'2' | b'3' => {
                data_records = data_records.saturating_add(1);
                if !data.is_empty() {
                    runs.push((addr, data.to_vec()));
                }
            }
            b'5' | b'6' => counted = Some(addr),
            b'7' | b'8' | b'9' => saw_end = true,
            _ => unreachable!(),
        }
    }
    if !saw_end {
        return Err(err("SREC file has no S7/S8/S9 termination record"));
    }
    if let Some(expected) = counted {
        if expected != data_records {
            return Err(err(format!(
                "SREC record count is {expected}, data records are {data_records}"
            )));
        }
    }
    coalesce(runs)
}

/// Write regions as S3 data records plus an S7 termination.
pub fn write(regions: &[Region]) -> String {
    // S0 payload is the three bytes "HDR".
    let mut out = String::from("S00600004844521B\n");
    let mut data_records = 0u32;
    for region in regions {
        let mut offset = 0usize;
        while offset < region.data.len() {
            let n = (region.data.len() - offset).min(16);
            let addr = region.address + offset as u32;
            out.push_str(&record(addr, 4, &region.data[offset..offset + n], b'3'));
            out.push('\n');
            data_records = data_records.saturating_add(1);
            offset += n;
        }
    }
    out.push_str(&record(data_records, 2, &[], b'5'));
    out.push('\n');
    out.push_str(&record(0, 4, &[], b'7'));
    out.push('\n');
    out
}

fn record(addr: u32, addr_len: usize, data: &[u8], kind: u8) -> String {
    let mut body = Vec::with_capacity(1 + addr_len + data.len());
    let count = addr_len + data.len() + 1;
    body.push(count as u8);
    for shift in (0..addr_len).rev() {
        body.push((addr >> (8 * shift)) as u8);
    }
    body.extend_from_slice(data);
    let sum = body.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    body.push(!sum);
    let mut line = String::with_capacity(2 + body.len() * 2);
    line.push('S');
    line.push(kind as char);
    for byte in body {
        line.push_str(&format!("{byte:02X}"));
    }
    line
}

fn read_addr(bytes: &[u8]) -> u32 {
    let mut addr = 0u32;
    for byte in bytes {
        addr = (addr << 8) | u32::from(*byte);
    }
    addr
}

fn decode_hex(line_no: usize, text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err(err(format!("SREC line {line_no}: odd hex length")));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi =
            hex_val(bytes[i]).ok_or_else(|| err(format!("SREC line {line_no}: bad hex digit")))?;
        let lo = hex_val(bytes[i + 1])
            .ok_or_else(|| err(format!("SREC line {line_no}: bad hex digit")))?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_bytes_and_rejects_a_bad_checksum() {
        let regions = vec![Region {
            address: 0x0800_0100,
            data: vec![1, 2, 3, 4, 5, 0xFF, 9],
        }];
        let text = write(&regions);
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed, regions);

        let mut broken = text.into_bytes();
        let s3 = broken.windows(2).position(|w| w == b"S3").unwrap();
        let digit = broken[s3 + 12];
        broken[s3 + 12] = if digit == b'0' { b'1' } else { b'0' };
        let broken = String::from_utf8(broken).unwrap();
        assert!(parse(&broken).is_err());
    }

    #[test]
    fn adjacent_runs_coalesce_and_s5_must_match() {
        let text = "\
S3090000000001020304EC
S3090000000405060708D8
S5030002FA
S9030000FC
";
        // Two adjacent 4-byte records at 0 and 4.
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].address, 0);
        assert_eq!(parsed[0].data, vec![1, 2, 3, 4, 5, 6, 7, 8]);

        let bad_count = text.replace("S5030002FA", "S5030003F9");
        assert!(parse(&bad_count)
            .unwrap_err()
            .message
            .contains("record count"));
    }
}
