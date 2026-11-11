//! Flat images, gap maps, and 2 KB sector views.
//!
//! Bytes that no S-record covers are gaps. The model fills them with 0xFF so
//! a strategy can diff a sector, but a LOAD must not use them as a source:
//! the device's gap contents are not defined.

use crate::error::{err, Result};

use super::pack::SECTOR;
use super::SPAN_LIMIT;

/// One contiguous run of real bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub address: u32,
    pub data: Vec<u8>,
}

/// A half-open byte span. Offsets in a [`Sector`] are relative to its base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

/// Coalesced regions, sorted, disjoint, and not adjacent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub regions: Vec<Region>,
}

/// One aligned flash sector, with gaps stored as 0xFF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sector {
    pub base: u32,
    pub old: Vec<u8>,
    pub new: Vec<u8>,
    pub old_real: Vec<Span>,
    pub new_real: Vec<Span>,
}

impl Image {
    pub fn from_regions(regions: Vec<Region>) -> Result<Self> {
        let image = Self { regions };
        image.check_span()?;
        Ok(image)
    }

    pub fn empty() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    pub fn base_addr(&self) -> Option<u32> {
        self.regions.first().map(|r| r.address)
    }

    pub fn end_addr(&self) -> Option<u32> {
        self.regions.last().map(|r| r.address + r.data.len() as u32)
    }

    pub fn real_bytes(&self) -> u64 {
        self.regions.iter().map(|r| r.data.len() as u64).sum()
    }

    pub fn range_is_real(&self, addr: u32, len: u32) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = addr.checked_add(len) else {
            return false;
        };
        let mut cursor = addr;
        for region in &self.regions {
            let region_end = region.address + region.data.len() as u32;
            if region_end <= cursor {
                continue;
            }
            if region.address > cursor {
                return false;
            }
            cursor = region_end;
            if cursor >= end {
                return true;
            }
        }
        false
    }

    fn check_span(&self) -> Result<()> {
        if let (Some(base), Some(end)) = (self.base_addr(), self.end_addr()) {
            if end - base > SPAN_LIMIT {
                return Err(err(format!(
                    "image span is {} bytes, above the {SPAN_LIMIT} byte limit",
                    end - base
                )));
            }
        }
        Ok(())
    }
}

/// Merge adjacent runs and reject overlaps. `runs` need not be sorted.
pub fn coalesce(mut runs: Vec<(u32, Vec<u8>)>) -> Result<Vec<Region>> {
    runs.sort_by_key(|(addr, _)| *addr);
    let mut regions: Vec<Region> = Vec::new();
    for (addr, data) in runs {
        if data.is_empty() {
            continue;
        }
        addr.checked_add(data.len() as u32)
            .ok_or_else(|| err("SREC address overflow"))?;
        if let Some(prev) = regions.last_mut() {
            let prev_end = prev.address + prev.data.len() as u32;
            if addr < prev_end {
                return Err(err(format!(
                    "SREC regions overlap at {addr:#x} and {:#x}",
                    prev.address
                )));
            }
            if addr == prev_end {
                prev.data.extend(data);
                continue;
            }
        }
        regions.push(Region {
            address: addr,
            data,
        });
    }
    Ok(regions)
}

/// Sectors covering the union of `old` and `new`, ascending, 2 KB aligned.
pub fn sectors(old: &Image, new: &Image) -> Result<Vec<Sector>> {
    let bases = [old.base_addr(), new.base_addr()];
    let ends = [old.end_addr(), new.end_addr()];
    let Some(lo) = bases.into_iter().flatten().min() else {
        return Ok(Vec::new());
    };
    let hi = ends.into_iter().flatten().max().unwrap();
    if hi - lo > SPAN_LIMIT {
        return Err(err(format!(
            "combined image span is {} bytes, above the {SPAN_LIMIT} byte limit",
            hi - lo
        )));
    }
    let first = lo & !(SECTOR - 1);
    let last = (hi - 1) & !(SECTOR - 1);
    let mut out = Vec::new();
    let mut base = first;
    loop {
        let (old_bytes, old_real) = fill(old, base);
        let (new_bytes, new_real) = fill(new, base);
        out.push(Sector {
            base,
            old: old_bytes,
            new: new_bytes,
            old_real,
            new_real,
        });
        if base == last {
            break;
        }
        base += SECTOR;
    }
    Ok(out)
}

/// A sector must be stored when some new real byte is not already sitting in
/// old real data. A gap is not old real data, even though the model fills it
/// with 0xFF: the device is not required to hold 0xFF there.
pub fn needs_store(sector: &Sector) -> bool {
    for span in &sector.new_real {
        let mut i = 0u32;
        while i < span.len {
            let off = span.start + i;
            if !covered(&sector.old_real, off, 1)
                || sector.old[off as usize] != sector.new[off as usize]
            {
                return true;
            }
            i += 1;
        }
    }
    false
}

/// Highest real-data end, from either image, measured from the sector base.
pub fn store_extent(sector: &Sector) -> u32 {
    let mut end = 0u32;
    for span in sector.old_real.iter().chain(sector.new_real.iter()) {
        end = end.max(span.start + span.len);
    }
    end
}

pub fn covered(spans: &[Span], start: u32, len: u32) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let mut cursor = start;
    for span in spans {
        let span_end = span.start + span.len;
        if span_end <= cursor {
            continue;
        }
        if span.start > cursor {
            return false;
        }
        cursor = span_end;
        if cursor >= end {
            return true;
        }
    }
    false
}

fn fill(image: &Image, base: u32) -> (Vec<u8>, Vec<Span>) {
    let mut bytes = vec![0xFFu8; SECTOR as usize];
    let mut spans = Vec::new();
    let sector_end = base + SECTOR;
    for region in &image.regions {
        let region_end = region.address + region.data.len() as u32;
        let lo = region.address.max(base);
        let hi = region_end.min(sector_end);
        if lo >= hi {
            continue;
        }
        let dest = (lo - base) as usize;
        let src = (lo - region.address) as usize;
        let n = (hi - lo) as usize;
        bytes[dest..dest + n].copy_from_slice(&region.data[src..src + n]);
        spans.push(Span {
            start: lo - base,
            len: hi - lo,
        });
    }
    (bytes, spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unaligned_image_still_uses_the_aligned_sector() {
        let old = Image::from_regions(vec![Region {
            address: 0x0800_0080,
            data: vec![1, 2, 3, 4],
        }])
        .unwrap();
        let new = Image::from_regions(vec![Region {
            address: 0x0800_0080,
            data: vec![1, 9, 3, 4],
        }])
        .unwrap();
        let secs = sectors(&old, &new).unwrap();
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].base, 0x0800_0000);
        assert_eq!(
            secs[0].old_real,
            vec![Span {
                start: 0x80,
                len: 4
            }]
        );
        assert!(needs_store(&secs[0]));
        assert_eq!(store_extent(&secs[0]), 0x84);
    }

    #[test]
    fn a_new_ff_byte_in_an_old_gap_still_needs_a_store() {
        let old = Image::from_regions(vec![Region {
            address: 0x0800_0000,
            data: vec![1, 2],
        }])
        .unwrap();
        let new = Image::from_regions(vec![Region {
            address: 0x0800_0000,
            data: vec![1, 2, 0xFF],
        }])
        .unwrap();
        let sec = &sectors(&old, &new).unwrap()[0];
        assert_eq!(sec.old[2], 0xFF);
        assert!(needs_store(sec));
    }
}
