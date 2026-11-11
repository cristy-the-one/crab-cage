//! The cage score counts only when the reference decoder accepts the kernel.

use std::path::PathBuf;

use crab_cage::delta::{eval_candidate, Measurement};

#[test]
fn decoder_scores_the_kernels_and_rejects_a_gap_load() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let nugget = root.join("nuggets/delta-kernel");
    let locked: Measurement =
        serde_json::from_str(&std::fs::read_to_string(root.join("corpus/results.json")).unwrap())
            .unwrap();
    let add: u64 = locked.pairs.iter().map(|row| row.add_only).sum();
    let strategy_a: u64 = locked.pairs.iter().map(|row| row.strategy_a).sum();
    let strategy_b: u64 = locked.pairs.iter().map(|row| row.strategy_b).sum();
    let reference: u64 = locked.pairs.iter().map(|row| row.reference).sum();
    let optimal: u64 = locked.pairs.iter().map(|row| row.optimal).sum();
    let cases = [
        ("00-add-only.rs", Some(add)),
        ("01-gap-load.rs", None),
        ("02-strategy-a.rs", Some(strategy_a)),
        ("03-strategy-b.rs", Some(strategy_b)),
        ("04-best-of.rs", Some(reference)),
        ("05-family.rs", Some(locked.family_total)),
        ("06-optimal.rs", Some(optimal)),
    ];
    for (name, expect) in cases {
        let report = eval_candidate(&nugget, &nugget.join("candidates").join(name))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        match expect {
            Some(bytes) => {
                assert!(report.ok, "{name}: {}", report.note);
                assert_eq!(report.bytes, Some(bytes), "{name}");
            }
            None => {
                assert!(!report.ok, "{name} should have no score");
                assert!(report.bytes.is_none(), "{name}");
                assert!(report.note.contains("no width"), "{name}: {}", report.note);
            }
        }
    }
}
