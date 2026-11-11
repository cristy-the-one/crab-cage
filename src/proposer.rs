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
//! the template with the current best region in place, plus the last rejected
//! verdict this proposer itself produced.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{err, Result};
use crate::genome::rust_f64;
use crate::llm;
use crate::nugget::{Nugget, Objective};
use crate::pipeline::{candidate_hash, Diagnostic, Status, Verdict};
use crate::rng::Pcg64;

/// PCG stream for [`LiteralProposer`]. A different stream keeps this sequence
/// from matching a generator that was given the same seed and stream 0.
const TWEAK_STREAM: u64 = 16;

/// What the run loop knows when it asks for another candidate.
pub struct Feedback<'a> {
    pub best: Option<(&'a str, f64)>,
    pub last: Option<(&'a str, &'a Verdict)>,
}

/// A source of candidate region text.
pub trait Proposer {
    fn name(&self) -> &str;
    /// The next candidate text, or None when this proposer has nothing more.
    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Option<String>>;
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

    fn next(&mut self, _nugget: &Nugget, _feedback: &Feedback) -> Result<Option<String>> {
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
            return Ok(None);
        };
        let path = self.dir.join(&name);
        let text = read_candidate(&path)?;
        self.yielded.insert(name);
        Ok(Some(text))
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

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Option<String>> {
        let text = match feedback.best {
            Some((best, _)) => best,
            None => nugget.seed.as_str(),
        };
        let literals = find_literals(text);
        if literals.is_empty() {
            return Ok(None);
        }
        let index = self.rng.gen_index(literals.len());
        let Literal { start, end, kind } = literals[index];
        let replacement = match kind {
            LiteralKind::Int(value) => tweak_int(&mut self.rng, value),
            LiteralKind::Float(value) => tweak_float(&mut self.rng, value, &text[start..end]),
        };
        Ok(Some(replace_span(text, start, end, &replacement)))
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
const SYSTEM_PREFIX: &str = "\
You write the body of one region of a Rust program. Reply with only the new region text, no markdown fence, no explanation. Stay inside this subset of Rust:";

/// Asks a loopback chat model for the next region.
///
/// The capability summary is fixed at construction (empty when lust is
/// absent). A reply that still contains a region marker, or an empty reply,
/// or a chat error, is not returned to the run. Three of those in a row
/// finish the proposer. `run::execute` treats `Err` from [`Proposer::next`]
/// as fatal and this package does not change that loop, so the failure stays
/// here and `next` returns [`None`](Option::None) after printing the reason.
///
/// Repeats are not filtered here. The run already stops after 50 proposals
/// in a row that leave no journal line, and the hash it uses is
/// [`candidate_hash`](crate::pipeline::candidate_hash).
pub struct ModelProposer {
    endpoint: String,
    model: String,
    capabilities: String,
    timeout: Duration,
    memory: ModelMemory,
}

impl ModelProposer {
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
        })
    }
}

impl Proposer for ModelProposer {
    fn name(&self) -> &str {
        "local"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Option<String>> {
        if self.memory.stopped.is_some() {
            return Ok(None);
        }
        let (system, user) = prepare_prompt(&mut self.memory, &self.capabilities, nugget, feedback);
        loop {
            let raw = llm::chat(&self.endpoint, &self.model, &system, &user, self.timeout);
            match take_attempt(&mut self.memory, raw) {
                Attempt::Propose(text) => return Ok(Some(text)),
                Attempt::Finished => return Ok(None),
                Attempt::Again => {}
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
        }
    }
}

impl Proposer for MockModelProposer {
    fn name(&self) -> &str {
        "mock"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Option<String>> {
        if self.memory.stopped.is_some() || self.replies.is_empty() {
            return Ok(None);
        }
        let (system, user) = prepare_prompt(&mut self.memory, &self.capabilities, nugget, feedback);
        self.prompts.push(Asked { system, user });
        loop {
            if self.replies.is_empty() {
                return Ok(None);
            }
            let raw = self.replies.remove(0);
            match take_attempt(&mut self.memory, Ok(raw)) {
                Attempt::Propose(text) => return Ok(Some(text)),
                Attempt::Finished => return Ok(None),
                Attempt::Again => {}
            }
        }
    }
}

#[derive(Default)]
struct ModelMemory {
    rejected_in_a_row: usize,
    errors_in_a_row: usize,
    /// [`candidate_hash`](crate::pipeline::candidate_hash) of the last region
    /// this proposer returned. `feedback.last` counts only when it matches.
    proposed_hash: Option<String>,
    /// Hash already folded into `rejected_in_a_row`, so one verdict is one count.
    accounted_hash: Option<String>,
    own: Option<Verdict>,
    stopped: Option<String>,
}

impl ModelMemory {
    fn observe(&mut self, feedback: &Feedback) {
        let Some((text, verdict)) = feedback.last else {
            return;
        };
        let Some(proposed) = &self.proposed_hash else {
            return;
        };
        let hash = candidate_hash(text);
        if &hash != proposed || self.accounted_hash.as_ref() == Some(&hash) {
            return;
        }
        self.accounted_hash = Some(hash);
        if verdict.status == Status::Rejected {
            self.rejected_in_a_row += 1;
        } else {
            self.rejected_in_a_row = 0;
        }
        self.own = Some(verdict.clone());
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
}

fn prepare_prompt(
    memory: &mut ModelMemory,
    capabilities: &str,
    nugget: &Nugget,
    feedback: &Feedback,
) -> (String, String) {
    memory.observe(feedback);
    let base = memory.base_text(nugget, feedback);
    let system = system_prompt(capabilities);
    let user = user_prompt(nugget, &base, memory.own.as_ref());
    (system, user)
}

fn system_prompt(capabilities: &str) -> String {
    let mut out = String::from(SYSTEM_PREFIX);
    if !capabilities.is_empty() {
        out.push('\n');
        out.push_str(capabilities);
    }
    out
}

fn user_prompt(nugget: &Nugget, base: &str, own: Option<&Verdict>) -> String {
    let mut out = nugget.splice(base);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(objective_sentence(nugget.objective));
    out.push('\n');
    if let Some(verdict) = own {
        if verdict.status == Status::Rejected {
            append_rejection(&mut out, verdict);
        }
    }
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
    }
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

fn accept_region(raw: &str) -> Result<String> {
    let text = llm::strip_fences(raw);
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

enum Attempt {
    Propose(String),
    Again,
    Finished,
}

fn take_attempt(memory: &mut ModelMemory, raw: Result<String>) -> Attempt {
    match raw.and_then(|text| accept_region(&text)) {
        Ok(text) => {
            memory.errors_in_a_row = 0;
            memory.proposed_hash = Some(candidate_hash(&text));
            Attempt::Propose(text)
        }
        Err(error) => {
            memory.errors_in_a_row += 1;
            if memory.errors_in_a_row >= PROPOSAL_ERROR_LIMIT {
                let line = format!(
                    "local proposer finished: {PROPOSAL_ERROR_LIMIT} proposal errors in a row: {}",
                    error.message
                );
                println!("{line}");
                memory.stopped = Some(line);
                Attempt::Finished
            } else {
                Attempt::Again
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::nugget;
    use crate::pipeline::{Gate, Ms};

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
        };
        let mut proposer = FolderProposer::new(&dir);
        assert_eq!(proposer.name(), "folder");
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap().as_deref(),
            Some("ten\n")
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap().as_deref(),
            Some("two\n")
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap().as_deref(),
            Some("a\n")
        );
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap().as_deref(),
            Some("b\n")
        );
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), None);

        fs::write(dir.join("c.rs"), "c\n").unwrap();
        assert_eq!(
            proposer.next(&nugget, &feedback).unwrap().as_deref(),
            Some("c\n")
        );
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), None);
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = scratch("missing");
        fs::remove_dir_all(&dir).unwrap();
        let nugget = demo();
        let feedback = Feedback {
            best: None,
            last: None,
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
        }
    }

    #[test]
    fn a_commented_digit_stays_and_the_float_changes() {
        let source = "let k = 0.5; // 7";
        let nugget = nugget_with_seed("let n = 9;");
        let feedback = Feedback {
            best: Some((source, 1.0)),
            last: None,
        };
        let mut proposer = LiteralProposer::new(1);
        assert_eq!(proposer.name(), "tweak");
        let got = proposer.next(&nugget, &feedback).unwrap().unwrap();

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
        let got = proposer.next(&nugget, &quiet()).unwrap().unwrap();

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
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), None);
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), None);
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
        assert!(from_first.iter().all(|item| item.is_some()));

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
        let got = proposer.next(&nugget, &quiet()).unwrap().unwrap();
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
        }
    }

    #[test]
    fn the_second_prompt_contains_the_first_verdicts_code_and_region_line() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer = MockModelProposer::new(
            vec!["let a = 1;\n".to_string(), "let b = 2;\n".to_string()],
            "no unsafe",
        );
        let first = proposer.next(&nugget, &quiet()).unwrap().unwrap();
        assert_eq!(first, "let a = 1;");
        assert!(proposer.prompts[0].user.contains("SEED_REGION"));
        assert!(proposer.prompts[0].user.contains("Objective: pass_all."));
        assert!(!proposer.prompts[0].user.contains("L0110"));
        assert!(proposer.prompts[0].system.contains("no unsafe"));
        assert!(proposer.prompts[0].system.starts_with(
            "You write the body of one region of a Rust program. Reply with only the new region text, no markdown fence, no explanation. Stay inside this subset of Rust:"
        ));

        // Line 4 is already region-relative. region_first_line is 3, so a
        // mistaken shift would print line 6.
        let verdict = rejected("L0110", 4, "SECRET_NOTE");
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 2.0)),
            last: Some((first.as_str(), &verdict)),
        };
        let second = proposer.next(&nugget, &feedback).unwrap().unwrap();
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
        proposer.next(&nugget, &quiet()).unwrap().unwrap();
        let verdict = rejected("L0999", 8, "");
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: Some(("not-ours\n", &verdict)),
        };
        proposer.next(&nugget, &feedback).unwrap().unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(!prompt.contains("L0999"), "{prompt}");
        assert!(prompt.contains("BEST_REGION"), "{prompt}");
        assert!(prompt.contains("Objective: minimize."), "{prompt}");
        assert!(prompt.contains("score: <number>"), "{prompt}");
        assert_eq!(
            proposer.prompts[0].system,
            "You write the body of one region of a Rust program. Reply with only the new region text, no markdown fence, no explanation. Stay inside this subset of Rust:"
        );
    }

    #[test]
    fn a_rejected_note_is_used_when_there_are_no_diagnostics() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["one\n".to_string(), "two\n".to_string()], "");
        let first = proposer.next(&nugget, &quiet()).unwrap().unwrap();
        let verdict = rejected("", 0, "lust check failed");
        let feedback = Feedback {
            best: None,
            last: Some((first.as_str(), &verdict)),
        };
        proposer.next(&nugget, &feedback).unwrap().unwrap();
        assert!(proposer.prompts[1].user.contains("lust check failed"));
    }

    #[test]
    fn three_rejects_in_a_row_send_the_seed_instead_of_the_best() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let replies = (0..4).map(|i| format!("body {i}\n")).collect();
        let mut proposer = MockModelProposer::new(replies, "");
        let mut previous = proposer.next(&nugget, &quiet()).unwrap().unwrap();
        let verdict = rejected("L0110", 1, "");
        for round in 0..3 {
            let feedback = Feedback {
                best: Some(("BEST_REGION\n", 1.0)),
                last: Some((previous.as_str(), &verdict)),
            };
            previous = proposer.next(&nugget, &feedback).unwrap().unwrap();
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
            fenced.next(&nugget, &quiet()).unwrap().as_deref(),
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
            retried.next(&nugget, &quiet()).unwrap().as_deref(),
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
        assert_eq!(proposer.next(&nugget, &quiet()).unwrap(), None);
        let reason = proposer.memory.stopped.clone().unwrap();
        assert!(reason.contains("3 proposal errors in a row"), "{reason}");
        assert!(
            reason.contains("empty region") || reason.contains("CRAB"),
            "{reason}"
        );
        assert_eq!(proposer.replies, vec!["unused\n".to_string()]);
        assert_eq!(proposer.next(&nugget, &quiet()).unwrap(), None);
        assert_eq!(proposer.replies, vec!["unused\n".to_string()]);
    }

    #[test]
    fn at_most_ten_diagnostics_are_quoted() {
        let nugget = region_nugget(nugget::Objective::PassAll);
        let mut proposer =
            MockModelProposer::new(vec!["one\n".to_string(), "two\n".to_string()], "");
        let first = proposer.next(&nugget, &quiet()).unwrap().unwrap();
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
        let feedback = Feedback {
            best: Some(("BEST_REGION\n", 1.0)),
            last: Some((first.as_str(), &verdict)),
        };
        proposer.next(&nugget, &feedback).unwrap().unwrap();
        let prompt = &proposer.prompts[1].user;
        assert!(prompt.contains("D0"), "{prompt}");
        assert!(prompt.contains("D9"), "{prompt}");
        assert!(!prompt.contains("D10"), "{prompt}");
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
}
