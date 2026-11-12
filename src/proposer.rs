//! Where the next candidate text comes from.
//!
//! [`FolderProposer`] reads each `*.rs` file in a directory once, in file-name
//! order. It lists the directory again on every call, so a file that appears
//! while a run is in progress is still offered.
//!
//! [`LiteralProposer`] rewrites one numeric literal of the best candidate, or
//! of the nugget seed when the run has no best text yet.
//!
//! [`ModelProposer`] asks a loopback model for the next region. The prompt is
//! the template with the current best region in place, the verdicts this
//! proposer itself produced (found in [`Feedback::recent`]), and the last
//! eight regions it already tried.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{err, Result};
use crate::genome::rust_f64;
use crate::llm;
use crate::nugget::{Nugget, Objective};
use crate::pipeline::{
    candidate_hash, clip_chars, CaseFailure, Diagnostic, Status, Verdict, TEXT_CLIP,
};
use crate::rng::Pcg64;

/// PCG stream for [`LiteralProposer`]. A different stream keeps this sequence
/// from matching a generator that was given the same seed and stream 0.
const TWEAK_STREAM: u64 = 16;

/// What the run loop knows when it asks for another candidate.
pub struct Feedback<'a> {
    pub best: Option<(&'a str, f64)>,
    pub last: Option<(&'a str, &'a Verdict)>,
    /// Settled candidates, oldest first. The run keeps the last 32.
    /// [`ModelProposer`] finds the verdicts it produced by hashing the text
    /// with [`candidate_hash`], so a verdict that is not `last` still counts.
    pub recent: &'a [(&'a str, &'a Verdict)],
    /// The verdict of [`Feedback::best`]'s text, when the run has it. A best
    /// restored from an earlier run's journal has none.
    pub best_verdict: Option<&'a Verdict>,
}

/// What a [`Proposer`] has for the batch being gathered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    /// A candidate region.
    Candidate(String),
    /// Nothing more until the candidates already proposed have verdicts.
    /// The run stops asking this proposer for the current batch and asks
    /// again after the batch settles.
    Later,
    /// Nothing more for the rest of the run.
    Finished,
}

impl Proposal {
    /// The candidate text, or None for [`Proposal::Later`] and
    /// [`Proposal::Finished`].
    pub fn into_text(self) -> Option<String> {
        match self {
            Proposal::Candidate(text) => Some(text),
            Proposal::Later | Proposal::Finished => None,
        }
    }
}

/// A source of candidate region text.
pub trait Proposer {
    fn name(&self) -> &str;
    /// The next candidate text; [`Proposal::Later`] when nothing more can come
    /// before the pending candidates settle; [`Proposal::Finished`] when this
    /// proposer has nothing more for the run.
    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal>;
}

/// Yields the text of each top-level `*.rs` file in `dir` once, in name order.
pub struct FolderProposer {
    dir: PathBuf,
    yielded: HashSet<String>,
}

impl FolderProposer {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            yielded: HashSet::new(),
        }
    }
}

impl Proposer for FolderProposer {
    fn name(&self) -> &str {
        "folder"
    }

    fn next(&mut self, _nugget: &Nugget, _feedback: &Feedback) -> Result<Proposal> {
        let mut names = Vec::new();
        let entries = fs::read_dir(&self.dir)
            .map_err(|error| err(format!("{}: {error}", self.dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|error| err(format!("{}: {error}", self.dir.display())))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err(err(format!("{}: file name is not utf-8", path.display())));
            };
            if !is_rust_file(name) || self.yielded.contains(name) {
                continue;
            }
            names.push(name.to_string());
        }
        names.sort();
        let Some(name) = names.into_iter().next() else {
            return Ok(Proposal::Finished);
        };
        let path = self.dir.join(&name);
        let text = read_candidate(&path)?;
        self.yielded.insert(name);
        Ok(Proposal::Candidate(text))
    }
}

/// `*.rs`, but not a name that is only the extension.
fn is_rust_file(name: &str) -> bool {
    name.len() > ".rs".len() && name.ends_with(".rs")
}

fn read_candidate(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).map_err(|error| err(format!("{}: {error}", path.display())))?;
    Ok(text.replace("\r\n", "\n"))
}

/// Rewrites one numeric literal per call. The base text is the best candidate,
/// or the nugget seed when [`Feedback::best`] has no text.
///
/// The scan is a digit run, then an optional `.` and more digits, then an
/// optional exponent. A digit run that follows a letter, a digit, `_`, or `.`
/// is not a literal. Digits inside `"…"` and after `//` are not either. A
/// float is scaled by `1 + gaussian * 0.2`, except a zero (or any value
/// whose product prints back unchanged), which takes an additive step of
/// `gaussian * 0.1`. No literal means this proposer is finished.
pub struct LiteralProposer {
    rng: Pcg64,
}

impl LiteralProposer {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Pcg64::new(seed, TWEAK_STREAM),
        }
    }
}

impl Proposer for LiteralProposer {
    fn name(&self) -> &str {
        "tweak"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
        let text = match feedback.best {
            Some((best, _)) => best,
            None => nugget.seed.as_str(),
        };
        let literals = find_literals(text);
        if literals.is_empty() {
            return Ok(Proposal::Finished);
        }
        let index = self.rng.gen_index(literals.len());
        let Literal { start, end, kind } = literals[index];
        let replacement = match kind {
            LiteralKind::Int(value) => tweak_int(&mut self.rng, value),
            LiteralKind::Float(value) => tweak_float(&mut self.rng, value, &text[start..end]),
        };
        Ok(Proposal::Candidate(replace_span(
            text,
            start,
            end,
            &replacement,
        )))
    }
}

#[derive(Clone, Copy)]
struct Literal {
    start: usize,
    end: usize,
    kind: LiteralKind,
}

#[derive(Clone, Copy)]
enum LiteralKind {
    Int(u128),
    Float(f64),
}

fn find_literals(text: &str) -> Vec<Literal> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if ch == '"' {
            i = skip_string(text, i);
            continue;
        }
        if ch == '/' && text[i..].starts_with("//") {
            i = skip_line_comment(text, i);
            continue;
        }
        if ch.is_ascii_digit() && !preceded_by_word(text, i) {
            let end = end_of_literal(text, i);
            if let Some(kind) = classify(&text[i..end]) {
                found.push(Literal {
                    start: i,
                    end,
                    kind,
                });
            }
            i = end;
            continue;
        }
        i += ch.len_utf8();
    }
    found
}

/// A letter, a digit, `_`, or `.` glues this digit run to the previous token.
fn preceded_by_word(text: &str, start: usize) -> bool {
    let Some(prev) = text[..start].chars().next_back() else {
        return false;
    };
    prev.is_alphabetic() || prev.is_ascii_digit() || prev == '_' || prev == '.'
}

fn skip_string(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                let Some(ch) = text[i..].chars().next() else {
                    break;
                };
                i += ch.len_utf8();
            }
            b'"' => return i + 1,
            b'\n' => return i,
            _ => {
                let Some(ch) = text[i..].chars().next() else {
                    break;
                };
                i += ch.len_utf8();
            }
        }
    }
    text.len()
}

fn skip_line_comment(text: &str, start: usize) -> usize {
    match text[start..].find('\n') {
        Some(offset) => start + offset + 1,
        None => text.len(),
    }
}

/// `digits ('.' digits)? ([eE][+-]? digits)?`. A trailing `.` with no digits,
/// or an `e` with no digits, stays outside the literal.
fn end_of_literal(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        let digits = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits {
            i = j;
        }
    }
    i
}

fn classify(token: &str) -> Option<LiteralKind> {
    if token
        .bytes()
        .any(|byte| byte == b'.' || byte == b'e' || byte == b'E')
    {
        token.parse::<f64>().ok().map(LiteralKind::Float)
    } else {
        token.parse::<u128>().ok().map(LiteralKind::Int)
    }
}

fn tweak_int(rng: &mut Pcg64, value: u128) -> String {
    let choices = integer_choices(value);
    if choices.is_empty() {
        return value.to_string();
    }
    choices[rng.gen_index(choices.len())].to_string()
}

/// `value + d` for `d` in −2, −1, +1, +2, dropping a step that would pass
/// below 0 or overflow `u128`.
fn integer_choices(value: u128) -> Vec<u128> {
    let mut choices = Vec::with_capacity(4);
    for delta in [-2, -1, 1, 2] {
        if let Some(next) = apply_delta(value, delta) {
            choices.push(next);
        }
    }
    choices
}

fn apply_delta(value: u128, delta: i32) -> Option<u128> {
    if delta >= 0 {
        value.checked_add(u128::from(delta as u32))
    } else {
        value.checked_sub(u128::from((-delta) as u32))
    }
}

/// Multiply by `1 + gaussian * 0.2`. When that product is the same number, or
/// it prints as the original token, add `gaussian * 0.1` instead. A literal
/// `0.0` would otherwise stay `0.0`.
fn tweak_float(rng: &mut Pcg64, value: f64, original: &str) -> String {
    let gauss = rng.gauss();
    let scaled = value * (1.0 + gauss * 0.2);
    let printed = rust_f64(scaled);
    if printed != original && scaled != value {
        return printed;
    }
    rust_f64(value + gauss * 0.1)
}

fn replace_span(text: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len() - (end - start) + replacement.len());
    out.push_str(&text[..start]);
    out.push_str(replacement);
    out.push_str(&text[end..]);
    out
}

const MODEL_TIMEOUT: Duration = Duration::from_secs(180);
/// Rejected verdicts in a row before the next prompt is based on the seed.
const REJECTS_BEFORE_RESET: usize = 3;
/// Endpoint failures or unusable replies in a row before this proposer stops.
const PROPOSAL_ERROR_LIMIT: usize = 3;
/// Regions with a known outcome kept for the "already tried" list.
const TRIED_LIMIT: usize = 8;
/// The retries after a reply repeats a region in that list: each one hotter
/// than the last. In the model ladder of 2026-11-12 every unsolved run ended
/// on repeats with most of its budget left, one of them after going from 8
/// failing cases to 2 in four candidates.
const REPEAT_TEMPERATURES: [f64; 3] = [0.8, 1.0, 1.2];
/// Added after the request when the reply before was a repeat.
const REPEAT_NOTE: &str = "Your last reply was the same as lines already tried. Reply with lines that differ from them.\n";
/// Rejections in a row with the same first lust code before rustc is asked
/// what it says about the same program.
const SAME_CODE_BEFORE_RUSTC: usize = 2;
const SECOND_OPINION_TIMEOUT: Duration = Duration::from_secs(60);

/// Given the whole program with a rejected region spliced in, the lines to
/// add to the prompt about it, or None.
pub type SecondOpinion = Box<dyn Fn(&str) -> Option<String>>;
const SYSTEM_PREFIX: &str = "\
You rewrite one marked region of a Rust program. Your reply replaces every line between the two markers, so it must be complete Rust statements or an expression that compile in that place. Reply with the replacement lines only: no markers, no markdown fence, no explanation, and never a fragment that continues the old code. Stay inside this subset of Rust:";

/// Asks a loopback chat model for the next region.
///
/// The wording of the prompt was settled against a real 7B coding model
/// (2026-11-11). Asked fresh, it wrote the whole region with either wording.
/// After one rejected fragment (`.map(|x| x * x)`, a continuation of the old
/// line), the earlier wording ("write the lines that go between the
/// markers") got a fragment back ten times out of ten; saying that the reply
/// *replaces* the lines, showing the lines, and ruling out a fragment got
/// the whole region ten times out of ten. The order matters as much: the
/// request has to be the last thing in the prompt, after the feedback.
///
/// The capability summary is fixed at construction (empty when lust is
/// absent). A reply that still contains a region marker, or an empty reply,
/// or a chat error, is not returned to the run. Three of those in a row
/// finish the proposer ([`Proposal::Finished`]) when no region it proposed
/// is still awaiting a verdict; with one awaiting, `next` returns
/// [`Proposal::Later`] instead, because the next prompt will carry that
/// verdict. `run::execute` treats `Err` from [`Proposer::next`] as fatal, so
/// the failure stays here and is printed when the proposer finishes.
///
/// A reply whose [`candidate_hash`] matches a region already proposed — one of
/// the last [`TRIED_LIMIT`] with an outcome, or one still awaiting its
/// verdict — is not returned. The prompt is asked again with
/// [`REPEAT_NOTE`] after it, at each of [`REPEAT_TEMPERATURES`] in turn (only
/// the first while a region is awaiting its verdict). A repeat after the
/// last of them counts as a proposal error, and when a region is awaiting
/// its verdict it also ends this batch with [`Proposal::Later`]: with
/// `--jobs` above 1 the model cannot write a second, different region from
/// the same prompt.
///
/// When lust rejects two of this proposer's regions in a row with the same
/// first code, the prompt also carries what rustc says about the last one
/// (see [`rustc_second_opinion`]). lust's message can point at the wrong
/// line: for a field read on a `Vec::new()` whose element type is not known
/// it says "no field", where rustc says "type annotations needed".
///
/// A region still awaiting a verdict after one [`Proposal::Later`] was never
/// evaluated (the journal already had it, or its evaluation failed). It
/// moves to the "already tried" list, so it does not hold the proposer in
/// `Later` for the rest of the run.
pub struct ModelProposer {
    endpoint: String,
    model: String,
    capabilities: String,
    timeout: Duration,
    memory: ModelMemory,
    second_opinion: Option<SecondOpinion>,
}

impl ModelProposer {
    /// Ask `opinion` about a region after repeated rejections with the same
    /// lust code.
    pub fn with_second_opinion(mut self, opinion: SecondOpinion) -> Self {
        self.second_opinion = Some(opinion);
        self
    }

    pub fn new(
        endpoint: impl AsRef<str>,
        model: impl Into<String>,
        capabilities: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            endpoint: llm::normalize_endpoint(endpoint)?,
            model: model.into(),
            capabilities: capabilities.into(),
            timeout: MODEL_TIMEOUT,
            memory: ModelMemory::default(),
            second_opinion: None,
        })
    }
}

impl Proposer for ModelProposer {
    fn name(&self) -> &str {
        "local"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
        if self.memory.stopped.is_some() {
            return Ok(Proposal::Finished);
        }
        let (system, user) = prepare_prompt(
            &mut self.memory,
            &self.capabilities,
            nugget,
            feedback,
            self.second_opinion.as_ref(),
        );
        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let timeout = self.timeout;
        loop {
            let temperature = self.memory.next_temperature;
            let asked = self.memory.prompt_now(&user);
            let raw = llm::chat(&endpoint, &model, &system, &asked, timeout, temperature);
            match self.memory.consume(raw) {
                Step::Return(text) => return Ok(Proposal::Candidate(text)),
                Step::Later => return Ok(Proposal::Later),
                Step::Stop => return Ok(Proposal::Finished),
                Step::AskAgain => {}
            }
        }
    }
}

/// Test stand-in. Each call takes the next scripted reply. No network.
pub struct MockModelProposer {
    replies: Vec<String>,
    capabilities: String,
    memory: ModelMemory,
    prompts: Vec<Asked>,
    /// Temperature of each scripted reply. Read by the unit tests.
    #[allow(dead_code)]
    asked_temperatures: Vec<f64>,
    /// The user prompt of each scripted reply. Read by the unit tests.
    #[allow(dead_code)]
    asked_users: Vec<String>,
    second_opinion: Option<SecondOpinion>,
}

struct Asked {
    // Read by the unit tests. The library build only records the prompt.
    #[allow(dead_code)]
    system: String,
    #[allow(dead_code)]
    user: String,
}

impl MockModelProposer {
    pub fn new(replies: Vec<String>, capabilities: impl Into<String>) -> Self {
        Self {
            replies,
            capabilities: capabilities.into(),
            memory: ModelMemory::default(),
            prompts: Vec::new(),
            asked_temperatures: Vec::new(),
            asked_users: Vec::new(),
            second_opinion: None,
        }
    }

    pub fn with_second_opinion(mut self, opinion: SecondOpinion) -> Self {
        self.second_opinion = Some(opinion);
        self
    }
}

impl Proposer for MockModelProposer {
    fn name(&self) -> &str {
        "mock"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
        if self.memory.stopped.is_some() || self.replies.is_empty() {
            return Ok(Proposal::Finished);
        }
        let (system, user) = prepare_prompt(
            &mut self.memory,
            &self.capabilities,
            nugget,
            feedback,
            self.second_opinion.as_ref(),
        );
        self.prompts.push(Asked {
            system,
            user: user.clone(),
        });
        loop {
            if self.replies.is_empty() {
                return Ok(Proposal::Finished);
            }
            let temperature = self.memory.next_temperature;
            self.asked_temperatures.push(temperature);
            self.asked_users.push(self.memory.prompt_now(&user));
            let raw = Ok(self.replies.remove(0));
            match self.memory.consume(raw) {
                Step::Return(text) => return Ok(Proposal::Candidate(text)),
                Step::Later => return Ok(Proposal::Later),
                Step::Stop => return Ok(Proposal::Finished),
                Step::AskAgain => {}
            }
        }
    }
}

struct ModelMemory {
    rejected_in_a_row: usize,
    errors_in_a_row: usize,
    /// Regions returned and not yet matched to a settled verdict.
    pending: Vec<Pending>,
    /// Last [`TRIED_LIMIT`] regions that have an outcome, oldest first.
    tried: Vec<Tried>,
    own: Option<Verdict>,
    stopped: Option<String>,
    /// Temperature for the next reply.
    next_temperature: f64,
    /// Repeats since the last reply that was returned or counted as an
    /// error: how many of [`REPEAT_TEMPERATURES`] have been used.
    repeats: usize,
    /// The region of `own`.
    own_text: Option<String>,
    /// First lust code of the last rejection, and how many rejections in a
    /// row began with it.
    last_code: Option<String>,
    same_code: usize,
    /// The second opinion on the region with this hash, kept so that a batch
    /// of prompts asks once.
    opinion: Option<(String, Option<String>)>,
}

struct Tried {
    hash: String,
    text: String,
    outcome: String,
}

struct Pending {
    /// [`candidate_hash`] of `text`.
    hash: String,
    text: String,
    /// A [`Proposal::Later`] was returned while this region was pending, so
    /// its batch has settled since.
    waited: bool,
}

/// Outcome shown for a pending region that got no verdict in its batch.
const NOT_EVALUATED: &str = "not evaluated: already in the journal, or its evaluation failed";

impl Default for ModelMemory {
    fn default() -> Self {
        Self {
            rejected_in_a_row: 0,
            errors_in_a_row: 0,
            pending: Vec::new(),
            tried: Vec::new(),
            own: None,
            stopped: None,
            next_temperature: llm::DEFAULT_TEMPERATURE,
            repeats: 0,
            own_text: None,
            last_code: None,
            same_code: 0,
            opinion: None,
        }
    }
}

impl ModelMemory {
    fn observe(&mut self, feedback: &Feedback) {
        for &(text, verdict) in feedback.recent {
            let hash = candidate_hash(text);
            let Some(pos) = self.pending.iter().position(|item| item.hash == hash) else {
                continue;
            };
            let region = self.pending.remove(pos).text;
            self.pending.retain(|item| item.hash != hash);
            if verdict.status == Status::Rejected {
                self.rejected_in_a_row += 1;
                let code = verdict
                    .diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.code.clone())
                    .filter(|code| !code.is_empty());
                if code.is_some() && code == self.last_code {
                    self.same_code += 1;
                } else {
                    self.same_code = usize::from(code.is_some());
                }
                self.last_code = code;
            } else {
                self.rejected_in_a_row = 0;
                self.same_code = 0;
                self.last_code = None;
            }
            self.own = Some(verdict.clone());
            self.own_text = Some(region.clone());
            self.remember_tried(hash, region, outcome_of(verdict));
        }
        // Still pending after its batch settled: never evaluated.
        let (stale, live): (Vec<Pending>, Vec<Pending>) =
            self.pending.drain(..).partition(|item| item.waited);
        self.pending = live;
        for item in stale {
            self.remember_tried(item.hash, item.text, NOT_EVALUATED.to_string());
        }
    }

    fn remember_tried(&mut self, hash: String, text: String, outcome: String) {
        self.tried.push(Tried {
            hash,
            text,
            outcome,
        });
        if self.tried.len() > TRIED_LIMIT {
            self.tried.remove(0);
        }
    }

    fn base_text(&mut self, nugget: &Nugget, feedback: &Feedback) -> String {
        if self.rejected_in_a_row >= REJECTS_BEFORE_RESET {
            self.rejected_in_a_row = 0;
            return nugget.seed.clone();
        }
        match feedback.best {
            Some((text, _)) => text.to_string(),
            None => nugget.seed.clone(),
        }
    }

    /// The user prompt to send now: `user`, with [`REPEAT_NOTE`] after it
    /// when the reply before was a repeat.
    fn prompt_now(&self, user: &str) -> String {
        if self.repeats == 0 {
            user.to_string()
        } else {
            format!("{user}{REPEAT_NOTE}")
        }
    }

    /// What `opinion` says about this proposer's last region, when lust has
    /// rejected [`SAME_CODE_BEFORE_RUSTC`] of its regions in a row with the
    /// same first code.
    fn second_opinion(
        &mut self,
        nugget: &Nugget,
        opinion: Option<&SecondOpinion>,
    ) -> Option<String> {
        let opinion = opinion?;
        if self.same_code < SAME_CODE_BEFORE_RUSTC {
            return None;
        }
        let hash = self.own.as_ref()?.hash.clone();
        if let Some((known, text)) = &self.opinion {
            if *known == hash {
                return text.clone();
            }
        }
        let text = opinion(&nugget.splice(self.own_text.as_ref()?));
        self.opinion = Some((hash, text.clone()));
        text
    }

    fn consume(&mut self, raw: Result<String>) -> Step {
        match raw.and_then(|text| accept_region(&text)) {
            Ok(text) => {
                let hash = candidate_hash(&text);
                if self.is_repeat(&hash) {
                    // With a region awaiting its verdict one hotter retry
                    // is enough: the verdict will change the prompt.
                    let retries = if self.pending.is_empty() {
                        REPEAT_TEMPERATURES.len()
                    } else {
                        1
                    };
                    if self.repeats >= retries {
                        self.repeats = 0;
                        self.next_temperature = llm::DEFAULT_TEMPERATURE;
                        let error = err("the local model repeated a region it already tried");
                        let stop = self.fail(error);
                        // The same prompt gives the same region. Wait for
                        // the pending verdicts, which change the prompt.
                        if !self.pending.is_empty() {
                            return self.later();
                        }
                        if stop {
                            return Step::Stop;
                        }
                        return Step::AskAgain;
                    }
                    self.next_temperature = REPEAT_TEMPERATURES[self.repeats];
                    self.repeats += 1;
                    return Step::AskAgain;
                }
                self.errors_in_a_row = 0;
                self.repeats = 0;
                self.next_temperature = llm::DEFAULT_TEMPERATURE;
                self.pending.push(Pending {
                    hash,
                    text: text.clone(),
                    waited: false,
                });
                Step::Return(text)
            }
            Err(error) => {
                self.repeats = 0;
                self.next_temperature = llm::DEFAULT_TEMPERATURE;
                let stop = self.fail(error);
                if self.errors_in_a_row < PROPOSAL_ERROR_LIMIT {
                    Step::AskAgain
                } else if stop {
                    Step::Stop
                } else {
                    self.later()
                }
            }
        }
    }

    /// End this batch for the proposer. Every region pending now gets its
    /// verdict when the batch settles, or it never will.
    fn later(&mut self) -> Step {
        for item in &mut self.pending {
            item.waited = true;
        }
        Step::Later
    }

    /// A region already proposed, whether or not its verdict has come back.
    /// With `--jobs` above 1 the batch is filled before any of it settles,
    /// so a pending region has to count or the same reply is returned again.
    fn is_repeat(&self, hash: &str) -> bool {
        self.tried.iter().any(|item| item.hash == hash)
            || self.pending.iter().any(|item| item.hash == hash)
    }

    /// Count one proposal error. True when the limit is reached and nothing
    /// is pending, which finishes the proposer. With a region pending the
    /// count stays, and the caller returns [`Step::Later`].
    fn fail(&mut self, error: crate::error::Error) -> bool {
        self.errors_in_a_row += 1;
        if self.errors_in_a_row >= PROPOSAL_ERROR_LIMIT && self.pending.is_empty() {
            let line = format!(
                "local proposer finished: {PROPOSAL_ERROR_LIMIT} proposal errors in a row: {}",
                error.message
            );
            crate::console::line(&line);
            self.stopped = Some(line);
            true
        } else {
            false
        }
    }
}

fn prepare_prompt(
    memory: &mut ModelMemory,
    capabilities: &str,
    nugget: &Nugget,
    feedback: &Feedback,
    opinion: Option<&SecondOpinion>,
) -> (String, String) {
    memory.observe(feedback);
    let second = memory.second_opinion(nugget, opinion);
    let base = memory.base_text(nugget, feedback);
    let best_score = feedback.best.map(|(_, score)| score);
    let system = system_prompt(capabilities);
    let user = user_prompt(
        nugget,
        &base,
        base_verdict(&base, feedback),
        memory.own.as_ref(),
        second.as_deref(),
        best_score,
        &memory.tried,
        &memory.pending,
    );
    (system, user)
}

/// The verdict of the region the prompt shows as current: the best's own
/// verdict, or the latest settled verdict for the same text.
fn base_verdict<'a>(base: &str, feedback: &Feedback<'a>) -> Option<&'a Verdict> {
    let hash = candidate_hash(base);
    if let (Some((best, _)), Some(verdict)) = (feedback.best, feedback.best_verdict) {
        if candidate_hash(best) == hash {
            return Some(verdict);
        }
    }
    feedback
        .recent
        .iter()
        .rev()
        .find(|(text, _)| candidate_hash(text) == hash)
        .map(|&(_, verdict)| verdict)
}

fn system_prompt(capabilities: &str) -> String {
    let mut out = String::from(SYSTEM_PREFIX);
    if !capabilities.is_empty() {
        out.push('\n');
        out.push_str(capabilities);
    }
    out
}

fn user_prompt(
    nugget: &Nugget,
    base: &str,
    base_verdict: Option<&Verdict>,
    own: Option<&Verdict>,
    second_opinion: Option<&str>,
    best_score: Option<f64>,
    tried: &[Tried],
    pending: &[Pending],
) -> String {
    let mut out = nugget.splice(base);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(objective_sentence(nugget.objective));
    out.push('\n');
    // A rejection is about the proposer's last reply. Failed cases are
    // about the region shown as current when its verdict is known, so the
    // case quoted is one that region fails.
    if let Some(verdict) = own {
        if verdict.status == Status::Rejected {
            append_rejection(&mut out, verdict);
            if let Some(text) = second_opinion {
                out.push_str(text);
                if !text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    match base_verdict.filter(|verdict| fails_cases(verdict)) {
        Some(current) => append_current_scored(&mut out, current, own),
        None => {
            if let Some(verdict) = own.filter(|verdict| fails_cases(verdict)) {
                append_scored(&mut out, verdict, best_score);
            }
        }
    }
    append_tried(&mut out, tried, pending, base);
    // The request comes last, after the feedback. With the request first and
    // a rejected fragment listed after it, the model continued the fragment
    // eight times out of eight; with the request last it wrote the whole
    // region eight times out of eight.
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str("The lines between // CRAB:BEGIN and // CRAB:END are now:\n");
    out.push_str(base);
    if !base.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("Reply with the complete new lines that replace them.\n");
    out
}

fn objective_sentence(objective: Objective) -> &'static str {
    match objective {
        Objective::PassAll => {
            "Objective: pass_all. The score is the number of failed cases, and 0 is perfect. Lower is better."
        }
        Objective::Minimize => {
            "Objective: minimize. Each case's last stdout line must be `score: <number>`, and the score is their sum. Lower is better."
        }
        Objective::Fastest => {
            "Objective: fastest. Every case must print the expected output; among candidates that do, the one that runs in the least time wins. Lower is better."
        }
    }
}

/// Ran and failed at least one case.
fn fails_cases(verdict: &Verdict) -> bool {
    matches!(verdict.status, Status::Scored | Status::Proven) && verdict.passed < verdict.total
}

/// The failed count and the failing case of the region shown as current,
/// after one line for the proposer's own last region when that region ran,
/// failed cases, and is not the current one.
fn append_current_scored(out: &mut String, current: &Verdict, own: Option<&Verdict>) {
    if let Some(own) = own.filter(|own| fails_cases(own) && own.hash != current.hash) {
        let failed = own.total.saturating_sub(own.passed);
        out.push_str(&format!(
            "Your last region ran and failed {failed} of {} cases.\n",
            own.total
        ));
    }
    let failed = current.total.saturating_sub(current.passed);
    out.push_str(&format!(
        "The current lines ran and failed {failed} of {} cases.",
        current.total
    ));
    match &current.failure {
        Some(failure) => {
            out.push_str(" One of them:\n");
            append_failure(out, failure);
        }
        None => out.push('\n'),
    }
}

fn append_scored(out: &mut String, verdict: &Verdict, best_score: Option<f64>) {
    let failed = verdict.total.saturating_sub(verdict.passed);
    out.push_str(&format!(
        "Your last region ran and failed {failed} of {} cases.",
        verdict.total
    ));
    if let Some(score) = best_score {
        out.push_str(&format!(" The best so far has score {score}."));
    }
    out.push('\n');
    if let Some(failure) = &verdict.failure {
        append_failure(out, failure);
    }
}

fn append_failure(out: &mut String, failure: &CaseFailure) {
    out.push_str("case ");
    out.push_str(&failure.id);
    out.push('\n');
    append_labeled(out, "input", &failure.input);
    append_labeled(out, "expected", &failure.expected);
    append_labeled(out, "actual", &failure.actual);
}

fn append_labeled(out: &mut String, label: &str, text: &str) {
    out.push_str(label);
    out.push_str(":\n");
    out.push_str(text);
    if !text.ends_with('\n') {
        out.push('\n');
    }
}

/// Each tried or pending region, then its outcome. `base` is the region the
/// prompt shows as current. A region of at most [`WHOLE_REGION_LINES`] lines
/// is quoted whole; a longer one only by its lines that are not in `base`,
/// at most [`DIFF_LINES`] of them and [`TEXT_CLIP`] characters. Long regions
/// often differ in one line, and quoted whole and cut they all look alike.
fn append_tried(out: &mut String, tried: &[Tried], pending: &[Pending], base: &str) {
    if tried.is_empty() && pending.is_empty() {
        return;
    }
    out.push('\n');
    out.push_str("Already tried, do not repeat:\n");
    for item in tried {
        append_region_line(out, &item.text, base, &item.outcome);
    }
    for item in pending {
        append_region_line(out, &item.text, base, "proposed, awaiting result");
    }
}

/// Regions this short are quoted whole in the "already tried" list.
const WHOLE_REGION_LINES: usize = 3;
/// Lines quoted from a longer region: those not in the current one.
const DIFF_LINES: usize = 6;

fn append_region_line(out: &mut String, text: &str, base: &str, outcome: &str) {
    let region = if text.lines().count() <= WHOLE_REGION_LINES {
        clip_chars(text, TEXT_CLIP)
    } else {
        region_difference(text, base)
    };
    out.push_str(&region);
    if !region.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(outcome);
    out.push('\n');
}

/// The lines of `text` that are not lines of `base`, compared without
/// leading and trailing spaces, under a heading. Blank lines are left out.
fn region_difference(text: &str, base: &str) -> String {
    let known: HashSet<&str> = base.lines().map(str::trim).collect();
    let new: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty() && !known.contains(line.trim()))
        .map(str::trim_end)
        .collect();
    if new.is_empty() {
        return if text.trim() == base.trim() {
            "(the same as the current lines)".to_string()
        } else {
            "(the current lines, with some removed or moved)".to_string()
        };
    }
    let mut body = String::from("the current lines, but with:\n");
    for line in new.iter().take(DIFF_LINES) {
        body.push_str(line);
        body.push('\n');
    }
    let mut out = clip_chars(&body, TEXT_CLIP);
    if new.len() > DIFF_LINES {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("({} more changed lines)", new.len() - DIFF_LINES));
    }
    out
}

fn outcome_of(verdict: &Verdict) -> String {
    if verdict.status == Status::Rejected {
        return format!(
            "rejected at {}: {}",
            verdict.gate.as_str(),
            rejection_detail(verdict)
        );
    }
    let failed = verdict.total.saturating_sub(verdict.passed);
    if failed > 0 {
        return format!("ran, failed {failed} of {} cases", verdict.total);
    }
    if let Some(score) = verdict.score {
        return format!("score {score}");
    }
    format!("rejected at {}: {}", verdict.gate.as_str(), verdict.note)
}

fn rejection_detail(verdict: &Verdict) -> String {
    if let Some(diagnostic) = verdict.diagnostics.first() {
        let text = match (diagnostic.code.is_empty(), diagnostic.message.is_empty()) {
            (false, false) => format!("{} {}", diagnostic.code, diagnostic.message),
            (false, true) => diagnostic.code.clone(),
            (true, false) => diagnostic.message.clone(),
            (true, true) => String::new(),
        };
        if !text.is_empty() {
            return text;
        }
    }
    verdict.note.clone()
}

fn append_rejection(out: &mut String, verdict: &Verdict) {
    if verdict.diagnostics.is_empty() {
        if !verdict.note.is_empty() {
            out.push_str(&verdict.note);
            out.push('\n');
        }
        return;
    }
    for diagnostic in verdict.diagnostics.iter().take(10) {
        out.push_str(&format_diagnostic(diagnostic));
        out.push('\n');
    }
}

fn format_diagnostic(diagnostic: &Diagnostic) -> String {
    format!(
        "line {}, col {}: {} {}. Hint: {}",
        diagnostic.line, diagnostic.col, diagnostic.code, diagnostic.message, diagnostic.fix
    )
}

/// A [`SecondOpinion`] that checks the program with rustc, in `dir`.
///
/// rustc only checks it (`--emit=metadata`), which takes a fraction of a
/// build. When rustc rejects it too, the answer is rustc's errors, each with
/// the source line it points at. When rustc accepts it, the answer says that
/// lust does not support something in the lines. None when rustc is missing,
/// times out, or the files cannot be written.
pub fn rustc_second_opinion(dir: PathBuf) -> SecondOpinion {
    Box::new(move |program: &str| {
        let rustc = crate::lust::which("rustc")?;
        fs::create_dir_all(&dir).ok()?;
        let source = dir.join("second-opinion.rs");
        fs::write(&source, program).ok()?;
        let mut command = std::process::Command::new(rustc);
        command
            .args(["--edition", "2021", "--error-format=short"])
            .arg("--emit=metadata")
            .arg("-o")
            .arg(dir.join("second-opinion.rmeta"))
            .arg(&source);
        let limits = crate::supervisor::Limits {
            wall: SECOND_OPINION_TIMEOUT,
            memory_bytes: None,
            scrub_env: false,
        };
        let ran = crate::supervisor::run_capture(command, &limits, b"").ok()?;
        match ran.kind {
            crate::supervisor::StopKind::Exited(0) => Some(
                "rustc accepts these lines, so they use something lust does not support. Write them another way, with simpler Rust.\n"
                    .to_string(),
            ),
            crate::supervisor::StopKind::Exited(_) => {
                let summary = rustc_summary(&ran.stderr, program);
                if summary.is_empty() {
                    None
                } else {
                    Some(format!(
                        "rustc rejects these lines too. It says:\n{summary}"
                    ))
                }
            }
            _ => None,
        }
    })
}

/// The errors in rustc's `--error-format=short` output, at most three: the
/// message, then the source line of `program` it points at. A line there is
/// `path:line:col: error[E0282]: message`.
fn rustc_summary(stderr: &str, program: &str) -> String {
    let source: Vec<&str> = program.lines().collect();
    let mut out = String::new();
    let mut errors = 0;
    for line in stderr.lines() {
        let Some(at) = line.find(": error") else {
            continue;
        };
        if errors == 3 {
            break;
        }
        errors += 1;
        out.push_str(line[at + 2..].trim_end());
        out.push('\n');
        // The path may hold a colon (`C:\...`), so read from the right.
        let mut parts = line[..at].rsplit(':');
        let _col = parts.next();
        let number = parts.next().and_then(|n| n.parse::<usize>().ok());
        if let Some(text) = number.and_then(|n| source.get(n.checked_sub(1)?)) {
            out.push_str("  at: ");
            out.push_str(text.trim());
            out.push('\n');
        }
    }
    out
}

fn accept_region(raw: &str) -> Result<String> {
    let text = region_text(raw);
    if text.trim().is_empty() {
        return Err(err("the local model returned an empty region"));
    }
    if text.contains("// CRAB:BEGIN") || text.contains("// CRAB:END") {
        return Err(err(
            "the local model returned // CRAB:BEGIN or // CRAB:END; the reply must be the region text only",
        ));
    }
    Ok(text)
}

/// Fences are stripped. An unfenced reply keeps the indent on its first
/// line; only trailing whitespace is dropped. [`llm::strip_fences`] trims
/// both ends, which the mutation parser wants and a region does not.
fn region_text(raw: &str) -> String {
    if raw.trim_start().starts_with("```") {
        llm::strip_fences(raw)
    } else {
        raw.trim_end().to_string()
    }
}

enum Step {
    Return(String),
    AskAgain,
    /// Nothing new before the pending regions settle.
    Later,
    Stop,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::nugget;
    use crate::pipeline::{CaseFailure, Gate, Ms};

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crab-cage-proposer-{label}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn demo() -> Nugget {
        nugget::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("nuggets")
                .join("sum-squares"),
        )
        .unwrap()
    }

    #[test]
    fn lists_rs_files_once_in_name_order_and_sees_a_file_added_later() {
        let dir = scratch("order");
        fs::write(dir.join("b.rs"), "b\r\n").unwrap();
        fs::write(dir.join("a.rs"), "a\n").unwrap();
        fs::write(dir.join("10.rs"), "ten\n").unwrap();
        fs::write(dir.join("2.rs"), "two\n").unwrap();
        fs::write(dir.join("note.txt"), "nope\n").unwrap();
        fs::write(dir.join(".rs"), "hidden\n").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("z.rs"), "nested\n").unwrap();
        fs::create_dir(dir.join("d.rs")).unwrap();

        let nugget = demo();
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let mut proposer = FolderProposer::new(&dir);
        assert_eq!(proposer.name(), "folder");
        assert_eq!(
            proposer
                .next(&nugget, &feedback)
                .unwrap()
                .into_text()
                .as_deref(),
            Some("ten\n")
        );
        assert_eq!(
            proposer
                .next(&nugget, &feedback)
                .unwrap()
                .into_text()
                .as_deref(),
            Some("two\n")
        );
        assert_eq!(
            proposer
                .next(&nugget, &feedback)
                .unwrap()
                .into_text()
                .as_deref(),
            Some("a\n")
        );
        assert_eq!(
            proposer
                .next(&nugget, &feedback)
                .unwrap()
                .into_text()
                .as_deref(),
            Some("b\n")
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Finished
        );

        fs::write(dir.join("c.rs"), "c\n").unwrap();
        assert_eq!(
            proposer
                .next(&nugget, &feedback)
                .unwrap()
                .into_text()
                .as_deref(),
            Some("c\n")
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Finished
        );
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = scratch("missing");
        fs::remove_dir_all(&dir).unwrap();
        let nugget = demo();
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let mut proposer = FolderProposer::new(&dir);
        let message = proposer
            .next(&nugget, &feedback)
            .expect_err("missing directory should fail")
            .to_string();
        assert!(message.contains(&dir.display().to_string()), "{message}");
    }

    fn nugget_with_seed(seed: &str) -> Nugget {
        Nugget {
            name: "n".to_string(),
            objective: nugget::Objective::PassAll,
            timeout: std::time::Duration::from_millis(1),
            max_memory: 1,
            prefix: String::new(),
            seed: seed.to_string(),
            suffix: String::new(),
            region_first_line: 1,
            cases: Vec::new(),
        }
    }

    fn quiet() -> Feedback<'static> {
        Feedback {
            best: None,
            last: None,
            recent: &[],
            best_verdict: None,
        }
    }

    #[test]
    fn a_commented_digit_stays_and_the_float_changes() {
        let source = "let k = 0.5; // 7";
        let nugget = nugget_with_seed("let n = 9;");
        let feedback = Feedback {
            best: Some((source, 1.0)),
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let mut proposer = LiteralProposer::new(1);
        assert_eq!(proposer.name(), "tweak");
        let got = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();

        let spans = find_literals(source);
        assert_eq!(spans.len(), 1, "the 7 in the comment is not a literal");
        assert_eq!(&source[spans[0].start..spans[0].end], "0.5");
        assert_eq!(&got[..spans[0].start], &source[..spans[0].start]);
        let suffix = &source[spans[0].end..];
        assert!(got.ends_with(suffix), "{got}");
        let middle = &got[spans[0].start..got.len() - suffix.len()];

        let mut rng = Pcg64::new(1, TWEAK_STREAM);
        let _ = rng.gen_index(1);
        let expected = rust_f64(0.5 * (1.0 + rng.gauss() * 0.2));
        assert_eq!(middle, expected);
        assert_ne!(middle, "0.5");
        assert!(!got.contains("let n = 9"), "{got}");
    }

    #[test]
    fn a_zero_float_takes_an_additive_step() {
        let source = "let k = 0.0;";
        let nugget = nugget_with_seed(source);
        let mut proposer = LiteralProposer::new(1);
        let got = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();

        let spans = find_literals(source);
        assert_eq!(spans.len(), 1);
        assert_eq!(&source[spans[0].start..spans[0].end], "0.0");
        let suffix = &source[spans[0].end..];
        assert_eq!(&got[..spans[0].start], &source[..spans[0].start]);
        assert!(got.ends_with(suffix), "{got}");
        let middle = &got[spans[0].start..got.len() - suffix.len()];

        let mut rng = Pcg64::new(1, TWEAK_STREAM);
        let _ = rng.gen_index(1);
        let gauss = rng.gauss();
        let scaled = 0.0 * (1.0 + gauss * 0.2);
        assert_eq!(scaled, 0.0);
        let expected = rust_f64(0.0 + gauss * 0.1);
        assert_eq!(middle, expected);
        assert_ne!(middle, "0.0");
        assert_ne!(got, source);
    }

    #[test]
    fn an_identifier_and_a_string_hold_no_literals() {
        let source = "x1 + \"3\"";
        let nugget = nugget_with_seed(source);
        let feedback = quiet();
        let mut proposer = LiteralProposer::new(1);
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Finished
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Finished
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_sequence_twice() {
        let nugget = nugget_with_seed("let k = 0.5;\nlet n = 4;\n");
        let feedback = quiet();
        let mut first = LiteralProposer::new(11);
        let mut second = LiteralProposer::new(11);
        let mut from_first = Vec::new();
        let mut from_second = Vec::new();
        for _ in 0..8 {
            from_first.push(first.next(&nugget, &feedback).unwrap());
            from_second.push(second.next(&nugget, &feedback).unwrap());
        }
        assert_eq!(from_first, from_second);
        assert!(from_first
            .iter()
            .all(|item| matches!(item, Proposal::Candidate(_))));

        let mut other = LiteralProposer::new(12);
        let mut from_other = Vec::new();
        for _ in 0..8 {
            from_other.push(other.next(&nugget, &feedback).unwrap());
        }
        assert_ne!(from_first, from_other);
    }

    #[test]
    fn an_integer_steps_by_one_or_two_and_stays_non_negative() {
        assert_eq!(integer_choices(0), vec![1, 2]);
        assert_eq!(integer_choices(1), vec![0, 2, 3]);
        assert_eq!(integer_choices(5), vec![3, 4, 6, 7]);
        assert_eq!(
            integer_choices(u128::MAX),
            vec![u128::MAX - 2, u128::MAX - 1]
        );

        let nugget = nugget_with_seed("let n = 5;");
        let mut proposer = LiteralProposer::new(1);
        let got = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let mut rng = Pcg64::new(1, TWEAK_STREAM);
        let _ = rng.gen_index(1);
        let choices = integer_choices(5);
        let expected = choices[rng.gen_index(choices.len())].to_string();
        assert_eq!(got, format!("let n = {expected};"));
    }

    #[test]
    fn the_scanner_keeps_a_fraction_and_an_exponent_with_the_digits() {
        assert_eq!(
            literal_texts("let x = 1.5e-2; b = 3E+1;"),
            vec!["1.5e-2", "3E+1"]
        );
        assert_eq!(literal_texts("n = 10;"), vec!["10"]);
        // `.5` is glued to the dot. `5.` is the integer 5; the dot has no digits.
        assert_eq!(literal_texts(".5 + 5."), vec!["5"]);
        assert_eq!(literal_texts("let _2 = 8;"), vec!["8"]);
        assert_eq!(literal_texts("1e;"), vec!["1"]);
        assert_eq!(literal_texts("s = \"\\\"3\"; t = 4;"), vec!["4"]);
    }

    fn literal_texts(source: &str) -> Vec<&str> {
        find_literals(source)
            .iter()
            .map(|literal| &source[literal.start..literal.end])
            .collect()
    }

    fn region_nugget(objective: nugget::Objective) -> Nugget {
        Nugget {
            name: "region".to_string(),
            objective,
            timeout: std::time::Duration::from_millis(1),
            max_memory: 1,
            prefix: "HEAD\n// CRAB:BEGIN\n".to_string(),
            seed: "SEED_REGION\n".to_string(),
            suffix: "// CRAB:END\n".to_string(),
            region_first_line: 3,
            cases: Vec::new(),
        }
    }

    fn rejected(code: &str, line: u32, note: &str) -> Verdict {
        Verdict {
            hash: "bbbbbbbbbbbbbbbb".to_string(),
            gate: Gate::Check,
            status: Status::Rejected,
            score: None,
            passed: 0,
            total: 1,
            diagnostics: if code.is_empty() {
                Vec::new()
            } else {
                vec![Diagnostic {
                    code: code.to_string(),
                    message: "expected a semicolon".to_string(),
                    line,
                    col: 9,
                    fix: "add a semicolon".to_string(),
                }]
            },
            ms: Ms::default(),
            note: note.to_string(),
            failure: None,
        }
    }

    fn scored(passed: u32, total: u32, score: f64, failure: Option<CaseFailure>) -> Verdict {
        Verdict {
            hash: "cccccccccccccccc".to_string(),
            gate: Gate::Run,
            status: Status::Scored,
            score: Some(score),
            passed,
            total,
            diagnostics: Vec::new(),
            ms: Ms::default(),
            note: String::new(),
            failure,
        }
    }

    #[test]
    fn the_second_prompt_contains_the_first_verdicts_code_and_region_line() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec!["let a = 1;\n".to_string(), "let b = 2;\n".to_string()],
            "no unsafe",
        );
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(first, "let a = 1;");
        assert!(proposer.prompts[0].user.contains("SEED_REGION"));
        assert!(proposer.prompts[0].user.contains("Objective: pass_all."));
        assert!(proposer.prompts[0]
            .user
            .contains("Reply with the complete new lines that replace them."));
        assert!(!proposer.prompts[0].user.contains("L0110"));
        assert!(proposer.prompts[0].system.contains("no unsafe"));
        assert!(proposer.prompts[0].system.starts_with(
            "You rewrite one marked region of a Rust program. Your reply replaces every line between the two markers, so it must be complete Rust statements or an expression that compile in that place. Reply with the replacement lines only: no markers, no markdown fence, no explanation, and never a fragment that continues the old code. Stay inside this subset of Rust:"
        ));

        // Line 4 is already region-relative. region_first_line is 3, so a
        // mistaken shift would print line 6.
        let verdict = rejected("L0110", 4, "SECRET_NOTE");
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 2.0)),
            last: Some((first.as_str(), &verdict)),
            recent: &recent,
            best_verdict: None,
        };
        let second = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(second, "let b = 2;");
        let prompt = &proposer.prompts[1].user;
        assert!(prompt.contains("L0110"), "{prompt}");
        assert!(prompt.contains("line 4"), "{prompt}");
        assert!(prompt.contains("col 9"), "{prompt}");
        assert!(prompt.contains("Hint: add a semicolon"), "{prompt}");
        assert!(prompt.contains("BEST_REGION"), "{prompt}");
        assert!(!prompt.contains("line 6"), "{prompt}");
        assert!(!prompt.contains("SECRET_NOTE"), "{prompt}");
    }

    #[test]
    fn a_verdict_for_someone_else_is_not_fed_back() {
        let nugget = region_nugget(nugget::Objective::Minimize);
        let mut proposer =
            MockModelProposer::new(vec!["one\n".to_string(), "two\n".to_string()], "");
        proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let verdict = rejected("L0999", 8, "");
        let recent = [("not-ours\n", &verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: Some(("not-ours\n", &verdict)),
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(!prompt.contains("L0999"), "{prompt}");
        assert!(prompt.contains("BEST_REGION"), "{prompt}");
        assert!(prompt.contains("Objective: minimize."), "{prompt}");
        assert!(prompt.contains("score: <number>"), "{prompt}");
        assert_eq!(
            proposer.prompts[0].system,
            "You rewrite one marked region of a Rust program. Your reply replaces every line between the two markers, so it must be complete Rust statements or an expression that compile in that place. Reply with the replacement lines only: no markers, no markdown fence, no explanation, and never a fragment that continues the old code. Stay inside this subset of Rust:"
        );
    }

    #[test]
    fn a_rejected_note_is_used_when_there_are_no_diagnostics() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["one\n".to_string(), "two\n".to_string()], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let verdict = rejected("", 0, "lust check failed");
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: Some((first.as_str(), &verdict)),
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert!(proposer.prompts[1].user.contains("lust check failed"));
    }

    #[test]
    fn three_rejects_in_a_row_send_the_seed_instead_of_the_best() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let replies = (0..4).map(|i| format!("body {i}\n")).collect();
        let mut proposer = MockModelProposer::new(replies, "");
        let mut previous = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let verdict = rejected("L0110", 1, "");
        for round in 0..3 {
            previous = {
                let recent = [(previous.as_str(), &verdict)];
                let feedback = Feedback {
                    best: Some(("BEST_REGION\n", 1.0)),
                    last: Some((previous.as_str(), &verdict)),
                    recent: &recent,
                    best_verdict: None,
                };
                proposer
                    .next(&nugget, &feedback)
                    .unwrap()
                    .into_text()
                    .unwrap()
            };
            let prompt = &proposer.prompts.last().unwrap().user;
            if round < 2 {
                assert!(prompt.contains("BEST_REGION"), "round {round}: {prompt}");
                assert!(!prompt.contains("SEED_REGION"), "round {round}: {prompt}");
            } else {
                assert!(prompt.contains("SEED_REGION"), "reset: {prompt}");
                assert!(!prompt.contains("BEST_REGION"), "reset: {prompt}");
            }
        }
    }

    #[test]
    fn fences_are_stripped_and_three_bad_replies_finish_without_an_error() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut fenced = MockModelProposer::new(
            vec!["```rust\n    xs.iter().map(|x| x * x).sum()\n```\n".to_string()],
            "",
        );
        assert_eq!(
            fenced
                .next(&nugget, &quiet())
                .unwrap()
                .into_text()
                .as_deref(),
            Some("    xs.iter().map(|x| x * x).sum()")
        );

        let mut retried = MockModelProposer::new(
            vec![
                "// CRAB:BEGIN\nwhole\n// CRAB:END\n".to_string(),
                "let n = 1;\n".to_string(),
            ],
            "",
        );
        assert_eq!(
            retried
                .next(&nugget, &quiet())
                .unwrap()
                .into_text()
                .as_deref(),
            Some("let n = 1;")
        );
        assert!(retried.memory.stopped.is_none());
        assert_eq!(retried.memory.errors_in_a_row, 0);

        let mut proposer = MockModelProposer::new(
            vec![
                "// CRAB:BEGIN\n".to_string(),
                "```\n// CRAB:END\n```".to_string(),
                "   \n".to_string(),
                "unused\n".to_string(),
            ],
            "",
        );
        assert_eq!(
            proposer.next(&nugget, &quiet()).unwrap(),
            Proposal::Finished
        );
        let reason = proposer.memory.stopped.clone().unwrap();
        assert!(reason.contains("3 proposal errors in a row"), "{reason}");
        assert!(
            reason.contains("empty region") || reason.contains("CRAB"),
            "{reason}"
        );
        assert_eq!(proposer.replies, vec!["unused\n".to_string()]);
        assert_eq!(
            proposer.next(&nugget, &quiet()).unwrap(),
            Proposal::Finished
        );
        assert_eq!(proposer.replies, vec!["unused\n".to_string()]);
    }

    #[test]
    fn at_most_ten_diagnostics_are_quoted() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["one\n".to_string(), "two\n".to_string()], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let mut verdict = rejected("D0", 1, "");
        verdict.diagnostics = (0..11)
            .map(|index| Diagnostic {
                code: format!("D{index}"),
                message: "m".to_string(),
                line: index,
                col: 1,
                fix: "f".to_string(),
            })
            .collect();
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: Some((first.as_str(), &verdict)),
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(prompt.contains("D0"), "{prompt}");
        assert!(prompt.contains("D9"), "{prompt}");
        assert!(!prompt.contains("D10"), "{prompt}");
    }

    #[test]
    fn a_repeated_reply_is_not_proposed_twice() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec![
                "same region\n".to_string(),
                "same region\n".to_string(),
                "fresh region\n".to_string(),
            ],
            "",
        );
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(first, "same region");
        assert_eq!(proposer.asked_temperatures, vec![0.2]);

        let verdict = rejected("L0110", 1, "");
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        let second = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(second, "fresh region");
        assert_ne!(second, first);
        assert_eq!(proposer.asked_temperatures, vec![0.2, 0.2, 0.8]);

        // One region, then a repeat and its three hotter retries.
        let mut stuck = MockModelProposer::new(vec!["again\n".to_string(); 5], "");
        let once = stuck.next(&nugget, &quiet()).unwrap().into_text().unwrap();
        let recent = [(once.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        assert_eq!(stuck.next(&nugget, &feedback).unwrap(), Proposal::Finished);
        assert_eq!(stuck.memory.errors_in_a_row, 1);
        assert!(stuck.memory.stopped.is_none());
        assert!(stuck.replies.is_empty());
    }

    #[test]
    fn the_third_prompt_lists_the_first_two_regions_with_their_outcomes() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec![
                "region one\n".to_string(),
                "region two\n".to_string(),
                "region three\n".to_string(),
            ],
            "",
        );
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let rejected_verdict = rejected("L0110", 4, "SECRET_NOTE");
        let recent = [(first.as_str(), &rejected_verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        let second = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();

        let scored_verdict = scored(1, 3, 2.0, None);
        let recent = [
            (first.as_str(), &rejected_verdict),
            (second.as_str(), &scored_verdict),
        ];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[2].user;
        assert!(prompt.contains("Already tried, do not repeat:"), "{prompt}");
        let one = prompt.find("region one").expect("region one");
        let two = prompt.find("region two").expect("region two");
        assert!(one < two, "{prompt}");
        assert!(
            prompt.contains("rejected at check: L0110 expected a semicolon"),
            "{prompt}"
        );
        assert!(prompt.contains("ran, failed 2 of 3 cases"), "{prompt}");
        assert!(!prompt.contains("SECRET_NOTE"), "{prompt}");
        assert!(prompt.contains("Reply with the complete new lines that replace them."));
    }

    #[test]
    fn a_scored_verdict_names_the_failed_case() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["body one\n".to_string(), "body two\n".to_string()], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let verdict = scored(
            1,
            4,
            3.0,
            Some(CaseFailure {
                id: "negatives".to_string(),
                input: "-4\n5\n".to_string(),
                expected: "EXPECTED_41".to_string(),
                actual: "ACTUAL_1".to_string(),
            }),
        );
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 4.0)),
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        let line = "Your last region ran and failed 3 of 4 cases. The best so far has score 4.";
        let at = prompt.find(line).unwrap_or_else(|| panic!("{prompt}"));
        let expected = prompt
            .find("EXPECTED_41")
            .unwrap_or_else(|| panic!("{prompt}"));
        let actual = prompt
            .find("ACTUAL_1")
            .unwrap_or_else(|| panic!("{prompt}"));
        assert!(at < expected && expected < actual, "{prompt}");
    }

    fn failure(id: &str, expected: &str, actual: &str) -> Option<CaseFailure> {
        Some(CaseFailure {
            id: id.to_string(),
            input: format!("{id} input\n"),
            expected: expected.to_string(),
            actual: actual.to_string(),
        })
    }

    #[test]
    fn the_failing_case_shown_is_the_one_the_shown_region_fails() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec![
                "ours\n".to_string(),
                "again\n".to_string(),
                "third\n".to_string(),
            ],
            "",
        );
        let ours = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let mut own = scored(3, 10, 7.0, failure("climb", "OWN_EXPECTED", "OWN_ACTUAL"));
        own.hash = candidate_hash(&ours);
        let mut best = scored(4, 10, 6.0, failure("slashes", "a/b", "a//b"));
        best.status = Status::Proven;
        best.hash = candidate_hash("BEST_REGION\n");
        let recent = [(ours.as_str(), &own)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 6.0)),
            last: None,
            recent: &recent,
            best_verdict: Some(&best),
        };
        proposer.next(&nugget, &feedback).unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(
            prompt.contains("Your last region ran and failed 7 of 10 cases.\n"),
            "{prompt}"
        );
        assert!(
            prompt.contains("The current lines ran and failed 6 of 10 cases. One of them:\ncase slashes\ninput:\nslashes input\nexpected:\na/b\nactual:\na//b\n"),
            "{prompt}"
        );
        assert!(!prompt.contains("OWN_EXPECTED"), "{prompt}");
        assert!(!prompt.contains("case climb"), "{prompt}");
        let shown = prompt.find("case slashes").unwrap();
        let now = prompt
            .find("The lines between // CRAB:BEGIN and // CRAB:END are now:\nBEST_REGION\n")
            .unwrap();
        assert!(shown < now, "{prompt}");

        // Without `best_verdict` the base's verdict is found in `recent`, and
        // the proposer's own rejection is still quoted before it.
        let rejection = rejected("L0110", 2, "");
        let mut seen_best = best.clone();
        seen_best.failure = failure("root", "/", "");
        let recent = [("BEST_REGION\n", &seen_best), ("again", &rejection)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 6.0)),
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer.next(&nugget, &feedback).unwrap();
        let prompt = &proposer.prompts[2].user;
        let diagnostic = prompt.find("line 2, col 9: L0110").expect("diagnostic");
        let case = prompt.find("case root").expect("base case");
        assert!(diagnostic < case, "{prompt}");
        assert!(!prompt.contains("Your last region ran"), "{prompt}");
        assert!(!prompt.contains("case slashes"), "{prompt}");
    }

    #[test]
    fn a_scored_verdict_is_found_when_it_is_not_the_last_settled() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec!["our region\n".to_string(), "next region\n".to_string()],
            "",
        );
        let ours = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let early = rejected("EARLY_CODE", 1, "early note");
        let late = rejected("LATE_CODE", 2, "late note");
        let scored_verdict = scored(
            0,
            2,
            2.0,
            Some(CaseFailure {
                id: "seven".to_string(),
                input: "7\n".to_string(),
                expected: "EXPECTED_OURS".to_string(),
                actual: "ACTUAL_OURS".to_string(),
            }),
        );
        let recent = [
            ("someone-early\n", &early),
            (ours.as_str(), &scored_verdict),
            ("someone-late\n", &late),
        ];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 5.0)),
            last: Some(("someone-late\n", &late)),
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(
            prompt.contains(
                "Your last region ran and failed 2 of 2 cases. The best so far has score 5."
            ),
            "{prompt}"
        );
        assert!(prompt.contains("EXPECTED_OURS"), "{prompt}");
        assert!(prompt.contains("ACTUAL_OURS"), "{prompt}");
        assert!(!prompt.contains("LATE_CODE"), "{prompt}");
        assert!(!prompt.contains("EARLY_CODE"), "{prompt}");
        assert!(!prompt.contains("late note"), "{prompt}");
    }

    #[test]
    fn an_already_tried_region_is_cut_to_400_characters() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let long = "R".repeat(450);
        let mut proposer =
            MockModelProposer::new(vec![format!("{long}\n"), "other\n".to_string()], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(first.chars().count(), 450);
        let verdict = rejected("L0110", 1, "");
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(prompt.contains(&"R".repeat(400)), "{prompt}");
        assert!(!prompt.contains(&"R".repeat(401)), "{prompt}");
        assert!(
            prompt.contains("rejected at check: L0110 expected a semicolon"),
            "{prompt}"
        );
    }

    #[test]
    fn the_prompt_keeps_only_the_last_eight_tried_regions() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let replies: Vec<String> = (0..10).map(|index| format!("region {index}\n")).collect();
        let mut proposer = MockModelProposer::new(replies, "");
        let verdict = rejected("", 0, "nope");
        let mut previous = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let mut history = Vec::new();
        for _ in 0..9 {
            history.push((previous.clone(), verdict.clone()));
            previous = {
                let view: Vec<(&str, &Verdict)> = history
                    .iter()
                    .map(|(text, item)| (text.as_str(), item))
                    .collect();
                let feedback = Feedback {
                    best: None,
                    last: None,
                    recent: &view,
                    best_verdict: None,
                };
                proposer
                    .next(&nugget, &feedback)
                    .unwrap()
                    .into_text()
                    .unwrap()
            };
        }
        let prompt = &proposer.prompts.last().unwrap().user;
        assert!(!prompt.contains("region 0\n"), "{prompt}");
        assert!(prompt.contains("region 1\n"), "{prompt}");
        assert!(prompt.contains("region 8\n"), "{prompt}");
        assert!(!prompt.contains("region 9\n"), "{prompt}");
    }

    fn long_region(changed: &str) -> String {
        format!(
            "    let mut parts = Vec::new();\n    for part in path.split('/') {{\n{changed}\n        parts.push(part);\n    }}\n    parts.join(\"/\")\n"
        )
    }

    #[test]
    fn a_long_tried_region_shows_only_the_lines_that_differ_from_the_current_one() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let best = long_region("        if part == \".\" { continue; }");
        let one = long_region("        if part.is_empty() { continue; }");
        let two = long_region("        if part == \"..\" { parts.pop(); continue; }");
        let mut proposer = MockModelProposer::new(
            vec![
                one.clone(),
                two.clone(),
                best.clone(),
                "short one\nshort two\n".to_string(),
                "next\n".to_string(),
            ],
            "",
        );
        let verdict = scored(4, 10, 6.0, None);
        let mut history: Vec<(String, Verdict)> = Vec::new();
        for _ in 0..4 {
            let view: Vec<(&str, &Verdict)> = history
                .iter()
                .map(|(text, item)| (text.as_str(), item))
                .collect();
            let feedback = Feedback {
                best: Some((best.as_str(), 6.0)),
                last: None,
                recent: &view,
                best_verdict: None,
            };
            let text = proposer.next(&nugget, &feedback).unwrap();
            let text = text.into_text().unwrap();
            drop(view);
            history.push((text, verdict.clone()));
        }
        let view: Vec<(&str, &Verdict)> = history
            .iter()
            .map(|(text, item)| (text.as_str(), item))
            .collect();
        let feedback = Feedback {
            best: Some((best.as_str(), 6.0)),
            last: None,
            recent: &view,
            best_verdict: None,
        };
        proposer.next(&nugget, &feedback).unwrap();
        let prompt = &proposer.prompts[4].user;
        let tried = &prompt[prompt.find("Already tried, do not repeat:").unwrap()..];
        let tried = &tried[..tried.find("The lines between").unwrap()];
        assert!(
            tried.contains("if part.is_empty() { continue; }"),
            "{tried}"
        );
        assert!(tried.contains("if part == \"..\" { parts.pop(); continue; }"));
        // The lines every region shares are not repeated in the list.
        assert!(!tried.contains("parts.push(part);"), "{tried}");
        assert!(!tried.contains("Vec::new()"), "{tried}");
        assert_eq!(tried.matches("the current lines, but with:").count(), 2);
        // The region that equals the current one is a marker and its outcome.
        assert!(
            tried.contains("(the same as the current lines)\nran, failed 6 of 10 cases"),
            "{tried}"
        );
        // A short region is still quoted whole.
        assert!(
            tried.contains("short one\nshort two\nran, failed 6 of 10 cases"),
            "{tried}"
        );
        assert_eq!(tried.matches("ran, failed 6 of 10 cases").count(), 4);
        assert!(prompt.ends_with("Reply with the complete new lines that replace them.\n"));
    }

    #[test]
    fn many_changed_lines_are_cut_to_six() {
        let base = "a\nb\nc\nd\n";
        let text = (0..9)
            .map(|index| format!("new {index}\n"))
            .collect::<String>();
        let shown = region_difference(&text, base);
        assert!(shown.contains("new 0\n"), "{shown}");
        assert!(shown.contains("new 5\n"), "{shown}");
        assert!(!shown.contains("new 6"), "{shown}");
        assert!(shown.ends_with("(3 more changed lines)"), "{shown}");
        assert_eq!(
            region_difference("a\nb\nd\n", base),
            "(the current lines, with some removed or moved)"
        );
        let wide = format!("{}\n{}\n", "x".repeat(300), "y".repeat(300));
        assert!(region_difference(&wide, base).chars().count() <= TEXT_CLIP);
    }

    #[test]
    fn a_region_awaiting_its_result_is_not_proposed_again_and_keeps_its_indent() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let region = "    let total: i64 = \"no\";\n    total\n";
        let mut proposer = MockModelProposer::new(vec![region.to_string(); 12], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(first, "    let total: i64 = \"no\";\n    total");
        // The repeat and its hotter retry, then wait for the verdict: the
        // proposer is not finished.
        assert_eq!(proposer.next(&nugget, &quiet()).unwrap(), Proposal::Later);
        assert!(proposer.memory.stopped.is_none());
        assert_eq!(proposer.replies.len(), 9);
        assert_eq!(proposer.asked_temperatures, vec![0.2, 0.2, 0.8]);
        let prompt = &proposer.prompts[1].user;
        assert!(prompt.contains("proposed, awaiting result"), "{prompt}");
        assert!(prompt.contains("    let total: i64 = \"no\";"), "{prompt}");
        assert!(prompt.contains("\n    total\n"), "{prompt}");

        // The batch settled without a verdict for it: it was never
        // evaluated. It is tried now, not pending, and repeats of it count
        // to the limit with nothing pending, so the proposer finishes.
        assert_eq!(
            proposer.next(&nugget, &quiet()).unwrap(),
            Proposal::Finished
        );
        let prompt = &proposer.prompts[2].user;
        assert!(!prompt.contains("proposed, awaiting result"), "{prompt}");
        assert!(prompt.contains(NOT_EVALUATED), "{prompt}");
        assert!(proposer.memory.pending.is_empty());
        let reason = proposer.memory.stopped.clone().unwrap();
        assert!(reason.contains("3 proposal errors in a row"), "{reason}");
        assert_eq!(proposer.replies.len(), 1);
    }

    #[test]
    fn errors_with_a_region_pending_wait_and_without_one_finish() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec![
                "first\n".to_string(),
                "   \n".to_string(),
                "\n".to_string(),
                " \n".to_string(),
                "second\n".to_string(),
            ],
            "",
        );
        assert_eq!(
            proposer.next(&nugget, &quiet()).unwrap(),
            Proposal::Candidate("first".to_string())
        );
        // Three empty replies with `first` awaiting its verdict.
        assert_eq!(proposer.next(&nugget, &quiet()).unwrap(), Proposal::Later);
        assert!(proposer.memory.stopped.is_none());
        let verdict = rejected("L0110", 1, "");
        let recent = [("first", &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Candidate("second".to_string())
        );
        assert_eq!(proposer.memory.errors_in_a_row, 0);

        let mut lonely = MockModelProposer::new(
            vec!["   \n".to_string(), "\n".to_string(), " \n".to_string()],
            "",
        );
        assert_eq!(lonely.next(&nugget, &quiet()).unwrap(), Proposal::Finished);
        assert!(lonely.memory.stopped.is_some());
    }

    /// Answers like a model at low temperature: the same prompt state gives
    /// the same region. Region `k` is the reply once `k` verdicts are back.
    struct OnePerVerdict {
        regions: Vec<String>,
        memory: ModelMemory,
        asks: usize,
    }

    impl Proposer for OnePerVerdict {
        fn name(&self) -> &str {
            "one-per-verdict"
        }

        fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
            if self.memory.stopped.is_some() {
                return Ok(Proposal::Finished);
            }
            let _ = prepare_prompt(&mut self.memory, "", nugget, feedback, None);
            loop {
                self.asks += 1;
                let index = self.memory.tried.len().min(self.regions.len() - 1);
                match self.memory.consume(Ok(self.regions[index].clone())) {
                    Step::Return(text) => return Ok(Proposal::Candidate(text)),
                    Step::Later => return Ok(Proposal::Later),
                    Step::Stop => return Ok(Proposal::Finished),
                    Step::AskAgain => {}
                }
            }
        }
    }

    #[test]
    fn later_with_three_jobs_keeps_the_model_for_the_next_batch() {
        let out = std::env::temp_dir().join(format!(
            "crab-cage-proposer-jobs-later-{}",
            std::process::id()
        ));
        if out.exists() {
            fs::remove_dir_all(&out).unwrap();
        }
        fs::create_dir_all(&out).unwrap();
        let mut model = OnePerVerdict {
            regions: (0..4)
                .map(|index| format!("    std::fs::read(\"{index}\");\n"))
                .collect(),
            memory: ModelMemory::default(),
            asks: 0,
        };
        let mut proposers: Vec<&mut dyn crate::proposer::Proposer> = vec![&mut model];
        crate::run::execute(
            &crate::run::RunOptions {
                nugget_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("nuggets")
                    .join("sum-squares"),
                budget: 5,
                jobs: 3,
                out_dir: out.clone(),
                lust_bin: None,
                lust_daemon: false,
            },
            &mut proposers,
        )
        .unwrap();
        // Before `Later`, the second ask of the first batch repeated the
        // pending region three times and the proposer was finished for the
        // run, leaving the seed and one candidate.
        assert!(model.memory.stopped.is_none());
        let journal = fs::read_to_string(out.join("journal.jsonl")).unwrap();
        assert_eq!(journal.lines().count(), 5, "{journal}");
        for index in 0..4 {
            assert!(
                journal.contains(&candidate_hash(&format!("    std::fs::read(\"{index}\");"))),
                "region {index}: {journal}"
            );
        }
        // One new region and one repeat with its retry per batch.
        assert!(model.asks <= 12, "asked {}", model.asks);
    }

    #[test]
    fn jobs_do_not_ask_a_repeated_region_fifty_times() {
        let out = std::env::temp_dir().join(format!(
            "crab-cage-proposer-jobs-repeat-{}",
            std::process::id()
        ));
        if out.exists() {
            fs::remove_dir_all(&out).unwrap();
        }
        fs::create_dir_all(&out).unwrap();
        let mut model =
            MockModelProposer::new(vec!["    std::fs::read(\"a\");\n".to_string(); 40], "");
        let mut proposers: Vec<&mut dyn crate::proposer::Proposer> = vec![&mut model];
        crate::run::execute(
            &crate::run::RunOptions {
                nugget_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("nuggets")
                    .join("sum-squares"),
                budget: 8,
                jobs: 3,
                out_dir: out,
                lust_bin: None,
                lust_daemon: false,
            },
            &mut proposers,
        )
        .unwrap();
        assert!(
            model.asked_temperatures.len() <= 12,
            "asked {} times: {:?}",
            model.asked_temperatures.len(),
            model.asked_temperatures
        );
        assert!(model.replies.len() >= 28, "left {}", model.replies.len());
        assert!(model.asked_temperatures.contains(&0.8));
        assert!(model.prompts[1].user.contains("proposed, awaiting result"));
        assert!(model.prompts[1].user.contains("    std::fs::read"));
    }

    #[test]
    fn a_proven_verdict_that_still_fails_cases_names_the_failure() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["body\n".to_string(), "next\n".to_string()], "");
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let mut verdict = scored(
            2,
            3,
            1.0,
            Some(CaseFailure {
                id: "negatives".to_string(),
                input: "-4\n5\n".to_string(),
                expected: "41".to_string(),
                actual: "49".to_string(),
            }),
        );
        verdict.status = Status::Proven;
        verdict.gate = Gate::Oracle;
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(
            prompt.contains(
                "Your last region ran and failed 1 of 3 cases. The best so far has score 1."
            ),
            "{prompt}"
        );
        let line = prompt
            .find("Your last region ran and failed 1 of 3 cases.")
            .unwrap_or_else(|| panic!("{prompt}"));
        let expected = prompt
            .find("expected:\n41")
            .unwrap_or_else(|| panic!("{prompt}"));
        let actual = prompt
            .find("actual:\n49")
            .unwrap_or_else(|| panic!("{prompt}"));
        assert!(line < expected && expected < actual, "{prompt}");
    }

    #[test]
    fn the_model_endpoint_stays_loopback_http() {
        let refused = ModelProposer::new("http://8.8.8.8:11434", "m", "");
        assert!(refused.is_err());
        let proposer = ModelProposer::new("http://127.0.0.1:11434", "qwen", "caps").unwrap();
        assert_eq!(proposer.endpoint, "http://127.0.0.1:11434/v1");
        assert_eq!(proposer.name(), "local");
        assert_eq!(proposer.capabilities, "caps");
    }

    #[test]
    fn a_repeat_is_asked_again_hotter_each_time_with_a_note() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec![
                "one\n".to_string(),
                "one\n".to_string(),
                "one\n".to_string(),
                "one\n".to_string(),
                "two\n".to_string(),
            ],
            "",
        );
        let first = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        let verdict = rejected("L0110", 1, "");
        let recent = [(first.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        // Three repeats, each asked again hotter; the fourth reply is new.
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap(),
            Proposal::Candidate("two".to_string())
        );
        assert_eq!(proposer.asked_temperatures, vec![0.2, 0.2, 0.8, 1.0, 1.2]);
        assert_eq!(proposer.memory.errors_in_a_row, 0);
        // The first ask of a prompt has no note; each ask after a repeat
        // ends with it, after the request.
        assert!(!proposer.asked_users[1].contains(REPEAT_NOTE));
        for asked in &proposer.asked_users[2..] {
            assert!(asked.ends_with(REPEAT_NOTE), "{asked}");
            assert!(
                asked.contains("Reply with the complete new lines that replace them.\n"),
                "{asked}"
            );
        }
    }

    #[test]
    fn rustc_is_asked_after_two_rejections_with_the_same_code() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let nugget = region_nugget(nugget::Objective::PassAll);
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let mut proposer = MockModelProposer::new(
            vec![
                "one\n".to_string(),
                "two\n".to_string(),
                "three\n".to_string(),
                "four\n".to_string(),
                "five\n".to_string(),
            ],
            "",
        )
        .with_second_opinion(Box::new(move |program: &str| {
            record.borrow_mut().push(program.to_string());
            Some("RUSTC_SAYS\n".to_string())
        }));

        let mut last = proposer
            .next(&nugget, &quiet())
            .unwrap()
            .into_text()
            .unwrap();
        // One rejection: rustc is not asked.
        let mut verdict = rejected("L0103", 1, "");
        verdict.hash = candidate_hash(&last);
        let recent = [(last.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        last = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert!(seen.borrow().is_empty());
        assert!(!proposer.prompts[1].user.contains("RUSTC_SAYS"));

        // The same code again: rustc is asked about the whole program with
        // the last region in it, and its answer follows lust's message.
        let mut verdict = rejected("L0103", 1, "");
        verdict.hash = candidate_hash(&last);
        let recent = [(last.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        last = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(seen.borrow()[0], nugget.splice("two"));
        let prompt = &proposer.prompts[2].user;
        let lust_at = prompt.find("L0103").unwrap();
        let rustc_at = prompt.find("RUSTC_SAYS").unwrap();
        assert!(lust_at < rustc_at, "{prompt}");
        assert!(
            prompt.ends_with("Reply with the complete new lines that replace them.\n"),
            "{prompt}"
        );

        // A different code: the count starts again and rustc is not asked.
        let mut verdict = rejected("L0110", 1, "");
        verdict.hash = candidate_hash(&last);
        let recent = [(last.as_str(), &verdict)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        proposer.next(&nugget, &feedback).unwrap();
        assert_eq!(seen.borrow().len(), 1);
        assert!(!proposer.prompts[3].user.contains("RUSTC_SAYS"));
    }

    #[test]
    fn rustc_summary_keeps_the_message_and_the_source_line() {
        let program = "fn main() {\n    let mut v = Vec::new();\n    v.len();\n}\n";
        let stderr = "C:\\work\\second-opinion.rs:2:9: error[E0282]: type annotations needed for `Vec<_>`\nerror: aborting due to 1 previous error\n";
        assert_eq!(
            rustc_summary(stderr, program),
            "error[E0282]: type annotations needed for `Vec<_>`\n  at: let mut v = Vec::new();\n"
        );
        assert_eq!(rustc_summary("warning: unused\n", program), "");
    }

    #[test]
    fn rustc_gives_a_second_opinion_on_a_real_program() {
        if crate::lust::which("rustc").is_none() {
            return;
        }
        let dir = scratch("second-opinion");
        let opinion = rustc_second_opinion(dir.clone());
        // The program that cost a model every run of `merge-ranges`: lust
        // says "no field `1`", rustc says where the type is missing.
        let rejected = "fn main() {\n    let mut merged = Vec::new();\n    if let Some(last) = merged.last_mut() {\n        last.1 = 2;\n    }\n}\n";
        let text = opinion(rejected).unwrap();
        assert!(text.starts_with("rustc rejects these lines too."), "{text}");
        assert!(text.contains("E0282"), "{text}");
        assert!(text.contains("at: let mut merged = Vec::new();"), "{text}");

        let accepted = "fn main() {\n    println!(\"ok\");\n}\n";
        let text = opinion(accepted).unwrap();
        assert!(text.starts_with("rustc accepts these lines"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
