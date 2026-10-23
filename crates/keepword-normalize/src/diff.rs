//! Diffs between normalized captures, and silent-edit classification.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

/// Words publishers flag an edit or a new version with. If an edit adds no
/// notice and doesn't move the machine-readable modification date, it is
/// "silent". Policies use these words in their prose too ("we may update
/// this policy", "the right to correction"), so only a line shaped like a
/// notice counts: see `notice`.
static UPDATE_NOTICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\b(updated?|updating|last modified|zuletzt (geändert|aktualisiert)|correction|corrected|clarification|editor'?s note|this (article|story|post) (has been|was) (updated|amended|changed)|aktualisiert|aktualisierung|korrektur|berichtigung|richtigstellung|anmerkung der redaktion|in einer früheren version|mise à jour|rectificatif|effective|(comes?|came|coming) into effect|(takes?|took|taking) effect|in kraft|en vigueur)\b|\b(update|stand):)",
    )
    .expect("valid regex")
});

/// A date ("May 15, 2026", "16. Januar 2026", "2026-06-03", "27.09.2026")
/// or a time. No word boundary after the year: descriptions run words
/// together ("Effective Date: 04 December 2024Introduction").
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(\d{4}-\d{2}-\d{2}|\d{1,2}\.\s?\d{1,2}\.\s?\d{2,4}|\d{1,2}/\d{1,2}/\d{2,4}|\d{1,2}:\d{2}|(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept?|oct|nov|dec|januar|jänner|februar|märz|mai|juni|juli|oktober|dezember|janvier|février|mars|avril|juin|juillet|août|septembre|octobre|novembre|décembre)\.?,?\s+(\d{1,2}(st|nd|rd|th|er)?,?\s+)?\d{4})",
    )
    .expect("valid regex")
});

/// A notice's own form: a label opening the line ("Update:", "Stand:"), or
/// a page saying it was changed. "Correction:" and "Berichtigung:" aren't
/// labels here: policies list the right to correction with them.
static NOTICE_FORM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^([a-z0-9]+: )?(updated?|clarification|editor'?s note|aktualisiert|aktualisierung|korrektur|anmerkung der redaktion|mise à jour|rectificatif|stand)\s*:|this (article|story|post) (has been|was) (updated|amended|changed)|an earlier version of this|\b(dieser|der) (artikel|beitrag|text) wurde\b|in einer früheren version",
    )
    .expect("valid regex")
});

/// The notice word in `line`, if it is a notice: a notice word next to a
/// date ("Last updated: May 15, 2026", "Effective Date: July 20, 2026"),
/// or in a notice's own form ("Update: …"). Of the 248 lines in 57 live
/// policies with such a word, this kept 52, all notices, and left 196, all
/// prose but for three announcements without a date.
fn notice(line: &str) -> Option<&str> {
    let m = UPDATE_NOTICE.find(line)?;
    (DATE.is_match(line) || NOTICE_FORM.is_match(line)).then_some(m.as_str())
}

/// How long a diff may search for the fewest changes; past it, what is left
/// shows as replaced. A page can change every line on every capture, and
/// the search takes time in the square of the lines changed: two minutes
/// for 100 000 lines, days for the millions a big page can have.
const DIFF_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Disclosure {
    /// The page itself signals the change.
    Disclosed { reasons: Vec<String> },
    /// Content changed with no visible or machine-readable notice.
    Silent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub added: usize,
    pub removed: usize,
    pub disclosure: Disclosure,
    /// Unified diff of the normalized text.
    pub unified: String,
    /// Line-level operations for rendering.
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Op {
    pub tag: OpTag,
    pub line: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpTag {
    Equal,
    Insert,
    Delete,
}

impl Change {
    pub fn is_silent(&self) -> bool {
        self.disclosure == Disclosure::Silent
    }

    pub fn summary(&self) -> String {
        let kind = match &self.disclosure {
            Disclosure::Silent => "silent edit".to_string(),
            Disclosure::Disclosed { reasons } => format!("disclosed edit ({})", reasons.join("; ")),
        };
        format!("{kind}: +{} -{} lines", self.added, self.removed)
    }
}

/// Diff two normalized texts. Returns `None` when they are identical.
pub fn diff(old: &str, new: &str) -> Option<Change> {
    if old == new {
        return None;
    }
    let td = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(old, new);
    // How many more times each line is inserted than deleted. A line the
    // diff deletes as often as it inserts was only moved, or realigned once
    // the deadline passed: a notice or date the page already had.
    let mut net: HashMap<&str, isize> = HashMap::new();
    for c in td.iter_all_changes() {
        match c.tag() {
            ChangeTag::Insert => *net.entry(c.value()).or_default() += 1,
            ChangeTag::Delete => *net.entry(c.value()).or_default() -= 1,
            ChangeTag::Equal => {}
        }
    }
    let mut ops = Vec::new();
    let (mut added, mut removed) = (0, 0);
    let mut reasons = Vec::new();
    for c in td.iter_all_changes() {
        let line = c.value().trim_end_matches('\n').to_string();
        let tag = match c.tag() {
            ChangeTag::Equal => OpTag::Equal,
            ChangeTag::Insert => {
                added += 1;
                let fresh = net.get_mut(c.value()).is_some_and(|n| {
                    *n -= 1;
                    *n >= 0
                });
                if fresh {
                    if let Some(date) = line.strip_prefix("modified: ") {
                        reasons.push(format!("modification date now {date}"));
                    } else if let Some(m) = notice(&line) {
                        reasons.push(format!("notice {m:?}"));
                    }
                }
                OpTag::Insert
            }
            ChangeTag::Delete => {
                removed += 1;
                OpTag::Delete
            }
        };
        ops.push(Op { tag, line });
    }
    reasons.dedup();
    let disclosure = if reasons.is_empty() {
        Disclosure::Silent
    } else {
        Disclosure::Disclosed { reasons }
    };
    let unified = td
        .unified_diff()
        .context_radius(3)
        .header("before", "after")
        .to_string();
    Some(Change {
        added,
        removed,
        disclosure,
        unified,
        ops,
    })
}

/// A line's words and spaces, each marked if it changed.
pub type Words = Vec<(bool, String)>;

/// The words and spaces of two versions of a line, each marked when the
/// other version lacks it: what changed within the line.
pub fn words(old: &str, new: &str) -> (Words, Words) {
    let td = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_words(old, new);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for c in td.iter_all_changes() {
        let w = c.value().to_string();
        match c.tag() {
            ChangeTag::Equal => {
                a.push((false, w.clone()));
                b.push((false, w));
            }
            ChangeTag::Delete => a.push((true, w)),
            ChangeTag::Insert => b.push((true, w)),
        }
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_words() {
        let (a, b) = words("p: kept for 30 days", "p: kept for 3 years");
        let marked = |v: &[(bool, String)]| -> Vec<String> {
            v.iter()
                .filter(|(c, _)| *c)
                .map(|(_, w)| w.clone())
                .collect()
        };
        assert_eq!(marked(&a), ["30", "days"]);
        assert_eq!(marked(&b), ["3", "years"]);
    }

    #[test]
    fn identical_is_none() {
        assert!(diff("a\nb\n", "a\nb\n").is_none());
    }

    #[test]
    fn silent_edit() {
        let c = diff("h1: X\np: said yes\n", "h1: X\np: said no\n").unwrap();
        assert!(c.is_silent());
        assert_eq!((c.added, c.removed), (1, 1));
        assert!(c.unified.contains("-p: said yes"));
    }

    #[test]
    fn disclosed_by_notice_or_date() {
        let c = diff("p: a\n", "p: b\np: Update: we corrected the figure.\n").unwrap();
        assert!(!c.is_silent());
        let c = diff("modified: 1\np: a\n", "modified: 2\np: b\n").unwrap();
        assert!(!c.is_silent(), "{c:?}");
        let c = diff("p: a\n", "p: b\np: Dieser Artikel wurde aktualisiert.\n").unwrap();
        assert!(!c.is_silent());
        let c = diff(
            "p: Stand: 27.09.2026 10:15 Uhr\np: a\n",
            "p: Stand: 27.09.2026 11:30 Uhr\np: b\n",
        )
        .unwrap();
        assert!(!c.is_silent(), "{c:?}");
        let c = diff("p: a\n", "p: b\np: UPDATE: figures revised\n").unwrap();
        assert!(!c.is_silent());
    }

    #[test]
    fn notices_are_shaped_like_notices() {
        // Lines from live policies.
        for line in [
            "p: Last Updated: May 15, 2026",
            "p: Zuletzt aktualisiert im Dezember 2022",
            "p: Letzte Aktualisierung: 16. Januar 2026",
            "p: Effective Date: July 20, 2026",
            "h5: Effective January 12th 2026",
            "description: Effective Date: 04 December 2024Introduction",
            "p: This Privacy Policy will be updated on November 20, 2026. Please review the upcoming changes here.",
            "p: Our new Privacy Policy for UK users comes into effect on 11 November 2026.",
            "p: Correction: An earlier version of this story misspelled a name.",
        ] {
            assert!(notice(line).is_some(), "{line}");
        }
        for line in [
            "p: We may update this Privacy Policy from time to time.",
            "h2: How we update this Privacy Notice",
            "li: Correction: the right to request that we correct inaccurate personal data",
            "li: Recht auf Berichtigung oder Löschung",
            "p: We will provide at least 15 days’ advance notice before the changes take effect.",
            "li: to ensure that content is presented in the most effective way for you",
            "p: Please review the full text of the updated Terms of Service.",
        ] {
            assert_eq!(notice(line), None, "{line}");
        }
        // Rewording a clause that mentions updates is a silent edit.
        let c = diff(
            "p: We may update this policy. We give 30 days' notice.\n",
            "p: We may update this policy. We give 5 days' notice.\n",
        )
        .unwrap();
        assert!(c.is_silent(), "{:?}", c.disclosure);
    }

    #[test]
    fn rewriting_every_line_finishes() {
        // Without a timeout, this took two minutes.
        let a: String = (0..100_000).map(|i| format!("p: a{i}\n")).collect();
        let b: String = (0..100_000).map(|i| format!("p: b{i}\n")).collect();
        let c = diff(&a, &b).unwrap();
        assert_eq!((c.added, c.removed), (100_000, 100_000));
        assert!(c.is_silent());
    }

    #[test]
    fn notices_the_page_already_had_dont_disclose() {
        // Moved past lines that stayed.
        let c = diff(
            "p: Update: new figures\np: a\np: b\np: c\n",
            "p: a\np: b\np: c, edited\np: Update: new figures\n",
        )
        .unwrap();
        assert!(c.is_silent(), "{:?}", c.disclosure);
        // Past the deadline, a notice in the middle of a rewrite can show
        // as removed and added again.
        let lines = |p: &str| -> String {
            (0..100_000)
                .map(|i| match i {
                    50_000 => "p: Correction: an earlier version misspelled a name\n".into(),
                    7 => "modified: 2026-10-01T10:00:00Z\n".into(),
                    _ => format!("p: {p}{i}\n"),
                })
                .collect()
        };
        let c = diff(&lines("a"), &lines("b")).unwrap();
        assert!(c.is_silent(), "{:?}", c.disclosure);
        // A second copy is news.
        let c = diff("p: Update: x\np: a\n", "p: Update: x\np: b\np: Update: x\n").unwrap();
        assert!(!c.is_silent());
    }

    #[test]
    fn removing_a_notice_is_not_disclosure() {
        let c = diff("p: Correction: x\np: a\n", "p: a\n").unwrap();
        assert!(c.is_silent());
    }
}
