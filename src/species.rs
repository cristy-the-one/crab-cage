//! Quality-diversity species archive for evolutionary code search.
//!
//! Each candidate is mapped to a structural `species_id` by normalizing its
//! Rust source without an AST parser: comments are stripped, string and char
//! literals become placeholders, numeric literals (including negative signs
//! in value position and numeric suffixes) become `#`, and local variable
//! identifiers are alpha-normalized (`v0`, `v1`, ...) while Rust keywords and
//! standard library names stay intact.
//!
//! Consequences:
//! - Constant tweaks of an algorithm keep the same `species_id`.
//! - Trivial variable renames by a model keep the same `species_id`.
//! - Distinct control flow, data structures, or operators form new species.
//!
//! `lust` evaluates every candidate cheaply. Only candidates that beat the
//! global best, beat their species' proven champion, or establish a competitive
//! new species (by overall score or by winning an individual test case) are
//! promoted to the `rustc` oracle. Only `proven` candidates become species
//! champions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::error::{err, Result};
use crate::nugget::{Nugget, Objective};
use crate::pipeline::{candidate_hash, Status, Verdict, WRONG_OUTPUT_SCORE};

/// Shared handle so the run loop updates the archive while proposers read from it.
pub type SpeciesHandle = Rc<RefCell<SpeciesArchive>>;

/// Maximum scored variants retained per species for parametric crossover / DE.
const MAX_VARIANTS_PER_SPECIES: usize = 6;

/// Default UCB1 exploration weight when selecting a parent species.
pub const DEFAULT_UCB_C: f64 = 0.85;

/// One scored or proven variant inside a species, kept for constant crossover.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesVariant {
    pub hash: String,
    pub text: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_scores: Vec<(String, f64)>,
    pub proven: bool,
}

/// A proven best-of-species champion and its evolutionary statistics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesEntry {
    pub species_id: String,
    pub hash: String,
    pub text: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_scores: Vec<(String, f64)>,
    pub proposer: String,
    pub evaluations: u64,
    pub selections: u64,
    pub improvements: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<SpeciesVariant>,
}

/// Read-only snapshot returned to proposers so borrows stay short.
#[derive(Clone, Debug)]
pub struct SpeciesSnapshot {
    pub species_id: String,
    pub hash: String,
    pub text: String,
    pub score: f64,
    pub case_scores: Vec<(String, f64)>,
    pub proposer: String,
    pub evaluations: u64,
    pub selections: u64,
    pub improvements: u64,
    pub variants: Vec<SpeciesVariant>,
}

impl From<&SpeciesEntry> for SpeciesSnapshot {
    fn from(entry: &SpeciesEntry) -> Self {
        Self {
            species_id: entry.species_id.clone(),
            hash: entry.hash.clone(),
            text: entry.text.clone(),
            score: entry.score,
            case_scores: entry.case_scores.clone(),
            proposer: entry.proposer.clone(),
            evaluations: entry.evaluations,
            selections: entry.selections,
            improvements: entry.improvements,
            variants: entry.variants.clone(),
        }
    }
}

/// Case-level advantage `(case_id, inspiration_score, primary_score)` where an
/// inspiration species strictly beats the primary parent.
#[derive(Clone, Debug, PartialEq)]
pub struct CaseAdvantage {
    pub case_id: String,
    pub inspiration_score: f64,
    pub primary_score: f64,
}

#[derive(Serialize, Deserialize)]
struct ManifestFile {
    max_species: usize,
    total_selections: u64,
    entries: Vec<SpeciesEntry>,
}

/// Bounded archive of proven species champions plus scored same-species variants.
#[derive(Clone, Debug)]
pub struct SpeciesArchive {
    max_species: usize,
    total_selections: u64,
    entries: Vec<SpeciesEntry>,
    /// Fast-pass evaluations per species_id, including species with no proven champion yet.
    seen_counts: HashMap<String, u64>,
    /// Scored (not yet proven) variants keyed by species_id so same-species DE works early.
    scored_variants: HashMap<String, Vec<SpeciesVariant>>,
}

impl Default for SpeciesArchive {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SpeciesArchive {
    pub fn new(max_species: usize) -> Self {
        Self {
            max_species: max_species.max(1),
            total_selections: 0,
            entries: Vec::new(),
            seen_counts: HashMap::new(),
            scored_variants: HashMap::new(),
        }
    }

    pub fn new_handle(max_species: usize) -> SpeciesHandle {
        Rc::new(RefCell::new(Self::new(max_species)))
    }

    pub fn max_species(&self) -> usize {
        self.max_species
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[SpeciesEntry] {
        &self.entries
    }

    pub fn best(&self) -> Option<&SpeciesEntry> {
        self.entries.first()
    }

    /// Number of distinct structural species observed across all evaluations.
    pub fn observed_species_count(&self) -> usize {
        self.seen_counts.len()
    }

    /// Whether a `Scored` fast-pass candidate merits full `rustc` oracle validation.
    ///
    /// With `max_species == 1`, this matches the single-best rule (`score < best`).
    /// With `max_species > 1`, a candidate also qualifies when it improves its own
    /// species' proven score, fills an open species slot, beats the weakest
    /// evictable species, or achieves a strict best-known score on any test case.
    pub fn worthy_of_oracle(
        &self,
        candidate: &str,
        verdict: &Verdict,
        case_scores: &[(String, f64)],
        objective: Objective,
    ) -> bool {
        if verdict.status != Status::Scored {
            return false;
        }
        let Some(score) = verdict.score.filter(|s| s.is_finite()) else {
            return false;
        };
        if let Some(best) = self.entries.first() {
            if score < best.score {
                return true;
            }
        } else {
            return true;
        }
        if self.max_species <= 1 {
            return false;
        }
        // Do not spend oracle builds on `fastest` candidates whose outputs are wrong
        // once at least one correct candidate is proven.
        if objective == Objective::Fastest
            && score >= WRONG_OUTPUT_SCORE
            && self.entries.iter().any(|e| e.score < WRONG_OUTPUT_SCORE)
        {
            return false;
        }
        // On `pass_all`, once a working species passes at least one case and we
        // have at least two species, skip 0-pass candidates.
        if objective == Objective::PassAll
            && verdict.passed == 0
            && self.entries.len() >= 2
            && self
                .entries
                .iter()
                .any(|e| e.score < f64::from(verdict.total))
        {
            return false;
        }

        let beats_some_case = self.strictly_beats_all_on_some_case(case_scores);
        // Reject wildly degraded mutations (e.g. 50x worse than the leader on every
        // case) from polluting open species slots or wasting `rustc` oracle builds.
        if !beats_some_case && !self.is_viable_score(score, objective) {
            return false;
        }

        let sp_id = species_id(candidate);
        if let Some(existing) = self.entries.iter().find(|e| e.species_id == sp_id) {
            if score < existing.score {
                return true;
            }
            return beats_some_case;
        }

        if self.entries.len() < self.max_species {
            return true;
        }
        if beats_some_case {
            return true;
        }
        if let Some(worst_score) = self.evictable_worst_score() {
            return score < worst_score;
        }
        false
    }

    /// Record a settled candidate. Only `Status::Proven` creates or upgrades a
    /// species champion; `Status::Scored` still contributes to within-species
    /// variant pools for parametric crossover and differential evolution.
    ///
    /// Returns `true` when the proven species archive changed.
    pub fn observe(
        &mut self,
        candidate: &str,
        verdict: &Verdict,
        case_scores: &[(String, f64)],
        proposer: &str,
    ) -> bool {
        let sp_id = species_id(candidate);
        let eval_count = {
            let count = self.seen_counts.entry(sp_id.clone()).or_insert(0);
            *count += 1;
            *count
        };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.species_id == sp_id) {
            entry.evaluations = eval_count;
        }

        let Some(score) = verdict.score.filter(|s| s.is_finite()) else {
            return false;
        };
        if !matches!(verdict.status, Status::Scored | Status::Proven) {
            return false;
        }

        let variant = SpeciesVariant {
            hash: verdict.hash.clone(),
            text: candidate.to_string(),
            score,
            case_scores: case_scores.to_vec(),
            proven: verdict.status == Status::Proven,
        };
        push_variant(
            self.scored_variants.entry(sp_id.clone()).or_default(),
            variant.clone(),
        );
        if let Some(entry) = self.entries.iter_mut().find(|e| e.species_id == sp_id) {
            push_variant(&mut entry.variants, variant);
        }

        if verdict.status != Status::Proven {
            return false;
        }

        if let Some(pos) = self.entries.iter().position(|e| e.species_id == sp_id) {
            let improves = score < self.entries[pos].score;
            let adds_case_win = !improves && self.strictly_beats_all_on_some_case(case_scores);
            if improves {
                self.entries[pos].hash = verdict.hash.clone();
                self.entries[pos].text = candidate.to_string();
                self.entries[pos].score = score;
                self.entries[pos].case_scores = case_scores.to_vec();
                self.entries[pos].proposer = proposer.to_string();
                self.entries[pos].improvements += 1;
                self.sort_entries();
                return true;
            }
            if adds_case_win && self.entries[pos].case_scores.is_empty() {
                self.entries[pos].case_scores = case_scores.to_vec();
                return true;
            }
            return false;
        }

        let pool = self
            .scored_variants
            .get(&sp_id)
            .cloned()
            .unwrap_or_default();
        let new_entry = SpeciesEntry {
            species_id: sp_id,
            hash: verdict.hash.clone(),
            text: candidate.to_string(),
            score,
            case_scores: case_scores.to_vec(),
            proposer: proposer.to_string(),
            evaluations: eval_count,
            selections: 0,
            improvements: 0,
            variants: pool,
        };

        let is_specialist = self.strictly_beats_all_on_some_case(case_scores);
        if !is_specialist && !self.is_viable_score(score, Objective::Minimize) {
            return false;
        }
        if self.entries.len() < self.max_species {
            self.entries.push(new_entry);
            self.sort_entries();
            return true;
        }

        if let Some(evict_idx) = self.find_eviction_index() {
            if is_specialist || score < self.entries[evict_idx].score {
                self.entries[evict_idx] = new_entry;
                self.sort_entries();
                return true;
            }
        }
        false
    }

    /// Select a parent species using UCB1: any unselected proven species goes
    /// first (in score order), then rank reward + `c * sqrt(ln(total) / n_i)`.
    pub fn select_parent(&mut self, exploration_c: f64) -> Option<SpeciesSnapshot> {
        if self.entries.is_empty() {
            return None;
        }
        if let Some(unvisited_idx) = self.entries.iter().position(|e| e.selections == 0) {
            self.total_selections += 1;
            self.entries[unvisited_idx].selections += 1;
            return Some(SpeciesSnapshot::from(&self.entries[unvisited_idx]));
        }
        let total = (self.total_selections.max(1)) as f64;
        let ln_total = total.ln().max(0.0);
        let mut best_idx = 0usize;
        let mut best_ucb = f64::NEG_INFINITY;
        for (rank, entry) in self.entries.iter().enumerate() {
            let exploit = 1.0 / (1.0 + rank as f64);
            let explore = exploration_c * (ln_total / (entry.selections as f64)).sqrt();
            let ucb = exploit + explore;
            if ucb > best_ucb {
                best_ucb = ucb;
                best_idx = rank;
            }
        }
        self.total_selections += 1;
        self.entries[best_idx].selections += 1;
        Some(SpeciesSnapshot::from(&self.entries[best_idx]))
    }

    /// Pick a secondary proven species distinct from `primary_species_id` for
    /// cross-species synthesis. Prefers a species that beats `primary_cases` on
    /// at least one test case; otherwise returns the highest-ranked other species.
    pub fn select_inspiration(
        &self,
        primary_species_id: &str,
        primary_cases: &[(String, f64)],
    ) -> Option<(SpeciesSnapshot, Vec<CaseAdvantage>)> {
        let others: Vec<&SpeciesEntry> = self
            .entries
            .iter()
            .filter(|e| e.species_id != primary_species_id)
            .collect();
        if others.is_empty() {
            return None;
        }
        let mut best_specialist: Option<(&SpeciesEntry, Vec<CaseAdvantage>)> = None;
        for entry in &others {
            let wins = case_advantages(&entry.case_scores, primary_cases);
            if !wins.is_empty() {
                let take = match &best_specialist {
                    Some((_, prev)) => wins.len() > prev.len(),
                    None => true,
                };
                if take {
                    best_specialist = Some((*entry, wins));
                }
            }
        }
        if let Some((entry, wins)) = best_specialist {
            return Some((SpeciesSnapshot::from(entry), wins));
        }
        Some((SpeciesSnapshot::from(others[0]), Vec::new()))
    }

    /// Variants recorded for `species_id`, best score first.
    pub fn variants_of(&self, species_id: &str) -> Vec<SpeciesVariant> {
        if let Some(entry) = self.entries.iter().find(|e| e.species_id == species_id) {
            if !entry.variants.is_empty() {
                return entry.variants.clone();
            }
        }
        self.scored_variants
            .get(species_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Persist `<out_dir>/species/manifest.json` and per-species source files.
    pub fn save_to_dir(&self, nugget: &Nugget, out_dir: &Path) -> Result<()> {
        if self.max_species <= 1 || self.entries.is_empty() {
            return Ok(());
        }
        let dir = out_dir.join("species");
        fs::create_dir_all(&dir)?;
        // Remove stale ranked files before writing the current ranking.
        if let Ok(existing) = fs::read_dir(&dir) {
            for item in existing.flatten() {
                let name = item.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".rs") {
                    let _ = fs::remove_file(item.path());
                }
            }
        }
        for (idx, entry) in self.entries.iter().enumerate() {
            let rank = idx + 1;
            let stem = format!("{rank:02}-{}", entry.species_id);
            fs::write(dir.join(format!("{stem}.region.rs")), &entry.text)?;
            fs::write(dir.join(format!("{stem}.rs")), nugget.splice(&entry.text))?;
        }
        let manifest = ManifestFile {
            max_species: self.max_species,
            total_selections: self.total_selections,
            entries: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(dir.join("manifest.json"), format!("{json}\n"))?;
        Ok(())
    }

    /// Restore a saved archive from `<out_dir>/species/manifest.json` if present,
    /// seeding from `best_text` / `best_score` when resuming a legacy run.
    pub fn load_or_init(
        out_dir: &Path,
        max_species: usize,
        restored_best: Option<(&str, f64)>,
    ) -> Result<Self> {
        let manifest_path = out_dir.join("species").join("manifest.json");
        if manifest_path.is_file() {
            let raw = fs::read_to_string(&manifest_path)
                .map_err(|e| err(format!("{}: {e}", manifest_path.display())))?;
            let parsed: ManifestFile = serde_json::from_str(&raw)
                .map_err(|e| err(format!("{}: {e}", manifest_path.display())))?;
            let cap = max_species.max(1);
            let mut entries = parsed.entries;
            entries.sort_by(|a, b| a.score.total_cmp(&b.score));
            entries.truncate(cap);
            let mut seen_counts = HashMap::new();
            let mut scored_variants = HashMap::new();
            for entry in &entries {
                seen_counts.insert(entry.species_id.clone(), entry.evaluations.max(1));
                scored_variants.insert(entry.species_id.clone(), entry.variants.clone());
            }
            return Ok(Self {
                max_species: cap,
                total_selections: parsed.total_selections,
                entries,
                seen_counts,
                scored_variants,
            });
        }

        let mut archive = Self::new(max_species);
        if let Some((text, score)) = restored_best {
            let sp_id = species_id(text);
            let hash = candidate_hash(text);
            let variant = SpeciesVariant {
                hash: hash.clone(),
                text: text.to_string(),
                score,
                case_scores: Vec::new(),
                proven: true,
            };
            archive.seen_counts.insert(sp_id.clone(), 1);
            archive
                .scored_variants
                .insert(sp_id.clone(), vec![variant.clone()]);
            archive.entries.push(SpeciesEntry {
                species_id: sp_id,
                hash,
                text: text.to_string(),
                score,
                case_scores: Vec::new(),
                proposer: "restored".to_string(),
                evaluations: 1,
                selections: 0,
                improvements: 0,
                variants: vec![variant],
            });
        }
        Ok(archive)
    }

    fn sort_entries(&mut self) {
        self.entries.sort_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then_with(|| a.species_id.cmp(&b.species_id))
        });
    }

    /// True if `case_scores` strictly beats every archive entry on at least one case.
    fn strictly_beats_all_on_some_case(&self, case_scores: &[(String, f64)]) -> bool {
        if case_scores.is_empty() || self.entries.is_empty() {
            return false;
        }
        for (case_id, val) in case_scores {
            if !val.is_finite() {
                continue;
            }
            let mut compared = false;
            let mut beats_all = true;
            for entry in &self.entries {
                if let Some((_, other)) = entry.case_scores.iter().find(|(id, _)| id == case_id) {
                    compared = true;
                    if *val >= *other - 1e-9 {
                        beats_all = false;
                        break;
                    }
                }
            }
            if compared && beats_all {
                return true;
            }
        }
        false
    }

    /// Whether `entry_idx` is the sole holder of the best score on any case.
    fn is_unique_case_champion(&self, entry_idx: usize) -> bool {
        let target = &self.entries[entry_idx];
        for (case_id, val) in &target.case_scores {
            if !val.is_finite() {
                continue;
            }
            let mut compared = false;
            let mut sole_best = true;
            for (idx, other_entry) in self.entries.iter().enumerate() {
                if idx == entry_idx {
                    continue;
                }
                if let Some((_, other)) =
                    other_entry.case_scores.iter().find(|(id, _)| id == case_id)
                {
                    compared = true;
                    if *other <= *val + 1e-9 {
                        sole_best = false;
                        break;
                    }
                }
            }
            if compared && sole_best {
                return true;
            }
        }
        false
    }

    /// Index of the worst evictable species (never index 0, preferring non-specialists).
    fn find_eviction_index(&self) -> Option<usize> {
        if self.entries.len() <= 1 {
            return None;
        }
        for idx in (1..self.entries.len()).rev() {
            if !self.is_unique_case_champion(idx) {
                return Some(idx);
            }
        }
        Some(self.entries.len() - 1)
    }

    fn evictable_worst_score(&self) -> Option<f64> {
        self.find_eviction_index()
            .map(|idx| self.entries[idx].score)
    }

    /// A new species without any single-case win must still be within a sane
    /// factor of the current global leader (2.5x + 10.0) so degenerate mutants
    /// do not consume `rustc` oracle builds or UCB exploration turns.
    fn is_viable_score(&self, score: f64, objective: Objective) -> bool {
        let Some(best) = self.entries.first() else {
            return true;
        };
        if objective == Objective::PassAll {
            return true;
        }
        if best.score >= 0.0 {
            score <= best.score * 2.5 + 10.0
        } else {
            score <= best.score * 0.4 + 10.0
        }
    }
}

fn push_variant(pool: &mut Vec<SpeciesVariant>, variant: SpeciesVariant) {
    if let Some(existing) = pool.iter_mut().find(|v| v.hash == variant.hash) {
        if variant.proven {
            existing.proven = true;
        }
        if variant.score < existing.score {
            existing.score = variant.score;
        }
        if existing.case_scores.is_empty() && !variant.case_scores.is_empty() {
            existing.case_scores = variant.case_scores;
        }
        return;
    }
    pool.push(variant);
    pool.sort_by(|a, b| a.score.total_cmp(&b.score));
    pool.truncate(MAX_VARIANTS_PER_SPECIES);
}

fn case_advantages(inspiration: &[(String, f64)], primary: &[(String, f64)]) -> Vec<CaseAdvantage> {
    let mut wins = Vec::new();
    for (case_id, insp_val) in inspiration {
        if !insp_val.is_finite() {
            continue;
        }
        if let Some((_, prim_val)) = primary.iter().find(|(id, _)| id == case_id) {
            if *prim_val > *insp_val + 1e-9 {
                wins.push(CaseAdvantage {
                    case_id: case_id.clone(),
                    inspiration_score: *insp_val,
                    primary_score: *prim_val,
                });
            }
        }
    }
    wins.sort_by(|a, b| {
        (b.primary_score - b.inspiration_score).total_cmp(&(a.primary_score - a.inspiration_score))
    });
    wins
}

/// Read `<out_dir>/species/manifest.json` as raw JSON for `crab-cage summary --json`.
pub fn read_manifest_json(out_dir: &Path) -> Option<serde_json::Value> {
    let path = out_dir.join("species").join("manifest.json");
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let entries = value.get("entries")?.as_array()?;
    let summary_entries: Vec<serde_json::Value> = entries
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            serde_json::json!({
                "rank": idx + 1,
                "species_id": e.get("species_id"),
                "hash": e.get("hash"),
                "score": e.get("score"),
                "proposer": e.get("proposer"),
                "evaluations": e.get("evaluations"),
                "improvements": e.get("improvements"),
            })
        })
        .collect();
    Some(serde_json::json!({
        "max_species": value.get("max_species"),
        "count": summary_entries.len(),
        "champions": summary_entries,
    }))
}

/// Compute the 12-hex-digit structural species identifier for `candidate`.
pub fn species_id(candidate: &str) -> String {
    let skel = skeleton(candidate);
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in skel.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:012x}", hash & 0x0000_ffff_ffff_ffff)
}

/// Normalize Rust source into a structural token stream without an AST parser.
pub fn skeleton(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut ident_map: HashMap<String, usize> = HashMap::new();
    let mut i = 0usize;
    let mut last_sig: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        // Whitespace
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Nested block comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
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
            continue;
        }
        // Raw string literal r#"..."# or r"..."
        if b == b'r' && i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            if let Some(end) = skip_raw_string(bytes, i) {
                push_token(&mut out, "\"$\"");
                last_sig = Some(b'"');
                i = end;
                continue;
            }
        }
        // Normal string literal
        if b == b'"' {
            i = skip_quoted(bytes, i, b'"');
            push_token(&mut out, "\"$\"");
            last_sig = Some(b'"');
            continue;
        }
        // Char literal vs lifetime ('a' vs 'a)
        if b == b'\'' {
            if let Some(end) = skip_char_literal(source, i) {
                push_token(&mut out, "'$'");
                last_sig = Some(b'\'');
                i = end;
                continue;
            }
        }
        // Signed numeric literal in value position (`= -5`, `(-0.2)`, `, -1`)
        if b == b'-'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && is_unary_value_pos(last_sig)
        {
            i = skip_number(bytes, i + 1);
            push_token(&mut out, "#");
            last_sig = Some(b'#');
            continue;
        }
        // Unsigned numeric literal
        if b.is_ascii_digit() {
            i = skip_number(bytes, i);
            push_token(&mut out, "#");
            last_sig = Some(b'#');
            continue;
        }
        // Identifier or keyword
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &source[start..i];
            // Method call `.foo` or path segment `::foo` keeps its name.
            let after_dot_or_colon = matches!(last_sig, Some(b'.') | Some(b':'));
            if after_dot_or_colon || is_preserved_word(word) {
                push_token(&mut out, word);
            } else {
                let next_id = ident_map.len();
                let id = *ident_map.entry(word.to_string()).or_insert(next_id);
                push_token(&mut out, &format!("v{id}"));
            }
            last_sig = Some(if word == "return" { b'(' } else { b'a' });
            continue;
        }
        // Multi-char punctuation vs single-char punctuation
        if let Some(ch) = source[i..].chars().next() {
            let len = ch.len_utf8();
            if len == 1 {
                out.push(ch);
                last_sig = Some(b);
            } else {
                push_token(&mut out, &source[i..i + len]);
                last_sig = Some(b'?');
            }
            i += len;
        } else {
            break;
        }
    }
    out
}

fn push_token(out: &mut String, tok: &str) {
    if let (Some(prev), Some(first)) = (out.bytes().last(), tok.bytes().next()) {
        if (is_ident_continue(prev) || prev == b'#') && (is_ident_continue(first) || first == b'#')
        {
            out.push(' ');
        }
    }
    out.push_str(tok);
}

fn is_unary_value_pos(last: Option<u8>) -> bool {
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

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut hashes = 0usize;
    while i < bytes.len() && bytes[i] == b'#' {
        hashes += 1;
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'"' {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"'
            && i + hashes < bytes.len()
            && bytes[i + 1..i + 1 + hashes].iter().all(|&b| b == b'#')
        {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    Some(bytes.len())
}

fn skip_char_literal(source: &str, start: usize) -> Option<usize> {
    let rest = &source[start + 1..];
    if rest.starts_with('\\') {
        let mut chars = rest.char_indices();
        let _ = chars.next()?;
        let _ = chars.next()?;
        let (pos, ch) = chars.next()?;
        if ch == '\'' {
            return Some(start + 1 + pos + 1);
        }
        return None;
    }
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' {
        return None;
    }
    let (pos, second) = chars.next()?;
    if second == '\'' {
        Some(start + 1 + pos + 1)
    } else {
        None
    }
}

fn skip_number(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    // Hex, binary, octal prefix
    if i + 1 < bytes.len()
        && bytes[i] == b'0'
        && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        i += 2;
        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
    } else {
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
    }
    // Optional numeric type suffix (`usize`, `u64`, `i32`, `f64`, ...)
    if i < bytes.len() && matches!(bytes[i], b'u' | b'i' | b'f') {
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
        }
        let suffix = &bytes[i..j];
        if matches!(
            suffix,
            b"u8"
                | b"u16"
                | b"u32"
                | b"u64"
                | b"u128"
                | b"usize"
                | b"i8"
                | b"i16"
                | b"i32"
                | b"i64"
                | b"i128"
                | b"isize"
                | b"f32"
                | b"f64"
        ) {
            i = j;
        }
    }
    i
}

fn is_preserved_word(word: &str) -> bool {
    matches!(
        word,
        "as" | "break"
            | "const"
            | "continue"
            | "else"
            | "enum"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "use"
            | "where"
            | "while"
            | "bool"
            | "char"
            | "str"
            | "u8"
            | "u16"
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
            | "Vec"
            | "String"
            | "Option"
            | "Some"
            | "None"
            | "Result"
            | "Ok"
            | "Err"
            | "HashMap"
            | "HashSet"
            | "BTreeMap"
            | "BTreeSet"
            | "VecDeque"
            | "BinaryHeap"
            | "std"
            | "cmp"
            | "mem"
            | "iter"
            | "vec"
            | "format"
            | "print"
            | "println"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{Gate, Ms};

    fn make_verdict(text: &str, status: Status, score: f64, passed: u32, total: u32) -> Verdict {
        Verdict {
            hash: candidate_hash(text),
            gate: if status == Status::Proven {
                Gate::Oracle
            } else {
                Gate::Run
            },
            status,
            score: Some(score),
            passed,
            total,
            diagnostics: Vec::new(),
            ms: Ms::default(),
            note: String::new(),
            failure: None,
        }
    }

    #[test]
    fn constant_tweaks_and_alpha_renames_share_species_while_structures_differ() {
        let a = "    let mut acc = 0;\n    if rem < 15 { acc += 1; }\n    acc\n";
        let b = "    // comment\n    let mut total = -4;\n    if rem < 30usize { total += 2; }\n    total\n";
        let c = "    let mut acc = 0;\n    if rem <= 15 { acc += 1; }\n    acc\n";
        let d = "    xs.iter().map(|x| x * x).sum()\n";

        assert_eq!(species_id(a), species_id(b));
        assert_ne!(species_id(a), species_id(c));
        assert_ne!(species_id(a), species_id(d));
    }

    #[test]
    fn archive_promotes_new_species_and_protects_case_specialists() {
        let mut archive = SpeciesArchive::new(2);
        let sp_a = "    if x < 10 { 1 } else { 2 }\n";
        let sp_b = "    while x > 0 { x -= 1; }\n    x\n";
        let sp_c = "    match x { 0 => 1, _ => 2 }\n";

        let va = make_verdict(sp_a, Status::Proven, 100.0, 2, 2);
        let cases_a = vec![("c1".to_string(), 30.0), ("c2".to_string(), 70.0)];
        assert!(archive.observe(sp_a, &va, &cases_a, "seed"));

        // Second distinct species with worse total score (110 vs 100) still qualifies
        // for oracle because max_species == 2 has an open slot, and wins case c2!
        let vb_scored = make_verdict(sp_b, Status::Scored, 110.0, 2, 2);
        let cases_b = vec![("c1".to_string(), 80.0), ("c2".to_string(), 30.0)];
        assert!(archive.worthy_of_oracle(sp_b, &vb_scored, &cases_b, Objective::Minimize));
        let vb_proven = make_verdict(sp_b, Status::Proven, 110.0, 2, 2);
        assert!(archive.observe(sp_b, &vb_proven, &cases_b, "local"));
        assert_eq!(archive.len(), 2);

        // Inspiration query from primary sp_a finds sp_b and reports its win on c2.
        let (insp, wins) = archive
            .select_inspiration(&species_id(sp_a), &cases_a)
            .expect("inspiration species");
        assert_eq!(insp.species_id, species_id(sp_b));
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0].case_id, "c2");
        assert_eq!(wins[0].inspiration_score, 30.0);
        assert_eq!(wins[0].primary_score, 70.0);

        // A worse third species (score 120, beats nobody on any case) does NOT
        // trigger an oracle build.
        let vc_worse = make_verdict(sp_c, Status::Scored, 120.0, 2, 2);
        let cases_c_worse = vec![("c1".to_string(), 60.0), ("c2".to_string(), 60.0)];
        assert!(!archive.worthy_of_oracle(sp_c, &vc_worse, &cases_c_worse, Objective::Minimize));

        // Same species as sp_b with a better score (105 < 110) DOES trigger oracle!
        let sp_b_tuned = "    while x > 2 { x -= 1; }\n    x\n";
        assert_eq!(species_id(sp_b), species_id(sp_b_tuned));
        let vb_tuned = make_verdict(sp_b_tuned, Status::Scored, 105.0, 2, 2);
        assert!(archive.worthy_of_oracle(sp_b_tuned, &vb_tuned, &cases_b, Objective::Minimize));
    }

    #[test]
    fn ucb_selection_explores_unvisited_species_before_repeating() {
        let mut archive = SpeciesArchive::new(3);
        let s1 = "    if a < 10 { 1 } else { 2 }\n";
        let s2 = "    for i in 0..10 { sum += i; }\n";
        archive.observe(
            s1,
            &make_verdict(s1, Status::Proven, 10.0, 1, 1),
            &[],
            "seed",
        );
        archive.observe(
            s2,
            &make_verdict(s2, Status::Proven, 20.0, 1, 1),
            &[],
            "local",
        );

        let p1 = archive.select_parent(DEFAULT_UCB_C).unwrap();
        let p2 = archive.select_parent(DEFAULT_UCB_C).unwrap();
        let p3 = archive.select_parent(DEFAULT_UCB_C).unwrap();
        // Best goes first, then the unexplored second species gets a turn!
        assert_eq!(p1.species_id, species_id(s1));
        assert!(
            p2.species_id == species_id(s2) || p3.species_id == species_id(s2),
            "second species must be explored by UCB"
        );
    }
}
