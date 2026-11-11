//! Where the next candidate text comes from.
//!
//! [`FolderProposer`] reads each `*.rs` file in a directory once, in file-name
//! order. It lists the directory again on every call, so a file that appears
//! while a run is in progress is still offered.
//!
//! [`LiteralProposer`] rewrites one numeric literal of the best candidate, or
//! of the nugget seed when the run has no best text yet.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{err, Result};
use crate::genome::rust_f64;
use crate::nugget::Nugget;
use crate::pipeline::Verdict;
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::nugget;

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
}
