//! Load a nugget directory and splice a candidate into its template.
//!
//! A nugget is `nugget.json`, `template.rs`, and `cases/<id>.in` (plus
//! `<id>.out` when the objective is `pass_all` or `fastest`). The template has one
//! `// CRAB:BEGIN` line and a later `// CRAB:END` line. The seed is the text
//! between them.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{err, Result};
use crate::supervisor::parse_memory;

const DEFAULT_TIMEOUT_MS: u64 = 2000;
const DEFAULT_MAX_MEMORY: &str = "64M";
const BEGIN: &str = "// CRAB:BEGIN";
const END: &str = "// CRAB:END";

/// One stdin case. `expected` is the `.out` file when that file exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: String,
    pub input: String,
    pub expected: Option<String>,
}

/// How runs of one candidate become a score. Lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    PassAll,
    Minimize,
    /// Every case must print its `.out`; among candidates that do, the
    /// score is the run time in milliseconds.
    Fastest,
}

impl Objective {
    /// Whether every case needs a `.out` file and is compared with it.
    pub fn needs_expected(self) -> bool {
        matches!(self, Objective::PassAll | Objective::Fastest)
    }

    /// The name used in `nugget.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Objective::PassAll => "pass_all",
            Objective::Minimize => "minimize",
            Objective::Fastest => "fastest",
        }
    }
}

/// A loaded nugget. `cases` is sorted by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nugget {
    pub name: String,
    pub objective: Objective,
    /// Wall-clock limit for one run. Defaults to 2000 ms.
    pub timeout: Duration,
    /// Byte cap from `max_memory`. Defaults to `64M`.
    pub max_memory: u64,
    /// Template text up to and including the `// CRAB:BEGIN` line.
    pub prefix: String,
    /// Text between the markers. This is the seed candidate.
    pub seed: String,
    /// The `// CRAB:END` line and everything after it.
    pub suffix: String,
    /// 1-based line number of the first seed line in `template.rs`.
    pub region_first_line: usize,
    /// Cases sorted by [`Case::id`].
    pub cases: Vec<Case>,
}

impl Nugget {
    /// Prefix, the candidate, a newline when the candidate does not end with
    /// one, then the suffix.
    pub fn splice(&self, candidate: &str) -> String {
        let mut out = String::new();
        out.push_str(&self.prefix);
        out.push_str(candidate);
        if !candidate.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&self.suffix);
        out
    }
}

/// Read `dir` as a nugget.
///
/// Fails when a marker is missing or repeated, when there are no cases, when
/// `pass_all` is missing a `.out`, or when `nugget.json` cannot be used.
/// Text is read with `\r\n` turned into `\n`.
pub fn load(dir: &Path) -> Result<Nugget> {
    let spec_path = dir.join("nugget.json");
    let spec = read_spec(&spec_path)?;
    let objective = parse_objective(&spec_path, &spec.objective)?;
    let timeout = Duration::from_millis(spec.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let max_memory = parse_memory(spec.max_memory.as_deref().unwrap_or(DEFAULT_MAX_MEMORY))?;
    let template_path = dir.join("template.rs");
    let template = read_text(&template_path)?;
    let (prefix, seed, suffix, region_first_line) = split_template(&template_path, &template)?;
    let cases = load_cases(dir, objective)?;
    Ok(Nugget {
        name: spec.name,
        objective,
        timeout,
        max_memory,
        prefix,
        seed,
        suffix,
        region_first_line,
        cases,
    })
}

#[derive(Deserialize)]
struct NuggetFile {
    name: String,
    objective: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_memory: Option<String>,
}

fn read_spec(path: &Path) -> Result<NuggetFile> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|error| err(format!("{}: {error}", path.display())))
}

fn parse_objective(path: &Path, objective: &str) -> Result<Objective> {
    match objective {
        "pass_all" => Ok(Objective::PassAll),
        "minimize" => Ok(Objective::Minimize),
        "fastest" => Ok(Objective::Fastest),
        other => Err(err(format!(
            "{}: unknown objective `{other}`; expected pass_all, minimize or fastest",
            path.display()
        ))),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).map_err(|error| err(format!("{}: {error}", path.display())))?;
    Ok(text.replace("\r\n", "\n"))
}

#[derive(Clone, Copy)]
struct LineSpan {
    line: usize,
    start: usize,
    end: usize,
}

enum Marker {
    Begin,
    End,
}

/// A marker line is exactly the marker text. Leading ASCII spaces are allowed.
fn marker_of(line: &str) -> Option<Marker> {
    let body = line.strip_suffix('\n').unwrap_or(line);
    match body.trim_start_matches(' ') {
        BEGIN => Some(Marker::Begin),
        END => Some(Marker::End),
        _ => None,
    }
}

fn find_markers(text: &str) -> (Vec<LineSpan>, Vec<LineSpan>) {
    let mut begins = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0usize;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let span = LineSpan {
            line: index + 1,
            start: offset,
            end: offset + line.len(),
        };
        match marker_of(line) {
            Some(Marker::Begin) => begins.push(span),
            Some(Marker::End) => ends.push(span),
            None => {}
        }
        offset += line.len();
    }
    (begins, ends)
}

fn require_one(path: &Path, marker: &str, spans: &[LineSpan]) -> Result<LineSpan> {
    match spans {
        [span] => Ok(*span),
        [] => Err(err(format!("{}: missing {marker}", path.display()))),
        _ => Err(err(format!(
            "{}: {marker} appears more than once",
            path.display()
        ))),
    }
}

fn split_template(path: &Path, text: &str) -> Result<(String, String, String, usize)> {
    let (begins, ends) = find_markers(text);
    let begin = require_one(path, BEGIN, &begins)?;
    let end = require_one(path, END, &ends)?;
    if end.start < begin.end {
        return Err(err(format!(
            "{}: // CRAB:END appears before // CRAB:BEGIN",
            path.display()
        )));
    }
    let prefix = text[..begin.end].to_string();
    let seed = text[begin.end..end.start].to_string();
    let suffix = text[end.start..].to_string();
    Ok((prefix, seed, suffix, begin.line + 1))
}

fn load_cases(dir: &Path, objective: Objective) -> Result<Vec<Case>> {
    let cases_dir = dir.join("cases");
    if !cases_dir.is_dir() {
        return Err(err(format!("{}: nugget has no cases", cases_dir.display())));
    }
    let mut cases = Vec::new();
    let entries = fs::read_dir(&cases_dir)
        .map_err(|error| err(format!("{}: {error}", cases_dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|error| err(format!("{}: {error}", cases_dir.display())))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(err(format!(
                "{}: case file name is not utf-8",
                path.display()
            )));
        };
        let Some(id) = name.strip_suffix(".in") else {
            continue;
        };
        if id.is_empty() {
            return Err(err(format!("{}: case id is empty", path.display())));
        }
        let input = read_text(&path)?;
        let out_path = cases_dir.join(format!("{id}.out"));
        let expected = if out_path.is_file() {
            Some(read_text(&out_path)?)
        } else if objective.needs_expected() {
            return Err(err(format!(
                "{}: case `{id}` has no .out file and objective is {}",
                out_path.display(),
                objective.as_str()
            )));
        } else {
            None
        };
        cases.push(Case {
            id: id.to_string(),
            input,
            expected,
        });
    }
    if cases.is_empty() {
        return Err(err(format!("{}: nugget has no cases", cases_dir.display())));
    }
    cases.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::*;

    fn demo_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("nuggets")
            .join("sum-squares")
    }

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crab-cage-nugget-{label}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(dir.join("cases")).unwrap();
        dir
    }

    fn write_json(dir: &Path, body: &str) {
        fs::write(dir.join("nugget.json"), body).unwrap();
    }

    fn write_template(dir: &Path, body: &str) {
        fs::write(dir.join("template.rs"), body).unwrap();
    }

    fn write_case(dir: &Path, id: &str, input: &str, expected: Option<&str>) {
        let cases = dir.join("cases");
        fs::create_dir_all(&cases).unwrap();
        fs::write(cases.join(format!("{id}.in")), input).unwrap();
        if let Some(expected) = expected {
            fs::write(cases.join(format!("{id}.out")), expected).unwrap();
        }
    }

    fn probe_json(objective: &str) -> String {
        format!(
            r#"{{"name":"probe","objective":"{objective}","timeout_ms":1500,"max_memory":"32M"}}"#
        )
    }

    fn load_error(dir: &Path) -> String {
        load(dir).expect_err("load should fail").to_string()
    }

    fn integers(input: &str) -> Vec<i64> {
        input
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.parse().expect("case input is integers"))
            .collect()
    }

    #[test]
    fn loads_the_sum_squares_demo_and_splices_the_seed() {
        let dir = demo_dir();
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.name, "sum-squares");
        assert_eq!(nugget.objective, Objective::PassAll);
        assert_eq!(nugget.timeout, Duration::from_millis(2000));
        assert_eq!(nugget.max_memory, 64 * 1024 * 1024);

        let raw = fs::read(dir.join("template.rs")).unwrap();
        let normalized = String::from_utf8(raw.clone())
            .unwrap()
            .replace("\r\n", "\n");
        let spliced = nugget.splice(&nugget.seed);
        assert_eq!(spliced, normalized);
        if !raw.contains(&b'\r') {
            assert_eq!(spliced.as_bytes(), raw.as_slice());
        }
        assert_eq!(
            format!("{}{}{}", nugget.prefix, nugget.seed, nugget.suffix),
            normalized
        );

        assert_eq!(nugget.seed, "    xs.iter().sum()\n");
        assert!(nugget.prefix.ends_with("// CRAB:BEGIN\n"));
        assert!(nugget.suffix.starts_with("    // CRAB:END\n"));
        let begin_line = normalized
            .lines()
            .position(|line| line.trim_start_matches(' ') == "// CRAB:BEGIN")
            .unwrap();
        assert_eq!(nugget.region_first_line, begin_line + 2);

        let ids: Vec<&str> = nugget.cases.iter().map(|case| case.id.as_str()).collect();
        assert_eq!(ids, vec!["negatives", "range", "seven"]);
        for case in &nugget.cases {
            let values = integers(&case.input);
            let squares: i64 = values.iter().map(|value| value * value).sum();
            let plain: i64 = values.iter().sum();
            assert_ne!(squares, plain, "{} does not expose the wrong seed", case.id);
            let expected = format!("{squares}\n");
            assert_eq!(case.expected.as_deref(), Some(expected.as_str()));
        }
    }

    #[test]
    fn splice_inserts_a_newline_only_when_the_candidate_lacks_one() {
        let nugget = Nugget {
            name: "n".to_string(),
            objective: Objective::Minimize,
            timeout: Duration::from_millis(1),
            max_memory: 1,
            prefix: "P\n".to_string(),
            seed: "S\n".to_string(),
            suffix: "E\n".to_string(),
            region_first_line: 2,
            cases: Vec::new(),
        };
        assert_eq!(nugget.splice("S\n"), "P\nS\nE\n");
        assert_eq!(nugget.splice("alt"), "P\nalt\nE\n");
        assert_eq!(nugget.splice("alt\n"), "P\nalt\nE\n");
        assert_eq!(nugget.splice(""), "P\n\nE\n");
    }

    #[test]
    fn prefix_includes_the_begin_line_and_suffix_starts_at_end() {
        let dir = scratch("split");
        write_json(&dir, &probe_json("minimize"));
        let template = "head\n    // CRAB:BEGIN\n    xs.iter().sum()\n    // CRAB:END\ntail\n";
        write_template(&dir, template);
        write_case(&dir, "only", "1\n", None);
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.prefix, "head\n    // CRAB:BEGIN\n");
        assert_eq!(nugget.seed, "    xs.iter().sum()\n");
        assert_eq!(nugget.suffix, "    // CRAB:END\ntail\n");
        assert_eq!(nugget.region_first_line, 3);
        assert_eq!(nugget.splice(&nugget.seed), template);
        assert_eq!(
            nugget.splice("    1"),
            "head\n    // CRAB:BEGIN\n    1\n    // CRAB:END\ntail\n"
        );
    }

    #[test]
    fn missing_end_marker_is_an_error() {
        let dir = scratch("no-end");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(text.contains("missing // CRAB:END"), "{text}");
    }

    #[test]
    fn missing_begin_marker_is_an_error() {
        let dir = scratch("no-begin");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "seed\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(text.contains("missing // CRAB:BEGIN"), "{text}");
    }

    #[test]
    fn begin_marker_twice_is_an_error() {
        let dir = scratch("begin-twice");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:BEGIN\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(
            text.contains("// CRAB:BEGIN appears more than once"),
            "{text}"
        );
    }

    #[test]
    fn end_marker_twice_is_an_error() {
        let dir = scratch("end-twice");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(
            text.contains("// CRAB:END appears more than once"),
            "{text}"
        );
    }

    #[test]
    fn end_before_begin_is_an_error() {
        let dir = scratch("end-first");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:END\n// CRAB:BEGIN\nseed\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(
            text.contains("// CRAB:END appears before // CRAB:BEGIN"),
            "{text}"
        );
    }

    #[test]
    fn trailing_text_is_not_a_marker() {
        let dir = scratch("trailing");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN \nseed\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(text.contains("missing // CRAB:BEGIN"), "{text}");
    }

    #[test]
    fn no_cases_is_an_error() {
        let dir = scratch("no-cases");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        let text = load_error(&dir);
        assert!(text.contains("nugget has no cases"), "{text}");
    }

    #[test]
    fn missing_cases_directory_is_an_error() {
        let dir = scratch("no-cases-dir");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        fs::remove_dir_all(dir.join("cases")).unwrap();
        let text = load_error(&dir);
        assert!(text.contains("nugget has no cases"), "{text}");
    }

    #[test]
    fn pass_all_case_missing_out_is_an_error() {
        let dir = scratch("missing-out");
        write_json(&dir, &probe_json("pass_all"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        write_case(&dir, "alpha", "1\n", Some("1\n"));
        write_case(&dir, "beta", "2\n", None);
        let text = load_error(&dir);
        assert!(text.contains("case `beta`"), "{text}");
        assert!(text.contains("pass_all"), "{text}");
        assert!(text.contains(".out"), "{text}");
    }

    #[test]
    fn minimize_sorts_cases_and_allows_a_missing_out() {
        let dir = scratch("minimize");
        write_json(&dir, &probe_json("minimize"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        write_case(&dir, "b", "2\n", None);
        write_case(&dir, "a", "1\r\n", Some("9\r\n"));
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.objective, Objective::Minimize);
        assert_eq!(nugget.timeout, Duration::from_millis(1500));
        assert_eq!(nugget.max_memory, 32 * 1024 * 1024);
        assert_eq!(nugget.cases.len(), 2);
        assert_eq!(nugget.cases[0].id, "a");
        assert_eq!(nugget.cases[0].input, "1\n");
        assert_eq!(nugget.cases[0].expected.as_deref(), Some("9\n"));
        assert_eq!(nugget.cases[1].id, "b");
        assert_eq!(nugget.cases[1].input, "2\n");
        assert_eq!(nugget.cases[1].expected, None);
    }

    #[test]
    fn crlf_template_normalizes_and_splices() {
        let dir = scratch("crlf");
        write_json(&dir, &probe_json("pass_all"));
        let template = "// CRAB:BEGIN\r\n    xs.iter().sum()\r\n// CRAB:END\r\n";
        write_template(&dir, template);
        write_case(&dir, "n", "1\r\n2\r\n", Some("5\r\n"));
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.seed, "    xs.iter().sum()\n");
        assert!(!nugget.prefix.contains('\r'));
        assert!(!nugget.suffix.contains('\r'));
        assert_eq!(nugget.splice(&nugget.seed), template.replace("\r\n", "\n"));
        assert_eq!(nugget.cases[0].input, "1\n2\n");
        assert_eq!(nugget.cases[0].expected.as_deref(), Some("5\n"));
        assert_eq!(nugget.region_first_line, 2);
    }

    #[test]
    fn defaults_timeout_and_memory() {
        let dir = scratch("defaults");
        write_json(&dir, r#"{"name":"defaults","objective":"pass_all"}"#);
        write_template(&dir, "// CRAB:BEGIN\n1\n// CRAB:END\n");
        write_case(&dir, "only", "1\n", Some("1\n"));
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.name, "defaults");
        assert_eq!(nugget.objective, Objective::PassAll);
        assert_eq!(nugget.timeout, Duration::from_millis(2000));
        assert_eq!(nugget.max_memory, 64 * 1024 * 1024);
        assert_eq!(nugget.region_first_line, 2);
        assert_eq!(
            nugget.splice(&nugget.seed),
            "// CRAB:BEGIN\n1\n// CRAB:END\n"
        );
    }

    #[test]
    fn unknown_objective_is_an_error() {
        let dir = scratch("objective");
        write_json(&dir, r#"{"name":"probe","objective":"quickest"}"#);
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(text.contains("unknown objective `quickest`"), "{text}");
    }

    #[test]
    fn fastest_case_missing_out_is_an_error() {
        let dir = scratch("fastest-missing-out");
        write_json(&dir, &probe_json("fastest"));
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        write_case(&dir, "alpha", "1\n", Some("1\n"));
        write_case(&dir, "beta", "2\n", None);
        let text = load_error(&dir);
        assert!(text.contains("case `beta`"), "{text}");
        assert!(text.contains("fastest"), "{text}");
        assert!(text.contains(".out"), "{text}");
    }

    #[test]
    fn loads_the_fast_sum_demo() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("nuggets")
            .join("fast-sum");
        let nugget = load(&dir).unwrap();
        assert_eq!(nugget.name, "fast-sum");
        assert_eq!(nugget.objective, Objective::Fastest);
        assert_eq!(nugget.cases.len(), 3);
        assert!(nugget.cases.iter().all(|case| case.expected.is_some()));
    }

    #[test]
    fn bad_memory_is_an_error() {
        let dir = scratch("bad-memory");
        write_json(
            &dir,
            r#"{"name":"probe","objective":"pass_all","max_memory":"nope"}"#,
        );
        write_template(&dir, "// CRAB:BEGIN\nseed\n// CRAB:END\n");
        write_case(&dir, "a", "1\n", Some("1\n"));
        let text = load_error(&dir);
        assert!(text.contains("memory"), "{text}");
    }
}
