use std::io::{self, BufRead};

// One real run inside a 2 KB sector. Offsets are from the sector base.
struct Span {
    start: u32,
    len: u32,
}

// op 1 is LOAD (addr, buf, len). op 2 is ADD (buf, len). The ADD payload is
// the new sector's bytes at buf; the kernel does not choose literal bytes.
struct Cmd {
    op: u8,
    addr: u32,
    buf: u32,
    len: u32,
}

struct Sec {
    base: u32,
    old: Vec<u8>,
    newb: Vec<u8>,
    old_real: Vec<Span>,
    new_real: Vec<Span>,
}

struct Plan {
    base: u32,
    cmds: Vec<Cmd>,
}

// Kernel contract. The lines between the markers are the whole strategy,
// including any helper functions it needs. The harness calls encode_sector
// once per stored sector per add_pat width.
//
// old and newb are 2048 bytes. A byte is real only inside old_real / new_real.
// A LOAD whose source is not entirely old-real is rejected, even if that
// flash byte would read as 0xFF. RAM starts at 0xFF, and a 0xFF new-real byte
// may be left untouched.
//
// ls_cost is the LOAD header in bytes (5 on the 2 KB profile). add_header is
// the ADD header in bytes for this width. max_add_len is the longest ADD the
// width can encode; a length of 2048 is legal at every width (the 0 sentinel).
// Lengths from max_add_len+1 through 2047 are not.
//
// Do not print. The harness prints the command list and the score. The score
// counts only when the reference decoder accepts the packed v0.03 file.
//
// CRAB:BEGIN
fn encode_sector(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    max_add_len: u32,
) -> Vec<Cmd> {
    let _ = (old, old_real, newb, ls_cost, add_header);
    let mut ops = Vec::new();
    let mut s = 0usize;
    while s < new_real.len() {
        let mut left = new_real[s].len;
        let mut off = new_real[s].start;
        while left > 0 {
            let mut n = left;
            if n != 2048 && n > max_add_len {
                n = max_add_len;
            }
            if n == 2048 && off != 0 {
                n = max_add_len;
                if n > left {
                    n = left;
                }
            }
            if n == 0 {
                break;
            }
            ops.push(Cmd {
                op: 2,
                addr: 0,
                buf: off,
                len: n,
            });
            off += n;
            left -= n;
        }
        s += 1;
    }
    ops
}
// CRAB:END

fn main() {
    let stdin = io::stdin();
    let mut lines = Vec::new();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                println!("error: stdin {error}");
                return;
            }
        };
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    let sectors = match parse_sectors(&lines) {
        Ok(sectors) => sectors,
        Err(error) => {
            println!("error: {error}");
            return;
        }
    };
    let mut any = false;
    let mut s = 0usize;
    while s < sectors.len() {
        if needs_store(&sectors[s]) {
            any = true;
            break;
        }
        s += 1;
    }
    if !any {
        println!("sectors 0");
        println!("score: 0");
        return;
    }
    let mut best_size: u64 = u64::MAX;
    let mut best: Vec<Plan> = Vec::new();
    let mut width = 1u32;
    while width <= 11 {
        let header = add_header_bytes(width);
        let max_add = (1u32 << width) - 1;
        let mut plans = Vec::new();
        let mut legal = true;
        let mut s = 0usize;
        while s < sectors.len() {
            if needs_store(&sectors[s]) {
                let cmds = encode_sector(
                    &sectors[s].old,
                    &sectors[s].old_real,
                    &sectors[s].newb,
                    &sectors[s].new_real,
                    5,
                    header,
                    max_add,
                );
                if !acceptable(&sectors[s], &cmds, max_add) {
                    legal = false;
                    break;
                }
                plans.push(Plan {
                    base: sectors[s].base,
                    cmds,
                });
            }
            s += 1;
        }
        if legal {
            let bits = tuned_bits(&plans);
            let size = file_size(&plans, bits);
            if size < best_size {
                best_size = size;
                best = plans;
            }
        }
        width += 1;
    }
    if best_size == u64::MAX {
        println!("error: no width covered the new bytes");
        return;
    }
    println!("sectors {}", best.len());
    let mut p = 0usize;
    while p < best.len() {
        println!("base {}", best[p].base);
        let mut c = 0usize;
        while c < best[p].cmds.len() {
            let cmd = &best[p].cmds[c];
            if cmd.op == 1 {
                println!("L {} {} {}", cmd.addr, cmd.buf, cmd.len);
            } else if cmd.op == 2 {
                println!("A {} {}", cmd.buf, cmd.len);
            }
            c += 1;
        }
        println!(".");
        p += 1;
    }
    println!("score: {}", best_size);
}

fn parse_sectors(lines: &[String]) -> Result<Vec<Sec>, String> {
    if lines.is_empty() || !lines[0].starts_with("pair ") {
        return Err("expected a pair line".to_string());
    }
    if lines.len() < 2 || !lines[1].starts_with("sectors ") {
        return Err("expected a sector count".to_string());
    }
    let count: usize = match lines[1][8..].trim().parse() {
        Ok(count) => count,
        Err(_) => return Err("bad sector count".to_string()),
    };
    let mut i = 2usize;
    let mut out = Vec::new();
    let mut n = 0usize;
    while n < count {
        if i >= lines.len() || !lines[i].starts_with("base ") {
            return Err("expected a sector base".to_string());
        }
        let base: u32 = match lines[i][5..].trim().parse() {
            Ok(base) => base,
            Err(_) => return Err("bad sector base".to_string()),
        };
        i += 1;
        let old_real = match take_spans(lines, &mut i, "old_spans ") {
            Ok(spans) => spans,
            Err(error) => return Err(error),
        };
        let new_real = match take_spans(lines, &mut i, "new_spans ") {
            Ok(spans) => spans,
            Err(error) => return Err(error),
        };
        if i >= lines.len() || lines[i] != "old" {
            return Err("expected old bytes".to_string());
        }
        i += 1;
        if i >= lines.len() {
            return Err("missing old hex".to_string());
        }
        let old = match unhex(&lines[i]) {
            Ok(bytes) => bytes,
            Err(error) => return Err(error),
        };
        i += 1;
        if i >= lines.len() || lines[i] != "new" {
            return Err("expected new bytes".to_string());
        }
        i += 1;
        if i >= lines.len() {
            return Err("missing new hex".to_string());
        }
        let newb = match unhex(&lines[i]) {
            Ok(bytes) => bytes,
            Err(error) => return Err(error),
        };
        i += 1;
        out.push(Sec {
            base,
            old,
            newb,
            old_real,
            new_real,
        });
        n += 1;
    }
    if i != lines.len() {
        return Err("trailing input".to_string());
    }
    Ok(out)
}

fn take_spans(lines: &[String], i: &mut usize, label: &str) -> Result<Vec<Span>, String> {
    if *i >= lines.len() || !lines[*i].starts_with(label) {
        return Err(format!("expected {label}"));
    }
    let count: usize = match lines[*i][label.len()..].trim().parse() {
        Ok(count) => count,
        Err(_) => return Err(format!("bad {label}")),
    };
    *i += 1;
    let mut spans = Vec::new();
    let mut n = 0usize;
    while n < count {
        if *i >= lines.len() {
            return Err("missing span".to_string());
        }
        let mut parts = lines[*i].split_whitespace();
        let start = match parts.next() {
            Some(text) => match text.parse::<u32>() {
                Ok(value) => value,
                Err(_) => return Err("bad span".to_string()),
            },
            None => return Err("bad span".to_string()),
        };
        let len = match parts.next() {
            Some(text) => match text.parse::<u32>() {
                Ok(value) => value,
                Err(_) => return Err("bad span".to_string()),
            },
            None => return Err("bad span".to_string()),
        };
        if parts.next().is_some() {
            return Err("bad span".to_string());
        }
        spans.push(Span { start, len });
        *i += 1;
        n += 1;
    }
    Ok(spans)
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if bytes.len() != 4096 {
        return Err("sector hex is not 2048 bytes".to_string());
    }
    let mut out = Vec::new();
    let mut n = 0usize;
    while n < bytes.len() {
        let hi = match hex_val(bytes[n]) {
            Some(value) => value,
            None => return Err("bad hex".to_string()),
        };
        let lo = match hex_val(bytes[n + 1]) {
            Some(value) => value,
            None => return Err("bad hex".to_string()),
        };
        out.push((hi << 4) | lo);
        n += 2;
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Option<u8> {
    if byte >= b'0' && byte <= b'9' {
        Some(byte - b'0')
    } else if byte >= b'a' && byte <= b'f' {
        Some(byte - b'a' + 10)
    } else if byte >= b'A' && byte <= b'F' {
        Some(byte - b'A' + 10)
    } else {
        None
    }
}

fn needs_store(sec: &Sec) -> bool {
    let mut s = 0usize;
    while s < sec.new_real.len() {
        let mut i = 0u32;
        while i < sec.new_real[s].len {
            let off = sec.new_real[s].start + i;
            if !covered(&sec.old_real, off, 1) || sec.old[off as usize] != sec.newb[off as usize] {
                return true;
            }
            i += 1;
        }
        s += 1;
    }
    false
}

fn covered(spans: &[Span], start: u32, len: u32) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let mut cursor = start;
    let mut i = 0usize;
    while i < spans.len() {
        let span_end = spans[i].start + spans[i].len;
        if span_end > cursor {
            if spans[i].start > cursor {
                return false;
            }
            cursor = span_end;
            if cursor >= end {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn acceptable(sec: &Sec, cmds: &[Cmd], max_add: u32) -> bool {
    let mut ram = Vec::new();
    let mut i = 0usize;
    while i < 2048 {
        ram.push(0xFFu8);
        i += 1;
    }
    let mut c = 0usize;
    while c < cmds.len() {
        let cmd = &cmds[c];
        if cmd.len < 1 || cmd.buf.checked_add(cmd.len).is_none() || cmd.buf + cmd.len > 2048 {
            return false;
        }
        if cmd.op == 1 {
            if cmd.addr.checked_add(cmd.len).is_none() || cmd.addr + cmd.len > 2048 {
                return false;
            }
            if cmd.len != 2048 && cmd.len > 2047 {
                return false;
            }
            if !covered(&sec.old_real, cmd.addr, cmd.len) {
                return false;
            }
            let mut n = 0u32;
            while n < cmd.len {
                ram[(cmd.buf + n) as usize] = sec.old[(cmd.addr + n) as usize];
                n += 1;
            }
        } else if cmd.op == 2 {
            if cmd.len != 2048 && cmd.len > max_add {
                return false;
            }
            let mut n = 0u32;
            while n < cmd.len {
                ram[(cmd.buf + n) as usize] = sec.newb[(cmd.buf + n) as usize];
                n += 1;
            }
        } else {
            return false;
        }
        c += 1;
    }
    let mut s = 0usize;
    while s < sec.new_real.len() {
        let mut n = 0u32;
        while n < sec.new_real[s].len {
            let off = sec.new_real[s].start + n;
            if ram[off as usize] != sec.newb[off as usize] {
                return false;
            }
            n += 1;
        }
        s += 1;
    }
    true
}

fn tuned_bits(plans: &[Plan]) -> u32 {
    let mut bits = 1u32;
    let mut p = 0usize;
    while p < plans.len() {
        let mut c = 0usize;
        while c < plans[p].cmds.len() {
            if plans[p].cmds[c].op == 2 {
                let len = plans[p].cmds[c].len;
                let mut width = 1u32;
                while width < 11 && !fits_pat(len, width) {
                    width += 1;
                }
                if width > bits {
                    bits = width;
                }
            }
            c += 1;
        }
        p += 1;
    }
    bits
}

fn fits_pat(len: u32, bits: u32) -> bool {
    len == 2048 || (len >= 1 && len <= (1u32 << bits) - 1)
}

fn add_header_bytes(bits: u32) -> u32 {
    let used = 2 + bits + 11;
    (used + unused_bits(used)) / 8
}

fn unused_bits(used: u32) -> u32 {
    let rem = used % 8;
    if rem == 0 {
        8
    } else {
        8 - rem
    }
}

fn file_size(plans: &[Plan], bits: u32) -> u64 {
    let mut size = 12u64 + 4 * plans.len() as u64;
    let add_h = u64::from(add_header_bytes(bits));
    let mut p = 0usize;
    while p < plans.len() {
        let mut c = 0usize;
        while c < plans[p].cmds.len() {
            if plans[p].cmds[c].op == 1 {
                size += 5;
            } else {
                size += add_h + u64::from(plans[p].cmds[c].len);
            }
            c += 1;
        }
        size += 5;
        p += 1;
    }
    size
}
