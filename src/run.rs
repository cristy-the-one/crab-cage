//! `crab-cage run`: the seed, then candidates from proposers, one journal line each.
//!
//! A candidate whose hash is already in the journal is not evaluated again.
//! When a scored candidate would beat the proven best, it is evaluated a
//! second time with `oracle: true` in the same work directory, and only that
//! verdict is appended. Only a `proven` verdict becomes the best. `diverged`,
//! and a reject whose note is `nondeterministic output`, do not.
//!
//! A resumed run keeps the lowest proven score already in the journal. The
//! text comes from `best.region.rs` when that file hashes to the same line.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{err, Result};
use crate::journal::Journal;
use crate::lust;
use crate::nugget::{self, Nugget, Objective};
use crate::pipeline::{self, candidate_hash, EvalOptions, Gate, Status, Toolchain, Verdict};
use crate::proposer::{Feedback, LiteralProposer, ModelProposer, Proposal, Proposer};

/// Settled candidates kept for [`Feedback::recent`]. Older ones are dropped.
const RECENT_LIMIT: usize = 32;

const HOST_MEMORY: u64 = 1024 * 1024 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(180);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Proposals that are already in the journal, or whose evaluation returns an
/// error, do not append a line. This many in a row ends the run.
const UNJOURNALED_LIMIT: usize = 50;
/// The tweak proposer's seed without `--seed`. 1 is the evolve command's
/// default seed, so a tweak run repeats on every machine.
const TWEAK_SEED: u64 = 1;
const LOCAL_ENDPOINT: &str = "http://127.0.0.1:11434/v1";
const LOCAL_MODEL: &str = "qwen2.5-coder:14b";

/// Knobs for [`execute`]. The command line's default budget is 200.
pub struct RunOptions {
    pub nugget_dir: PathBuf,
    /// Journaled verdicts to record, including the seed. Zero records nothing.
    pub budget: usize,
    /// Candidates evaluated at the same time. 0 and 1 both mean one.
    pub jobs: usize,
    pub out_dir: PathBuf,
    pub lust_bin: Option<PathBuf>,
    /// Use lust's warm daemon for the length of the run. It is started in
    /// `<out>/work`, every candidate is compiled as `slot.rs` there, and it
    /// is stopped when the run ends. Ignored without `lust_bin`.
    pub lust_daemon: bool,
}

/// Whether this run's journal meets the objective's exit rule.
///
/// `pass_all` needs a proven score of 0. `minimize` needs any proven candidate.
pub struct RunOutput {
    pub solved: bool,
}

struct Best {
    /// Present when `best.region.rs` is the text of `score`'s journal line.
    text: Option<String>,
    score: f64,
    /// The verdict that made it the best. None for a best restored from the
    /// journal.
    verdict: Option<Verdict>,
}

/// Stops the lust daemon started in this directory when it is dropped.
struct DaemonGuard(PathBuf);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        lust::stop_daemon(&self.0);
    }
}

struct Outcome {
    recorded: bool,
    stop: bool,
}

/// Append the literal-tweak proposer when `--tweak` is set. `seed` is
/// `--seed`; without it the proposer uses [`TWEAK_SEED`].
///
/// Proposers already in the list stay ahead of it, so a folder proposer still
/// goes first and the two alternate. `slot` owns the proposer for the rest of
/// the run.
pub fn attach_tweak<'a>(
    enabled: bool,
    seed: Option<u64>,
    slot: &'a mut Option<LiteralProposer>,
    proposers: &mut Vec<&'a mut dyn Proposer>,
) {
    if !enabled {
        return;
    }
    *slot = Some(LiteralProposer::new(seed.unwrap_or(TWEAK_SEED)));
    if let Some(proposer) = slot.as_mut() {
        proposers.push(proposer);
    }
}

/// Append the local-model proposer when `--backend local` is set.
///
/// Proposers already in the list stay ahead of it. `slot` owns the proposer
/// for the rest of the run. The endpoint must be loopback http. The lust
/// capability summary is read once here. It is empty with no `--lust-bin`,
/// and empty when lust runs but the summary command fails. A `--lust-bin`
/// that cannot be probed is an error. `seed` is `--seed`: with it, each
/// request to the model carries a seed.
pub fn attach_model<'a>(
    backend: Option<&str>,
    endpoint: Option<&str>,
    model: Option<&str>,
    lust_bin: Option<&Path>,
    seed: Option<u64>,
    slot: &'a mut Option<ModelProposer>,
    proposers: &mut Vec<&'a mut dyn Proposer>,
) -> Result<()> {
    if backend.is_none() && endpoint.is_none() && model.is_none() {
        return Ok(());
    }
    match backend {
        Some("local") => {}
        Some(other) => {
            return Err(err(format!("unknown backend `{other}` (expected local)")));
        }
        None => {
            return Err(err("--endpoint and --model need --backend local"));
        }
    }
    let endpoint = endpoint.unwrap_or(LOCAL_ENDPOINT);
    let model = model.unwrap_or(LOCAL_MODEL);
    // Reject a non-loopback URL before probing lust.
    crate::llm::normalize_endpoint(endpoint)?;
    let capabilities = capability_text(lust_bin)?;
    // Private to this process: two runs must not write the same file.
    let opinion_dir =
        std::env::temp_dir().join(format!("crab-cage-second-opinion-{}", std::process::id()));
    *slot = Some(
        ModelProposer::new(endpoint, model, capabilities)?
            .with_second_opinion(crate::proposer::rustc_second_opinion(opinion_dir))
            .with_seed(seed),
    );
    if let Some(proposer) = slot.as_mut() {
        proposers.push(proposer);
    }
    Ok(())
}

fn capability_text(lust_bin: Option<&Path>) -> Result<String> {
    let Some(path) = lust_bin else {
        return Ok(String::new());
    };
    let tool = lust::probe(path, PROBE_TIMEOUT)?;
    Ok(lust::capabilities_summary(&tool))
}

/// Evaluate the seed, then ask `proposers` in round-robin until the budget is
/// spent, every proposer is [`Proposal::Finished`], a batch ends empty because
/// every live proposer said [`Proposal::Later`], a proven `pass_all`
/// candidate scores 0, or [`UNJOURNALED_LIMIT`] proposals in a row leave no
/// journal line.
///
/// A proposer that says `Later` is not asked again while the current batch
/// is gathered; it is asked again after the batch settles. `Finished` is for
/// the rest of the run.
///
/// Writes `<out>/journal.jsonl`, and `<out>/best.rs` plus `<out>/best.region.rs`
/// when a proven candidate becomes the best. Prints one line per journaled
/// candidate, a stop line when the unjournaled limit is hit, and then the
/// journal summary.
///
/// [`RunOutput::solved`] is true for `pass_all` only when a proven score is 0,
/// and for `minimize` when any candidate is proven.
pub fn execute(options: &RunOptions, proposers: &mut [&mut dyn Proposer]) -> Result<RunOutput> {
    let nugget = nugget::load(&options.nugget_dir)?;
    let lust = match &options.lust_bin {
        Some(path) => Some(lust::probe(path, PROBE_TIMEOUT)?),
        None => None,
    };
    let mut lust = lust;
    let work_dir = options.out_dir.join("work");
    // Declared before anything that can fail, so the daemon is stopped on
    // every way out of this function, an error included.
    let _daemon = match lust.as_mut() {
        Some(tool) if options.lust_daemon => {
            tool.daemon = true;
            Some(DaemonGuard(work_dir.clone()))
        }
        _ => None,
    };
    let tools = Toolchain { lust };
    fs::create_dir_all(&options.out_dir)?;
    let journal_path = options.out_dir.join("journal.jsonl");
    let mut journal = Journal::open(&journal_path)?;

    let mut best = restored_best(&options.out_dir, &journal_path)?;
    let mut last: Option<(String, Verdict)> = None;
    let mut recent: Vec<(String, Verdict)> = Vec::new();
    let mut evaluated = 0usize;
    let mut unjournaled = 0usize;
    let mut cursor = 0usize;
    let mut exhausted = vec![false; proposers.len()];

    if options.budget > 0 && !journal.seen(&candidate_hash(&nugget.seed)) {
        let first = first_pass(&nugget, &nugget.seed, &tools, &work_dir, 0);
        let outcome = settle(
            &nugget,
            &nugget.seed,
            "seed",
            true,
            first,
            &tools,
            &work_dir,
            &options.out_dir,
            &mut journal,
            &journal_path,
            &mut best,
            &mut last,
            &mut recent,
        )?;
        if outcome.recorded {
            evaluated += 1;
        }
        if outcome.stop {
            return finish(&journal_path, nugget.objective);
        }
    }

    let jobs = options.jobs.max(1);
    let mut done = false;
    while !done && evaluated < options.budget {
        // Gather a batch. Every proposal in it sees the same feedback: the
        // best and the settled verdicts as they were before the batch. The
        // window is the last 32, so a proposer still finds its own verdict
        // when another job settled later and `last` moved on.
        let want = jobs.min(options.budget - evaluated);
        let mut batch: Vec<(String, String)> = Vec::with_capacity(want);
        let mut waiting = vec![false; proposers.len()];
        while batch.len() < want {
            let next = {
                let view: Vec<(&str, &Verdict)> = recent
                    .iter()
                    .map(|(text, verdict)| (text.as_str(), verdict))
                    .collect();
                let feedback = feedback_of(&best, &last, &view);
                next_candidate(
                    proposers,
                    &mut cursor,
                    &mut exhausted,
                    &mut waiting,
                    &nugget,
                    &feedback,
                )?
            };
            let Some((name, text)) = next else {
                // Nobody has more for this batch. With a proposer only
                // waiting, the batch is evaluated and it is asked again; an
                // empty batch ends the run below.
                if exhausted.iter().all(|finished| *finished) {
                    done = true;
                }
                break;
            };
            let in_batch = batch.iter().any(|(_, other)| *other == text);
            let seen = journal.seen(&candidate_hash(&text));
            if in_batch || seen {
                if note_unjournaled(&mut unjournaled) {
                    done = true;
                    break;
                }
                // The same region is already waiting in this batch. Asking
                // again repeats the prompt, and a model at temperature 0.2
                // answers it the same way. Evaluate what we have.
                if in_batch {
                    break;
                }
                continue;
            }
            batch.push((name, text));
        }
        if batch.is_empty() {
            break;
        }

        // The first pass (no oracle) runs one candidate per slot at the same
        // time. Everything after it is in proposal order on this thread.
        let firsts = first_passes(&nugget, &batch, &tools, &work_dir);
        for ((name, text), first) in batch.iter().zip(firsts) {
            let outcome = settle(
                &nugget,
                text,
                name,
                false,
                first,
                &tools,
                &work_dir,
                &options.out_dir,
                &mut journal,
                &journal_path,
                &mut best,
                &mut last,
                &mut recent,
            )?;
            if outcome.recorded {
                evaluated += 1;
                unjournaled = 0;
            } else if note_unjournaled(&mut unjournaled) {
                done = true;
            }
            if outcome.stop {
                // Solved. The rest of the batch is dropped unjournaled, as
                // if it had never been proposed.
                done = true;
                break;
            }
        }
    }

    finish(&journal_path, nugget.objective)
}

/// One candidate through the funnel without the oracle, on `slot`.
fn first_pass(
    nugget: &Nugget,
    candidate: &str,
    tools: &Toolchain,
    work_dir: &Path,
    slot: usize,
) -> Result<Verdict> {
    pipeline::evaluate(
        nugget,
        candidate,
        tools,
        &eval_options(work_dir, false, slot),
    )
}

/// The first pass of every candidate in `batch`, each on its own slot and
/// its own thread. The results are in batch order.
fn first_passes(
    nugget: &Nugget,
    batch: &[(String, String)],
    tools: &Toolchain,
    work_dir: &Path,
) -> Vec<Result<Verdict>> {
    if batch.len() == 1 {
        return vec![first_pass(nugget, &batch[0].1, tools, work_dir, 0)];
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = batch
            .iter()
            .enumerate()
            .map(|(slot, (_, text))| {
                scope.spawn(move || first_pass(nugget, text, tools, work_dir, slot))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(err("an evaluation thread panicked")))
            })
            .collect()
    })
}

fn note_unjournaled(count: &mut usize) -> bool {
    *count += 1;
    if *count >= UNJOURNALED_LIMIT {
        crate::console::line(&unjournaled_stop_line());
        true
    } else {
        false
    }
}

fn unjournaled_stop_line() -> String {
    format!("stopped: {UNJOURNALED_LIMIT} proposals in a row left no journal line")
}

fn finish(journal_path: &Path, objective: Objective) -> Result<RunOutput> {
    let summary = Journal::summary(journal_path)?;
    crate::console::print(&summary);
    let proven = scan_proven(journal_path)?;
    Ok(RunOutput {
        solved: exit_solved(objective, &proven),
    })
}

fn exit_solved(objective: Objective, proven: &ProvenScan) -> bool {
    match objective {
        Objective::PassAll => proven.zero,
        Objective::Minimize | Objective::Fastest => proven.any,
    }
}

struct ProvenScan {
    lowest_score: Option<f64>,
    lowest_hash: Option<String>,
    any: bool,
    zero: bool,
}

/// Lowest finite `proven` score in the journal. A tie keeps the earlier line.
/// A truncated final line is ignored, matching [`Journal::open`].
fn scan_proven(path: &Path) -> Result<ProvenScan> {
    let text = if path.is_file() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let ends_with_newline = text.ends_with('\n');
    let segments: Vec<&str> = text.split('\n').collect();
    let mut scan = ProvenScan {
        lowest_score: None,
        lowest_hash: None,
        any: false,
        zero: false,
    };
    for (index, raw) in segments.iter().enumerate() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let is_last = index + 1 == segments.len();
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) if is_last && !ends_with_newline => continue,
            Err(error) => {
                return Err(err(format!(
                    "{}: journal line {}: {error}",
                    path.display(),
                    index + 1
                )));
            }
        };
        if value.get("status").and_then(|status| status.as_str()) != Some("proven") {
            continue;
        }
        scan.any = true;
        let Some(score) = value.get("score").and_then(serde_json::Value::as_f64) else {
            continue;
        };
        if !score.is_finite() {
            continue;
        }
        if score == 0.0 {
            scan.zero = true;
        }
        let Some(hash) = value.get("hash").and_then(|hash| hash.as_str()) else {
            continue;
        };
        let take = match scan.lowest_score {
            Some(best) => score < best,
            None => true,
        };
        if take {
            scan.lowest_score = Some(score);
            scan.lowest_hash = Some(hash.to_string());
        }
    }
    Ok(scan)
}

fn restored_best(out_dir: &Path, journal_path: &Path) -> Result<Option<Best>> {
    let scan = scan_proven(journal_path)?;
    let (Some(score), Some(hash)) = (scan.lowest_score, scan.lowest_hash) else {
        return Ok(None);
    };
    let region = out_dir.join("best.region.rs");
    let text = if region.is_file() {
        let body = fs::read_to_string(&region)
            .map_err(|error| err(format!("{}: {error}", region.display())))?;
        if candidate_hash(&body) == hash {
            Some(body)
        } else {
            None
        }
    } else {
        None
    };
    Ok(Some(Best {
        text,
        score,
        verdict: None,
    }))
}

fn feedback_of<'a>(
    best: &'a Option<Best>,
    last: &'a Option<(String, Verdict)>,
    recent: &'a [(&'a str, &'a Verdict)],
) -> Feedback<'a> {
    Feedback {
        best: best
            .as_ref()
            .and_then(|best| best.text.as_deref().map(|text| (text, best.score))),
        last: last
            .as_ref()
            .map(|(text, verdict)| (text.as_str(), verdict)),
        recent,
        best_verdict: best
            .as_ref()
            .filter(|best| best.text.is_some())
            .and_then(|best| best.verdict.as_ref()),
    }
}

fn push_recent(recent: &mut Vec<(String, Verdict)>, candidate: &str, verdict: &Verdict) {
    recent.push((candidate.to_string(), verdict.clone()));
    if recent.len() > RECENT_LIMIT {
        recent.drain(0..recent.len() - RECENT_LIMIT);
    }
}

/// The next candidate in round-robin order, skipping proposers that are
/// `exhausted` (finished for the run) or `waiting` (said
/// [`Proposal::Later`] in this batch). None when no proposer has one now.
fn next_candidate(
    proposers: &mut [&mut dyn Proposer],
    cursor: &mut usize,
    exhausted: &mut [bool],
    waiting: &mut [bool],
    nugget: &Nugget,
    feedback: &Feedback,
) -> Result<Option<(String, String)>> {
    let len = proposers.len();
    for _ in 0..len {
        if *cursor >= len {
            *cursor = 0;
        }
        let index = *cursor;
        *cursor += 1;
        if exhausted[index] || waiting[index] {
            continue;
        }
        let name = proposers[index].name().to_string();
        match proposers[index].next(nugget, feedback)? {
            Proposal::Candidate(text) => return Ok(Some((name, text))),
            Proposal::Later => waiting[index] = true,
            Proposal::Finished => exhausted[index] = true,
        }
    }
    Ok(None)
}

/// Finish one candidate whose first pass is `first`: the oracle if it would
/// be a new best, then the journal line, the printed line, and the best files.
#[allow(clippy::too_many_arguments)]
fn settle(
    nugget: &Nugget,
    candidate: &str,
    proposer: &str,
    seed: bool,
    first: Result<Verdict>,
    tools: &Toolchain,
    work_dir: &Path,
    out_dir: &Path,
    journal: &mut Journal,
    journal_path: &Path,
    best: &mut Option<Best>,
    last: &mut Option<(String, Verdict)>,
    recent: &mut Vec<(String, Verdict)>,
) -> Result<Outcome> {
    let verdict = match first {
        Ok(verdict) => verdict,
        Err(error) => return unevaluated(seed, error),
    };
    // Oracle is the same `evaluate` call with the flag set. Journal the
    // verdict from that call only, never a scored line plus a second line.
    let verdict = if would_improve(&verdict, best) {
        match pipeline::evaluate(nugget, candidate, tools, &eval_options(work_dir, true, 0)) {
            Ok(verdict) => verdict,
            Err(error) => return unevaluated(seed, error),
        }
    } else {
        verdict
    };

    journal.append(&verdict, proposer)?;
    let n = last_journal_n(journal_path)?;
    crate::console::line(&format_candidate_line(n, &verdict, proposer));
    remember(nugget, candidate, &verdict, best, last, recent, out_dir)?;
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

fn eval_options(work_dir: &Path, oracle: bool, slot: usize) -> EvalOptions {
    EvalOptions {
        work_dir: work_dir.to_path_buf(),
        host_memory: HOST_MEMORY,
        compile_timeout: COMPILE_TIMEOUT,
        oracle,
        slot,
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
    recent: &mut Vec<(String, Verdict)>,
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
                    text: Some(candidate.to_string()),
                    score,
                    verdict: Some(verdict.clone()),
                });
            }
        }
    }
    *last = Some((candidate.to_string(), verdict.clone()));
    push_recent(recent, candidate, verdict);
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;
    use crate::nugget::{Case, Objective};
    use crate::pipeline::{Diagnostic, Gate, Ms, Status, Verdict};
    use crate::proposer::{Feedback, Proposal, Proposer};

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
            failure: None,
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
    fn only_a_finite_scored_improvement_asks_for_the_oracle() {
        assert!(would_improve(&verdict(Status::Scored, Some(3.0)), &None));
        let best = Some(Best {
            text: Some("old".to_string()),
            score: 3.0,
            verdict: None,
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
        remember(
            &nugget,
            "worse\n",
            &diverged,
            &mut best,
            &mut last,
            &mut Vec::new(),
            &dir,
        )
        .unwrap();
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
            &mut Vec::new(),
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
            &mut Vec::new(),
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
            &mut Vec::new(),
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
            &mut Vec::new(),
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

    #[test]
    fn exit_follows_the_objective() {
        let none = ProvenScan {
            lowest_score: None,
            lowest_hash: None,
            any: false,
            zero: false,
        };
        let nonzero = ProvenScan {
            lowest_score: Some(1.0),
            lowest_hash: Some("h".to_string()),
            any: true,
            zero: false,
        };
        let zero = ProvenScan {
            lowest_score: Some(0.0),
            lowest_hash: Some("h".to_string()),
            any: true,
            zero: true,
        };
        assert!(!exit_solved(Objective::PassAll, &none));
        assert!(!exit_solved(Objective::PassAll, &nonzero));
        assert!(exit_solved(Objective::PassAll, &zero));
        assert!(!exit_solved(Objective::Minimize, &none));
        assert!(exit_solved(Objective::Minimize, &nonzero));
        assert!(exit_solved(Objective::Minimize, &zero));
    }

    #[test]
    fn a_resumed_best_keeps_its_score_when_the_region_file_matches() {
        let dir =
            std::env::temp_dir().join(format!("crab-cage-run-restore-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let kept = "keep\n";
        let hash = candidate_hash(kept);
        let body = format!(
            "{}\n{}\n{}\n",
            journal_line(&hash, "proven", "1"),
            journal_line("bbbbbbbbbbbbbbbb", "proven", "4"),
            journal_line("cccccccccccccccc", "diverged", "0")
        );
        let journal = dir.join("journal.jsonl");
        fs::write(&journal, body).unwrap();
        fs::write(dir.join("best.region.rs"), kept).unwrap();
        fs::write(dir.join("best.rs"), "old\n").unwrap();

        let best = restored_best(&dir, &journal).unwrap().unwrap();
        assert_eq!(best.score, 1.0);
        assert_eq!(best.text.as_deref(), Some(kept));

        fs::write(dir.join("best.region.rs"), "other\n").unwrap();
        let best = restored_best(&dir, &journal).unwrap().unwrap();
        assert_eq!(best.score, 1.0);
        assert_eq!(best.text, None);

        let nugget = nugget(Objective::PassAll);
        let mut stored = Some(best);
        let mut last = None;
        remember(
            &nugget,
            "worse\n",
            &verdict(Status::Proven, Some(2.0)),
            &mut stored,
            &mut last,
            &mut Vec::new(),
            &dir,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(dir.join("best.rs")).unwrap(), "old\n");
        assert_eq!(stored.as_ref().map(|best| best.score), Some(1.0));
    }

    fn journal_line(hash: &str, status: &str, score: &str) -> String {
        format!(r#"{{"hash":"{hash}","status":"{status}","score":{score}}}"#)
    }

    struct List {
        label: &'static str,
        left: Vec<String>,
    }

    impl Proposer for List {
        fn name(&self) -> &str {
            self.label
        }

        fn next(&mut self, _nugget: &Nugget, _feedback: &Feedback) -> Result<Proposal> {
            if self.left.is_empty() {
                Ok(Proposal::Finished)
            } else {
                Ok(Proposal::Candidate(self.left.remove(0)))
            }
        }
    }

    #[test]
    fn round_robin_asks_the_next_live_proposer() {
        let nugget = nugget(Objective::PassAll);
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &[],
            best_verdict: None,
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
        let mut waiting = vec![false; 2];
        let mut pull = |proposers: &mut [&mut dyn Proposer]| {
            next_candidate(
                proposers,
                &mut cursor,
                &mut exhausted,
                &mut waiting,
                &nugget,
                &feedback,
            )
            .unwrap()
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

    #[test]
    fn tweak_joins_the_round_robin_after_the_proposers_already_there() {
        let mut folder = List {
            label: "folder",
            left: vec!["a".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut folder];
        let mut slot = None;
        attach_tweak(false, None, &mut slot, &mut proposers);
        assert_eq!(proposers.len(), 1);
        assert_eq!(proposers[0].name(), "folder");

        let mut folder = List {
            label: "folder",
            left: vec!["a".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut folder];
        let mut slot = None;
        attach_tweak(true, None, &mut slot, &mut proposers);
        assert_eq!(proposers.len(), 2);
        assert_eq!(proposers[0].name(), "folder");
        assert_eq!(proposers[1].name(), "tweak");
    }

    #[test]
    fn the_local_model_joins_after_the_proposers_already_there() {
        let mut folder = List {
            label: "folder",
            left: vec!["a".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut folder];
        let mut slot = None;
        attach_model(None, None, None, None, None, &mut slot, &mut proposers).unwrap();
        assert_eq!(proposers.len(), 1);
        assert!(slot.is_none());

        let mut folder = List {
            label: "folder",
            left: vec!["a".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut folder];
        let mut slot = None;
        attach_model(
            Some("local"),
            None,
            Some("demo"),
            None,
            None,
            &mut slot,
            &mut proposers,
        )
        .unwrap();
        assert_eq!(proposers.len(), 2);
        assert_eq!(proposers[0].name(), "folder");
        assert_eq!(proposers[1].name(), "local");

        let mut slot = None;
        let mut proposers: Vec<&mut dyn Proposer> = Vec::new();
        let refused = attach_model(
            Some("local"),
            Some("http://api.openai.com/v1"),
            None,
            None,
            None,
            &mut slot,
            &mut proposers,
        )
        .unwrap_err();
        assert!(refused.to_string().contains("refusing"), "{refused}");
        drop(proposers);
        assert!(slot.is_none());

        let mut slot = None;
        let mut proposers: Vec<&mut dyn Proposer> = Vec::new();
        let unknown = attach_model(
            Some("mock"),
            None,
            None,
            None,
            None,
            &mut slot,
            &mut proposers,
        )
        .unwrap_err();
        assert!(unknown.to_string().contains("unknown backend"), "{unknown}");

        let mut slot = None;
        let mut proposers: Vec<&mut dyn Proposer> = Vec::new();
        let needs_backend = attach_model(
            None,
            Some("http://127.0.0.1:11434/v1"),
            None,
            None,
            None,
            &mut slot,
            &mut proposers,
        )
        .unwrap_err();
        assert!(
            needs_backend.to_string().contains("--backend local"),
            "{needs_backend}"
        );
    }

    #[test]
    fn feedback_keeps_the_last_32_settled_candidates() {
        let mut recent = Vec::new();
        let sample = verdict(Status::Scored, Some(1.0));
        for index in 0..33 {
            push_recent(&mut recent, &format!("c{index}"), &sample);
        }
        assert_eq!(recent.len(), 32);
        assert_eq!(recent[0].0, "c1");
        assert_eq!(recent[31].0, "c32");
        let view: Vec<(&str, &Verdict)> = recent
            .iter()
            .map(|(text, verdict)| (text.as_str(), verdict))
            .collect();
        let feedback = feedback_of(&None, &None, &view);
        assert_eq!(feedback.recent.len(), 32);
        assert_eq!(feedback.recent[0].0, "c1");
        assert_eq!(feedback.recent[31].0, "c32");
        assert!(feedback.last.is_none());
    }

    /// Returns one fixed region. `third_saw_own` is set when the third call
    /// already sees that region in [`Feedback::recent`], which is only true
    /// if the duplicate ended the batch and the region was settled.
    struct SameRegion {
        calls: usize,
        third_saw_own: bool,
    }

    impl Proposer for SameRegion {
        fn name(&self) -> &str {
            "same"
        }

        fn next(&mut self, _nugget: &Nugget, feedback: &Feedback) -> Result<Proposal> {
            self.calls += 1;
            if self.calls == 3 {
                self.third_saw_own = feedback
                    .recent
                    .iter()
                    .any(|(text, _)| text.contains("std::fs"));
            }
            Ok(Proposal::Candidate(
                "    std::fs::read(\"a\");\n".to_string(),
            ))
        }
    }

    #[test]
    fn a_duplicate_in_the_batch_is_settled_before_the_next_ask() {
        let out =
            std::env::temp_dir().join(format!("crab-cage-run-batch-dup-{}", std::process::id()));
        if out.exists() {
            fs::remove_dir_all(&out).unwrap();
        }
        fs::create_dir_all(&out).unwrap();
        let mut same = SameRegion {
            calls: 0,
            third_saw_own: false,
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut same];
        execute(
            &RunOptions {
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
        assert!(same.third_saw_own, "calls {}", same.calls);
        assert!(same.calls < 60, "calls {}", same.calls);
    }

    #[test]
    fn feedback_carries_the_best_verdict_only_with_the_best_text() {
        let dir =
            std::env::temp_dir().join(format!("crab-cage-run-best-verdict-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let mut best = None;
        let mut last = None;
        let mut proven = verdict(Status::Proven, Some(1.0));
        proven.note = "THE_BEST".to_string();
        remember(
            &nugget(Objective::PassAll),
            "body\n",
            &proven,
            &mut best,
            &mut last,
            &mut Vec::new(),
            &dir,
        )
        .unwrap();
        let feedback = feedback_of(&best, &last, &[]);
        assert_eq!(feedback.best, Some(("body\n", 1.0)));
        assert_eq!(
            feedback.best_verdict.map(|v| v.note.as_str()),
            Some("THE_BEST")
        );

        let restored = Some(Best {
            text: None,
            score: 1.0,
            verdict: Some(proven.clone()),
        });
        let feedback = feedback_of(&restored, &None, &[]);
        assert!(feedback.best.is_none());
        assert!(feedback.best_verdict.is_none());
    }

    /// Gives one region, then says `Later` on every call.
    struct OnceThenLater {
        calls: usize,
    }

    impl Proposer for OnceThenLater {
        fn name(&self) -> &str {
            "later"
        }

        fn next(&mut self, _nugget: &Nugget, _feedback: &Feedback) -> Result<Proposal> {
            self.calls += 1;
            if self.calls == 1 {
                Ok(Proposal::Candidate(
                    "    std::fs::read(\"a\");\n".to_string(),
                ))
            } else {
                Ok(Proposal::Later)
            }
        }
    }

    #[test]
    fn later_is_asked_again_after_the_batch_and_an_empty_batch_ends_the_run() {
        let nugget = nugget(Objective::PassAll);
        let feedback = Feedback {
            best: None,
            last: None,
            recent: &[],
            best_verdict: None,
        };
        let mut waiter = OnceThenLater { calls: 1 };
        let mut list = List {
            label: "list",
            left: vec!["a".to_string(), "b".to_string()],
        };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut waiter, &mut list];
        let mut cursor = 0usize;
        let mut exhausted = vec![false; 2];
        let mut waiting = vec![false; 2];
        let mut pull = |proposers: &mut [&mut dyn Proposer], waiting: &mut [bool]| {
            next_candidate(
                proposers,
                &mut cursor,
                &mut exhausted,
                waiting,
                &nugget,
                &feedback,
            )
            .unwrap()
        };
        // The waiter says Later and is skipped for the rest of the batch.
        assert_eq!(
            pull(&mut proposers, &mut waiting),
            Some(("list".to_string(), "a".to_string()))
        );
        assert_eq!(
            pull(&mut proposers, &mut waiting),
            Some(("list".to_string(), "b".to_string()))
        );
        assert_eq!(pull(&mut proposers, &mut waiting), None);
        assert_eq!(waiting, vec![true, false]);
        // A new batch asks the waiter again.
        let mut waiting = vec![false; 2];
        assert_eq!(pull(&mut proposers, &mut waiting), None);
        drop(proposers);
        assert_eq!(waiter.calls, 3);

        let out =
            std::env::temp_dir().join(format!("crab-cage-run-later-end-{}", std::process::id()));
        if out.exists() {
            fs::remove_dir_all(&out).unwrap();
        }
        fs::create_dir_all(&out).unwrap();
        let mut waiter = OnceThenLater { calls: 0 };
        let mut proposers: Vec<&mut dyn Proposer> = vec![&mut waiter];
        execute(
            &RunOptions {
                nugget_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("nuggets")
                    .join("sum-squares"),
                budget: 20,
                jobs: 3,
                out_dir: out.clone(),
                lust_bin: None,
                lust_daemon: false,
            },
            &mut proposers,
        )
        .unwrap();
        // Seed, the one region, then a batch where the only proposer waits.
        assert_eq!(waiter.calls, 3);
        let journal = fs::read_to_string(out.join("journal.jsonl")).unwrap();
        assert_eq!(journal.lines().count(), 2, "{journal}");
    }
}
