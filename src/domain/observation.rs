use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Coordinates relative to the observed region, with a top-left origin.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub fn validate(&self) -> Result<()> {
        if ![self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            || self.x < 0.0
            || self.y < 0.0
            || self.width <= 0.0
            || self.height <= 0.0
            || self.x + self.width > 1.000_001
            || self.y + self.height > 1.000_001
        {
            bail!("invalid normalized rectangle");
        }
        Ok(())
    }
}

/// An OCR line is not a message: sender, bubble and direction remain unknown.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct TextObservation {
    pub text: String,
    pub bounds: NormalizedRect,
    pub confidence: f64,
}

impl TextObservation {
    pub fn validate(&self) -> Result<()> {
        self.bounds.validate()?;
        if self.text.trim().is_empty()
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
        {
            bail!("invalid text observation");
        }
        Ok(())
    }
}

/// Process/window identity and geometry for the observation, in desktop points.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Window {
    pub window_id: u32,
    pub pid: i32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub on_screen: bool,
}

impl Window {
    pub fn validate(&self) -> Result<()> {
        if self.window_id == 0
            || self.pid <= 0
            || !self.on_screen
            || ![self.x, self.y, self.width, self.height]
                .into_iter()
                .all(f64::is_finite)
            || self.width <= 0.0
            || self.height <= 0.0
        {
            bail!("desktop window is not observable");
        }
        Ok(())
    }
}

/// Visual evidence independent of the IPC transport or persistence mechanism.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Snapshot {
    pub epoch: String,
    pub sequence: u64,
    pub window: Window,
    pub region: NormalizedRect,
    pub digest: String,
    pub captured_at_ms: u64,
    pub ocr_cached: bool,
    pub lines: Vec<TextObservation>,
    #[serde(default)]
    pub title_lines: Vec<TextObservation>,
}

impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        self.window.validate()?;
        self.region.validate()?;
        if self.epoch.is_empty()
            || self.sequence == 0
            || self.captured_at_ms == 0
            || self.digest.len() != 64
            || !self.digest.bytes().all(|b| b.is_ascii_hexdigit())
            || self.lines.len() + self.title_lines.len() > 10_000
        {
            bail!("invalid desktop snapshot metadata");
        }
        for line in self.lines.iter().chain(&self.title_lines) {
            line.validate()?;
        }
        Ok(())
    }
}

/// A customer-bound viewport; neither the target name nor the lines prove identity.
pub(crate) struct ChatCapture {
    pub recipient: Option<String>,
    pub text_lines: Vec<String>,
    pub evidence: Option<Snapshot>,
}

impl ChatCapture {
    pub fn validate(&self) -> Result<()> {
        if self.text_lines.len() > 10_000
            || self.text_lines.iter().any(|line| line.trim().is_empty())
        {
            bail!("invalid captured text lines");
        }
        if let Some(snapshot) = &self.evidence {
            snapshot.validate()?;
            if self
                .recipient
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
                || !self
                    .text_lines
                    .iter()
                    .map(String::as_str)
                    .eq(snapshot.lines.iter().map(|line| line.text.trim()))
            {
                bail!("chat capture does not match its visual evidence");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_reject_invalid_geometry_and_accept_boundary() {
        let rect = NormalizedRect {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        assert!(rect.validate().is_ok());
        for invalid in [
            NormalizedRect {
                x: f64::NAN,
                ..rect
            },
            NormalizedRect { x: -0.1, ..rect },
            NormalizedRect { width: 0.0, ..rect },
            NormalizedRect {
                height: f64::INFINITY,
                ..rect
            },
            NormalizedRect { width: 0.6, ..rect },
        ] {
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn invalid_text_and_confidence_are_not_silent_empty_observations() {
        let line = TextObservation {
            text: "消息".into(),
            bounds: NormalizedRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            confidence: 0.8,
        };
        assert!(line.validate().is_ok());
        assert!(
            TextObservation {
                text: " ".into(),
                ..line.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TextObservation {
                confidence: 1.1,
                ..line
            }
            .validate()
            .is_err()
        );
    }
}
