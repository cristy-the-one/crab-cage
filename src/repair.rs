//! The repair proposer: rustc's own fixes for a rejected candidate, applied
//! without a model.
//!
//! rustc marks each change it suggests with how far it can be trusted. A
//! `MachineApplicable` one is meant to be applied by a tool: for
//! `.filter(|c| c != ".")` on a `&&str` it is a `*` in front of `c`. A small
//! model was shown that fix in more than a hundred prompts of one ladder run
//! and used it once. This proposer applies it and hands the result to the
//! run as a candidate of its own. The funnel judges it like any other.

use std::collections::HashSet;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::Result;
use crate::nugget::Nugget;
use crate::pipeline::{candidate_hash, Gate, Status};
use crate::proposer::{Feedback, Proposal, Proposer};

/// Wall-clock limit of one rustc check.
const RUSTC_TIMEOUT: Duration = Duration::from_secs(60);

/// What the repair proposer has proposed, oldest first: the hash of the
/// candidate it repaired, and the repaired text. The model proposer reads
/// it to find out what became of a region of its own that did not compile.
pub type RepairLog = std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>>;

/// Given a whole program, rustc's diagnostics for it as JSON lines
/// (`--error-format=json`). None when rustc accepts the program or cannot
/// be run.
pub type Diagnose = Box<dyn Fn(&str) -> Option<String>>;

/// Proposes a rejected candidate again with rustc's fixes applied.
///
/// It looks at the settled candidates the run shows it, newest first, and
/// takes the first one that was rejected at the check or the build, that it
/// has not looked at before, and for which rustc has a fix inside the
/// region. Each candidate is looked at once, so a candidate rustc cannot
/// fix costs one rustc check and no more. Its own candidates are looked at
/// like any other: a second error is fixed in a second step.
pub struct RepairProposer {
    diagnose: Diagnose,
    /// Hashes of the candidates already given to rustc.
    examined: HashSet<String>,
    log: Option<RepairLog>,
}

impl RepairProposer {
    pub fn new(diagnose: Diagnose) -> Self {
        Self {
            diagnose,
            examined: HashSet::new(),
            log: None,
        }
    }

    /// Record every repair in `log`.
    pub fn with_log(mut self, log: RepairLog) -> Self {
        self.log = Some(log);
        self
    }
}

impl Proposer for RepairProposer {
    fn name(&self) -> &str {
        "repair"
    }

    fn next(&mut self, nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
        for (text, verdict) in feedback.recent.iter().rev() {
            let failed_to_compile = verdict.status == Status::Rejected
                && matches!(verdict.gate, Gate::Check | Gate::Build);
            if !failed_to_compile || !self.examined.insert(candidate_hash(text)) {
                continue;
            }
            let program = nugget.splice(text);
            let Some(diagnostics) = (self.diagnose)(&program) else {
                continue;
            };
            let start = nugget.prefix.len();
            let region = start..start + text.len();
            if let Some(fixed) = apply_fixes(&program, &diagnostics, region) {
                if let Some(log) = &self.log {
                    log.borrow_mut().push((candidate_hash(text), fixed.clone()));
                }
                return Ok(Proposal::Candidate(fixed));
            }
        }
        // Nothing to repair now. A later rejection may bring something.
        Ok(Proposal::Later)
    }
}

/// One replacement in the program text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edit {
    range: Range<usize>,
    text: String,
}

/// The region of `program` with rustc's fixes applied, or None when rustc
/// offers none that can be used.
///
/// `diagnostics` is rustc's `--error-format=json` output and `region` is
/// the byte range of the candidate inside `program`. A fix is used when it
/// belongs to an error (not a warning), every part of it is
/// `MachineApplicable`, every part lies inside the region, and no part
/// overlaps a fix already taken. A fix of several parts is taken whole or
/// not at all.
pub fn apply_fixes(program: &str, diagnostics: &str, region: Range<usize>) -> Option<String> {
    let mut taken: Vec<Edit> = Vec::new();
    for line in diagnostics.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message["level"] != "error" {
            continue;
        }
        // A fix is a group of spans on the error itself or on one of its
        // children (rustc's `help:` lines).
        let children = message["children"].as_array().cloned().unwrap_or_default();
        for holder in std::iter::once(&message).chain(children.iter()) {
            let Some(group) = fix_of(holder, program, &region) else {
                continue;
            };
            let clashes = group.iter().any(|edit| {
                taken.iter().any(|other| {
                    edit.range.start < other.range.end && other.range.start < edit.range.end
                        || edit.range == other.range
                })
            });
            if !clashes {
                taken.extend(group);
            }
        }
    }
    if taken.is_empty() {
        return None;
    }
    // From the end, so the ranges still to apply do not move.
    taken.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let mut fixed = program.to_string();
    let mut region_end = region.end;
    for edit in &taken {
        fixed.replace_range(edit.range.clone(), &edit.text);
        region_end = region_end + edit.text.len() - edit.range.len();
    }
    let text = fixed[region.start..region_end].to_string();
    if text == program[region.clone()] {
        None
    } else {
        Some(text)
    }
}

/// The edits of one diagnostic's suggestion, or None when it suggests
/// nothing or any part of it cannot be used.
fn fix_of(holder: &serde_json::Value, program: &str, region: &Range<usize>) -> Option<Vec<Edit>> {
    let mut group = Vec::new();
    for span in holder["spans"].as_array()? {
        let Some(text) = span["suggested_replacement"].as_str() else {
            continue;
        };
        if span["suggestion_applicability"] != "MachineApplicable" {
            return None;
        }
        let start = usize::try_from(span["byte_start"].as_u64()?).ok()?;
        let end = usize::try_from(span["byte_end"].as_u64()?).ok()?;
        let inside = region.start <= start && start <= end && end <= region.end;
        if !inside || !program.is_char_boundary(start) || !program.is_char_boundary(end) {
            return None;
        }
        group.push(Edit {
            range: start..end,
            text: text.to_string(),
        });
    }
    if group.is_empty() {
        None
    } else {
        Some(group)
    }
}

/// A [`Diagnose`] that checks the program with rustc, in `dir`.
///
/// rustc only checks it (`--emit=metadata`). The program is written byte
/// for byte, so rustc's byte offsets are offsets into the text given.
pub fn rustc_diagnose(dir: PathBuf) -> Diagnose {
    Box::new(move |program: &str| {
        let rustc = crate::lust::which("rustc")?;
        fs::create_dir_all(&dir).ok()?;
        let source = dir.join("repair.rs");
        fs::write(&source, program).ok()?;
        let mut command = std::process::Command::new(rustc);
        command
            .args(["--edition", "2021", "--error-format=json"])
            .arg("--emit=metadata")
            .arg("-o")
            .arg(dir.join("repair.rmeta"))
            .arg(&source);
        let limits = crate::supervisor::Limits {
            wall: RUSTC_TIMEOUT,
            memory_bytes: None,
            scrub_env: false,
        };
        let ran = crate::supervisor::run_capture(command, &limits, b"").ok()?;
        match ran.kind {
            crate::supervisor::StopKind::Exited(0) => None,
            crate::supervisor::StopKind::Exited(_) => Some(ran.stderr),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nugget::Objective;
    use crate::pipeline::{Ms, Verdict};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn nugget() -> Nugget {
        Nugget {
            name: "region".to_string(),
            objective: Objective::PassAll,
            timeout: Duration::from_millis(1),
            max_memory: 1,
            prefix: "fn main() {\n    let path = \"a/./b\";\n// CRAB:BEGIN\n".to_string(),
            seed: "SEED\n".to_string(),
            suffix: "// CRAB:END\n    println!(\"{}\", parts.len());\n}\n".to_string(),
            region_first_line: 4,
            cases: Vec::new(),
        }
    }

    fn verdict(text: &str, gate: Gate, status: Status) -> Verdict {
        Verdict {
            hash: candidate_hash(text),
            gate,
            status,
            score: None,
            passed: 0,
            total: 1,
            diagnostics: Vec::new(),
            ms: Ms::default(),
            note: String::new(),
            failure: None,
        }
    }

    /// One rustc JSON line: an error with one `help` child that replaces
    /// `start..end` with `text`.
    fn error_with_fix(start: usize, end: usize, text: &str, applicability: &str) -> String {
        serde_json::json!({
            "level": "error",
            "message": "can't compare",
            "spans": [{"byte_start": start, "byte_end": end, "suggested_replacement": null}],
            "children": [{
                "level": "help",
                "message": "consider dereferencing here",
                "spans": [{
                    "byte_start": start,
                    "byte_end": end,
                    "suggested_replacement": text,
                    "suggestion_applicability": applicability,
                }],
                "children": [],
            }],
        })
        .to_string()
    }

    const BAD: &str =
        "    let parts: Vec<&str> = path.split('/').filter(|c| c != \".\").collect();\n";

    #[test]
    fn a_machine_applicable_fix_inside_the_region_is_applied() {
        let nugget = nugget();
        let program = nugget.splice(BAD);
        let region = nugget.prefix.len()..nugget.prefix.len() + BAD.len();
        let at = program.find("c != ").unwrap();
        let json = error_with_fix(at, at, "*", "MachineApplicable");
        assert_eq!(
            apply_fixes(&program, &json, region.clone()).unwrap(),
            "    let parts: Vec<&str> = path.split('/').filter(|c| *c != \".\").collect();\n"
        );
        // A fix rustc is not sure of is left to the model.
        let unsure = error_with_fix(at, at, "*", "MaybeIncorrect");
        assert_eq!(apply_fixes(&program, &unsure, region.clone()), None);
        let placeholder = error_with_fix(at, at, ": Vec<T>", "HasPlaceholders");
        assert_eq!(apply_fixes(&program, &placeholder, region.clone()), None);
        // A fix outside the region would change the nugget, not the candidate.
        let outside = error_with_fix(3, 3, "*", "MachineApplicable");
        assert_eq!(apply_fixes(&program, &outside, region.clone()), None);
        // A warning's fix is not taken, and text that is not JSON is skipped.
        let warning = json.replace("\"level\":\"error\"", "\"level\":\"warning\"");
        assert_eq!(apply_fixes(&program, &warning, region.clone()), None);
        assert_eq!(apply_fixes(&program, "error: aborting\n", region), None);
    }

    #[test]
    fn two_fixes_are_both_applied_and_an_overlapping_one_is_dropped() {
        let nugget = nugget();
        let text = "let a = x != \".\" && y != \"..\";\n";
        let program = nugget.splice(text);
        let region = nugget.prefix.len()..nugget.prefix.len() + text.len();
        let x = program.find("x != ").unwrap();
        let y = program.find("y != ").unwrap();
        let json = format!(
            "{}\n{}\n{}\n",
            error_with_fix(x, x, "*", "MachineApplicable"),
            error_with_fix(y, y, "*", "MachineApplicable"),
            error_with_fix(x, x, "**", "MachineApplicable"),
        );
        assert_eq!(
            apply_fixes(&program, &json, region).unwrap(),
            "let a = *x != \".\" && *y != \"..\";\n"
        );
    }

    #[test]
    fn the_proposer_repairs_a_rejected_candidate_once() {
        let nugget = nugget();
        let asked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&asked);
        let log = RepairLog::default();
        let mut proposer = RepairProposer::new(Box::new(move |program: &str| {
            record.borrow_mut().push(program.to_string());
            let at = program.find("c != ")?;
            Some(error_with_fix(at, at, "*", "MachineApplicable"))
        }))
        .with_log(RepairLog::clone(&log));
        assert_eq!(proposer.name(), "repair");

        // Nothing settled yet, and a candidate that ran: nothing to repair.
        let ran = verdict("RAN\n", Gate::Run, Status::Scored);
        let recent = [("RAN\n", &ran)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), Proposal::Later);
        assert!(asked.borrow().is_empty());

        // A candidate rejected at the check: rustc is asked about the whole
        // program, and the region comes back with the fix.
        let rejected = verdict(BAD, Gate::Check, Status::Rejected);
        let recent = [("RAN\n", &ran), (BAD, &rejected)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        let fixed = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert!(fixed.contains("|c| *c != "), "{fixed}");
        assert_eq!(asked.borrow().len(), 1);
        assert_eq!(asked.borrow()[0], nugget.splice(BAD));

        // The repair is in the log, under the hash of what it repaired.
        assert_eq!(*log.borrow(), vec![(candidate_hash(BAD), fixed.clone())]);

        // The same candidate is not given to rustc a second time.
        assert_eq!(proposer.next(&nugget, &feedback).unwrap(), Proposal::Later);
        assert_eq!(asked.borrow().len(), 1);
    }

    #[test]
    fn rustc_fixes_the_line_that_stopped_a_model_on_normalize_path() {
        if crate::lust::which("rustc").is_none() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("crab-cage-repair-test-{}", std::process::id()));
        let nugget = nugget();
        let mut proposer = RepairProposer::new(rustc_diagnose(dir.clone()));
        let rejected = verdict(BAD, Gate::Check, Status::Rejected);
        let recent = [(BAD, &rejected)];
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &recent,
            best_verdict: None,
        };
        let fixed = proposer
            .next(&nugget, &feedback)
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(
            fixed,
            "    let parts: Vec<&str> = path.split('/').filter(|c| *c != \".\").collect();\n"
        );
        // rustc accepts the repaired program.
        assert!(rustc_diagnose(dir.clone())(&nugget.splice(&fixed)).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
