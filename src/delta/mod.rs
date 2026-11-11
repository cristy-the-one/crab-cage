//! Firmware delta kernels for the v0.03 LOAD/ADD/STORE format.
//!
//! The format and its correctness rules are fixed. A kernel is the per-sector
//! encoder: old sector plus its real-byte spans, new sector plus its spans,
//! and the command costs for one `add_pat_size_bits` width. It returns LOAD
//! and ADD commands. This module is the trusted host side. It parses SREC,
//! packs bits, decodes, and checks the result. A kernel's byte count is a
//! score only after that check passes.
//!
//! Kernels themselves live in `nuggets/delta-kernel` and run inside the
//! sandbox. They do not pack bits and they do not grade themselves.
//!
//! [`SPAN_LIMIT`] caps the address span of one pair. A sparse pair whose
//! regions sit gigabytes apart would otherwise allocate a flat buffer that
//! large.

/// The largest address span, in bytes, accepted for one old/new pair.
pub const SPAN_LIMIT: u32 = 8 * 1024 * 1024;

mod check;
mod corpus;
mod encode;
mod image;
mod kernel;
mod pack;
mod srec;

pub use check::{verify_bytes, VerifyReport};
pub use corpus::{load_pairs, measure, sync_cases, write_synthetic, Measurement, Pair, PAIRS_DIR};
pub use encode::{
    encode_add_only, encode_family, encode_optimal_file, encode_reference, encode_strategy_a,
    encode_strategy_b_local, proposal_cross_sector, search_family, FamilyParams, Planned,
};
pub use image::{Image, Sector};
pub use kernel::{case_input, eval_candidate, score_stdout, KernelOracle};
pub use pack::{add_header_bytes, crc32, ls_header_bytes, SECTOR};
