//! `crab-cage run`: the seed, then candidates from proposers, one journal line each.
//!
//! A candidate whose hash is already in the journal is not evaluated again.
//! When a scored candidate would beat the proven best, it is evaluated a
//! second time with `oracle: true` in the same work directory, and only that
//! verdict is appended. Only a `proven` verdict becomes the best. `diverged`,
//! and a reject whose note is `nondeterministic output`, do not.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{err, Result};
use crate::journal::Journal;
use crate::lust;
use crate::nugget::{self, Nugget, Objective};
use crate::pipeline::{self, EvalOptions, Gate, Status, Toolchain, Verdict};
use crate::proposer::{Feedback, Proposer};

const HOST_MEMORY: u64 = 1024 * 1024 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(180);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Knobs for [`execute`]. The command line's default budget is 200.
pub struct RunOptions {
    pub nugget_dir: PathBuf,
    /// Journaled verdicts to record, including the seed. Zero records nothing.
    pub budget: usize,
    pub out_dir: PathBuf,
    pub lust_bin: Option<PathBuf>,
}

/// Whether the journal contains a proven candidate after the run.
pub struct RunOutput {
    pub proven: bool,
}

struct Best {
    text: String,
    score: f64,
}

struct Outcome {
    recorded: bool,
    stop: bool,
}

/// Evaluate the seed, then ask `proposers` in round-robin until the budget is
/// spent, every proposer returns [`None`](Proposer::next), or a proven
/// `pass_all` candidate scores 0.
///
/// Writes `<out>/journal.jsonl`, and `<out>/best.rs` plus `<out>/best.region.rs`
/// when a proven candidate becomes the best. Prints one line per journaled
/// candidate and then the journal summary.
pub fn execute(options: &RunOptions, proposers: &mut [&mut dyn Proposer]) -> Result<RunOutput> {
    let nugget = nugget::load(&options.nugget_dir)?;
    let lust = match &options.lust_bin {
        Some(path) => Some(lust::probe(path, PROBE_TIMEOUT)?),
        None => None,
    };
    let tools = Toolchain { lust };
    fs::create_dir_all(&options.out_dir)?;
    let journal_path = options.out_dir.join("journal.jsonl");
    let mut journal = Journal::open(&journal_path)?;
    let work_dir = options.out_dir.join("work");

    let mut best: Option<Best> = None;
    let mut last: Option<(String, Verdict)> = None;
    let mut evaluated = 0usize;
    let mut cursor = 0usize;
    let mut exhausted = vec![false; proposers.len()];

    if options.budget > 0 && !journal.seen(&candidate_hash(&nugget.seed)) {
        let outcome = record(
            &nugget,
            &nugget.seed,
            "seed",
            true,
            &tools,
            &work_dir,
            &options.out_dir,
            &mut journal,
            &journal_path,
            &mut best,
            &mut last,
        )?;
        if outcome.recorded {
            evaluated += 1;
        }
        if outcome.stop {
            return finish(&journal_path);
        }
    }

    while evaluated < options.budget {
        let next = {
            let feedback = feedback_of(&best, &last);
            next_candidate(proposers, &mut cursor, &mut exhausted, &nugget, &feedback)?
        };
        let Some((name, text)) = next else {
            break;
        };
        if journal.seen(&candidate_hash(&text)) {
            continue;
        }
        let outcome = record(
            &nugget,
            &text,
            &name,
            false,
            &tools,
            &work_dir,
            &options.out_dir,
            &mut journal,
            &journal_path,
            &mut best,
            &mut last,
        )?;
        if outcome.recorded {
            evaluated += 1;
        }
        if outcome.stop {
            break;
        }
    }

    finish(&journal_path)
}

fn finish(journal_path: &Path) -> Result<RunOutput> {
    let summary = Journal::summary(journal_path)?;
    print!("{summary}");
    Ok(RunOutput {
        proven: summary_has_proven(&summary),
    })
}

/// True when the summary's status line reports a non-zero proven count.
fn summary_has_proven(summary: &str) -> bool {
    for line in summary.lines() {
        let Some(rest) = line.strip_prefix("status ") else {
            continue;
        };
        for part in rest.split_whitespace() {
            if let Some(count) = part.strip_prefix("proven=") {
                return count != "0";
            }
        }
    }
    false
}

fn feedback_of<'a>(best: &'a Option<Best>, last: &'a Option<(String, Verdict)>) -> Feedback<'a> {
    Feedback {
        best: best.as_ref().map(|best| (best.text.as_str(), best.score)),
        last: last
            .as_ref()
            .map(|(text, verdict)| (text.as_str(), verdict)),
    }
}

fn next_candidate(
    proposers: &mut [&mut dyn Proposer],
    cursor: &mut usize,
    exhausted: &mut [bool],
    nugget: &Nugget,
    feedback: &Feedback,
) -> Result<Option<(String, String)>> {
    let len = proposers.len();
    if len == 0 || exhausted.iter().all(|done| *done) {
        return Ok(None);
    }
    for _ in 0..len {
        if *cursor >= len {
            *cursor = 0;
        }
        let index = *cursor;
        *cursor += 1;
        if exhausted[index] {
            continue;
        }
        let name = proposers[index].name().to_string();
        match proposers[index].next(nugget, feedback)? {
            Some(text) => return Ok(Some((name, text))),
            None => exhausted[index] = true,
        }
    }
    Ok(None)
}

fn record(
    nugget: &Nugget,
    candidate: &str,
    proposer: &str,
    seed: bool,
    tools: &Toolchain,
    work_dir: &Path,
    out_dir: &Path,
    journal: &mut Journal,
    journal_path: &Path,
    best: &mut Option<Best>,
    last: &mut Option<(String, Verdict)>,
) -> Result<Outcome> {
    let verdict = match pipeline::evaluate(nugget, candidate, tools, &eval_options(work_dir, false))
    {
        Ok(verdict) => verdict,
        Err(error) => return unevaluated(seed, error),
    };
    // Oracle is the same `evaluate` call with the flag set. Journal the
    // verdict from that call only, never a scored line plus a second line.
    let verdict = if would_improve(&verdict, best) {
        match pipeline::evaluate(nugget, candidate, tools, &eval_options(work_dir, true)) {
            Ok(verdict) => verdict,
            Err(error) => return unevaluated(seed, error),
        }
    } else {
        verdict
    };

    journal.append(&verdict, proposer)?;
    let n = last_journal_n(journal_path)?;
    println!("{}", format_candidate_line(n, &verdict, proposer));
    remember(nugget, candidate, &verdict, best, last, out_dir)?;
    Ok(Outcome {
        recorded: true,
        stop: solved(nugget, &verdict),
    })
}

fn unevaluated(seed: bool, error: crate::error::Error) -> Result<Outcome> {
    if seed {
        return Err(error);
    }
    // Not journaled, so a later run can retry this hash.
    eprintln!("crab-cage: {error}");
    Ok(Outcome {
        recorded: false,
        stop: false,
    })
}

fn eval_options(work_dir: &Path, oracle: bool) -> EvalOptions {
    EvalOptions {
        work_dir: work_dir.to_path_buf(),
        host_memory: HOST_MEMORY,
        compile_timeout: COMPILE_TIMEOUT,
        oracle,
    }
}

fn would_improve(verdict: &Verdict, best: &Option<Best>) -> bool {
    if verdict.status != Status::Scored {
        return false;
    }
    let Some(score) = finite_score(verdict) else {
        return false;
    };
    match best {
        Some(best) => score < best.score,
        None => true,
    }
}

fn remember(
    nugget: &Nugget,
    candidate: &str,
    verdict: &Verdict,
    best: &mut Option<Best>,
    last: &mut Option<(String, Verdict)>,
    out_dir: &Path,
) -> Result<()> {
    if verdict.status == Status::Proven {
        if let Some(score) = finite_score(verdict) {
            let replace = match best {
                Some(current) => score < current.score,
                None => true,
            };
            if replace {
                fs::write(out_dir.join("best.rs"), nugget.splice(candidate))?;
                fs::write(out_dir.join("best.region.rs"), candidate)?;
                *best = Some(Best {
                    text: candidate.to_string(),
                    score,
                });
            }
        }
    }
    *last = Some((candidate.to_string(), verdict.clone()));
    Ok(())
}

fn finite_score(verdict: &Verdict) -> Option<f64> {
    match verdict.score {
        Some(score) if score.is_finite() => Some(score),
        _ => None,
    }
}

fn solved(nugget: &Nugget, verdict: &Verdict) -> bool {
    nugget.objective == Objective::PassAll
        && verdict.status == Status::Proven
        && verdict.score == Some(0.0)
}

fn format_candidate_line(n: u64, verdict: &Verdict, proposer: &str) -> String {
    let score = match verdict.score {
        Some(score) => score.to_string(),
        None => "-".to_string(),
    };
    format!(
        "{n} {} {} {score} {proposer}",
        status_name(verdict.status),
        gate_name(verdict.gate)
    )
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Rejected => "rejected",
        Status::Scored => "scored",
        Status::Proven => "proven",
        Status::Diverged => "diverged",
    }
}

fn gate_name(gate: Gate) -> &'static str {
    match gate {
        Gate::Prefilter => "prefilter",
        Gate::Check => "check",
        Gate::Build => "build",
        Gate::Run => "run",
        Gate::Oracle => "oracle",
    }
}

fn last_journal_n(path: &Path) -> Result<u64> {
    let text = fs::read_to_string(path)?;
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| err(format!("{}: journal has no lines", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(line.trim_end_matches('\r'))
        .map_err(|error| err(format!("{}: {error}", path.display())))?;
    value
        .get("n")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| err(format!("{}: journal line has no n", path.display())))
}

/// FNV-1a 64, 16 lowercase hex digits. `pipeline` keeps this private, and
/// this package does not edit that file. The vectors match its test.
fn candidate_hash(candidate: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in candidate.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;
    use crate::nugget::{Case, Objective};
    use crate::pipeline::{Diagnostic, Gate, Ms, Status, Verdict};
    use crate::proposer::{Feedback, Proposer};

    fn verdict(status: Status, score: Option<f64>) -> Verdict {
        Verdict {
            hash: "aaaaaaaaaaaaaaaa".to_string(),
            gate: Gate::Run,
            status,
            score,
            passed: 0,
            total: 1,
            diagnostics: Vec::<Diagnostic>::new(),
            ms: Ms::default(),
            note: String::new(),
        }
    }

    fn nugget(objective: Objective) -> Nugget {
        Nugget {
            name: "n".to_string(),
            objective,
            timeout: Duration::from_millis(1),
            max_memory: 1,
            prefix: "P\n".to_string(),
            seed: "S\n".to_string(),
            suffix: "E\n".to_string(),
            region_first_line: 2,
            cases: vec![Case {
                id: "only".to_string(),
                input: String::new(),
                expected: None,
            }],
        }
    }

    #[test]
    fn candidate_hash_matches_the_pipeline_vectors() {
        assert_eq!(candidate_hash(""), "cbf29ce484222325");
        assert_eq!(candidate_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(candidate_hash("foobar"), "85944171f73967e8");
    }

    #[test]
    fn only_a_finite_scored_improvement_asks_for_the_oracle() {
        assert!(would_improve(&verdict(Status::Scored, Some(3.0)), &None));
        let best = Some(Best {
            text: "old".to_string(),
            score: 3.0,
        });
        assert!(would_improve(&verdict(Status::Scored, Some(1.0)), &best));
        assert!(!would_improve(&verdict(Status::Scored, Some(3.0)), &best));
        assert!(!would_improve(&verdict(Status::Scored, Some(4.0)), &best));
        assert!(!would_improve(&verdict(Status::Diverged, Some(0.0)), &None));
        assert!(!would_improve(&verdict(Status::Rejected, None), &None));
        assert!(!would_improve(
            &verdict(Status::Scored, Some(f64::NAN)),
            &None
        ));
    }

    #[test]
    fn only_a_proven_improvement_is_written_as_best() {
        let dir = std::env::temp_dir().join(format!("crab-cage-run-best-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let nugget = nugget(Objective::PassAll);
        let mut best = None;
        let mut last = None;

        let mut diverged = verdict(Status::Diverged, Some(0.0));
        diverged.note.clear();
        remember(&nugget, "worse\n", &diverged, &mut best, &mut last, &dir).unwrap();
        assert!(best.is_none());
        assert!(!dir.join("best.rs").exists());

        let mut nondeterministic = verdict(Status::Rejected, Some(0.0));
        nondeterministic.note = "nondeterministic output".to_string();
        remember(
            &nugget,
            "flip\n",
            &nondeterministic,
            &mut best,
            &mut last,
            &dir,
        )
        .unwrap();
        assert!(best.is_none());

        remember(
            &nugget,
            "body\n",
            &verdict(Status::Proven, Some(2.0)),
            &mut best,
            &mut last,
            &dir,
        )
        .unwrap();
        assert_eq!(best.as_ref().map(|best| best.score), Some(2.0));
        assert_eq!(
            fs::read_to_string(dir.join("best.rs")).unwrap(),
            "P\nbody\nE\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("best.region.rs")).unwrap(),
            "body\n"
        );

        remember(
            &nugget,
            "tie\n",
            &verdict(Status::Proven, Some(2.0)),
            &mut best,
            &mut last,
            &dir,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("best.region.rs")).unwrap(),
            "body\n"
        );

        remember(
            &nugget,
            "better\n",
            &verdict(Status::Proven, Some(0.0)),
            &mut best,
            &mut last,
            &dir,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("best.rs")).unwrap(),
            "P\nbetter\nE\n"
        );
        assert_eq!(best.as_ref().map(|best| best.score), Some(0.0));
    }

    #[test]
    fn early_stop_is_a_proven_zero_on_pass_all() {
        let pass_all = nugget(Objective::PassAll);
        let minimize = nugget(Objective::Minimize);
        assert!(solved(&pass_all, &verdict(Status::Proven, Some(0.0))));
        assert!(!solved(&pass_all, &verdict(Status::Proven, Some(1.0))));
        assert!(!solved(&pass_all, &verdict(Status::Diverged, Some(0.0))));
        assert!(!solved(&pass_all, &verdict(Status::Scored, Some(0.0))));
        assert!(!solved(&minimize, &verdict(Status::Proven, Some(0.0))));
    }

    #[test]
    fn a_missing_score_prints_a_dash() {
        let line = format_candidate_line(4, &verdict(Status::Rejected, None), "seed");
        assert_eq!(line, "4 rejected run - seed");
    }

    struct List {
        label: &'static str,
        left: Vec<String>,
    }

    impl Proposer for List {
        fn name(&self) -> &str {
            self.label
        }

        fn next(&mut self, _nugget: &Nugget, _feedback: &Feedback) -> Result<Option<String>> {
            if self.left.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.left.remove(0)))
            }
        }
    }

    #[test]
    fn round_robin_asks_the_next_live_proposer() {
        let nugget = nugget(Objective::PassAll);
        let feedback = Feedback {
            best: None,
            last: None,
        };
        let mut first = List {
            label: "one",
            left: vec!["a".to_string(), "c".to_string()],
        };
        let mut second = List {
            label: "two",
            left: vec!["b".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut first, &mut second];
        let mut cursor = 0usize;
        let mut exhausted = vec![false; 2];
        let mut pull = |proposers: &mut [&mut dyn Proposer]| {
            next_candidate(proposers, &mut cursor, &mut exhausted, &nugget, &feedback).unwrap()
        };
        assert_eq!(
            pull(&mut proposers).unwrap(),
            ("one".to_string(), "a".to_string())
        );
        assert_eq!(
            pull(&mut proposers).unwrap(),
            ("two".to_string(), "b".to_string())
        );
        assert_eq!(
            pull(&mut proposers).unwrap(),
            ("one".to_string(), "c".to_string())
        );
        assert_eq!(pull(&mut proposers), None);
    }
}
