//! Stdin the sandboxed kernel sees, and the stdout the oracle trusts.
//!
//! The kernel never prints a score. The nugget harness does, after it has
//! simulated the commands. This module parses that transcript, packs a v0.03
//! file itself, and decodes it. The printed score counts only when the packed
//! size matches and the decoder's flash matches the new image.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::error::{err, Result};
use crate::lust::candidate_limits;
use crate::nugget::{self, Objective};
use crate::pipeline::{self, Status};
use crate::supervisor::{self, StopKind};

use super::check::{self, VerifyReport};
use super::image::{covered, needs_store, sectors, Image, Sector, Span};
use super::pack::{Op, SectorPlan, SECTOR};

#[derive(Clone, Debug, Serialize)]
pub struct PairOracle {
    pub name: String,
    pub ok: bool,
    pub bytes: Option<u64>,
    pub warnings: Vec<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KernelOracle {
    pub status: String,
    pub proven: bool,
    pub cage_score: Option<f64>,
    pub run_ms: u64,
    pub ok: bool,
    pub bytes: Option<u64>,
    pub pairs: Vec<PairOracle>,
    pub note: String,
}

pub fn case_input(name: &str, old: &Image, new: &Image) -> Result<String> {
    let views = sectors(old, new)?;
    let mut out = String::new();
    out.push_str("pair ");
    out.push_str(name);
    out.push('\n');
    out.push_str("sectors ");
    out.push_str(&views.len().to_string());
    out.push('\n');
    for sector in &views {
        out.push_str("base ");
        out.push_str(&sector.base.to_string());
        out.push('\n');
        push_spans(&mut out, "old_spans", &sector.old_real);
        push_spans(&mut out, "new_spans", &sector.new_real);
        out.push_str("old\n");
        out.push_str(&hex(&sector.old));
        out.push('\n');
        out.push_str("new\n");
        out.push_str(&hex(&sector.new));
        out.push('\n');
    }
    Ok(out)
}

/// Pack and check one harness transcript. `Err` is a harness failure.
/// `Ok(None)` is a kernel that refused (no score). `Ok(Some)` passed.
pub fn score_stdout(
    old: &Image,
    new: &Image,
    stdout: &str,
) -> Result<std::result::Result<VerifyReport, String>> {
    let text = stdout.replace("\r\n", "\n");
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Ok(Err("empty transcript".to_string()));
    }
    if lines[0].starts_with("error:") {
        return Ok(Err(lines[0].to_string()));
    }
    let score_line = lines[lines.len() - 1];
    let Some(score_text) = score_line.trim().strip_prefix("score:") else {
        return Ok(Err(format!("no score line: {score_line}")));
    };
    let printed: u64 = score_text
        .trim()
        .parse()
        .map_err(|_| err(format!("score is not an integer byte count: {score_line}")))?;
    let views = sectors(old, new)?;
    let mut plans = Vec::new();
    if lines.len() < 2 || !lines[0].starts_with("sectors ") {
        return Ok(Err("transcript does not start with sectors".to_string()));
    }
    let count: usize = lines[0][8..]
        .trim()
        .parse()
        .map_err(|_| err("bad sector count"))?;
    let mut i = 1usize;
    let mut seen = 0usize;
    while seen < count {
        if i >= lines.len() - 1 {
            return Ok(Err("transcript ended inside a sector".to_string()));
        }
        let Some(base_text) = lines[i].strip_prefix("base ") else {
            return Ok(Err(format!("expected base, found {}", lines[i])));
        };
        let base: u32 = base_text
            .trim()
            .parse()
            .map_err(|_| err(format!("bad base {}", lines[i])))?;
        i += 1;
        let mut ops = Vec::new();
        while i < lines.len() - 1 && lines[i] != "." {
            ops.push(parse_op(lines[i])?);
            i += 1;
        }
        if i >= lines.len() || lines[i] != "." {
            return Ok(Err("sector is missing its end marker".to_string()));
        }
        i += 1;
        let sector = views
            .iter()
            .find(|sector| sector.base == base)
            .ok_or_else(|| err(format!("transcript names unknown sector {base:#x}")))?;
        if !ops_fit(sector, &ops) {
            return Ok(Err(format!(
                "commands for sector {base:#x} leave the sector or a gap"
            )));
        }
        plans.push(SectorPlan {
            base,
            ops,
            store_len: super::image::store_extent(sector),
        });
        seen += 1;
    }
    if i != lines.len() - 1 {
        return Ok(Err(
            "transcript has trailing lines before the score".to_string()
        ));
    }
    if plans.is_empty() {
        let report = check::verify_bytes(old, new, &[])?;
        if printed != 0 {
            return Ok(Err(format!(
                "empty delta printed score {printed}, oracle size is 0"
            )));
        }
        return Ok(Ok(report));
    }
    let paired: Vec<(SectorPlan, &Sector)> = plans
        .iter()
        .map(|plan| {
            let sector = views
                .iter()
                .find(|sector| sector.base == plan.base)
                .ok_or_else(|| err("missing sector"))?;
            Ok((plan.clone(), sector))
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = match super::pack::pack_with(check::image_base(old, new), &paired) {
        Ok(bytes) => bytes,
        Err(error) => return Ok(Err(error.message)),
    };
    let report = match check::verify_bytes(old, new, &bytes) {
        Ok(report) => report,
        Err(error) => return Ok(Err(error.message)),
    };
    if report.delta_bytes != printed {
        return Ok(Err(format!(
            "printed score {printed} does not match the packed delta ({} bytes)",
            report.delta_bytes
        )));
    }
    Ok(Ok(report))
}

fn ops_fit(sector: &Sector, ops: &[Op]) -> bool {
    let mut ram = vec![0xFFu8; SECTOR as usize];
    for op in ops {
        match *op {
            Op::Load { addr, buf, len } => {
                if !range_ok(addr, len)
                    || !range_ok(buf, len)
                    || !covered(&sector.old_real, addr, len)
                {
                    return false;
                }
                let mut i = 0u32;
                while i < len {
                    ram[(buf + i) as usize] = sector.old[(addr + i) as usize];
                    i += 1;
                }
            }
            Op::Add { buf, len } => {
                if !range_ok(buf, len) {
                    return false;
                }
                let mut i = 0u32;
                while i < len {
                    ram[(buf + i) as usize] = sector.new[(buf + i) as usize];
                    i += 1;
                }
            }
        }
    }
    if !needs_store(sector) {
        return false;
    }
    for span in &sector.new_real {
        let mut i = 0u32;
        while i < span.len {
            let off = span.start + i;
            if ram[off as usize] != sector.new[off as usize] {
                return false;
            }
            i += 1;
        }
    }
    true
}

fn range_ok(off: u32, len: u32) -> bool {
    len >= 1
        && off
            .checked_add(len)
            .map(|end| end <= SECTOR)
            .unwrap_or(false)
}

fn parse_op(line: &str) -> Result<Op> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.first() == Some(&"L") && parts.len() == 4 {
        return Ok(Op::Load {
            addr: parse_u32(parts[1])?,
            buf: parse_u32(parts[2])?,
            len: parse_u32(parts[3])?,
        });
    }
    if parts.first() == Some(&"A") && parts.len() == 3 {
        return Ok(Op::Add {
            buf: parse_u32(parts[1])?,
            len: parse_u32(parts[2])?,
        });
    }
    Err(err(format!("bad command line: {line}")))
}

fn parse_u32(text: &str) -> Result<u32> {
    text.parse().map_err(|_| err(format!("bad integer {text}")))
}

/// Compile `candidate` through the nugget funnel, then grade its transcript
/// with the reference decoder. A kernel that fails either gate has no score.
pub fn eval_candidate(nugget_dir: &Path, candidate_path: &Path) -> Result<KernelOracle> {
    let nugget = nugget::load(nugget_dir)?;
    if nugget.objective != Objective::Minimize {
        return Err(err("delta nugget objective must be minimize"));
    }
    let candidate = fs::read_to_string(candidate_path)
        .map_err(|error| err(format!("{}: {error}", candidate_path.display())))?
        .replace("\r\n", "\n");
    let work = std::env::temp_dir().join(format!(
        "crab-cage-delta-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let verdict = pipeline::evaluate(
        &nugget,
        &candidate,
        &pipeline::Toolchain { lust: None },
        &pipeline::EvalOptions {
            work_dir: work.clone(),
            host_memory: 1024 * 1024 * 1024,
            compile_timeout: Duration::from_secs(180),
            oracle: true,
            slot: 0,
        },
    )?;
    let proven = verdict.status == Status::Proven;
    let mut oracle = KernelOracle {
        status: format!("{:?}", verdict.status).to_lowercase(),
        proven,
        cage_score: verdict.score,
        run_ms: verdict.ms.run,
        ok: false,
        bytes: None,
        pairs: Vec::new(),
        note: verdict.note.clone(),
    };
    if verdict.status != Status::Proven && verdict.status != Status::Scored {
        if oracle.note.is_empty() {
            oracle.note = format!("cage status {}", oracle.status);
        }
        if binary_exists(&work, &verdict.hash) {
            if let Some(detail) = first_stdout_note(&nugget, &work, &verdict.hash) {
                if oracle.note == "missing score" || oracle.note.starts_with("cage status") {
                    oracle.note = detail;
                }
            }
        }
        let _ = fs::remove_dir_all(&work);
        return Ok(oracle);
    }
    let binary = work.join(work_binary_name(&verdict.hash));
    if !binary.is_file() {
        oracle.note = format!("cage did not leave a binary at {}", binary.display());
        let _ = fs::remove_dir_all(&work);
        return Ok(oracle);
    }
    let limits = candidate_limits(
        nugget.timeout.saturating_add(Duration::from_millis(500)),
        1024 * 1024 * 1024,
    );
    let mut total = 0u64;
    let mut all_ok = true;
    for case in &nugget.cases {
        let (pair_name, old, new) = match load_case_images(nugget_dir, &case.id) {
            Ok(parsed) => parsed,
            Err(error) => {
                oracle.note = error.message;
                all_ok = false;
                break;
            }
        };
        let result =
            supervisor::run_capture(Command::new(&binary), &limits, case.input.as_bytes())?;
        if result.kind != StopKind::Exited(0) {
            all_ok = false;
            oracle.pairs.push(PairOracle {
                name: pair_name,
                ok: false,
                bytes: None,
                warnings: Vec::new(),
                note: format!("rerun status {:?}", result.kind),
            });
            continue;
        }
        match score_stdout(&old, &new, &result.stdout)? {
            Ok(report) => {
                total += report.delta_bytes;
                oracle.pairs.push(PairOracle {
                    name: pair_name,
                    ok: true,
                    bytes: Some(report.delta_bytes),
                    warnings: report.warnings,
                    note: String::new(),
                });
            }
            Err(note) => {
                all_ok = false;
                oracle.pairs.push(PairOracle {
                    name: pair_name,
                    ok: false,
                    bytes: None,
                    warnings: Vec::new(),
                    note,
                });
            }
        }
    }
    if all_ok && oracle.pairs.len() == nugget.cases.len() {
        if verdict.score != Some(total as f64) {
            oracle.ok = false;
            oracle.bytes = None;
            oracle.note = format!(
                "cage score {:?} does not match the oracle total {total}",
                verdict.score
            );
        } else {
            oracle.ok = true;
            oracle.bytes = Some(total);
            oracle.note.clear();
        }
    } else {
        oracle.ok = false;
        oracle.bytes = None;
        if oracle.note.is_empty() {
            oracle.note = "oracle rejected the kernel".to_string();
        }
    }
    let _ = fs::remove_dir_all(&work);
    Ok(oracle)
}

fn load_case_images(nugget_dir: &Path, case_id: &str) -> Result<(String, Image, Image)> {
    let root = nugget_dir
        .parent()
        .and_then(|dir| dir.parent())
        .ok_or_else(|| err("cannot find the repo root from the nugget directory"))?;
    let dir = root.join("corpus").join("pairs").join(case_id);
    let old = super::srec::parse(&read_srec(&dir.join("old.srec"))?)?;
    let new = super::srec::parse(&read_srec(&dir.join("new.srec"))?)?;
    Ok((
        case_id.to_string(),
        Image::from_regions(old)?,
        Image::from_regions(new)?,
    ))
}

fn read_srec(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|error| err(format!("{}: {error}", path.display())))
        .map(|text| text.replace("\r\n", "\n"))
}

fn work_binary_name(stem: &str) -> String {
    #[cfg(windows)]
    {
        format!("{stem}.exe")
    }
    #[cfg(not(windows))]
    {
        stem.to_string()
    }
}

fn binary_exists(work: &Path, hash: &str) -> bool {
    work.join(work_binary_name(hash)).is_file()
}

fn first_stdout_note(nugget: &nugget::Nugget, work: &Path, hash: &str) -> Option<String> {
    let binary = work.join(work_binary_name(hash));
    let limits = candidate_limits(Duration::from_secs(20), 1024 * 1024 * 1024);
    for case in &nugget.cases {
        let result =
            supervisor::run_capture(Command::new(&binary), &limits, case.input.as_bytes()).ok()?;
        for line in result.stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("error:") {
                return Some(trimmed.to_string());
            }
        }
        if result.kind != StopKind::Exited(0) {
            return Some(format!("rerun status {:?}", result.kind));
        }
    }
    None
}

fn push_spans(out: &mut String, label: &str, spans: &[Span]) {
    out.push_str(label);
    out.push(' ');
    out.push_str(&spans.len().to_string());
    out.push('\n');
    for span in spans {
        out.push_str(&span.start.to_string());
        out.push(' ');
        out.push_str(&span.len.to_string());
        out.push('\n');
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
