//! v0.03 bit packer and the sizes the generator costs against.
//!
//! Multi-byte header fields are big-endian. Command bits are MSB-first.
//! `pat_size` 0 means [`SECTOR`] bytes. Padding fields use the N+1 rule, so
//! a header that is already byte-aligned still carries 8 pad bits.

use crate::error::{err, Result};

use super::image::Sector;

/// Flash sector and RAM working buffer, in bytes.
pub const SECTOR: u32 = 2048;
const CMD_BITS: u32 = 2;
const ADDR_BITS: u32 = 11;
const LS_PAT_BITS: u32 = 11;
const SECT_OFF_BITS: u32 = 11;
const OP_LOAD: u32 = 0b01;
const OP_ADD: u32 = 0b10;
const OP_STORE: u32 = 0b11;

/// One LOAD or ADD. The ADD payload is the new sector's own bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Load { addr: u32, buf: u32, len: u32 },
    Add { buf: u32, len: u32 },
}

/// Commands for one sector, plus the STORE length the packer appends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectorPlan {
    pub base: u32,
    pub ops: Vec<Op>,
    pub store_len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cfg {
    pub cmd_bits: u32,
    pub addr_bits: u32,
    pub ls_pat_bits: u32,
    pub sect_off_bits: u32,
    pub ls_unused: u32,
    pub add_unused: u32,
    pub add_pat_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodedOp {
    Load {
        addr: u32,
        buf: u32,
        len: u32,
    },
    Add {
        buf: u32,
        len: u32,
        payload: Vec<u8>,
    },
    Store {
        addr: u32,
        buf: u32,
        len: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSector {
    pub base: u32,
    pub ops: Vec<DecodedOp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub version: u16,
    pub base_addr: u32,
    pub cfg: Cfg,
    pub sectors: Vec<DecodedSector>,
    /// `pat_size` fields that used the 0 sentinel.
    pub sentinel_count: u32,
    /// ADD commands whose payload is entirely 0xFF.
    pub all_ff_adds: u32,
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        let mut bit = 0;
        while bit < 8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            bit += 1;
        }
    }
    !crc
}

/// Pad bits that make `used` a multiple of 8. Already-aligned input yields 8.
pub fn unused_bits(used: u32) -> u32 {
    let rem = used % 8;
    if rem == 0 {
        8
    } else {
        8 - rem
    }
}

pub fn ls_header_bytes() -> u32 {
    let used = CMD_BITS + ADDR_BITS + LS_PAT_BITS + SECT_OFF_BITS;
    (used + unused_bits(used)) / 8
}

pub fn add_header_bytes(add_pat_bits: u32) -> u32 {
    let used = CMD_BITS + add_pat_bits + SECT_OFF_BITS;
    (used + unused_bits(used)) / 8
}

/// Largest non-sentinel `pat_size` a field of `bits` can hold.
pub fn max_pat(bits: u32) -> u32 {
    (1u32 << bits) - 1
}

pub fn fits_pat(len: u32, bits: u32) -> bool {
    len == SECTOR || (len >= 1 && len <= max_pat(bits))
}

/// Smallest `add_pat_size_bits` that can encode every length.
pub fn tuned_add_bits(lengths: &[u32]) -> Result<u32> {
    let mut bits = 1u32;
    for &len in lengths {
        if !fits_pat(len, 11) {
            return Err(err(format!("ADD length {len} is not encodable")));
        }
        let mut width = 1u32;
        while width < 11 && !fits_pat(len, width) {
            width += 1;
        }
        bits = bits.max(width);
    }
    Ok(bits)
}

pub fn encode_cfg(cfg: &Cfg) -> u32 {
    (cfg.cmd_bits - 1) << 29
        | (cfg.addr_bits - 1) << 24
        | (cfg.ls_pat_bits - 1) << 19
        | (cfg.sect_off_bits - 1) << 14
        | (cfg.ls_unused - 1) << 11
        | (cfg.add_unused - 1) << 8
        | (cfg.add_pat_bits - 1) << 3
}

pub fn decode_cfg(word: u32) -> Cfg {
    Cfg {
        cmd_bits: ((word >> 29) & 0b111) + 1,
        addr_bits: ((word >> 24) & 0b1_1111) + 1,
        ls_pat_bits: ((word >> 19) & 0b1_1111) + 1,
        sect_off_bits: ((word >> 14) & 0b1_1111) + 1,
        ls_unused: ((word >> 11) & 0b111) + 1,
        add_unused: ((word >> 8) & 0b111) + 1,
        add_pat_bits: ((word >> 3) & 0b1_1111) + 1,
    }
}

fn target_cfg(add_pat_bits: u32) -> Cfg {
    let ls_used = CMD_BITS + ADDR_BITS + LS_PAT_BITS + SECT_OFF_BITS;
    let add_used = CMD_BITS + add_pat_bits + SECT_OFF_BITS;
    Cfg {
        cmd_bits: CMD_BITS,
        addr_bits: ADDR_BITS,
        ls_pat_bits: LS_PAT_BITS,
        sect_off_bits: SECT_OFF_BITS,
        ls_unused: unused_bits(ls_used),
        add_unused: unused_bits(add_used),
        add_pat_bits,
    }
}

/// Pack a whole delta. `image_base` is the header's base address.
/// ADD payloads are copied from each sector's new-image bytes.
pub fn pack_with(image_base: u32, plans: &[(SectorPlan, &Sector)]) -> Result<Vec<u8>> {
    let mut add_lens = Vec::new();
    for (plan, _) in plans {
        for op in &plan.ops {
            if let Op::Add { len, .. } = op {
                add_lens.push(*len);
            }
        }
    }
    let add_bits = tuned_add_bits(&add_lens)?;
    let cfg = target_cfg(add_bits);
    validate_cfg(&cfg)?;
    let mut out = Vec::new();
    push_u16(&mut out, 0x0003);
    push_u32(&mut out, image_base);
    push_u32(&mut out, encode_cfg(&cfg));
    let count = u16::try_from(plans.len()).map_err(|_| err("too many sectors"))?;
    push_u16(&mut out, count);
    let mut prev = None;
    for (plan, _) in plans {
        if plan.base % SECTOR != 0 {
            return Err(err(format!(
                "sector base {:#x} is not 2 KB aligned",
                plan.base
            )));
        }
        if let Some(prev) = prev {
            if plan.base <= prev {
                return Err(err("sector bases are not strictly ascending"));
            }
        }
        prev = Some(plan.base);
        push_u32(&mut out, plan.base);
    }
    let mut bits = BitWriter::default();
    for (plan, sector) in plans {
        if plan.store_len == 0 || plan.store_len > SECTOR {
            return Err(err("STORE length is outside 1..=2048"));
        }
        for op in &plan.ops {
            match *op {
                Op::Load { addr, buf, len } => {
                    check_range(addr, len)?;
                    check_range(buf, len)?;
                    write_ls(&mut bits, &cfg, OP_LOAD, addr, len, buf)?;
                }
                Op::Add { buf, len } => {
                    check_range(buf, len)?;
                    if !fits_pat(len, add_bits) {
                        return Err(err(format!(
                            "ADD length {len} does not fit the tuned width"
                        )));
                    }
                    write_add_header(&mut bits, &cfg, len, buf)?;
                    let start = buf as usize;
                    let end = start + len as usize;
                    bits.write_bytes(&sector.new[start..end])?;
                }
            }
        }
        write_ls(&mut bits, &cfg, OP_STORE, 0, plan.store_len, 0)?;
    }
    out.extend(bits.finish()?);
    Ok(out)
}

pub fn unpack(bytes: &[u8]) -> Result<Decoded> {
    if bytes.len() < 12 {
        return Err(err("delta is shorter than the header"));
    }
    let version = read_u16(bytes, 0);
    if version != 0x0003 {
        return Err(err(format!("delta version {version:#06x} is not v0.03")));
    }
    let base_addr = read_u32(bytes, 2);
    let cfg_word = read_u32(bytes, 6);
    if cfg_word & 0b111 != 0 {
        return Err(err("CfgWord reserved bits are not zero"));
    }
    let cfg = decode_cfg(cfg_word);
    validate_cfg(&cfg)?;
    if cfg.cmd_bits != 2 || cfg.addr_bits != 11 || cfg.ls_pat_bits != 11 || cfg.sect_off_bits != 11
    {
        return Err(err(
            "CfgWord widths are not the 2 KB target (cmd 2, addr/pat/buf 11)",
        ));
    }
    let count = read_u16(bytes, 10) as usize;
    let header = 12 + 4 * count;
    if bytes.len() < header {
        return Err(err("delta sector table is truncated"));
    }
    if count == 0 {
        return Err(err("delta sector count is 0"));
    }
    let mut bases = Vec::with_capacity(count);
    let mut prev = None;
    for i in 0..count {
        let base = read_u32(bytes, 12 + 4 * i);
        if base % SECTOR != 0 {
            return Err(err(format!("sector base {base:#x} is not 2 KB aligned")));
        }
        if let Some(prev) = prev {
            if base <= prev {
                return Err(err("sector table is not strictly ascending"));
            }
        }
        prev = Some(base);
        bases.push(base);
    }
    let mut reader = BitReader {
        data: &bytes[header..],
        bit: 0,
    };
    let mut sectors = Vec::with_capacity(count);
    let mut sentinel_count = 0u32;
    let mut all_ff_adds = 0u32;
    for base in bases {
        let mut ops = Vec::new();
        loop {
            let opcode = reader.read(cfg.cmd_bits)?;
            if opcode == OP_LOAD || opcode == OP_STORE {
                let addr = reader.read(cfg.addr_bits)?;
                let raw_len = reader.read(cfg.ls_pat_bits)?;
                let buf = reader.read(cfg.sect_off_bits)?;
                let pad = reader.read(cfg.ls_unused)?;
                if pad != 0 {
                    return Err(err("LOAD/STORE padding is not zero"));
                }
                let len = decode_pat(raw_len, &mut sentinel_count);
                if opcode == OP_LOAD {
                    ops.push(DecodedOp::Load { addr, buf, len });
                } else {
                    ops.push(DecodedOp::Store { addr, buf, len });
                    break;
                }
            } else if opcode == OP_ADD {
                let raw_len = reader.read(cfg.add_pat_bits)?;
                let buf = reader.read(cfg.sect_off_bits)?;
                let pad = reader.read(cfg.add_unused)?;
                if pad != 0 {
                    return Err(err("ADD padding is not zero"));
                }
                if reader.bit % 8 != 0 {
                    return Err(err("ADD payload is not byte-aligned"));
                }
                let len = decode_pat(raw_len, &mut sentinel_count);
                let payload = reader.read_bytes(len as usize)?;
                if payload.iter().all(|b| *b == 0xFF) {
                    all_ff_adds = all_ff_adds.saturating_add(1);
                }
                ops.push(DecodedOp::Add { buf, len, payload });
            } else {
                return Err(err("reserved opcode 0b00"));
            }
        }
        sectors.push(DecodedSector { base, ops });
    }
    if reader.bit % 8 != 0 || reader.bit / 8 != reader.data.len() {
        return Err(err("trailing bytes after the last STORE"));
    }
    Ok(Decoded {
        version,
        base_addr,
        cfg,
        sectors,
        sentinel_count,
        all_ff_adds,
    })
}

fn decode_pat(raw: u32, sentinels: &mut u32) -> u32 {
    if raw == 0 {
        *sentinels = sentinels.saturating_add(1);
        SECTOR
    } else {
        raw
    }
}

fn validate_cfg(cfg: &Cfg) -> Result<()> {
    if cfg.cmd_bits < 2 {
        return Err(err("cmd_bits is below 2"));
    }
    for (name, width) in [
        ("cmd", cfg.cmd_bits),
        ("addr", cfg.addr_bits),
        ("ls_pat", cfg.ls_pat_bits),
        ("sect_off", cfg.sect_off_bits),
        ("ls_unused", cfg.ls_unused),
        ("add_unused", cfg.add_unused),
        ("add_pat", cfg.add_pat_bits),
    ] {
        if width == 0 || width > 32 {
            return Err(err(format!("{name} width {width} is outside 1..=32")));
        }
    }
    let ls = cfg.cmd_bits + cfg.addr_bits + cfg.ls_pat_bits + cfg.sect_off_bits + cfg.ls_unused;
    if ls % 8 != 0 {
        return Err(err("LOAD/STORE header is not a whole number of bytes"));
    }
    let add = cfg.cmd_bits + cfg.add_pat_bits + cfg.sect_off_bits + cfg.add_unused;
    if add % 8 != 0 {
        return Err(err("ADD header is not a whole number of bytes"));
    }
    Ok(())
}

fn check_range(off: u32, len: u32) -> Result<()> {
    if len == 0 || off.checked_add(len).map(|end| end > SECTOR).unwrap_or(true) {
        return Err(err(format!("range {off}+{len} is outside the sector")));
    }
    Ok(())
}

fn write_ls(bits: &mut BitWriter, cfg: &Cfg, op: u32, addr: u32, len: u32, buf: u32) -> Result<()> {
    if !fits_pat(len, cfg.ls_pat_bits) {
        return Err(err(format!("LOAD/STORE length {len} does not fit")));
    }
    bits.write(op, cfg.cmd_bits)?;
    bits.write(addr, cfg.addr_bits)?;
    bits.write(encode_pat(len), cfg.ls_pat_bits)?;
    bits.write(buf, cfg.sect_off_bits)?;
    bits.write(0, cfg.ls_unused)?;
    Ok(())
}

fn write_add_header(bits: &mut BitWriter, cfg: &Cfg, len: u32, buf: u32) -> Result<()> {
    bits.write(OP_ADD, cfg.cmd_bits)?;
    bits.write(encode_pat(len), cfg.add_pat_bits)?;
    bits.write(buf, cfg.sect_off_bits)?;
    bits.write(0, cfg.add_unused)?;
    if bits.bit_len % 8 != 0 {
        return Err(err("ADD header did not land on a byte"));
    }
    Ok(())
}

fn encode_pat(len: u32) -> u32 {
    if len == SECTOR {
        0
    } else {
        len
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    acc: u8,
    filled: u32,
    bit_len: usize,
}

impl BitWriter {
    fn write(&mut self, value: u32, n: u32) -> Result<()> {
        if n == 0 || n > 32 {
            return Err(err("bit field width is outside 1..=32"));
        }
        if n < 32 && value >> n != 0 {
            return Err(err("bit field value does not fit its width"));
        }
        let mut left = n;
        while left > 0 {
            let space = 8 - self.filled;
            let take = left.min(space);
            let shift = left - take;
            let mask = if take == 8 { 0xFF } else { (1u32 << take) - 1 };
            let chunk = ((value >> shift) & mask) as u8;
            if take == 8 {
                self.acc = chunk;
            } else {
                self.acc = (self.acc << take) | chunk;
            }
            self.filled += take;
            self.bit_len += take as usize;
            left -= take;
            if self.filled == 8 {
                self.bytes.push(self.acc);
                self.acc = 0;
                self.filled = 0;
            }
        }
        Ok(())
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        if self.filled != 0 {
            return Err(err("payload written while the stream is not byte-aligned"));
        }
        self.bytes.extend_from_slice(data);
        self.bit_len += data.len() * 8;
        Ok(())
    }

    fn finish(self) -> Result<Vec<u8>> {
        if self.filled != 0 {
            return Err(err("delta body ends mid-byte"));
        }
        Ok(self.bytes)
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn read(&mut self, n: u32) -> Result<u32> {
        let mut value = 0u32;
        let mut left = n;
        while left > 0 {
            if self.bit / 8 >= self.data.len() {
                return Err(err("delta body ended inside a command"));
            }
            let space = 8 - (self.bit % 8) as u32;
            let take = left.min(space);
            let shift = 7 - (self.bit % 8) as u32;
            let mut chunk = u32::from(self.data[self.bit / 8]);
            chunk >>= shift + 1 - take;
            chunk &= (1u32 << take) - 1;
            value = (value << take) | chunk;
            self.bit += take as usize;
            left -= take;
        }
        Ok(value)
    }

    fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        if self.bit % 8 != 0 {
            return Err(err("reading payload bytes off a byte boundary"));
        }
        let start = self.bit / 8;
        let end = start + n;
        if end > self.data.len() {
            return Err(err("ADD payload runs past the end of the delta"));
        }
        self.bit += n * 8;
        Ok(self.data[start..end].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_match_the_spec_examples() {
        assert_eq!(ls_header_bytes(), 5);
        assert_eq!(add_header_bytes(11), 4);
        assert_eq!(add_header_bytes(9), 3);
        assert_eq!(add_header_bytes(1), 2);
        assert_eq!(add_header_bytes(2), 2);
        assert_eq!(add_header_bytes(3), 3);
        assert_eq!(add_header_bytes(10), 3);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn cfg_word_round_trips_and_a_full_sector_uses_the_sentinel() {
        let cfg = target_cfg(9);
        assert_eq!(cfg.ls_unused, 5);
        assert_eq!(cfg.add_unused, 2);
        assert_eq!(decode_cfg(encode_cfg(&cfg)), cfg);

        let mut sector = Sector {
            base: 0x0800_0000,
            old: vec![0x11; SECTOR as usize],
            new: vec![0x11; SECTOR as usize],
            old_real: vec![],
            new_real: vec![],
        };
        sector.new[0] = 0x22;
        let plan = SectorPlan {
            base: sector.base,
            ops: vec![
                Op::Load {
                    addr: 0,
                    buf: 0,
                    len: SECTOR,
                },
                Op::Add { buf: 0, len: 1 },
            ],
            store_len: SECTOR,
        };
        let bytes = pack_with(0x0800_0000, &[(plan, &sector)]).unwrap();
        let decoded = unpack(&bytes).unwrap();
        assert_eq!(decoded.sentinel_count, 2);
        assert!(matches!(
            decoded.sectors[0].ops[0],
            DecodedOp::Load { len: SECTOR, .. }
        ));
        assert!(matches!(
            &decoded.sectors[0].ops[1],
            DecodedOp::Add { payload, .. } if payload == &vec![0x22]
        ));
    }
}
