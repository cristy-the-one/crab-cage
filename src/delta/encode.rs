//! Sector encoders.
//!
//! Strategy A and strategy B follow the v0.03 generator. The optimal parse is
//! a shortest path over the new sector's real bytes. A LOAD costs a fixed
//! header and may be patched by later ADDs (that is how a whole-sector copy
//! beats copying only the exact-match runs). An ADD costs its header plus its
//! span, and a 0xFF byte can be left to the erased RAM. `add_pat_size_bits`
//! is chosen after the commands exist, so every file-level encoder tries each
//! width and keeps the smallest packed file the reference decoder accepts.

use serde::{Deserialize, Serialize};

use crate::error::{err, Result};

use super::check::{self, VerifyReport};
use super::image::{covered, needs_store, sectors, store_extent, Image, Sector, Span};
use super::pack::{
    add_header_bytes, fits_pat, ls_header_bytes, max_pat, unused_bits, Op, SectorPlan, SECTOR,
};

/// A packed delta that has already passed the reference decoder.
#[derive(Clone, Debug)]
pub struct Planned {
    pub bytes: Vec<u8>,
    pub plans: Vec<SectorPlan>,
}

/// Knobs for the parameterized greedy kernel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyParams {
    pub window: usize,
    pub stride: usize,
    pub min_match: usize,
    pub multi_probe: bool,
    pub cap: usize,
    pub merge_gap: bool,
    pub demote: bool,
    pub suppress_ff: bool,
    pub skip_ff: bool,
}

impl FamilyParams {
    pub fn spec_b() -> Self {
        Self {
            window: 8,
            stride: 4,
            min_match: 8,
            multi_probe: true,
            cap: 64,
            merge_gap: false,
            demote: true,
            suppress_ff: true,
            skip_ff: false,
        }
    }
}

pub fn encode_add_only(old: &Image, new: &Image) -> Result<Planned> {
    best_width(old, new, |sector, _header, max_add| {
        split_adds(&sector.new_real, max_add)
    })
}

pub fn encode_strategy_a(old: &Image, new: &Image) -> Result<Planned> {
    best_width(old, new, |sector, header, _max_add| {
        strategy_a(sector, header)
    })
}

pub fn encode_strategy_b_local(old: &Image, new: &Image) -> Result<Planned> {
    let params = FamilyParams::spec_b();
    best_width(old, new, |sector, header, _max_add| {
        strategy_b_local(sector, header, &params)
    })
}

pub fn encode_family(old: &Image, new: &Image, params: &FamilyParams) -> Result<Planned> {
    best_width(old, new, |sector, header, _max_add| {
        strategy_b_local(sector, header, params)
    })
}

/// Spec dispatcher: pure ADD when the sector is new, otherwise the cheaper of
/// strategy A and full-image strategy B, both costed at the worst-case ADD
/// header. The width is tuned once, afterwards.
pub fn encode_reference(old: &Image, new: &Image) -> Result<Planned> {
    let views = sectors(old, new)?;
    let mut flat = Flat::covering(&views, old)?;
    let header = add_header_bytes(11);
    let ls = ls_header_bytes();
    let spec = FamilyParams::spec_b();
    let mut plans = Vec::new();
    for sector in &views {
        if !needs_store(sector) {
            continue;
        }
        let ops = if sector.old_real.is_empty() {
            split_adds(&sector.new_real, max_pat(11))
        } else {
            let a = strategy_a(sector, header);
            let b = strategy_b_flat(sector, &flat, header, &spec, false);
            if command_cost(&a, header, ls) <= command_cost(&b, header, ls) {
                a
            } else {
                b
            }
        };
        plans.push(plan_of(sector, ops));
        flat.patch_bytes(sector);
    }
    finish(old, new, &views, plans)
}

pub fn encode_optimal_file(old: &Image, new: &Image) -> Result<Planned> {
    best_width(old, new, |sector, header, max_add| {
        optimal_ops(sector, header, max_add, ls_header_bytes())
    })
}

/// Hypothetical format: a LOAD may read any real byte still in flash, with
/// the address field widened to cover the image. Not a v0.03 file.
pub fn proposal_cross_sector(old: &Image, new: &Image) -> Result<u64> {
    let views = sectors(old, new)?;
    let changed: Vec<&Sector> = views.iter().filter(|s| needs_store(s)).collect();
    if changed.is_empty() {
        return Ok(0);
    }
    let lo = views.first().unwrap().base;
    let hi = views.last().unwrap().base + SECTOR;
    let span = (hi - lo) as usize;
    let mut bytes = vec![0xFFu8; span];
    let mut real = vec![false; span];
    paint_image(&mut bytes, &mut real, lo, old);
    let addr_bits = ceil_log2(span.max(2)).max(11);
    let ls = ls_header_wide(addr_bits);
    let mut best = u64::MAX;
    let mut width = 1u32;
    while width <= 11 {
        let header = add_header_bytes(width);
        let max_add = max_pat(width);
        let mut flash_b = bytes.clone();
        let mut flash_r = real.clone();
        let mut cost = 12u64 + 4 * changed.len() as u64;
        let mut ok = true;
        for sector in &changed {
            let ops = optimal_external(sector, lo, &flash_b, &flash_r, header, max_add, ls);
            if !external_covers(sector, lo, &flash_b, &flash_r, &ops) {
                ok = false;
                break;
            }
            cost += command_cost_ext(&ops, header, ls) as u64;
            cost += u64::from(ls);
            paint_sector(&mut flash_b, &mut flash_r, lo, sector);
        }
        if ok && cost < best {
            best = cost;
        }
        width += 1;
    }
    if best == u64::MAX {
        return Err(err("cross-sector proposal could not cover a sector"));
    }
    Ok(best)
}

pub fn family_grid() -> Vec<FamilyParams> {
    let mut out = Vec::new();
    for min_match in [3usize, 4, 8] {
        for stride in [1usize, 4] {
            for window in [4usize, 8] {
                for skip_ff in [false, true] {
                    for multi_probe in [false, true] {
                        for merge_gap in [false, true] {
                            out.push(FamilyParams {
                                window,
                                stride,
                                min_match,
                                multi_probe,
                                cap: 64,
                                merge_gap,
                                demote: true,
                                suppress_ff: true,
                                skip_ff,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Smallest packed corpus total. The first grid point wins a tie.
pub fn search_family(pairs: &[(Image, Image)]) -> Result<(FamilyParams, u64)> {
    let grid = family_grid();
    let mut best_params = grid[0].clone();
    let mut best_bytes = u64::MAX;
    for params in &grid {
        let mut total = 0u64;
        for (old, new) in pairs {
            total += encode_family(old, new, params)?.bytes.len() as u64;
        }
        if total < best_bytes {
            best_bytes = total;
            best_params = params.clone();
        }
    }
    Ok((best_params, best_bytes))
}

fn best_width<F>(old: &Image, new: &Image, mut encode: F) -> Result<Planned>
where
    F: FnMut(&Sector, u32, u32) -> Vec<Op>,
{
    let views = sectors(old, new)?;
    let changed: Vec<&Sector> = views.iter().filter(|s| needs_store(s)).collect();
    if changed.is_empty() {
        return Ok(Planned {
            bytes: Vec::new(),
            plans: Vec::new(),
        });
    }
    let mut best: Option<Planned> = None;
    let mut width = 1u32;
    while width <= 11 {
        let header = add_header_bytes(width);
        let max_add = max_pat(width);
        let mut plans = Vec::new();
        let mut legal = true;
        for sector in &changed {
            let ops = encode(sector, header, max_add);
            if !ops_legal(&ops, max_add) {
                legal = false;
                break;
            }
            plans.push(plan_of(sector, ops));
        }
        if legal {
            if let Ok(planned) = finish(old, new, &views, plans) {
                let better = best
                    .as_ref()
                    .map(|have| planned.bytes.len() < have.bytes.len())
                    .unwrap_or(true);
                if better {
                    best = Some(planned);
                }
            }
        }
        width += 1;
    }
    best.ok_or_else(|| err("no add_pat width produced a delta the decoder accepts"))
}

fn finish(old: &Image, new: &Image, views: &[Sector], plans: Vec<SectorPlan>) -> Result<Planned> {
    if plans.is_empty() {
        let report = check::verify_bytes(old, new, &[])?;
        let _ = report;
        return Ok(Planned {
            bytes: Vec::new(),
            plans,
        });
    }
    let paired: Vec<(SectorPlan, &Sector)> = plans
        .iter()
        .map(|plan| {
            let sector = views
                .iter()
                .find(|sector| sector.base == plan.base)
                .ok_or_else(|| err("plan sector is missing"))?;
            Ok((plan.clone(), sector))
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = super::pack::pack_with(check::image_base(old, new), &paired)?;
    let report: VerifyReport = check::verify_bytes(old, new, &bytes)?;
    if report.delta_bytes != bytes.len() as u64 {
        return Err(err("verifier byte count does not match the packed file"));
    }
    Ok(Planned { bytes, plans })
}

fn plan_of(sector: &Sector, ops: Vec<Op>) -> SectorPlan {
    SectorPlan {
        base: sector.base,
        ops,
        store_len: store_extent(sector),
    }
}

fn ops_legal(ops: &[Op], max_add: u32) -> bool {
    for op in ops {
        match *op {
            Op::Load { addr, buf, len } => {
                if !range_ok(addr, len) || !range_ok(buf, len) || !fits_pat(len, 11) {
                    return false;
                }
            }
            Op::Add { buf, len } => {
                if !range_ok(buf, len) || (len != SECTOR && len > max_add) {
                    return false;
                }
            }
        }
    }
    true
}

fn range_ok(off: u32, len: u32) -> bool {
    len >= 1
        && off
            .checked_add(len)
            .map(|end| end <= SECTOR)
            .unwrap_or(false)
}

fn command_cost(ops: &[Op], add_header: u32, ls: u32) -> u32 {
    let mut cost = 0u32;
    for op in ops {
        cost = cost.saturating_add(match op {
            Op::Load { .. } => ls,
            Op::Add { len, .. } => add_header.saturating_add(*len),
        });
    }
    cost
}

fn split_adds(spans: &[Span], max_add: u32) -> Vec<Op> {
    let mut ops = Vec::new();
    for span in spans {
        let mut left = span.len;
        let mut off = span.start;
        while left > 0 {
            let mut n = left;
            if n != SECTOR && n > max_add {
                n = max_add;
            }
            if n == SECTOR && off != 0 {
                n = max_add.min(left);
            }
            if n == 0 {
                break;
            }
            ops.push(Op::Add { buf: off, len: n });
            off += n;
            left -= n;
        }
    }
    ops
}

fn strategy_a(sector: &Sector, add_header: u32) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut ram = vec![0xFFu8; SECTOR as usize];
    for span in &sector.old_real {
        ops.push(Op::Load {
            addr: span.start,
            buf: span.start,
            len: span.len,
        });
        let start = span.start as usize;
        let end = start + span.len as usize;
        ram[start..end].copy_from_slice(&sector.old[start..end]);
    }
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut off = 0u32;
    while off < SECTOR {
        if !covered(&sector.new_real, off, 1) {
            off += 1;
            continue;
        }
        if ram[off as usize] != sector.new[off as usize] {
            let start = off;
            while off < SECTOR
                && covered(&sector.new_real, off, 1)
                && ram[off as usize] != sector.new[off as usize]
            {
                off += 1;
            }
            runs.push((start, off));
        } else {
            off += 1;
        }
    }
    if runs.is_empty() {
        return ops;
    }
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for run in runs {
        if let Some(prev) = merged.last_mut() {
            if run.0 - prev.1 < add_header {
                prev.1 = run.1;
                continue;
            }
        }
        merged.push(run);
    }
    for (start, end) in merged {
        ops.push(Op::Add {
            buf: start,
            len: end - start,
        });
    }
    ops
}

fn strategy_b_local(sector: &Sector, add_header: u32, params: &FamilyParams) -> Vec<Op> {
    let flat = Flat::from_sector(sector);
    strategy_b_flat(sector, &flat, add_header, params, true)
}

fn strategy_b_flat(
    sector: &Sector,
    flat: &Flat,
    add_header: u32,
    params: &FamilyParams,
    local_only: bool,
) -> Vec<Op> {
    let index = Index::build(flat, params.window, params.stride);
    let ls = ls_header_bytes();
    let mut ops = Vec::new();
    for span in &sector.new_real {
        let end = span.start + span.len;
        let mut pos = span.start;
        let mut lit: Option<u32> = None;
        while pos < end {
            if params.skip_ff && sector.new[pos as usize] == 0xFF {
                flush_lit(&mut ops, &mut lit, pos);
                pos += 1;
                continue;
            }
            if let Some((src, len)) = best_match(sector, flat, &index, pos, end, params) {
                let inside = src >= sector.base && src + len <= sector.base + SECTOR;
                if len >= params.min_match as u32 && (inside || !local_only) {
                    flush_lit(&mut ops, &mut lit, pos);
                    if inside {
                        ops.push(Op::Load {
                            addr: src - sector.base,
                            buf: pos,
                            len,
                        });
                    } else {
                        ops.push(Op::Add { buf: pos, len });
                    }
                    pos += len;
                    continue;
                }
            }
            if lit.is_none() {
                lit = Some(pos);
            }
            pos += 1;
        }
        flush_lit(&mut ops, &mut lit, end);
    }
    post_passes(&mut ops, sector, add_header, ls, params);
    ops
}

fn flush_lit(ops: &mut Vec<Op>, lit: &mut Option<u32>, pos: u32) {
    if let Some(start) = lit.take() {
        if pos > start {
            ops.push(Op::Add {
                buf: start,
                len: pos - start,
            });
        }
    }
}

fn post_passes(
    ops: &mut Vec<Op>,
    sector: &Sector,
    add_header: u32,
    ls: u32,
    params: &FamilyParams,
) {
    if params.demote {
        for op in ops.iter_mut() {
            if let Op::Load { buf, len, .. } = *op {
                if ls >= add_header.saturating_add(len) {
                    *op = Op::Add { buf, len };
                }
            }
        }
    }
    merge_touching(ops);
    if params.merge_gap {
        merge_with_gap(ops, add_header);
    }
    if params.suppress_ff {
        suppress_ff(ops, sector);
    }
}

fn merge_touching(ops: &mut Vec<Op>) {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops.drain(..) {
        if let Op::Add { buf, len } = op {
            if let Some(Op::Add {
                buf: prev_buf,
                len: prev_len,
            }) = out.last_mut()
            {
                if *prev_buf + *prev_len == buf {
                    *prev_len += len;
                    continue;
                }
            }
        }
        out.push(op);
    }
    *ops = out;
}

fn merge_with_gap(ops: &mut Vec<Op>, add_header: u32) {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops.drain(..) {
        if let Op::Add { buf, len } = op {
            if let Some(Op::Add {
                buf: prev_buf,
                len: prev_len,
            }) = out.last_mut()
            {
                let prev_end = *prev_buf + *prev_len;
                if buf >= prev_end && buf - prev_end < add_header {
                    *prev_len = buf + len - *prev_buf;
                    continue;
                }
            }
        }
        out.push(op);
    }
    *ops = out;
}

fn suppress_ff(ops: &mut Vec<Op>, sector: &Sector) {
    let loads: Vec<(u32, u32)> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Load { buf, len, .. } => Some((*buf, *len)),
            Op::Add { .. } => None,
        })
        .collect();
    let mut kept = Vec::with_capacity(ops.len());
    for op in ops.drain(..) {
        if let Op::Add { buf, len } = op {
            let start = buf as usize;
            let end = start + len as usize;
            let all_ff = sector.new[start..end].iter().all(|b| *b == 0xFF);
            let covered_by_load = loads
                .iter()
                .any(|(load_buf, load_len)| ranges_overlap(*load_buf, *load_len, buf, len));
            if all_ff && !covered_by_load {
                continue;
            }
        }
        kept.push(op);
    }
    *ops = kept;
}

fn ranges_overlap(a: u32, a_len: u32, b: u32, b_len: u32) -> bool {
    let a_end = a + a_len;
    let b_end = b + b_len;
    a < b_end && b < a_end
}

struct Flat {
    base: u32,
    bytes: Vec<u8>,
    real: Vec<bool>,
}

impl Flat {
    fn from_sector(sector: &Sector) -> Self {
        let mut real = vec![false; sector.old.len()];
        for span in &sector.old_real {
            let mut i = 0u32;
            while i < span.len {
                real[(span.start + i) as usize] = true;
                i += 1;
            }
        }
        Self {
            base: sector.base,
            bytes: sector.old.clone(),
            real,
        }
    }

    fn covering(views: &[Sector], image: &Image) -> Result<Self> {
        let Some(first) = views.first() else {
            return Ok(Self {
                base: 0,
                bytes: Vec::new(),
                real: Vec::new(),
            });
        };
        let base = first.base;
        let end = views.last().unwrap().base + SECTOR;
        let n = (end - base) as usize;
        let mut bytes = vec![0xFFu8; n];
        let mut real = vec![false; n];
        for region in &image.regions {
            if region.address < base {
                return Err(err("region starts before the first sector"));
            }
            let at = (region.address - base) as usize;
            let len = region.data.len();
            if at + len > bytes.len() {
                return Err(err("region extends past the last sector"));
            }
            bytes[at..at + len].copy_from_slice(&region.data);
            let mut i = 0;
            while i < len {
                real[at + i] = true;
                i += 1;
            }
        }
        Ok(Self { base, bytes, real })
    }

    fn patch_bytes(&mut self, sector: &Sector) {
        if self.bytes.is_empty() {
            return;
        }
        let start = (sector.base - self.base) as usize;
        let end = start + SECTOR as usize;
        if end <= self.bytes.len() {
            self.bytes[start..end].copy_from_slice(&sector.new);
        }
    }

    fn get(&self, addr: u32) -> Option<u8> {
        if addr < self.base {
            return None;
        }
        self.bytes.get((addr - self.base) as usize).copied()
    }

    fn is_real(&self, addr: u32) -> bool {
        if addr < self.base {
            return false;
        }
        self.real
            .get((addr - self.base) as usize)
            .copied()
            .unwrap_or(false)
    }
}

struct Index {
    buckets: usize,
    head: Vec<i32>,
    offsets: Vec<u32>,
    hashes: Vec<u32>,
    next: Vec<i32>,
}

impl Index {
    fn build(flat: &Flat, window: usize, stride: usize) -> Self {
        let mut offsets = Vec::new();
        let mut hashes = Vec::new();
        if window >= 1 && stride >= 1 && flat.bytes.len() >= window {
            let mut pos = 0usize;
            while pos + window <= flat.bytes.len() {
                let slice = &flat.bytes[pos..pos + window];
                let mut real = true;
                let mut i = 0;
                while i < window {
                    if !flat.real[pos + i] {
                        real = false;
                        break;
                    }
                    i += 1;
                }
                let all_ff = slice.iter().all(|b| *b == 0xFF);
                if real && !all_ff {
                    offsets.push(flat.base + pos as u32);
                    hashes.push(fnv1a(slice));
                }
                pos += stride;
            }
        }
        let buckets = bucket_count(offsets.len());
        let mut head = vec![-1i32; buckets];
        let mut next = vec![-1i32; offsets.len()];
        let mut i = 0usize;
        while i < offsets.len() {
            let bucket = (hashes[i] as usize) & (buckets - 1);
            next[i] = head[bucket];
            head[bucket] = i as i32;
            i += 1;
        }
        Self {
            buckets,
            head,
            offsets,
            hashes,
            next,
        }
    }

    fn lookup(&self, hash: u32) -> Vec<u32> {
        let mut out = Vec::new();
        if self.buckets == 0 {
            return out;
        }
        let mut slot = self.head[(hash as usize) & (self.buckets - 1)];
        while slot >= 0 {
            let index = slot as usize;
            if self.hashes[index] == hash {
                out.push(self.offsets[index]);
            }
            slot = self.next[index];
        }
        out
    }
}

fn bucket_count(entries: usize) -> usize {
    let mut size = 2usize;
    let need = entries.saturating_mul(2).max(2);
    while size < need {
        size <<= 1;
    }
    size
}

fn fnv1a(data: &[u8]) -> u32 {
    let mut hash = 2_166_136_261u32;
    for &byte in data {
        hash ^= u32::from(byte);
        hash = ((hash as u64) * 16_777_619) as u32;
    }
    hash
}

fn best_match(
    sector: &Sector,
    flat: &Flat,
    index: &Index,
    pos: u32,
    interval_end: u32,
    params: &FamilyParams,
) -> Option<(u32, u32)> {
    if params.window == 0 || params.stride == 0 {
        return None;
    }
    let probes = if params.multi_probe { params.stride } else { 1 };
    let mut best_len = 0u32;
    let mut best_src = 0u32;
    let mut probe_i = 0usize;
    while probe_i < probes {
        let probe = pos + probe_i as u32;
        if probe + params.window as u32 > interval_end {
            probe_i += 1;
            continue;
        }
        let window = &sector.new[probe as usize..probe as usize + params.window];
        let hash = fnv1a(window);
        let mut cands = index.lookup(hash);
        cands.sort_by_key(|cand| cand.abs_diff(probe));
        if cands.len() > params.cap {
            cands.truncate(params.cap);
        }
        for cand in cands {
            if !window_eq(flat, cand, window) {
                continue;
            }
            if !extend_back(sector, flat, cand, probe, probe_i as u32) {
                continue;
            }
            let src = cand - probe_i as u32;
            let mut len = (probe_i + params.window) as u32;
            while pos + len < interval_end
                && flat.is_real(src + len)
                && flat.get(src + len) == Some(sector.new[(pos + len) as usize])
            {
                len += 1;
            }
            let extent = real_extent(flat, src, len);
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
        probe_i += 1;
    }
    if best_len >= params.min_match as u32 {
        Some((best_src, best_len))
    } else {
        None
    }
}

fn window_eq(flat: &Flat, addr: u32, window: &[u8]) -> bool {
    let mut i = 0usize;
    while i < window.len() {
        if flat.get(addr + i as u32) != Some(window[i]) || !flat.is_real(addr + i as u32) {
            return false;
        }
        i += 1;
    }
    true
}

fn extend_back(sector: &Sector, flat: &Flat, cand: u32, probe: u32, dist: u32) -> bool {
    if cand < dist || probe < dist {
        return false;
    }
    let mut back = 0u32;
    while back < dist {
        let src = cand - back - 1;
        let dst = probe - back - 1;
        if dst >= SECTOR || flat.get(src) != Some(sector.new[dst as usize]) || !flat.is_real(src) {
            return false;
        }
        back += 1;
    }
    true
}

fn real_extent(flat: &Flat, addr: u32, max_len: u32) -> u32 {
    let mut n = 0u32;
    while n < max_len && flat.is_real(addr + n) {
        n += 1;
    }
    n
}

struct Run<'a> {
    bytes: &'a [u8],
    origin: u32,
}

enum Choice {
    Skip,
    Add { k: usize },
    Load { src: u32, k: usize },
}

fn optimal_ops(sector: &Sector, add_header: u32, max_add: u32, ls: u32) -> Vec<Op> {
    let mut runs = Vec::new();
    for span in &sector.old_real {
        let start = span.start as usize;
        let end = start + span.len as usize;
        runs.push(Run {
            bytes: &sector.old[start..end],
            origin: span.start,
        });
    }
    optimal_from_runs(
        &sector.new,
        &sector.new_real,
        &runs,
        add_header,
        max_add,
        ls,
    )
}

/// Shortest path over new-real offsets.
///
/// A LOAD copies a contiguous slice of one source run. Bytes it gets wrong
/// are patched by ADDs that follow it, so one copy of an old interval plus
/// the mismatched bytes is a legal transition. LOADs in the path do not
/// overlap; a later ADD is what overwrites a copied byte.
fn optimal_from_runs(
    newb: &[u8],
    new_real: &[Span],
    runs: &[Run<'_>],
    add_header: u32,
    max_add: u32,
    ls: u32,
) -> Vec<Op> {
    let need = need_offsets(new_real);
    if need.is_empty() {
        return Vec::new();
    }
    let n = need.len();
    let inf = u32::MAX / 4;
    let mut dp = vec![inf; n + 1];
    let mut choice: Vec<Option<Choice>> = Vec::new();
    let mut fill = 0usize;
    while fill < n {
        choice.push(None);
        fill += 1;
    }
    dp[n] = 0;
    let mut i = n;
    while i > 0 {
        i -= 1;
        let pos = need[i];
        let mut best = inf;
        let mut chosen: Option<Choice> = None;
        if newb[pos as usize] == 0xFF && dp[i + 1] < best {
            best = dp[i + 1];
            chosen = Some(Choice::Skip);
        }
        let mut k = 1usize;
        while i + k <= n {
            let span = need[i + k - 1] + 1 - pos;
            if span > SECTOR {
                break;
            }
            if span == SECTOR || span <= max_add {
                let cost = add_header.saturating_add(span).saturating_add(dp[i + k]);
                if cost < best {
                    best = cost;
                    chosen = Some(Choice::Add { k });
                }
            }
            k += 1;
        }
        for run in runs {
            let mut s = 0usize;
            while s < run.bytes.len() {
                if run.bytes[s] != newb[pos as usize] {
                    s += 1;
                    continue;
                }
                let src0 = run.origin + s as u32;
                let mut hole = 0u32;
                let mut have = false;
                let mut group_start = 0u32;
                let mut last = 0u32;
                let first = ls.saturating_add(dp[i + 1]);
                if first < best {
                    best = first;
                    chosen = Some(Choice::Load { src: src0, k: 1 });
                }
                let mut j = i + 1;
                while j < n {
                    let dest = need[j];
                    let src_off = s as u32 + (dest - pos);
                    if src_off as usize >= run.bytes.len() {
                        break;
                    }
                    if run.bytes[src_off as usize] != newb[dest as usize] {
                        match note_hole(hole, have, group_start, last, dest, add_header, max_add) {
                            None => break,
                            Some((cost, next_have, start, end)) => {
                                hole = cost;
                                have = next_have;
                                group_start = start;
                                last = end;
                            }
                        }
                        if ls.saturating_add(hole) >= best {
                            break;
                        }
                        j += 1;
                        continue;
                    }
                    let cost = ls.saturating_add(hole).saturating_add(dp[j + 1]);
                    if cost < best {
                        best = cost;
                        chosen = Some(Choice::Load {
                            src: src0,
                            k: j + 1 - i,
                        });
                    }
                    j += 1;
                }
                s += 1;
            }
        }
        dp[i] = best;
        choice[i] = chosen;
    }
    if dp[0] >= inf {
        return split_adds(new_real, max_add.max(1));
    }
    let mut ops = Vec::new();
    let mut at = 0usize;
    while at < n {
        let Some(step) = choice[at].take() else {
            return split_adds(new_real, max_add.max(1));
        };
        match step {
            Choice::Skip => at += 1,
            Choice::Add { k } => {
                let span = need[at + k - 1] + 1 - need[at];
                ops.push(Op::Add {
                    buf: need[at],
                    len: span,
                });
                at += k;
            }
            Choice::Load { src, k } => {
                let buf = need[at];
                let len = need[at + k - 1] + 1 - buf;
                ops.push(Op::Load {
                    addr: src,
                    buf,
                    len,
                });
                ops.extend(patch_adds(
                    newb, &need, runs, at, k, src, add_header, max_add,
                ));
                at += k;
            }
        }
    }
    ops
}

fn note_hole(
    cost: u32,
    have: bool,
    group_start: u32,
    last: u32,
    off: u32,
    header: u32,
    max_add: u32,
) -> Option<(u32, bool, u32, u32)> {
    if !have {
        if max_add == 0 && 1 != SECTOR {
            return None;
        }
        return Some((header.saturating_add(1), true, off, off));
    }
    let gap = off - last - 1;
    let new_span = off - group_start + 1;
    if gap < header && (new_span <= max_add || new_span == SECTOR) {
        Some((cost.saturating_add(off - last), true, group_start, off))
    } else if max_add >= 1 || 1 == SECTOR {
        Some((
            cost.saturating_add(header).saturating_add(1),
            true,
            off,
            off,
        ))
    } else {
        None
    }
}

fn patch_adds(
    newb: &[u8],
    need: &[u32],
    runs: &[Run<'_>],
    at: usize,
    k: usize,
    src: u32,
    add_header: u32,
    max_add: u32,
) -> Vec<Op> {
    let buf = need[at];
    let mut ops = Vec::new();
    let mut hole = 0u32;
    let mut have = false;
    let mut group_start = 0u32;
    let mut last = 0u32;
    let mut j = at + 1;
    while j < at + k {
        let dest = need[j];
        let addr = src + (dest - buf);
        let Some(byte) = run_byte(runs, addr) else {
            return Vec::new();
        };
        if byte != newb[dest as usize] {
            let before_have = have;
            let before_start = group_start;
            let before_last = last;
            let Some((cost, next_have, start, end)) =
                note_hole(hole, have, group_start, last, dest, add_header, max_add)
            else {
                return Vec::new();
            };
            if before_have && start != before_start {
                ops.push(Op::Add {
                    buf: before_start,
                    len: before_last - before_start + 1,
                });
            }
            hole = cost;
            have = next_have;
            group_start = start;
            last = end;
        }
        j += 1;
    }
    if have {
        ops.push(Op::Add {
            buf: group_start,
            len: last - group_start + 1,
        });
    }
    let mut billed = 0u32;
    for op in &ops {
        if let Op::Add { len, .. } = op {
            billed = billed.saturating_add(add_header.saturating_add(*len));
        }
    }
    debug_assert_eq!(billed, hole);
    ops
}

fn run_byte(runs: &[Run<'_>], addr: u32) -> Option<u8> {
    for run in runs {
        if addr >= run.origin {
            let off = addr - run.origin;
            if (off as usize) < run.bytes.len() {
                return Some(run.bytes[off as usize]);
            }
        }
    }
    None
}

fn need_offsets(spans: &[Span]) -> Vec<u32> {
    let mut out = Vec::new();
    for span in spans {
        let mut i = 0u32;
        while i < span.len {
            out.push(span.start + i);
            i += 1;
        }
    }
    out
}

struct XOp {
    load_src: Option<u32>,
    buf: u32,
    len: u32,
}

fn optimal_external(
    sector: &Sector,
    origin: u32,
    flash: &[u8],
    real: &[bool],
    add_header: u32,
    max_add: u32,
    ls: u32,
) -> Vec<XOp> {
    let ranges = real_runs(real);
    let mut runs = Vec::new();
    for (lo, hi) in ranges {
        runs.push(Run {
            bytes: &flash[lo..hi],
            origin: origin + lo as u32,
        });
    }
    let ops = optimal_from_runs(
        &sector.new,
        &sector.new_real,
        &runs,
        add_header,
        max_add,
        ls,
    );
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::Load { addr, buf, len } => out.push(XOp {
                load_src: Some(addr),
                buf,
                len,
            }),
            Op::Add { buf, len } => out.push(XOp {
                load_src: None,
                buf,
                len,
            }),
        }
    }
    out
}

fn real_runs(real: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < real.len() {
        if real[i] {
            let start = i;
            while i < real.len() && real[i] {
                i += 1;
            }
            out.push((start, i));
        } else {
            i += 1;
        }
    }
    out
}

fn external_covers(sector: &Sector, origin: u32, flash: &[u8], real: &[bool], ops: &[XOp]) -> bool {
    let mut ram = vec![0xFFu8; SECTOR as usize];
    for op in ops {
        let end = op.buf + op.len;
        if end > SECTOR {
            return false;
        }
        if let Some(src) = op.load_src {
            let mut i = 0u32;
            while i < op.len {
                let abs = src + i;
                if abs < origin {
                    return false;
                }
                let at = (abs - origin) as usize;
                if at >= flash.len() || !real[at] {
                    return false;
                }
                ram[(op.buf + i) as usize] = flash[at];
                i += 1;
            }
        } else {
            let mut i = 0u32;
            while i < op.len {
                ram[(op.buf + i) as usize] = sector.new[(op.buf + i) as usize];
                i += 1;
            }
        }
    }
    for span in &sector.new_real {
        let mut i = 0u32;
        while i < span.len {
            let off = span.start + i;
            if ram[off as usize] != sector.new[off as usize] {
                return false;
            }
            i += 1;
        }
    }
    true
}

fn command_cost_ext(ops: &[XOp], add_header: u32, ls: u32) -> u32 {
    let mut cost = 0u32;
    for op in ops {
        if op.load_src.is_some() {
            cost = cost.saturating_add(ls);
        } else {
            cost = cost.saturating_add(add_header.saturating_add(op.len));
        }
    }
    cost
}

fn paint_image(bytes: &mut [u8], real: &mut [bool], origin: u32, image: &Image) {
    for region in &image.regions {
        let at = (region.address - origin) as usize;
        let len = region.data.len();
        bytes[at..at + len].copy_from_slice(&region.data);
        let mut i = 0;
        while i < len {
            real[at + i] = true;
            i += 1;
        }
    }
}

fn paint_sector(bytes: &mut [u8], real: &mut [bool], origin: u32, sector: &Sector) {
    let at = (sector.base - origin) as usize;
    let n = SECTOR as usize;
    bytes[at..at + n].fill(0xFF);
    real[at..at + n].fill(false);
    bytes[at..at + n].copy_from_slice(&sector.new);
    for span in &sector.new_real {
        let mut i = 0u32;
        while i < span.len {
            real[at + (span.start + i) as usize] = true;
            i += 1;
        }
    }
}

fn ls_header_wide(addr_bits: u32) -> u32 {
    let used = 2 + addr_bits + 11 + 11;
    (used + unused_bits(used)) / 8
}

fn ceil_log2(n: usize) -> u32 {
    let mut bits = 0u32;
    let mut v = 1usize;
    while v < n {
        v <<= 1;
        bits += 1;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::image::Region;

    fn image(addr: u32, data: &[u8]) -> Image {
        Image::from_regions(vec![Region {
            address: addr,
            data: data.to_vec(),
        }])
        .unwrap()
    }

    #[test]
    fn optimal_matches_a_brute_force_cover() {
        let mut old = vec![0u8; 24];
        let mut newb = vec![0u8; 24];
        let mut i = 0;
        while i < 24 {
            old[i] = (i as u8).wrapping_mul(3).wrapping_add(1);
            newb[i] = old[i];
            i += 1;
        }
        newb[4] = 0xFF;
        newb[5] = 9;
        newb[10] = 0xFF;
        newb[18] = old[3];
        let old_img = image(0x0800_0000, &old);
        let new_img = image(0x0800_0000, &newb);
        let sector = &sectors(&old_img, &new_img).unwrap()[0];
        let header = 3u32;
        let max_add = 31u32;
        let ls = 5u32;
        let ops = optimal_ops(sector, header, max_add, ls);
        let cost = command_cost(&ops, header, ls);
        assert_eq!(cost, brute_cost(sector, header, max_add, ls));
        let copied = strategy_a(sector, header);
        assert!(cost <= command_cost(&copied, header, ls));
    }

    fn brute_cost(sector: &Sector, add_header: u32, max_add: u32, ls: u32) -> u32 {
        let need = need_offsets(&sector.new_real);
        let mut memo = vec![None; need.len() + 1];
        fn rec(
            sector: &Sector,
            need: &[u32],
            i: usize,
            memo: &mut [Option<u32>],
            add_header: u32,
            max_add: u32,
            ls: u32,
        ) -> u32 {
            if i == need.len() {
                return 0;
            }
            if let Some(done) = memo[i] {
                return done;
            }
            let pos = need[i];
            let mut best = u32::MAX / 4;
            if sector.new[pos as usize] == 0xFF {
                best = best.min(rec(sector, need, i + 1, memo, add_header, max_add, ls));
            }
            let mut k = 1usize;
            while i + k <= need.len() {
                let span = need[i + k - 1] + 1 - pos;
                if span > SECTOR {
                    break;
                }
                if span == SECTOR || span <= max_add {
                    let rest = rec(sector, need, i + k, memo, add_header, max_add, ls);
                    best = best.min(add_header.saturating_add(span).saturating_add(rest));
                }
                k += 1;
            }
            let mut src = 0u32;
            while src < SECTOR {
                if covered(&sector.old_real, src, 1)
                    && sector.old[src as usize] == sector.new[pos as usize]
                {
                    let rest = rec(sector, need, i + 1, memo, add_header, max_add, ls);
                    best = best.min(ls.saturating_add(rest));
                    let mut hole = 0u32;
                    let mut have = false;
                    let mut group_start = 0u32;
                    let mut last = 0u32;
                    let mut k = 2usize;
                    while i + k <= need.len() {
                        let dest = need[i + k - 1];
                        let delta = dest - pos;
                        if !span_src_real(sector, src, delta + 1) {
                            break;
                        }
                        if sector.old[(src + delta) as usize] != sector.new[dest as usize] {
                            match note_hole(
                                hole,
                                have,
                                group_start,
                                last,
                                dest,
                                add_header,
                                max_add,
                            ) {
                                None => break,
                                Some((cost, next_have, start, end)) => {
                                    hole = cost;
                                    have = next_have;
                                    group_start = start;
                                    last = end;
                                }
                            }
                            k += 1;
                            continue;
                        }
                        let rest = rec(sector, need, i + k, memo, add_header, max_add, ls);
                        best = best.min(ls.saturating_add(hole).saturating_add(rest));
                        k += 1;
                    }
                }
                src += 1;
            }
            memo[i] = Some(best);
            best
        }
        rec(sector, &need, 0, &mut memo, add_header, max_add, ls)
    }

    fn span_src_real(sector: &Sector, src: u32, len: u32) -> bool {
        src + len <= SECTOR && covered(&sector.old_real, src, len)
    }

    #[test]
    fn a_moved_block_is_a_load_and_the_decoder_accepts_it() {
        let mut old = vec![0xFFu8; 64];
        let mut newb = vec![0xFFu8; 64];
        let mut i = 0;
        while i < 20 {
            old[i] = 0x40 + i as u8;
            newb[30 + i] = 0x40 + i as u8;
            i += 1;
        }
        let old_img = image(0x0800_0000, &old);
        let new_img = image(0x0800_0000, &newb);
        let planned = encode_optimal_file(&old_img, &new_img).unwrap();
        assert!(planned.bytes.len() < 40);
        let adds: u32 = planned.plans[0]
            .ops
            .iter()
            .map(|op| match op {
                Op::Add { len, .. } => *len,
                Op::Load { .. } => 0,
            })
            .sum();
        assert_eq!(adds, 0);
    }

    #[test]
    fn reference_and_optimal_round_trip_a_mutation() {
        let mut old = vec![0u8; 300];
        let mut i = 0;
        while i < old.len() {
            old[i] = ((i * 17) ^ 0x5A) as u8;
            i += 1;
        }
        let mut newb = old.clone();
        newb[10] = newb[10].wrapping_add(1);
        newb[40..60].copy_from_slice(&old[200..220]);
        let old_img = image(0x0800_0000, &old);
        let new_img = image(0x0800_0000, &newb);
        let reference = encode_reference(&old_img, &new_img).unwrap();
        let optimal = encode_optimal_file(&old_img, &new_img).unwrap();
        assert!(optimal.bytes.len() <= reference.bytes.len());
        let proposal = proposal_cross_sector(&old_img, &new_img).unwrap();
        assert!(proposal <= optimal.bytes.len() as u64);
    }

    #[test]
    fn a_load_from_a_gap_is_rejected() {
        let old_img = image(0x0800_0000, &[1, 2, 3, 4]);
        let new_img = image(0x0800_0000, &[1, 2, 3, 9]);
        let views = sectors(&old_img, &new_img).unwrap();
        let plan = SectorPlan {
            base: views[0].base,
            ops: vec![
                Op::Add { buf: 0, len: 4 },
                Op::Load {
                    addr: 100,
                    buf: 200,
                    len: 1,
                },
            ],
            store_len: 4,
        };
        let bytes = super::super::pack::pack_with(0x0800_0000, &[(plan, &views[0])]).unwrap();
        let error = super::super::check::verify_bytes(&old_img, &new_img, &bytes).unwrap_err();
        assert!(error.message.contains("gap"), "{}", error.message);
    }
}
