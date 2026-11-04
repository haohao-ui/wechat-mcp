//! Evidence-constrained comparison of durable, customer-scoped viewport snapshots.
use anyhow::{Result, bail};
use serde::Serialize;

use super::reconcile::{Continuity, reconcile};
use crate::domain::observation::ChatCapture;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    FirstCapture,
    EvidenceUnavailable,
    RecipientChanged,
    EpochChanged,
    WindowChanged,
    RegionChanged,
    EmptyViewport,
    AnchorMissing,
    RepeatedAnchor,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Baseline,
    Unchanged,
    Anchored,
    Gap,
    Ambiguous,
}

#[derive(Debug, Serialize)]
pub(crate) struct Comparison {
    pub state: State,
    pub reason: Option<Reason>,
    pub new_lines: Vec<String>,
}

impl Comparison {
    fn gap(reason: Reason) -> Self {
        Self {
            state: State::Gap,
            reason: Some(reason),
            new_lines: vec![],
        }
    }
}

/// Alignment describes visible text only; it cannot establish message completeness.
pub(crate) fn compare(previous: Option<&ChatCapture>, current: &ChatCapture) -> Result<Comparison> {
    current.validate()?;
    let Some(previous) = previous else {
        return Ok(Comparison {
            state: State::Baseline,
            reason: Some(Reason::FirstCapture),
            new_lines: vec![],
        });
    };
    previous.validate()?;
    if previous.recipient != current.recipient {
        return Ok(Comparison::gap(Reason::RecipientChanged));
    }
    let (Some(old), Some(new)) = (&previous.evidence, &current.evidence) else {
        return Ok(Comparison::gap(Reason::EvidenceUnavailable));
    };
    // Captures can finish persistence out of order after releasing the desktop lock.
    if new.captured_at_ms < old.captured_at_ms
        || (old.epoch == new.epoch && new.sequence < old.sequence)
    {
        bail!("stale chat capture; durable baseline unchanged");
    }
    if old.epoch != new.epoch {
        return Ok(Comparison::gap(Reason::EpochChanged));
    }
    if old.window != new.window {
        return Ok(Comparison::gap(Reason::WindowChanged));
    }
    if old.region != new.region {
        return Ok(Comparison::gap(Reason::RegionChanged));
    }
    if new.sequence == old.sequence
        && (new.digest != old.digest
            || new.lines != old.lines
            || new.title_lines != old.title_lines)
    {
        bail!("conflicting evidence for the same captured frame");
    }
    let alignment = reconcile(&previous.text_lines, &current.text_lines, true);
    Ok(match alignment.continuity {
        Continuity::Unchanged => Comparison {
            state: State::Unchanged,
            reason: None,
            new_lines: vec![],
        },
        Continuity::Anchored => Comparison {
            state: State::Anchored,
            reason: None,
            new_lines: alignment.new_lines,
        },
        Continuity::Gap => Comparison::gap(
            if previous.text_lines.is_empty() || current.text_lines.is_empty() {
                Reason::EmptyViewport
            } else {
                Reason::AnchorMissing
            },
        ),
        Continuity::Ambiguous => Comparison {
            state: State::Ambiguous,
            reason: Some(Reason::RepeatedAnchor),
            new_lines: vec![],
        },
    })
}
