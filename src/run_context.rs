//! Bind a resumed search to the inputs that gave its scores meaning.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};

use crate::error::{err, Result};
use crate::nugget::Nugget;
use crate::pipeline::Toolchain;

pub(crate) fn ensure(out: &Path, nugget: &Nugget, tools: &Toolchain, coverage: bool) -> Result<()> {
    let rustc = crate::supervisor::run_capture(
        {
            let mut cmd = std::process::Command::new("rustc");
            cmd.arg("--version").arg("--verbose");
            cmd
        },
        &crate::supervisor::Limits {
            wall: std::time::Duration::from_secs(10),
            memory_bytes: None,
            scrub_env: false,
        },
        &[],
    )?;
    if !rustc.success() {
        return Err(err("cannot identify rustc for run context"));
    }
    let context = json!({
        "schema": 1,
        "crab_cage": env!("CARGO_PKG_VERSION"),
        "rustc": rustc.stdout,
        "lust": tools.lust.as_ref().map(|t| json!({
            "path": t.bin, "version": t.version, "build": t.build,
        })),
        "coverage": coverage && tools.lust.is_some(),
        "objective": nugget.objective.as_str(),
        "timeout_ns": nugget.timeout.as_nanos().to_string(),
        "max_memory": nugget.max_memory,
        "prefix": nugget.prefix, "seed": nugget.seed, "suffix": nugget.suffix,
        "cases": nugget.cases.iter().map(|c| json!({
            "id": c.id, "input": c.input, "expected": c.expected,
        })).collect::<Vec<_>>(),
    });
    bind(out, &context)
}

fn bind(out: &Path, context: &Value) -> Result<()> {
    let path = out.join("run-context.json");
    if path.exists() {
        let previous: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if previous != *context {
            return Err(err(
                "run inputs or toolchain changed; use a new --out directory",
            ));
        }
        return Ok(());
    }
    // A new context must not adopt scores the run would restore. The journal
    // supplies proven scores and seen hashes. `species/manifest.json` supplies
    // species champions even when the journal is missing or empty.
    // `best.region.rs` is read only to recover text for a journal hash.
    if prior_evidence(out)? {
        return Err(err(
            "existing journal has no run-context.json; use a new --out directory",
        ));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(context)?)?;
    file.sync_all()?;
    Ok(())
}

/// True when `out` holds evidence [`crate::run::execute_evolved`] would restore
/// and that evidence is not yet bound to a `run-context.json`.
fn prior_evidence(out: &Path) -> Result<bool> {
    let journal = out.join("journal.jsonl");
    if journal.is_file() && fs::metadata(&journal)?.len() != 0 {
        return Ok(true);
    }
    Ok(out.join("species").join("manifest.json").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_journals_require_a_new_directory() {
        let dir = std::env::temp_dir().join(format!("crab-context-legacy-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("journal.jsonl"), "old evidence\n").unwrap();
        assert!(bind(&dir, &json!({"schema": 1})).is_err());
        assert!(!dir.join("run-context.json").exists());
        fs::remove_file(dir.join("journal.jsonl")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn mismatched_context_does_not_overwrite_existing_evidence() {
        let dir = std::env::temp_dir().join(format!("crab-context-bind-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let context = json!({"schema": 1, "cases": ["original"]});
        bind(&dir, &context).unwrap();
        bind(&dir, &context).unwrap();
        assert!(bind(&dir, &json!({"schema": 1, "cases": ["changed"]})).is_err());
        let stored: Value =
            serde_json::from_slice(&fs::read(dir.join("run-context.json")).unwrap()).unwrap();
        assert_eq!(stored, context);
        fs::remove_file(dir.join("run-context.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn species_manifest_without_context_is_not_bound() {
        let dir = std::env::temp_dir().join(format!("crab-context-species-{}", std::process::id()));
        fs::create_dir_all(dir.join("species")).unwrap();
        fs::write(dir.join("species").join("manifest.json"), "{}\n").unwrap();
        assert!(bind(&dir, &json!({"schema": 1})).is_err());
        assert!(!dir.join("run-context.json").exists());
        fs::remove_file(dir.join("species").join("manifest.json")).unwrap();
        fs::remove_dir(dir.join("species")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn changed_inputs_still_reject_when_a_manifest_is_present() {
        let dir =
            std::env::temp_dir().join(format!("crab-context-manifest-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let context = json!({"schema": 1, "cases": ["original"]});
        bind(&dir, &context).unwrap();
        fs::create_dir_all(dir.join("species")).unwrap();
        let manifest = "{\"entries\":[]}\n";
        fs::write(dir.join("species").join("manifest.json"), manifest).unwrap();
        assert!(bind(&dir, &json!({"schema": 1, "cases": ["changed"]})).is_err());
        let stored: Value =
            serde_json::from_slice(&fs::read(dir.join("run-context.json")).unwrap()).unwrap();
        assert_eq!(stored, context);
        assert_eq!(
            fs::read_to_string(dir.join("species").join("manifest.json")).unwrap(),
            manifest
        );
        fs::remove_file(dir.join("species").join("manifest.json")).unwrap();
        fs::remove_dir(dir.join("species")).unwrap();
        fs::remove_file(dir.join("run-context.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
