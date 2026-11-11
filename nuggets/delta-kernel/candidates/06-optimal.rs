fn encode_sector(
    old: &[u8],
    old_real: &[Span],
    newb: &[u8],
    new_real: &[Span],
    ls_cost: u32,
    add_header: u32,
    max_add_len: u32,
) -> Vec<Cmd> {
    let need = need_offsets(new_real);
    let n = need.len();
    if n == 0 {
        return Vec::new();
    }
    let inf = 1000000000u32;
    let mut dp = Vec::new();
    let mut kind = Vec::new();
    let mut ks = Vec::new();
    let mut srcs = Vec::new();
    let mut t = 0usize;
    while t < n {
        dp.push(inf);
        kind.push(0u8);
        ks.push(0usize);
        srcs.push(0u32);
        t += 1;
    }
    dp.push(0u32);
    let mut i = n;
    while i > 0 {
        i -= 1;
        let pos = need[i];
        let mut best = inf;
        let mut best_kind = 0u8;
        let mut best_k = 0usize;
        let mut best_src = 0u32;
        if newb[pos as usize] == 0xFF && dp[i + 1] < best {
            best = dp[i + 1];
            best_kind = 1;
            best_k = 1;
        }
        let mut k = 1usize;
        while i + k <= n {
            let span = need[i + k - 1] + 1 - pos;
            if span > 2048 {
                break;
            }
            if span == 2048 || span <= max_add_len {
                let cost = add_header + span + dp[i + k];
                if cost < best {
                    best = cost;
                    best_kind = 2;
                    best_k = k;
                }
            }
            k += 1;
        }
        let mut s = 0usize;
        while s < old_real.len() {
            let run_lo = old_real[s].start;
            let run_hi = old_real[s].start + old_real[s].len;
            let mut src = run_lo;
            while src < run_hi {
                if old[src as usize] != newb[pos as usize] {
                    src += 1;
                    continue;
                }
                let first = ls_cost + dp[i + 1];
                if first < best {
                    best = first;
                    best_kind = 3;
                    best_k = 1;
                    best_src = src;
                }
                let mut hole = 0u32;
                let mut have = false;
                let mut group_start = 0u32;
                let mut last = 0u32;
                let mut j = i + 1;
                while j < n {
                    let dest = need[j];
                    let src_off = src + (dest - pos);
                    if src_off >= run_hi {
                        break;
                    }
                    if old[src_off as usize] != newb[dest as usize] {
                        let noted = note_hole(
                            hole,
                            have,
                            group_start,
                            last,
                            dest,
                            add_header,
                            max_add_len,
                        );
                        if !noted.ok {
                            break;
                        }
                        hole = noted.cost;
                        have = noted.have;
                        group_start = noted.group_start;
                        last = noted.last;
                        if ls_cost + hole >= best {
                            break;
                        }
                        j += 1;
                        continue;
                    }
                    let cost = ls_cost + hole + dp[j + 1];
                    if cost < best {
                        best = cost;
                        best_kind = 3;
                        best_k = j + 1 - i;
                        best_src = src;
                    }
                    j += 1;
                }
                src += 1;
            }
            s += 1;
        }
        dp[i] = best;
        kind[i] = best_kind;
        ks[i] = best_k;
        srcs[i] = best_src;
    }
    if dp[0] >= inf {
        return split_new(new_real, max_add_len);
    }
    let mut ops = Vec::new();
    let mut at = 0usize;
    while at < n {
        if kind[at] == 1 {
            at += 1;
        } else if kind[at] == 2 {
            let k = ks[at];
            let buf = need[at];
            ops.push(Cmd {
                op: 2,
                addr: 0,
                buf,
                len: need[at + k - 1] + 1 - buf,
            });
            at += k;
        } else if kind[at] == 3 {
            let k = ks[at];
            let buf = need[at];
            let src = srcs[at];
            ops.push(Cmd {
                op: 1,
                addr: src,
                buf,
                len: need[at + k - 1] + 1 - buf,
            });
            push_holes(
                &mut ops,
                old,
                newb,
                &need,
                at,
                k,
                src,
                add_header,
                max_add_len,
            );
            at += k;
        } else {
            return split_new(new_real, max_add_len);
        }
    }
    ops
}

struct Hole {
    ok: bool,
    cost: u32,
    have: bool,
    group_start: u32,
    last: u32,
}

fn note_hole(
    cost: u32,
    have: bool,
    group_start: u32,
    last: u32,
    off: u32,
    header: u32,
    max_add: u32,
) -> Hole {
    if !have {
        if max_add == 0 {
            return Hole {
                ok: false,
                cost: 0,
                have: false,
                group_start: 0,
                last: 0,
            };
        }
        return Hole {
            ok: true,
            cost: header + 1,
            have: true,
            group_start: off,
            last: off,
        };
    }
    let gap = off - last - 1;
    let new_span = off - group_start + 1;
    if gap < header && (new_span <= max_add || new_span == 2048) {
        Hole {
            ok: true,
            cost: cost + (off - last),
            have: true,
            group_start,
            last: off,
        }
    } else if max_add >= 1 {
        Hole {
            ok: true,
            cost: cost + header + 1,
            have: true,
            group_start: off,
            last: off,
        }
    } else {
        Hole {
            ok: false,
            cost: 0,
            have: false,
            group_start: 0,
            last: 0,
        }
    }
}

fn push_holes(
    ops: &mut Vec<Cmd>,
    old: &[u8],
    newb: &[u8],
    need: &[u32],
    at: usize,
    k: usize,
    src: u32,
    add_header: u32,
    max_add: u32,
) {
    let buf = need[at];
    let mut hole = 0u32;
    let mut have = false;
    let mut group_start = 0u32;
    let mut last = 0u32;
    let mut j = at + 1;
    while j < at + k {
        let dest = need[j];
        let addr = src + (dest - buf);
        if (addr as usize) < old.len() && old[addr as usize] != newb[dest as usize] {
            let before_have = have;
            let before_start = group_start;
            let before_last = last;
            let noted = note_hole(hole, have, group_start, last, dest, add_header, max_add);
            if !noted.ok {
                return;
            }
            if before_have && noted.group_start != before_start {
                ops.push(Cmd {
                    op: 2,
                    addr: 0,
                    buf: before_start,
                    len: before_last - before_start + 1,
                });
            }
            hole = noted.cost;
            have = noted.have;
            group_start = noted.group_start;
            last = noted.last;
        }
        j += 1;
    }
    if have {
        ops.push(Cmd {
            op: 2,
            addr: 0,
            buf: group_start,
            len: last - group_start + 1,
        });
    }
}

fn need_offsets(spans: &[Span]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut s = 0usize;
    while s < spans.len() {
        let mut i = 0u32;
        while i < spans[s].len {
            out.push(spans[s].start + i);
            i += 1;
        }
        s += 1;
    }
    out
}

fn split_new(spans: &[Span], max_add: u32) -> Vec<Cmd> {
    let mut ops = Vec::new();
    let mut s = 0usize;
    while s < spans.len() {
        let mut left = spans[s].len;
        let mut off = spans[s].start;
        while left > 0 {
            let mut n = left;
            if n != 2048 && n > max_add {
                n = max_add;
            }
            if n == 2048 && off != 0 {
                n = max_add;
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
