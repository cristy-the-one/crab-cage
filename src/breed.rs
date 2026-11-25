//! Evolutionary code and parameter breeder (`--breed`).
//!
//! Augments LLM guidance with fast, sandbox-validated code evolution across the
//! [`SpeciesArchive`]:
//! 1. **UCB1 parent selection** across proven species so newly discovered
//!    structures get fast tuning turns alongside the global leader.
//! 2. **Directional momentum (1+1 line search)**: when a single-literal step
//!    improves a parent's score in `lust`, the next turn accelerates along that
//!    coordinate (`2x` step, plus half-step refinement).
//! 3. **Same-species parametric crossover & Differential Evolution**: combines
//!    and extrapolates numeric vectors across scored variants sharing a species
//!    skeleton (`best + F * (best - other)` and uniform/midpoint crossover).
//! 4. **Multi-scale & coupled two-literal numeric mutations**: fine (`±1, ±2`),
//!    relative (`±10%, ±25%, ±50%`), geometric (`×2, ÷2`), and anchor probes.
//! 5. **Genetic Improvement structural micro-mutations**: boundary relational
//!    flips (`<` ↔ `<=`, `>` ↔ `>=`, `==` ↔ `!=`), range bounds (`..` ↔ `..=`),
//!    extrema swaps (`.min(` ↔ `.max(`), saturating subs, and boolean connectives.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::Result;
use crate::genome::rust_f64;
use crate::nugget::Nugget;
use crate::pipeline::{candidate_hash, Status};
use crate::proposer::{Feedback, Proposal, Proposer};
use crate::rng::Pcg64;
use crate::species::{species_id, SpeciesHandle, SpeciesVariant, DEFAULT_UCB_C};

const BREED_STREAM: u64 = 42;
const MAX_DRAWS_PER_CALL: usize = 32;
const MAX_MOMENTUM_QUEUE: usize = 16;

#[derive(Clone, Debug)]
struct PendingStep {
    parent_score: f64,
    lit_index: usize,
    delta: f64,
    is_float: bool,
}

#[derive(Clone, Debug)]
struct MomentumMove {
    base_text: String,
    base_score: f64,
    lit_index: usize,
    step: f64,
    is_float: bool,
}

/// Evolutionary proposer combining parameter optimization, same-species DE
/// crossover, and Genetic Improvement micro-mutations.
pub struct BreedProposer {
    rng: Pcg64,
    species: Option<SpeciesHandle>,
    emitted: HashSet<String>,
    pending_steps: HashMap<String, PendingStep>,
    momentum: VecDeque<MomentumMove>,
    observed_hashes: HashSet<String>,
}

impl BreedProposer {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Pcg64::new(seed, BREED_STREAM),
            species: None,
            emitted: HashSet::new(),
            pending_steps: HashMap::new(),
            momentum: VecDeque::new(),
            observed_hashes: HashSet::new(),
        }
    }

    pub fn with_species(mut self, species: SpeciesHandle) -> Self {
        self.species = Some(species);
        self
    }

    fn observe(&mut self, nugget: &Nugget, feedback: &Feedback) {
        self.emitted.insert(candidate_hash(&nugget.seed));
        if let Some((best_text, _)) = feedback.best {
            self.emitted.insert(candidate_hash(best_text));
        }
        for &(text, verdict) in feedback.recent {
            let hash = candidate_hash(text);
            self.emitted.insert(hash.clone());
            if !self.observed_hashes.insert(hash.clone()) {
                continue;
            }
            let Some(step) = self.pending_steps.remove(&hash) else {
                continue;
            };
            if !matches!(verdict.status, Status::Scored | Status::Proven) {
                continue;
            }
            let Some(score) = verdict.score.filter(|s| s.is_finite()) else {
                continue;
            };
            if score < step.parent_score && self.momentum.len() < MAX_MOMENTUM_QUEUE {
                // Accelerate 2x along the improving coordinate, followed by a 1x step.
                let accel = if step.is_float {
                    step.delta * 1.8
                } else {
                    (step.delta * 2.0).signum() * (step.delta.abs() * 2.0).max(1.0)
                };
                self.momentum.push_back(MomentumMove {
                    base_text: text.to_string(),
                    base_score: score,
                    lit_index: step.lit_index,
                    step: accel,
                    is_float: step.is_float,
                });
                self.momentum.push_back(MomentumMove {
                    base_text: text.to_string(),
                    base_score: score,
                    lit_index: step.lit_index,
                    step: step.delta,
                    is_float: step.is_float,
                });
            }
        }
    }

    fn try_momentum(&mut self) -> Option<String> {
        while let Some(mv) = self.momentum.pop_front() {
            let lits = scan_signed_literals(&mv.base_text);
            let Some(lit) = lits.get(mv.lit_index) else {
                continue;
            };
            let Some(replacement) = apply_numeric_step(lit, mv.step, mv.is_float) else {
                continue;
            };
            let candidate = replace_range(&mv.base_text, lit.start, lit.end, &replacement);
            let hash = candidate_hash(&candidate);
            if self.emitted.insert(hash.clone()) {
                self.pending_steps.insert(
                    hash,
                    PendingStep {
                        parent_score: mv.base_score,
                        lit_index: mv.lit_index,
                        delta: mv.step,
                        is_float: mv.is_float,
                    },
                );
                return Some(candidate);
            }
        }
        None
    }

    fn choose_parent(
        &mut self,
        nugget: &Nugget,
        feedback: &Feedback,
    ) -> (String, f64, Vec<SpeciesVariant>) {
        if let Some(handle) = &self.species {
            let snap = handle.borrow_mut().select_parent(DEFAULT_UCB_C);
            if let Some(snap) = snap {
                let variants = if snap.variants.is_empty() {
                    handle.borrow().variants_of(&snap.species_id)
                } else {
                    snap.variants
                };
                return (snap.text, snap.score, variants);
            }
        }
        match feedback.best {
            Some((text, score)) => (text.to_string(), score, Vec::new()),
            None => (nugget.seed.clone(), f64::INFINITY, Vec::new()),
        }
    }
}

impl Proposer for BreedProposer {
    fn name(&self) -> &str {
        "breed"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
        self.observe(nugget, feedback);

        // 1. Follow any improving coordinate line search immediately.
        if let Some(candidate) = self.try_momentum() {
            return Ok(Proposal::Candidate(candidate));
        }

        let (parent_text, parent_score, variants) = self.choose_parent(nugget, feedback);
        let lits = scan_signed_literals(&parent_text);
        let code_sites = scan_code_mutations(&parent_text);

        if lits.is_empty() && code_sites.is_empty() {
            // If species archive can still gain new parents from another proposer, wait.
            if self.species.is_some() {
                return Ok(Proposal::Later);
            }
            return Ok(Proposal::Finished);
        }

        for attempt in 0..MAX_DRAWS_PER_CALL {
            let roll = self.rng.next_f64();
            let generated: Option<(String, Option<PendingStep>)> = if variants.len() >= 2
                && !lits.is_empty()
                && roll < 0.25
            {
                crossover_or_de(&mut self.rng, &variants).map(|text| (text, None))
            } else if !lits.is_empty() && (code_sites.is_empty() || roll < 0.72) {
                if lits.len() >= 2 && self.rng.chance(0.22) {
                    coupled_two_literal_tweak(&mut self.rng, &parent_text, &lits)
                        .map(|text| (text, None))
                } else {
                    single_literal_tweak(&mut self.rng, &parent_text, parent_score, &lits, attempt)
                }
            } else if !code_sites.is_empty() {
                let site = &code_sites[self.rng.gen_index(code_sites.len())];
                Some((
                    replace_range(&parent_text, site.start, site.end, site.replacement),
                    None,
                ))
            } else {
                None
            };

            let Some((candidate, pending)) = generated else {
                continue;
            };
            if crate::emit::sandbox_prefilter(&candidate).is_err() {
                continue;
            }
            let hash = candidate_hash(&candidate);
            if self.emitted.insert(hash.clone()) {
                if let Some(step) = pending {
                    self.pending_steps.insert(hash, step);
                }
                return Ok(Proposal::Candidate(candidate));
            }
        }

        // Current neighborhood is saturated; wait for pending evaluations or new LLM species.
        Ok(Proposal::Later)
    }
}

#[derive(Clone, Debug)]
struct SignedLit {
    start: usize,
    end: usize,
    value: f64,
    int_val: Option<i128>,
    unsigned_only: bool,
    suffix: String,
    raw: String,
}

impl SignedLit {
    fn is_float(&self) -> bool {
        self.int_val.is_none()
    }
}

/// Scan numeric literals outside comments, strings, and char literals,
/// capturing unary `-` in value position and numeric suffixes (`usize`, `f64`).
fn scan_signed_literals(source: &str) -> Vec<SignedLit> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut i = 0usize;
    let mut last_non_ws: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_string(bytes, i);
            last_non_ws = Some(b'"');
            continue;
        }
        if b == b'\'' {
            if let Some(end) = skip_char(source, i) {
                i = end;
                last_non_ws = Some(b'\'');
                continue;
            }
        }

        let neg = b == b'-'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && is_value_prefix(last_non_ws);
        let digits_start = if neg { i + 1 } else { i };

        if (neg || b.is_ascii_digit()) && !preceded_by_ident_or_dot(source, i) {
            // Skip hex/bin/octal bitmasks (`0xFF`, `0b1010`) so bit flags stay intact.
            if digits_start + 1 < bytes.len()
                && bytes[digits_start] == b'0'
                && matches!(
                    bytes[digits_start + 1],
                    b'x' | b'X' | b'b' | b'B' | b'o' | b'O'
                )
            {
                let mut end = digits_start + 2;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                i = end;
                last_non_ws = Some(b'0');
                continue;
            }

            let num_end = scan_decimal_end(bytes, digits_start);
            let (full_end, suffix) = scan_numeric_suffix(source, num_end);
            let num_token: String = source[i..num_end].chars().filter(|&c| c != '_').collect();
            let raw = source[i..full_end].to_string();
            let in_index_brackets =
                last_non_ws == Some(b'[') || source[full_end..].trim_start().starts_with(']');
            let is_float_tok = num_token
                .bytes()
                .any(|ch| ch == b'.' || ch == b'e' || ch == b'E')
                || matches!(suffix.as_str(), "f32" | "f64");

            if is_float_tok {
                if let Ok(val) = num_token.parse::<f64>() {
                    if val.is_finite() {
                        found.push(SignedLit {
                            start: i,
                            end: full_end,
                            value: val,
                            int_val: None,
                            unsigned_only: false,
                            suffix,
                            raw,
                        });
                    }
                }
            } else if let Ok(ival) = num_token.parse::<i128>() {
                let unsigned_only = suffix.starts_with('u') || in_index_brackets || !neg;
                found.push(SignedLit {
                    start: i,
                    end: full_end,
                    value: ival as f64,
                    int_val: Some(ival),
                    unsigned_only,
                    suffix,
                    raw,
                });
            }
            i = full_end;
            last_non_ws = Some(b'0');
            continue;
        }

        if let Some(ch) = source[i..].chars().next() {
            last_non_ws = Some(b);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    found
}

fn is_value_prefix(last: Option<u8>) -> bool {
    matches!(
        last,
        None | Some(
            b'=' | b'('
                | b'['
                | b'{'
                | b','
                | b':'
                | b';'
                | b'+'
                | b'-'
                | b'*'
                | b'/'
                | b'%'
                | b'<'
                | b'>'
                | b'!'
                | b'&'
                | b'|'
                | b'^'
        )
    )
}

fn preceded_by_ident_or_dot(source: &str, pos: usize) -> bool {
    let Some(prev) = source[..pos].chars().next_back() else {
        return false;
    };
    prev.is_alphabetic() || prev.is_ascii_digit() || prev == '_' || prev == '.'
}

fn scan_decimal_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        let digits = j;
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'_') {
            j += 1;
        }
        if j > digits {
            i = j;
        }
    }
    i
}

fn scan_numeric_suffix(source: &str, pos: usize) -> (usize, String) {
    let bytes = source.as_bytes();
    if pos >= bytes.len() || !matches!(bytes[pos], b'u' | b'i' | b'f') {
        return (pos, String::new());
    }
    let mut end = pos + 1;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
        end += 1;
    }
    let suffix = &source[pos..end];
    if matches!(
        suffix,
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    ) {
        (end, suffix.to_string())
    } else {
        (pos, String::new())
    }
}

fn format_lit_value(lit: &SignedLit, new_val: f64) -> Option<String> {
    if !new_val.is_finite() {
        return None;
    }
    if lit.is_float() {
        let body = rust_f64(new_val);
        let formatted = format!("{body}{}", lit.suffix);
        if formatted == lit.raw {
            None
        } else {
            Some(formatted)
        }
    } else {
        let mut rounded = new_val.round() as i128;
        if lit.unsigned_only && rounded < 0 {
            rounded = 0;
        }
        let formatted = format!("{rounded}{}", lit.suffix);
        if formatted == lit.raw {
            None
        } else {
            Some(formatted)
        }
    }
}

fn apply_numeric_step(lit: &SignedLit, step: f64, is_float: bool) -> Option<String> {
    let target = if is_float {
        lit.value + step
    } else {
        (lit.int_val? as f64) + step
    };
    format_lit_value(lit, target)
}

fn single_literal_tweak(
    rng: &mut Pcg64,
    parent_text: &str,
    parent_score: f64,
    lits: &[SignedLit],
    attempt: usize,
) -> Option<(String, Option<PendingStep>)> {
    let idx = rng.gen_index(lits.len());
    let lit = &lits[idx];
    let is_float = lit.is_float();

    let new_val = if is_float {
        let mode = (attempt + rng.gen_index(4)) % 4;
        match mode {
            0 => {
                // Fine Gaussian ~10%
                let g = rng.gauss();
                if lit.value.abs() < 1e-9 {
                    g * 0.05
                } else {
                    lit.value * (1.0 + g * 0.12)
                }
            }
            1 => {
                // Wider Gaussian ~35%
                let g = rng.gauss();
                if lit.value.abs() < 1e-9 {
                    g * 0.25
                } else {
                    lit.value * (1.0 + g * 0.35)
                }
            }
            2 => {
                // Geometric scale
                let factors = [0.5, 0.75, 1.25, 1.5, 2.0];
                let f = factors[rng.gen_index(factors.len())];
                if lit.value.abs() < 1e-9 {
                    if rng.chance(0.5) {
                        0.1
                    } else {
                        -0.1
                    }
                } else {
                    lit.value * f
                }
            }
            _ => {
                // Anchor values
                let anchors = [0.0, 0.5, 1.0];
                anchors[rng.gen_index(anchors.len())]
            }
        }
    } else {
        let cur = lit.int_val?;
        let mode = (attempt + rng.gen_index(4)) % 4;
        let next_int = match mode {
            0 => {
                // Fine step ±1, ±2, ±3
                let steps = [-3i128, -2, -1, 1, 2, 3];
                cur.saturating_add(steps[rng.gen_index(steps.len())])
            }
            1 => {
                // Relative step ±15%..±50% (meaningful for constants like 15, 30, 100)
                let mag = ((cur.abs() as f64) * rng.uniform(0.15, 0.5))
                    .round()
                    .max(2.0) as i128;
                if rng.chance(0.5) {
                    cur.saturating_add(mag)
                } else {
                    cur.saturating_sub(mag)
                }
            }
            2 => {
                // Halve or double
                if cur == 0 {
                    if rng.chance(0.5) {
                        4
                    } else {
                        8
                    }
                } else if rng.chance(0.5) {
                    cur / 2
                } else {
                    cur.saturating_mul(2)
                }
            }
            _ => {
                let anchors = [0i128, 1, 2, 4, 8, 16];
                anchors[rng.gen_index(anchors.len())]
            }
        };
        let clamped = if lit.unsigned_only && next_int < 0 {
            0
        } else {
            next_int
        };
        clamped as f64
    };

    let replacement = format_lit_value(lit, new_val)?;
    let delta = new_val - lit.value;
    let candidate = replace_range(parent_text, lit.start, lit.end, &replacement);
    let step = if parent_score.is_finite() && delta.abs() > 0.0 {
        Some(PendingStep {
            parent_score,
            lit_index: idx,
            delta,
            is_float,
        })
    } else {
        None
    };
    Some((candidate, step))
}

fn coupled_two_literal_tweak(
    rng: &mut Pcg64,
    parent_text: &str,
    lits: &[SignedLit],
) -> Option<String> {
    if lits.len() < 2 {
        return None;
    }
    let first = rng.gen_index(lits.len());
    let mut second = rng.gen_index(lits.len() - 1);
    if second >= first {
        second += 1;
    }
    let (lo, hi) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    let mut edits = Vec::with_capacity(2);
    for &idx in &[lo, hi] {
        let lit = &lits[idx];
        let target = if lit.is_float() {
            if lit.value.abs() < 1e-9 {
                rng.gauss() * 0.1
            } else {
                lit.value * (1.0 + rng.gauss() * 0.2)
            }
        } else {
            let cur = lit.int_val?;
            let step = ((cur.abs() as f64) * 0.25).round().max(1.0) as i128;
            let next = if rng.chance(0.5) {
                cur.saturating_add(step)
            } else {
                cur.saturating_sub(step)
            };
            (if lit.unsigned_only && next < 0 {
                0
            } else {
                next
            }) as f64
        };
        let rep = format_lit_value(lit, target)?;
        edits.push((lit.start, lit.end, rep));
    }
    Some(apply_descending_edits(parent_text, &edits))
}

/// Same-species Differential Evolution (`best + F * (best - other)`) and
/// uniform / midpoint blend crossover on matching literal vectors.
fn crossover_or_de(rng: &mut Pcg64, variants: &[SpeciesVariant]) -> Option<String> {
    if variants.len() < 2 {
        return None;
    }
    let best = &variants[0];
    let best_sp = species_id(&best.text);
    let best_lits = scan_signed_literals(&best.text);
    if best_lits.is_empty() {
        return None;
    }

    let mates: Vec<(&SpeciesVariant, Vec<SignedLit>)> = variants[1..]
        .iter()
        .filter(|v| species_id(&v.text) == best_sp)
        .filter_map(|v| {
            let vl = scan_signed_literals(&v.text);
            (vl.len() == best_lits.len()).then_some((v, vl))
        })
        .collect();
    if mates.is_empty() {
        return None;
    }
    let (_, other_lits) = &mates[rng.gen_index(mates.len())];
    let differing = best_lits
        .iter()
        .zip(other_lits.iter())
        .filter(|(b_lit, o_lit)| (b_lit.value - o_lit.value).abs() >= 1e-12)
        .count();
    if differing == 0 {
        return None;
    }

    // Uniform mate-copying only creates a new point when >= 2 coordinates differ;
    // when a single coordinate differs, extrapolate (DE) or interpolate (midpoint).
    let use_crossover = differing >= 2 && rng.chance(0.45);
    let scale_f = if rng.chance(0.5) { 1.0 } else { 0.5 };
    let mut edits = Vec::new();

    for (b_lit, o_lit) in best_lits.iter().zip(other_lits.iter()) {
        if (b_lit.value - o_lit.value).abs() < 1e-12 {
            continue;
        }
        let target = if use_crossover {
            if rng.chance(0.5) {
                // Midpoint interpolation
                0.5 * (b_lit.value + o_lit.value)
            } else {
                // Uniform pick from mate
                o_lit.value
            }
        } else {
            // Differential Evolution extrapolation away from the worse mate
            b_lit.value + scale_f * (b_lit.value - o_lit.value)
        };
        if let Some(rep) = format_lit_value(b_lit, target) {
            edits.push((b_lit.start, b_lit.end, rep));
        }
    }

    if edits.is_empty() {
        None
    } else {
        Some(apply_descending_edits(&best.text, &edits))
    }
}

#[derive(Clone, Debug)]
struct CodeSite {
    start: usize,
    end: usize,
    replacement: &'static str,
}

/// Scan safe Genetic Improvement structural micro-mutation sites outside
/// comments, string literals, char literals, and generic angle brackets.
fn scan_code_mutations(source: &str) -> Vec<CodeSite> {
    let bytes = source.as_bytes();
    let mut sites = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'\'' {
            if let Some(end) = skip_char(source, i) {
                i = end;
                continue;
            }
        }

        let rest = &source[i..];
        // Extrema method swaps
        if rest.starts_with(".min(") {
            sites.push(CodeSite {
                start: i,
                end: i + 5,
                replacement: ".max(",
            });
            i += 5;
            continue;
        }
        if rest.starts_with(".max(") {
            sites.push(CodeSite {
                start: i,
                end: i + 5,
                replacement: ".min(",
            });
            i += 5;
            continue;
        }
        // Range inclusivity `..=` <-> `..`
        if rest.starts_with("..=") {
            sites.push(CodeSite {
                start: i,
                end: i + 3,
                replacement: "..",
            });
            i += 3;
            continue;
        }
        if rest.starts_with("..")
            && !rest.starts_with("...")
            && rest[2..]
                .trim_start()
                .bytes()
                .next()
                .is_some_and(|next| next.is_ascii_alphanumeric() || next == b'(' || next == b'_')
        {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "..=",
            });
            i += 2;
            continue;
        }
        // Boolean connectives
        if rest.starts_with("&&") {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "||",
            });
            i += 2;
            continue;
        }
        if rest.starts_with("||") && !is_empty_closure(source, i) {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "&&",
            });
            i += 2;
            continue;
        }
        // Relational two-char operators (`<=`, `>=`, `==`, `!=`)
        if rest.starts_with("<=") {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "<",
            });
            i += 2;
            continue;
        }
        if rest.starts_with(">=") {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: ">",
            });
            i += 2;
            continue;
        }
        if rest.starts_with("==") {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "!=",
            });
            i += 2;
            continue;
        }
        if rest.starts_with("!=") {
            sites.push(CodeSite {
                start: i,
                end: i + 2,
                replacement: "==",
            });
            i += 2;
            continue;
        }
        // Skip shifts and arrows so `<` / `>` below never split `<<`, `>>`, `->`, `=>`
        if rest.starts_with("<<")
            || rest.starts_with(">>")
            || rest.starts_with("->")
            || rest.starts_with("=>")
        {
            i += 2;
            continue;
        }
        // Single-char `<` and `>` when surrounded by spaces (comparison, not `Vec<T>`)
        if (b == b'<' || b == b'>') && surrounded_by_spaces(bytes, i) {
            sites.push(CodeSite {
                start: i,
                end: i + 1,
                replacement: if b == b'<' { "<=" } else { ">=" },
            });
            i += 1;
            continue;
        }

        if let Some(ch) = rest.chars().next() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    sites
}

fn surrounded_by_spaces(bytes: &[u8], pos: usize) -> bool {
    pos > 0 && pos + 1 < bytes.len() && bytes[pos - 1] == b' ' && bytes[pos + 1] == b' '
}

fn is_empty_closure(source: &str, pos: usize) -> bool {
    let prev = source[..pos].trim_end().bytes().next_back();
    matches!(prev, None | Some(b'(' | b',' | b'=' | b'{' | b';'))
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'"' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    while i < bytes.len() && depth > 0 {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_char(source: &str, start: usize) -> Option<usize> {
    let rest = &source[start + 1..];
    if rest.starts_with('\\') {
        let mut chars = rest.char_indices();
        let _ = chars.next()?;
        let _ = chars.next()?;
        let (pos, ch) = chars.next()?;
        return (ch == '\'').then_some(start + 1 + pos + 1);
    }
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' {
        return None;
    }
    let (pos, second) = chars.next()?;
    (second == '\'').then_some(start + 1 + pos + 1)
}

fn replace_range(source: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(source.len() - (end - start) + replacement.len());
    out.push_str(&source[..start]);
    out.push_str(replacement);
    out.push_str(&source[end..]);
    out
}

fn apply_descending_edits(source: &str, edits: &[(usize, usize, String)]) -> String {
    let mut sorted = edits.to_vec();
    sorted.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    let mut out = source.to_string();
    for (start, end, rep) in sorted {
        out.replace_range(start..end, &rep);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nugget::{Case, Objective};
    use crate::pipeline::{Gate, Ms, Verdict};
    use crate::species::SpeciesArchive;
    use std::time::Duration;

    fn test_nugget(seed: &str) -> Nugget {
        Nugget {
            name: "probe".to_string(),
            objective: Objective::Minimize,
            timeout: Duration::from_millis(500),
            max_memory: 64 * 1024 * 1024,
            prefix: "fn f(x: i64) -> i64 {\n// CRAB:BEGIN\n".to_string(),
            seed: seed.to_string(),
            suffix: "// CRAB:END\n}\n".to_string(),
            region_first_line: 3,
            cases: vec![Case {
                id: "c1".to_string(),
                input: String::new(),
                expected: None,
            }],
        }
    }

    fn make_scored(text: &str, score: f64) -> Verdict {
        Verdict {
            hash: candidate_hash(text),
            gate: Gate::Run,
            status: Status::Scored,
            score: Some(score),
            passed: 1,
            total: 1,
            diagnostics: Vec::new(),
            ms: Ms::default(),
            note: String::new(),
            failure: None,
        }
    }

    #[test]
    fn momentum_accelerates_an_improving_coordinate_step() {
        let seed = "    let k = 10;\n    x + k\n";
        let nugget = test_nugget(seed);
        let mut breeder = BreedProposer::new(7);
        let fb0 = Feedback {
            best: Some((seed, 100.0)),
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let first = breeder.next(&nugget, &fb0).unwrap().into_text().unwrap();
        assert_ne!(first, seed);

        // Simulate `lust` reporting an improved score (80.0 < 100.0) on `first`:
        // `BreedProposer` should immediately queue a 2x momentum step in the same direction!
        let v1 = make_scored(&first, 80.0);
        let recent = [(first.as_str(), &v1)];
        let fb1 = Feedback {
            best: Some((seed, 100.0)),
            last: Some((first.as_str(), &v1)),
            recent: &recent,
            best_verdict: None,
        };
        let second = breeder.next(&nugget, &fb1).unwrap().into_text().unwrap();
        let v_seed = scan_signed_literals(seed)[0].int_val.unwrap();
        let v_first = scan_signed_literals(&first)[0].int_val.unwrap();
        let v_second = scan_signed_literals(&second)[0].int_val.unwrap();
        let d1 = v_first - v_seed;
        let d2 = v_second - v_first;
        assert_eq!(
            d1.signum(),
            d2.signum(),
            "momentum must continue in the winning direction: {v_seed} -> {v_first} -> {v_second}"
        );
        assert!(d2.abs() >= d1.abs());
    }

    #[test]
    fn differential_evolution_extrapolates_same_species_variants() {
        let best = SpeciesVariant {
            hash: "h1".to_string(),
            text: "    if rem < 20 { 1 } else { 0 }\n".to_string(),
            score: 50.0,
            case_scores: Vec::new(),
            proven: true,
        };
        let worse = SpeciesVariant {
            hash: "h2".to_string(),
            text: "    if rem < 30 { 1 } else { 0 }\n".to_string(),
            score: 80.0,
            case_scores: Vec::new(),
            proven: false,
        };
        let mut rng = Pcg64::new(3, BREED_STREAM);
        let child = crossover_or_de(&mut rng, &[best, worse]).unwrap();
        let lit = scan_signed_literals(&child)[0].int_val.unwrap();
        // Extrapolates from 20 away from 30 -> 10 or 15!
        assert!(lit == 10 || lit == 15, "got {child}");
    }

    #[test]
    fn code_mutation_scanner_touches_comparisons_and_skips_generics_and_closures() {
        let code = "    let v: Vec<usize> = xs.iter().filter(|| true).collect();\n    if a < b && c >= 0 { a.min(b) } else { 0 }\n";
        let sites = scan_code_mutations(code);
        let replacements: Vec<&str> = sites.iter().map(|s| s.replacement).collect();
        assert!(replacements.contains(&"<="));
        assert!(replacements.contains(&"||"));
        assert!(replacements.contains(&">"));
        assert!(replacements.contains(&".max("));
        // Must NOT touch `<usize>` or empty closure `||`
        for s in &sites {
            let mutated = replace_range(code, s.start, s.end, s.replacement);
            assert!(mutated.contains("Vec<usize>"), "{mutated}");
            assert!(mutated.contains("filter(|| true)"), "{mutated}");
        }
    }

    #[test]
    fn breeder_never_emits_the_same_candidate_twice_and_uses_species() {
        let handle = SpeciesArchive::new_handle(3);
        let s1 = "    if x < 12 { x + 1 } else { x }\n";
        let mut v = make_scored(s1, 10.0);
        v.status = Status::Proven;
        handle.borrow_mut().observe(s1, &v, &[], "seed");

        let nugget = test_nugget(s1);
        let mut breeder = BreedProposer::new(1).with_species(handle);
        let fb = Feedback {
            best: Some((s1, 10.0)),
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let mut seen = HashSet::new();
        seen.insert(candidate_hash(s1));
        for _ in 0..12 {
            if let Proposal::Candidate(c) = breeder.next(&nugget, &fb).unwrap() {
                assert!(seen.insert(candidate_hash(&c)), "duplicate emitted: {c}");
            }
        }
        assert!(seen.len() >= 8);
    }
}
