fn encode_sector(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    max_add_len: u32,
) -> Vec<Cmd> {
    let _ = (old, newb, ls_cost, add_header);
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
    let mut addr = 0u32;
    let mut found = false;
    while addr < 2048 {
        if !real_at(old_real, addr) {
            found = true;
            break;
        }
        addr += 1;
    }
    if found {
        ops.push(Cmd {
            op: 1,
            addr,
            buf: addr,
            len: 1,
        });
    } else {
        ops.push(Cmd {
            op: 1,
            addr: 0,
            buf: 0,
            len: 4096,
        });
    }
    ops
}

fn real_at(spans: &[Span], off: u32) -> bool {
    let mut i = 0usize;
    while i < spans.len() {
        if off >= spans[i].start && off < spans[i].start + spans[i].len {
            return true;
        }
        i += 1;
    }
    false
}
