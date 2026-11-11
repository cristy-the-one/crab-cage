//! Host decoder. Flash starts as the old image. After the delta, every new
//! real region must match byte for byte and by CRC-32. Gap loads fail even
//! when the model would have read 0xFF there.

use crate::error::{err, Result};

use super::image::Image;
use super::pack::{crc32, unpack, DecodedOp, SECTOR};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    pub delta_bytes: u64,
    pub regions: Vec<RegionCheck>,
    pub sentinel_count: u32,
    pub all_ff_adds: u32,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionCheck {
    pub address: u32,
    pub len: u32,
    pub crc: u32,
}

/// Decode `delta` onto `old` and compare with `new`.
pub fn verify_bytes(old: &Image, new: &Image, delta: &[u8]) -> Result<VerifyReport> {
    if delta.is_empty() {
        if new
            .regions
            .iter()
            .any(|region| !old_matches_region(old, region.address, &region.data))
        {
            return Err(err("empty delta does not reproduce the new image"));
        }
        return Ok(report_regions(new, 0, 0, 0));
    }
    let decoded = unpack(delta)?;
    let mut warnings = Vec::new();
    if decoded.all_ff_adds > 0 {
        warnings.push(format!(
            "{} ADD payload(s) are entirely 0xFF",
            decoded.all_ff_adds
        ));
    }
    let mut lo = decoded.sectors[0].base;
    let mut hi = decoded.sectors[0].base + SECTOR;
    for sector in &decoded.sectors {
        lo = lo.min(sector.base);
        hi = hi.max(sector.base + SECTOR);
    }
    if let Some(base) = old.base_addr() {
        lo = lo.min(base & !(SECTOR - 1));
    }
    if let Some(end) = old.end_addr() {
        hi = hi.max(end);
    }
    if let Some(base) = new.base_addr() {
        lo = lo.min(base & !(SECTOR - 1));
    }
    if let Some(end) = new.end_addr() {
        hi = hi.max(end);
    }
    if hi - lo > super::SPAN_LIMIT {
        return Err(err("decoded flash span exceeds the limit"));
    }
    let mut flash = vec![0xFFu8; (hi - lo) as usize];
    copy_image(&mut flash, lo, old);
    let mut ram = vec![0xFFu8; SECTOR as usize];
    for sector in &decoded.sectors {
        for op in &sector.ops {
            match op {
                DecodedOp::Load { addr, buf, len } => {
                    check_bounds(*addr, *buf, *len)?;
                    let abs = sector.base + *addr;
                    if !old.range_is_real(abs, *len) {
                        return Err(err(format!(
                            "LOAD at {:#x} length {len} reads a gap or another sector",
                            abs
                        )));
                    }
                    copy_span(&mut ram, *buf, &flash, lo, abs, *len);
                }
                DecodedOp::Add { buf, len, payload } => {
                    check_bounds(0, *buf, *len)?;
                    if payload.len() != *len as usize {
                        return Err(err("ADD payload length does not match pat_size"));
                    }
                    ram[*buf as usize..(*buf + *len) as usize].copy_from_slice(payload);
                }
                DecodedOp::Store { addr, buf, len } => {
                    check_bounds(*addr, *buf, *len)?;
                    let abs = sector.base + *addr;
                    fill_span(&mut flash, lo, sector.base, SECTOR, 0xFF);
                    copy_out(&mut flash, lo, abs, &ram, *buf, *len);
                    ram.fill(0xFF);
                }
            }
        }
        if !matches!(sector.ops.last(), Some(DecodedOp::Store { .. })) {
            return Err(err("sector command list does not end in STORE"));
        }
    }
    let mut regions = Vec::new();
    for region in &new.regions {
        let got = read_span(&flash, lo, region.address, region.data.len() as u32)?;
        if got != region.data {
            let mut at = 0usize;
            while at < got.len() && got[at] == region.data[at] {
                at += 1;
            }
            return Err(err(format!(
                "byte mismatch at {:#x}: delta has {:#04x}, new image has {:#04x}",
                region.address + at as u32,
                got[at],
                region.data[at]
            )));
        }
        regions.push(RegionCheck {
            address: region.address,
            len: region.data.len() as u32,
            crc: crc32(&got),
        });
    }
    // Every new region was compared above. A sector that needed a store and
    // was left out fails that compare.
    Ok(VerifyReport {
        delta_bytes: delta.len() as u64,
        regions,
        sentinel_count: decoded.sentinel_count,
        all_ff_adds: decoded.all_ff_adds,
        warnings,
    })
}

fn report_regions(new: &Image, delta_bytes: u64, sentinel: u32, ff: u32) -> VerifyReport {
    VerifyReport {
        delta_bytes,
        regions: new
            .regions
            .iter()
            .map(|region| RegionCheck {
                address: region.address,
                len: region.data.len() as u32,
                crc: crc32(&region.data),
            })
            .collect(),
        sentinel_count: sentinel,
        all_ff_adds: ff,
        warnings: Vec::new(),
    }
}

fn old_matches_region(old: &Image, addr: u32, data: &[u8]) -> bool {
    old.range_is_real(addr, data.len() as u32)
        && data.iter().enumerate().all(|(i, byte)| {
            let abs = addr + i as u32;
            old.regions.iter().any(|region| {
                abs >= region.address
                    && abs < region.address + region.data.len() as u32
                    && region.data[(abs - region.address) as usize] == *byte
            })
        })
}

fn check_bounds(addr: u32, buf: u32, len: u32) -> Result<()> {
    if len == 0
        || addr
            .checked_add(len)
            .map(|end| end > SECTOR)
            .unwrap_or(true)
        || buf.checked_add(len).map(|end| end > SECTOR).unwrap_or(true)
    {
        return Err(err(format!(
            "command range addr {addr} buf {buf} len {len} is outside the sector"
        )));
    }
    Ok(())
}

fn copy_image(flash: &mut [u8], base: u32, image: &Image) {
    for region in &image.regions {
        copy_out(
            flash,
            base,
            region.address,
            &region.data,
            0,
            region.data.len() as u32,
        );
    }
}

fn copy_span(dst: &mut [u8], dst_off: u32, src: &[u8], src_base: u32, src_addr: u32, len: u32) {
    let from = (src_addr - src_base) as usize;
    let n = len as usize;
    dst[dst_off as usize..dst_off as usize + n].copy_from_slice(&src[from..from + n]);
}

fn copy_out(dst: &mut [u8], dst_base: u32, dst_addr: u32, src: &[u8], src_off: u32, len: u32) {
    let at = (dst_addr - dst_base) as usize;
    let from = src_off as usize;
    let n = len as usize;
    dst[at..at + n].copy_from_slice(&src[from..from + n]);
}

fn fill_span(dst: &mut [u8], dst_base: u32, addr: u32, len: u32, value: u8) {
    let at = (addr - dst_base) as usize;
    let n = len as usize;
    dst[at..at + n].fill(value);
}

fn read_span(src: &[u8], base: u32, addr: u32, len: u32) -> Result<Vec<u8>> {
    let at = (addr - base) as usize;
    let end = at + len as usize;
    if end > src.len() {
        return Err(err("new image extends outside the simulated flash"));
    }
    Ok(src[at..end].to_vec())
}

pub fn image_base(old: &Image, new: &Image) -> u32 {
    old.base_addr()
        .into_iter()
        .chain(new.base_addr())
        .min()
        .unwrap_or(0)
}
