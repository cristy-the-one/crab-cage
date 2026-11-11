fn encode_sector(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    max_add_len: u32,
) -> Vec<Cmd> {
    let _ = max_add_len;
    // Family point the host grid kept: window 8, stride 1, min_match 3, one probe.
    greedy(
        old, old_real, newb, new_real, ls_cost, add_header, 8, 1, 3, false, 64, false, true,
        true, false,
    )
}

fn greedy(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    window: usize,
    stride: usize,
    min_match: u32,
    multi_probe: bool,
    cap: usize,
    merge_gap: bool,
    demote: bool,
    suppress: bool,
    skip_ff: bool,
) -> Vec<Cmd> {
    let index = build_index(old, old_real, window, stride);
    let mut ops = Vec::new();
    let mut s = 0usize;
    while s < new_real.len() {
        let end = new_real[s].start + new_real[s].len;
        let mut pos = new_real[s].start;
        let mut lit_on = false;
        let mut lit_at = 0u32;
        while pos < end {
            if skip_ff && newb[pos as usize] == 0xFF {
                flush_lit(&mut ops, &mut lit_on, lit_at, pos);
                pos += 1;
                continue;
            }
            let found = best_match(
                old, old_real, newb, &index, pos, end, window, stride, min_match, multi_probe, cap,
            );
            if found.ok {
                flush_lit(&mut ops, &mut lit_on, lit_at, pos);
                ops.push(Cmd {
                    op: 1,
                    addr: found.src,
                    buf: pos,
                    len: found.len,
                });
                pos += found.len;
                continue;
            }
            if !lit_on {
                lit_on = true;
                lit_at = pos;
            }
            pos += 1;
        }
        flush_lit(&mut ops, &mut lit_on, lit_at, end);
        s += 1;
    }
    post_passes(
        &mut ops, newb, ls_cost, add_header, demote, merge_gap, suppress,
    );
    ops
}

struct Found {
    ok: bool,
    src: u32,
    len: u32,
}

struct Index {
    buckets: usize,
    head: Vec<i32>,
    offsets: Vec<u32>,
    hashes: Vec<u32>,
    next: Vec<i32>,
}

fn build_index(old: &[u8], old_real: &[Span], window: usize, stride: usize) -> Index {
    let mut offsets = Vec::new();
    let mut hashes = Vec::new();
    if window >= 1 && stride >= 1 && old.len() >= window {
        let mut pos = 0usize;
        while pos + window <= old.len() {
            let mut real = true;
            let mut all_ff = true;
            let mut i = 0usize;
            while i < window {
                if !real_at(old_real, (pos + i) as u32) {
                    real = false;
                    break;
                }
                if old[pos + i] != 0xFF {
                    all_ff = false;
                }
                i += 1;
            }
            if real && !all_ff {
                offsets.push(pos as u32);
                hashes.push(fnv_at(old, pos, window));
            }
            pos += stride;
        }
    }
    let buckets = bucket_count(offsets.len());
    let mut head = Vec::new();
    let mut b = 0usize;
    while b < buckets {
        head.push(-1i32);
        b += 1;
    }
    let mut next = Vec::new();
    let mut i = 0usize;
    while i < offsets.len() {
        next.push(-1i32);
        i += 1;
    }
    let mut i = 0usize;
    while i < offsets.len() {
        let bucket = (hashes[i] as usize) & (buckets - 1);
        next[i] = head[bucket];
        head[bucket] = i as i32;
        i += 1;
    }
    Index {
        buckets,
        head,
        offsets,
        hashes,
        next,
    }
}

fn bucket_count(entries: usize) -> usize {
    let mut size = 2usize;
    let need = if entries.saturating_mul(2) < 2 {
        2
    } else {
        entries.saturating_mul(2)
    };
    while size < need {
        size <<= 1;
    }
    size
}

fn fnv_at(data: &[u8], at: usize, n: usize) -> u32 {
    let mut hash = 2166136261u32;
    let mut i = 0usize;
    while i < n {
        hash ^= u32::from(data[at + i]);
        hash = ((hash as u64) * 16777619u64) as u32;
        i += 1;
    }
    hash
}

fn best_match(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    index: &Index,
    pos: u32,
    interval_end: u32,
    window: usize,
    stride: usize,
    min_match: u32,
    multi_probe: bool,
    cap: usize,
) -> Found {
    let none = Found {
        ok: false,
        src: 0,
        len: 0,
    };
    if window == 0 || stride == 0 {
        return none;
    }
    let probes = if multi_probe { stride } else { 1 };
    let mut best_len = 0u32;
    let mut best_src = 0u32;
    let mut probe_i = 0usize;
    while probe_i < probes {
        let probe = pos + probe_i as u32;
        if probe + window as u32 > interval_end {
            probe_i += 1;
            continue;
        }
        let hash = fnv_at(newb, probe as usize, window);
        let mut cands = lookup(index, hash);
        sort_by_distance(&mut cands, probe);
        while cands.len() > cap {
            cands.pop();
        }
        let mut c = 0usize;
        while c < cands.len() {
            let cand = cands[c];
            if window_eq(old, old_real, cand, newb, probe, window)
                && extend_back(old, old_real, newb, cand, probe, probe_i as u32)
            {
                let src = cand - probe_i as u32;
                let mut len = (probe_i + window) as u32;
                while pos + len < interval_end
                    && real_at(old_real, src + len)
                    && old[(src + len) as usize] == newb[(pos + len) as usize]
                {
                    len += 1;
                }
                let extent = real_run(old_real, src, len);
                if extent < len {
                    len = extent;
                }
                if pos + len > interval_end {
                    len = interval_end - pos;
                }
                if len > best_len {
                    best_len = len;
                    best_src = src;
                }
            }
            c += 1;
        }
        probe_i += 1;
    }
    if best_len >= min_match {
        Found {
            ok: true,
            src: best_src,
            len: best_len,
        }
    } else {
        none
    }
}

fn lookup(index: &Index, hash: u32) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    if index.buckets == 0 {
        return out;
    }
    let mut slot = index.head[(hash as usize) & (index.buckets - 1)];
    while slot >= 0 {
        let at = slot as usize;
        if index.hashes[at] == hash {
            out.push(index.offsets[at]);
        }
        slot = index.next[at];
    }
    out
}

fn sort_by_distance(cands: &mut Vec<u32>, probe: u32) {
    let mut i = 1usize;
    while i < cands.len() {
        let key = cands[i];
        let key_d = abs_diff(key, probe);
        let mut j = i;
        while j > 0 && abs_diff(cands[j - 1], probe) > key_d {
            cands[j] = cands[j - 1];
            j -= 1;
        }
        cands[j] = key;
        i += 1;
    }
}

fn abs_diff(a: u32, b: u32) -> u32 {
    if a > b {
        a - b
    } else {
        b - a
    }
}

fn window_eq(old: &[u8], old_real: &[Span], addr: u32, newb: &[u8], dest: u32, window: usize) -> bool {
    let mut i = 0usize;
    while i < window {
        let src = addr + i as u32;
        let at = dest + i as u32;
        if !real_at(old_real, src) || old[src as usize] != newb[at as usize] {
            return false;
        }
        i += 1;
    }
    true
}

fn extend_back(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    cand: u32,
    probe: u32,
    dist: u32,
) -> bool {
    if cand < dist || probe < dist {
        return false;
    }
    let mut back = 0u32;
    while back < dist {
        let src = cand - back - 1;
        let dst = probe - back - 1;
        if !real_at(old_real, src) || old[src as usize] != newb[dst as usize] {
            return false;
        }
        back += 1;
    }
    true
}

fn real_run(spans: &[Span], addr: u32, max_len: u32) -> u32 {
    let mut n = 0u32;
    while n < max_len && real_at(spans, addr + n) {
        n += 1;
    }
    n
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

fn flush_lit(ops: &mut Vec<Cmd>, lit_on: &mut bool, lit_at: u32, pos: u32) {
    if *lit_on && pos > lit_at {
        ops.push(Cmd {
            op: 2,
            addr: 0,
            buf: lit_at,
            len: pos - lit_at,
        });
    }
    *lit_on = false;
}

fn post_passes(
    ops: &mut Vec<Cmd>,
    newb: &[u8],
    ls_cost: u32,
    add_header: u32,
    demote: bool,
    merge_gap: bool,
    suppress: bool,
) {
    if demote {
        let mut i = 0usize;
        while i < ops.len() {
            if ops[i].op == 1 && ls_cost >= add_header + ops[i].len {
                ops[i].op = 2;
                ops[i].addr = 0;
            }
            i += 1;
        }
    }
    merge_touching(ops);
    if merge_gap {
        merge_with_gap(ops, add_header);
    }
    if suppress {
        suppress_ff(ops, newb);
    }
}

fn merge_touching(ops: &mut Vec<Cmd>) {
    let mut out: Vec<Cmd> = Vec::new();
    let mut i = 0usize;
    while i < ops.len() {
        if ops[i].op == 2 && !out.is_empty() {
            let prev = out.len() - 1;
            if out[prev].op == 2 && out[prev].buf + out[prev].len == ops[i].buf {
                out[prev].len += ops[i].len;
                i += 1;
                continue;
            }
        }
        out.push(Cmd {
            op: ops[i].op,
            addr: ops[i].addr,
            buf: ops[i].buf,
            len: ops[i].len,
        });
        i += 1;
    }
    *ops = out;
}

fn merge_with_gap(ops: &mut Vec<Cmd>, add_header: u32) {
    let mut out: Vec<Cmd> = Vec::new();
    let mut i = 0usize;
    while i < ops.len() {
        if ops[i].op == 2 && !out.is_empty() {
            let prev = out.len() - 1;
            if out[prev].op == 2 {
                let prev_end = out[prev].buf + out[prev].len;
                if ops[i].buf >= prev_end && ops[i].buf - prev_end < add_header {
                    out[prev].len = ops[i].buf + ops[i].len - out[prev].buf;
                    i += 1;
                    continue;
                }
            }
        }
        out.push(Cmd {
            op: ops[i].op,
            addr: ops[i].addr,
            buf: ops[i].buf,
            len: ops[i].len,
        });
        i += 1;
    }
    *ops = out;
}

fn suppress_ff(ops: &mut Vec<Cmd>, newb: &[u8]) {
    let mut load_buf = Vec::new();
    let mut load_len = Vec::new();
    let mut i = 0usize;
    while i < ops.len() {
        if ops[i].op == 1 {
            load_buf.push(ops[i].buf);
            load_len.push(ops[i].len);
        }
        i += 1;
    }
    let mut kept: Vec<Cmd> = Vec::new();
    let mut i = 0usize;
    while i < ops.len() {
        if ops[i].op == 2 && all_ff(newb, ops[i].buf, ops[i].len) {
            let mut hit = false;
            let mut l = 0usize;
            while l < load_buf.len() {
                if overlaps(load_buf[l], load_len[l], ops[i].buf, ops[i].len) {
                    hit = true;
                    break;
                }
                l += 1;
            }
            if !hit {
                i += 1;
                continue;
            }
        }
        kept.push(Cmd {
            op: ops[i].op,
            addr: ops[i].addr,
            buf: ops[i].buf,
            len: ops[i].len,
        });
        i += 1;
    }
    *ops = kept;
}

fn all_ff(newb: &[u8], buf: u32, len: u32) -> bool {
    let mut i = 0u32;
    while i < len {
        if newb[(buf + i) as usize] != 0xFF {
            return false;
        }
        i += 1;
    }
    true
}

fn overlaps(a: u32, a_len: u32, b: u32, b_len: u32) -> bool {
    let a_end = a + a_len;
    let b_end = b + b_len;
    a < b_end && b < a_end
}
