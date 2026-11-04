//! Ordered text continuity; no content-set deduplication or inferred incoming status.
use serde::Serialize;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Continuity {
    Unchanged,
    Anchored,
    Gap,
    Ambiguous,
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct Reconciliation {
    pub continuity: Continuity,
    pub new_lines: Vec<String>,
}

/// Match the previous suffix against the current prefix, preserving occurrences.
/// An identical repeated viewport cannot prove whether another identical item arrived.
pub(crate) fn reconcile(
    previous: &[String],
    current: &[String],
    same_epoch: bool,
) -> Reconciliation {
    if !same_epoch || previous.is_empty() || current.is_empty() {
        return Reconciliation {
            continuity: Continuity::Gap,
            new_lines: vec![],
        };
    }
    if previous == current {
        return Reconciliation {
            continuity: Continuity::Unchanged,
            new_lines: vec![],
        };
    }
    let overlap = suffix_prefix_overlap(previous, current);
    match overlap {
        Some(length) => {
            // Any earlier occurrence, including a multi-line pattern, makes the anchor ambiguous.
            let anchor = &current[..length];
            if has_earlier_anchor(previous, anchor) {
                return Reconciliation {
                    continuity: Continuity::Ambiguous,
                    new_lines: vec![],
                };
            }
            Reconciliation {
                continuity: Continuity::Anchored,
                new_lines: current[length..].to_vec(),
            }
        }
        None => Reconciliation {
            continuity: Continuity::Gap,
            new_lines: vec![],
        },
    }
}

// Prefix matching keeps repetitive viewports linear instead of comparing every suffix.
fn prefix_table(pattern: &[String]) -> Vec<usize> {
    let mut prefix = vec![0; pattern.len()];
    for index in 1..pattern.len() {
        let mut matched = prefix[index - 1];
        while matched > 0 && pattern[index] != pattern[matched] {
            matched = prefix[matched - 1];
        }
        if pattern[index] == pattern[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    prefix
}

fn has_earlier_anchor(previous: &[String], anchor: &[String]) -> bool {
    let prefix = prefix_table(anchor);
    let mut matched = 0;
    // Exclude the last token so the suffix itself is not counted as an earlier match.
    for line in &previous[..previous.len() - 1] {
        while matched > 0 && line != &anchor[matched] {
            matched = prefix[matched - 1];
        }
        if line == &anchor[matched] {
            matched += 1;
        }
        if matched == anchor.len() {
            return true;
        }
    }
    false
}

fn suffix_prefix_overlap(previous: &[String], current: &[String]) -> Option<usize> {
    let prefix = prefix_table(current);
    let mut matched = 0;
    for line in previous {
        if matched == current.len() {
            matched = prefix[matched - 1];
        }
        while matched > 0 && line != &current[matched] {
            matched = prefix[matched - 1];
        }
        if line == &current[matched] {
            matched += 1;
        }
    }
    (matched > 0).then_some(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn identical_new_lines_keep_their_multiplicity() {
        assert_eq!(
            reconcile(
                &lines(&["前文", "收到"]),
                &lines(&["前文", "收到", "收到", "收到"]),
                true
            ),
            Reconciliation {
                continuity: Continuity::Anchored,
                new_lines: lines(&["收到", "收到"])
            }
        );
    }

    #[test]
    fn repeated_multi_line_and_overlapping_anchors_remain_ambiguous() {
        for (previous, current) in [
            (lines(&["甲", "乙", "甲", "乙"]), lines(&["甲", "乙", "丙"])),
            (lines(&["甲", "甲", "甲"]), lines(&["甲", "甲", "乙"])),
            (
                lines(&["甲", "乙", "丙", "甲", "乙"]),
                lines(&["甲", "乙", "丁"]),
            ),
        ] {
            let result = reconcile(&previous, &current, true);
            assert_eq!(result.continuity, Continuity::Ambiguous);
            assert!(result.new_lines.is_empty());
        }
    }

    #[test]
    fn overlapping_viewport_emits_only_the_ordered_tail() {
        assert_eq!(
            reconcile(
                &lines(&["甲", "乙", "丙"]),
                &lines(&["乙", "丙", "丁"]),
                true
            )
            .new_lines,
            lines(&["丁"])
        );
        assert_eq!(
            reconcile(&lines(&["甲", "乙"]), &lines(&["乙", "甲"]), true).new_lines,
            lines(&["甲"])
        );
    }

    #[test]
    fn restart_empty_frame_and_missing_anchor_do_not_invent_messages() {
        for result in [
            reconcile(&lines(&["旧"]), &lines(&["新"]), true),
            reconcile(&lines(&["旧"]), &lines(&["旧", "新"]), false),
            reconcile(&[], &lines(&["新"]), true),
            reconcile(&lines(&["旧"]), &[], true),
        ] {
            assert_eq!(result.continuity, Continuity::Gap);
            assert!(result.new_lines.is_empty());
        }
    }

    #[test]
    fn repeated_anchor_after_scroll_is_ambiguous() {
        let result = reconcile(&lines(&["收到", "收到"]), &lines(&["收到", "新"]), true);
        assert_eq!(result.continuity, Continuity::Ambiguous);
        assert!(result.new_lines.is_empty());
    }

    #[test]
    fn long_repeated_windows_without_a_suffix_anchor_report_a_gap() {
        let mut previous = vec!["重复".into(); 10_000];
        previous.push("结尾".into());
        let current = vec!["重复".into(); 10_000];
        assert_eq!(
            reconcile(&previous, &current, true).continuity,
            Continuity::Gap
        );
    }
}
