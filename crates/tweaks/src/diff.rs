//! What changed between two versions of a slot — the current value against
//! the one a vote proposes, or against an earlier value from this session.
//! Both sides are formatted first, so the diff shows real changes rather than
//! minification noise.

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeOp {
    Equal,
    Delete,
    Insert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Hunk {
    pub op: ChangeOp,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffView {
    pub hunks: Vec<Hunk>,
    /// The same change as a patch, for copying out.
    pub unified: String,
    pub added: u32,
    pub removed: u32,
}

/// Line diff of two already-formatted payloads.
pub fn diff(old: &str, new: &str) -> DiffView {
    let text = TextDiff::from_lines(old, new);
    let mut hunks = Vec::new();
    let (mut added, mut removed) = (0, 0);
    for change in text.iter_all_changes() {
        let op = match change.tag() {
            ChangeTag::Equal => ChangeOp::Equal,
            ChangeTag::Delete => {
                removed += 1;
                ChangeOp::Delete
            }
            ChangeTag::Insert => {
                added += 1;
                ChangeOp::Insert
            }
        };
        hunks.push(Hunk {
            op,
            old_line: change.old_index().map(|i| i as u32 + 1),
            new_line: change.new_index().map(|i| i as u32 + 1),
            text: change.value().trim_end_matches(['\r', '\n']).to_owned(),
        });
    }
    DiffView {
        unified: text
            .unified_diff()
            .header("current", "proposed")
            .to_string(),
        hunks,
        added,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_added_and_removed_lines_with_positions() {
        let old = "local a = 1\nlocal b = 2\n";
        let new = "local a = 1\nlocal b = 3\nlocal c = 4\n";
        let view = diff(old, new);
        assert_eq!((view.added, view.removed), (2, 1));
        assert!(view.unified.contains("-local b = 2"));
        assert!(view.unified.contains("+local c = 4"));

        let unchanged = view.hunks.first().unwrap();
        assert_eq!(unchanged.op, ChangeOp::Equal);
        assert_eq!((unchanged.old_line, unchanged.new_line), (Some(1), Some(1)));
        assert!(
            view.hunks
                .iter()
                .any(|h| h.op == ChangeOp::Insert && h.text == "local c = 4")
        );
    }

    #[test]
    fn identical_payloads_have_no_changes() {
        let view = diff("local a = 1\n", "local a = 1\n");
        assert_eq!((view.added, view.removed), (0, 0));
        assert!(view.hunks.iter().all(|h| h.op == ChangeOp::Equal));
    }
}
