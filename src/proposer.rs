//! Where the next candidate text comes from.
//!
//! [`FolderProposer`] reads each `*.rs` file in a directory once, in file-name
//! order. It lists the directory again on every call, so a file that appears
//! while a run is in progress is still offered.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{err, Result};
use crate::nugget::Nugget;
use crate::pipeline::Verdict;

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
}
