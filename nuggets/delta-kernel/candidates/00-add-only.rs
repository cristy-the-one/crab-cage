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
