fn encode_sector(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    max_add_len: u32,
) -> Vec<Cmd> {
    let _ = (ls_cost, max_add_len);
    let mut ops = Vec::new();
    let mut ram = Vec::new();
    let mut i = 0usize;
    while i < 2048 {
        ram.push(0xFFu8);
        i += 1;
    }
    let mut s = 0usize;
    while s < old_real.len() {
        let start = old_real[s].start;
        let len = old_real[s].len;
        ops.push(Cmd {
            op: 1,
            addr: start,
            buf: start,
            len,
        });
        let mut n = 0u32;
        while n < len {
            ram[(start + n) as usize] = old[(start + n) as usize];
            n += 1;
        }
        s += 1;
    }
    let mut runs_at = Vec::new();
    let mut runs_end = Vec::new();
    let mut off = 0u32;
    while off < 2048 {
        if !span_has(new_real, off) {
            off += 1;
            continue;
        }
        if ram[off as usize] != newb[off as usize] {
            let start = off;
            while off < 2048 && span_has(new_real, off) && ram[off as usize] != newb[off as usize] {
                off += 1;
            }
            runs_at.push(start);
            runs_end.push(off);
        } else {
            off += 1;
        }
    }
    let mut m_at = Vec::new();
    let mut m_end = Vec::new();
    let mut r = 0usize;
    while r < runs_at.len() {
        if !m_at.is_empty() {
            let prev = m_at.len() - 1;
            if runs_at[r] - m_end[prev] < add_header {
                m_end[prev] = runs_end[r];
                r += 1;
                continue;
            }
        }
        m_at.push(runs_at[r]);
        m_end.push(runs_end[r]);
        r += 1;
    }
    let mut a = 0usize;
    while a < m_at.len() {
        ops.push(Cmd {
            op: 2,
            addr: 0,
            buf: m_at[a],
            len: m_end[a] - m_at[a],
        });
        a += 1;
    }
    ops
}

fn span_has(spans: &[Span], off: u32) -> bool {
    let mut i = 0usize;
    while i < spans.len() {
        if off >= spans[i].start && off < spans[i].start + spans[i].len {
            return true;
        }
        i += 1;
    }
    false
}
