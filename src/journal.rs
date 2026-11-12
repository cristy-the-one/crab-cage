//! Append-only journal of one verdict per line, used to skip a hash already seen.
//!
//! Each line is the verdict object plus `"proposer"` and `"n"`. `n` starts at 1.
//! Opening an existing file continues `n` after the records already there.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use serde::Deserialize;

use crate::error::{err, Result};
use crate::pipeline::{Gate, Status, Verdict};

/// File handle plus the hashes already written.
pub struct Journal {
    file: File,
    seen: HashSet<String>,
    next_n: u64,
    /// The last record parsed and the file did not end in a newline. The next
    /// append writes that newline first so the new object stays on its own line.
    needs_leading_newline: bool,
    /// Byte length of the valid prefix when a crash left a partial last line.
    /// The next append drops that fragment before writing.
    partial_tail: Option<u64>,
}

impl Journal {
    /// Create `path` if it is missing. An existing file is not truncated;
    /// its hashes are loaded.
    pub fn open(path: &Path) -> Result<Journal> {
        let text = if path.is_file() {
            fs::read_to_string(path)?
        } else {
            String::new()
        };
        let loaded = parse_records(path, &text)?;
        let mut seen = HashSet::with_capacity(loaded.records.len());
        for record in &loaded.records {
            seen.insert(record.hash.clone());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        file.seek(SeekFrom::End(0))?;
        Ok(Journal {
            file,
            seen,
            next_n: following_n(loaded.records.len())?,
            needs_leading_newline: loaded.partial_tail.is_none()
                && !text.is_empty()
                && !text.ends_with('\n'),
            partial_tail: loaded.partial_tail,
        })
    }

    /// True when a line in this journal already has `hash`.
    pub fn seen(&self, hash: &str) -> bool {
        self.seen.contains(hash)
    }

    /// Write one JSON line — the verdict plus `proposer` and `n` — and flush.
    pub fn append(&mut self, verdict: &Verdict, proposer: &str) -> Result<()> {
        let n = self.next_n;
        let next = n
            .checked_add(1)
            .ok_or_else(|| err("journal line count overflow"))?;
        if let Some(len) = self.partial_tail {
            self.file.set_len(len)?;
            self.file.seek(SeekFrom::Start(len))?;
            self.partial_tail = None;
            self.needs_leading_newline = false;
        }
        let mut bytes = Vec::new();
        if self.needs_leading_newline {
            bytes.push(b'\n');
        }
        bytes.extend(encode_line(verdict, proposer, n)?);
        self.file.write_all(&bytes)?;
        self.needs_leading_newline = false;
        self.file.flush()?;
        self.seen.insert(verdict.hash.clone());
        self.next_n = next;
        Ok(())
    }

    /// Count the journal at `path`.
    ///
    /// ```text
    /// candidates <total>
    /// status rejected=<n> scored=<n> proven=<n> diverged=<n>
    /// rejected prefilter=<n> check=<n> build=<n> run=<n> oracle=<n>
    /// codes <code>=<n> ...
    /// best <score> <hash>
    /// ```
    ///
    /// `rejected` counts gates only for rejected verdicts. `codes` is the ten
    /// most frequent diagnostic codes (each diagnostic counts once); equal
    /// counts are ordered by the code. `best` is the lowest finite score among
    /// `scored` and `proven` lines; a tie keeps the earlier line. A `diverged`
    /// or `rejected` score is not a result. `codes none` and `best none` are
    /// used when there is nothing to report.
    ///
    /// A final line with no trailing newline that is not valid JSON is a
    /// crash fragment: it is skipped here and in [`Journal::open`]. A bad
    /// line anywhere else is an error.
    pub fn summary(path: &Path) -> Result<String> {
        let text = fs::read_to_string(path)?;
        let loaded = parse_records(path, &text)?;
        Ok(render_summary(&loaded.records))
    }
}

#[derive(Deserialize)]
struct Record {
    hash: String,
    gate: Gate,
    status: Status,
    score: Option<f64>,
    diagnostics: Vec<Code>,
    #[serde(rename = "proposer")]
    _proposer: String,
    #[serde(rename = "n")]
    _n: u64,
}

#[derive(Deserialize)]
struct Code {
    code: String,
}

struct Loaded {
    records: Vec<Record>,
    /// Start of an unparsable last line that has no trailing newline.
    partial_tail: Option<u64>,
}

fn parse_records(path: &Path, text: &str) -> Result<Loaded> {
    let ends_with_newline = text.ends_with('\n');
    let segments: Vec<&str> = text.split('\n').collect();
    let mut records = Vec::new();
    let mut partial_tail = None;
    for (index, raw) in segments.iter().enumerate() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let is_last = index + 1 == segments.len();
        match serde_json::from_str::<Record>(line) {
            Ok(record) => records.push(record),
            Err(source) => {
                if is_last && !ends_with_newline {
                    partial_tail = Some(tail_offset(text)?);
                    continue;
                }
                return Err(err(format!(
                    "{}: journal line {}: {source}",
                    path.display(),
                    index + 1
                )));
            }
        }
    }
    Ok(Loaded {
        records,
        partial_tail,
    })
}

fn tail_offset(text: &str) -> Result<u64> {
    let pos = text.rfind('\n').map(|index| index + 1).unwrap_or(0);
    u64::try_from(pos).map_err(|_| err("journal is too large"))
}

fn following_n(count: usize) -> Result<u64> {
    let count = u64::try_from(count).map_err(|_| err("journal has too many lines"))?;
    count
        .checked_add(1)
        .ok_or_else(|| err("journal has too many lines"))
}

fn encode_line(verdict: &Verdict, proposer: &str, n: u64) -> Result<Vec<u8>> {
    let mut value = serde_json::to_value(verdict)?;
    let Some(object) = value.as_object_mut() else {
        return Err(err("verdict JSON was not an object"));
    };
    object.insert(
        "proposer".to_string(),
        serde_json::Value::String(proposer.to_string()),
    );
    object.insert("n".to_string(), serde_json::Value::from(n));
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn render_summary(records: &[Record]) -> String {
    let mut rejected = 0u64;
    let mut scored = 0u64;
    let mut proven = 0u64;
    let mut diverged = 0u64;
    let mut prefilter = 0u64;
    let mut check = 0u64;
    let mut build = 0u64;
    let mut run = 0u64;
    let mut oracle = 0u64;
    let mut codes: HashMap<&str, u64> = HashMap::new();
    let mut best: Option<(f64, &str)> = None;

    for record in records {
        match record.status {
            Status::Rejected => {
                rejected += 1;
                match record.gate {
                    Gate::Prefilter => prefilter += 1,
                    Gate::Check => check += 1,
                    Gate::Build => build += 1,
                    Gate::Run => run += 1,
                    Gate::Oracle => oracle += 1,
                }
            }
            Status::Scored => scored += 1,
            Status::Proven => proven += 1,
            Status::Diverged => diverged += 1,
        }
        for diagnostic in &record.diagnostics {
            *codes.entry(diagnostic.code.as_str()).or_default() += 1;
        }
        if !matches!(record.status, Status::Scored | Status::Proven) {
            continue;
        }
        if let Some(score) = record.score {
            if !score.is_finite() {
                continue;
            }
            let take = match best {
                Some((best_score, _)) => score < best_score,
                None => true,
            };
            if take {
                best = Some((score, record.hash.as_str()));
            }
        }
    }

    let mut ranked: Vec<(&str, u64)> = codes.into_iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    ranked.truncate(10);
    let mut code_text = String::new();
    if ranked.is_empty() {
        code_text.push_str("none");
    } else {
        for (index, (code, count)) in ranked.iter().enumerate() {
            if index > 0 {
                code_text.push(' ');
            }
            code_text.push_str(code);
            code_text.push('=');
            code_text.push_str(&count.to_string());
        }
    }
    let best_text = match best {
        Some((score, hash)) => format!("{score} {hash}"),
        None => "none".to_string(),
    };
    format!(
        "candidates {total}\nstatus rejected={rejected} scored={scored} proven={proven} diverged={diverged}\nrejected prefilter={prefilter} check={check} build={build} run={run} oracle={oracle}\ncodes {code_text}\nbest {best_text}\n",
        total = records.len(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;
    use crate::pipeline::{Diagnostic, Gate, Ms, Status, Verdict};

    fn journal_path(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("crab-cage-journal-{label}-{}", std::process::id()));
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        path
    }

    fn verdict(
        hash: &str,
        gate: Gate,
        status: Status,
        score: Option<f64>,
        codes: &[&str],
    ) -> Verdict {
        Verdict {
            hash: hash.to_string(),
            gate,
            status,
            score,
            passed: 0,
            total: 3,
            diagnostics: codes
                .iter()
                .map(|code| Diagnostic {
                    code: (*code).to_string(),
                    message: "m".into(),
                    line: 1,
                    col: 1,
                    fix: String::new(),
                })
                .collect(),
            ms: Ms {
                check: 31,
                build: 0,
                run: 0,
                oracle: 0,
                opt: 0,
            },
            note: String::new(),
            failure: None,
        }
    }

    #[test]
    fn append_two_verdicts_reopen_and_both_are_seen() {
        let path = journal_path("append");
        let mut journal = Journal::open(&path).unwrap();
        assert!(path.is_file());
        assert!(!journal.seen("aaaaaaaaaaaaaaaa"));
        assert!(!journal.seen("bbbbbbbbbbbbbbbb"));

        let mut first = verdict(
            "aaaaaaaaaaaaaaaa",
            Gate::Check,
            Status::Rejected,
            None,
            &["L0110"],
        );
        first.note = "nope".to_string();
        journal.append(&first, "seed").unwrap();
        journal
            .append(
                &verdict(
                    "bbbbbbbbbbbbbbbb",
                    Gate::Run,
                    Status::Scored,
                    Some(2.5),
                    &[],
                ),
                "folder",
            )
            .unwrap();
        assert!(journal.seen("aaaaaaaaaaaaaaaa"));
        assert!(journal.seen("bbbbbbbbbbbbbbbb"));

        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let first_line: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first_line["hash"], "aaaaaaaaaaaaaaaa");
        assert_eq!(first_line["gate"], "check");
        assert_eq!(first_line["status"], "rejected");
        assert!(first_line["score"].is_null());
        assert_eq!(first_line["diagnostics"][0]["code"], "L0110");
        assert_eq!(first_line["ms"]["check"], 31);
        assert_eq!(first_line["note"], "nope");
        assert_eq!(first_line["proposer"], "seed");
        assert_eq!(first_line["n"], 1);
        let second_line: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second_line["hash"], "bbbbbbbbbbbbbbbb");
        assert_eq!(second_line["status"], "scored");
        assert_eq!(second_line["score"], 2.5);
        assert_eq!(second_line["proposer"], "folder");
        assert_eq!(second_line["n"], 2);

        drop(journal);
        let mut reopened = Journal::open(&path).unwrap();
        assert!(reopened.seen("aaaaaaaaaaaaaaaa"));
        assert!(reopened.seen("bbbbbbbbbbbbbbbb"));
        assert!(!reopened.seen("cccccccccccccccc"));
        reopened
            .append(
                &verdict("cccccccccccccccc", Gate::Build, Status::Rejected, None, &[]),
                "tweak",
            )
            .unwrap();
        assert!(reopened.seen("cccccccccccccccc"));
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines.len(), 3);
        let third: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(third["n"], 3);
        assert_eq!(third["proposer"], "tweak");
        assert_eq!(third["hash"], "cccccccccccccccc");
    }

    #[test]
    fn summary_of_a_hand_written_three_line_journal_has_the_counts() {
        let path = journal_path("summary");
        // L0001 and L0104 both occur once, so their order is the code.
        // The proven line's gate is oracle and must not be counted as rejected.
        let body = concat!(
            r#"{"hash":"aaaaaaaaaaaaaaaa","gate":"check","status":"rejected","score":null,"passed":0,"total":3,"diagnostics":[{"code":"L0110","message":"m","line":4,"col":9,"fix":""},{"code":"L0110","message":"again","line":5,"col":1,"fix":""},{"code":"L0104","message":"m","line":1,"col":1,"fix":""}],"ms":{"check":31,"build":0,"run":0,"oracle":0},"note":"","proposer":"seed","n":1}"#,
            "\n",
            r#"{"hash":"bbbbbbbbbbbbbbbb","gate":"run","status":"scored","score":4,"passed":1,"total":3,"diagnostics":[{"code":"L0001","message":"m","line":1,"col":1,"fix":""}],"ms":{"check":0,"build":10,"run":20,"oracle":0},"note":"","proposer":"folder","n":2}"#,
            "\n",
            r#"{"hash":"cccccccccccccccc","gate":"oracle","status":"proven","score":1,"passed":3,"total":3,"diagnostics":[{"code":"L0110","message":"m","line":2,"col":3,"fix":"f"}],"ms":{"check":0,"build":0,"run":0,"oracle":5},"note":"ok","proposer":"tweak","n":3}"#,
            "\n",
        );
        fs::write(&path, body).unwrap();
        let text = Journal::summary(&path).unwrap();
        assert_eq!(
            text,
            "\
candidates 3
status rejected=1 scored=1 proven=1 diverged=0
rejected prefilter=0 check=1 build=0 run=0 oracle=0
codes L0110=3 L0001=1 L0104=1
best 1 cccccccccccccccc
"
        );
    }

    fn line(hash: &str, status: &str, score: &str) -> String {
        format!(
            r#"{{"hash":"{hash}","gate":"run","status":"{status}","score":{score},"passed":0,"total":1,"diagnostics":[],"ms":{{"check":0,"build":0,"run":1,"oracle":0}},"note":"","proposer":"seed","n":1}}"#
        )
    }

    #[test]
    fn summary_best_ignores_a_diverged_score() {
        let path = journal_path("best-diverged");
        let body = format!(
            "{}\n{}\n",
            line("aaaaaaaaaaaaaaaa", "scored", "2"),
            line("bbbbbbbbbbbbbbbb", "diverged", "0")
        );
        fs::write(&path, body).unwrap();
        let text = Journal::summary(&path).unwrap();
        assert_eq!(
            text,
            "\
candidates 2
status rejected=0 scored=1 proven=0 diverged=1
rejected prefilter=0 check=0 build=0 run=0 oracle=0
codes none
best 2 aaaaaaaaaaaaaaaa
"
        );

        let only = journal_path("best-diverged-only");
        fs::write(
            &only,
            format!("{}\n", line("bbbbbbbbbbbbbbbb", "diverged", "0")),
        )
        .unwrap();
        let text = Journal::summary(&only).unwrap();
        assert!(text.contains("diverged=1"), "{text}");
        assert!(text.ends_with("best none\n"), "{text}");
    }

    #[test]
    fn a_truncated_final_line_is_skipped_and_a_bad_line_is_not() {
        let path = journal_path("truncated");
        let mut body = line("aaaaaaaaaaaaaaaa", "scored", "2");
        body.push('\n');
        body.push_str(r#"{"hash":"cccc","gate":"ru"#);
        fs::write(&path, &body).unwrap();

        let mut journal = Journal::open(&path).unwrap();
        assert!(journal.seen("aaaaaaaaaaaaaaaa"));
        assert!(!journal.seen("cccc"));
        let summary = Journal::summary(&path).unwrap();
        assert!(summary.contains("candidates 1\n"), "{summary}");
        assert!(summary.contains("best 2 aaaaaaaaaaaaaaaa\n"), "{summary}");

        journal
            .append(
                &verdict(
                    "dddddddddddddddd",
                    Gate::Run,
                    Status::Scored,
                    Some(1.0),
                    &[],
                ),
                "folder",
            )
            .unwrap();
        drop(journal);
        let reopened = Journal::open(&path).unwrap();
        assert!(reopened.seen("aaaaaaaaaaaaaaaa"));
        assert!(reopened.seen("dddddddddddddddd"));
        assert!(!reopened.seen("cccc"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"cccc\""), "{text}");
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["n"], 2);
        assert_eq!(second["hash"], "dddddddddddddddd");

        let bad = journal_path("bad-line");
        fs::write(&bad, "{\"hash\":\"nope\"}\n").unwrap();
        let message = match Journal::open(&bad) {
            Ok(_) => panic!("a terminated bad line must not open"),
            Err(error) => error.to_string(),
        };
        assert!(message.contains("journal line 1"), "{message}");
        let message = match Journal::summary(&bad) {
            Ok(text) => panic!("a terminated bad line must not summarize: {text}"),
            Err(error) => error.to_string(),
        };
        assert!(message.contains("journal line 1"), "{message}");

        let middle = journal_path("bad-middle");
        fs::write(&middle, "{\"nope\":true}\n{\"hash\":\"cccc\",\"gate\":\"ru").unwrap();
        let message = match Journal::open(&middle) {
            Ok(_) => panic!("a bad line before a fragment must not open"),
            Err(error) => error.to_string(),
        };
        assert!(message.contains("journal line 1"), "{message}");

        let whole = journal_path("valid-without-newline");
        fs::write(&whole, line("aaaaaaaaaaaaaaaa", "scored", "3")).unwrap();
        let opened = Journal::open(&whole).unwrap();
        assert!(opened.seen("aaaaaaaaaaaaaaaa"));
    }
}
