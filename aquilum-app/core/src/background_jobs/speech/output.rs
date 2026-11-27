use super::{Error, KIND, Result, edits};
use crate::background_jobs::{JobId, Queue};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::ops::Range;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperCue>,
}

#[derive(Deserialize)]
struct WhisperCue {
    offsets: Offsets,
    text: String,
}

#[derive(Deserialize)]
struct Offsets {
    from: i64,
    to: i64,
}

pub(super) fn decode_whisper(json: &[u8], window: Range<i64>) -> Result<Vec<Cue>> {
    let result: WhisperOutput = serde_json::from_slice(json)?;
    let duration = window.end - window.start;
    let mut cues = Vec::new();
    for cue in result.transcription {
        let from = cue.offsets.from.clamp(0, duration);
        let to = cue.offsets.to.clamp(0, duration);
        let text = cue.text.trim();
        if from < to && !text.is_empty() {
            cues.push(Cue {
                start_ms: window.start + from,
                end_ms: window.start + to,
                text: text.to_owned(),
            });
        }
    }
    cues.sort_by_key(|cue| (cue.start_ms, cue.end_ms));
    Ok(cues)
}

/// Export a consistent snapshot of committed speech, including a partial job.
pub fn write_vtt(queue: &Queue, id: JobId, mut output: impl Write) -> Result<()> {
    let _snapshot = edits::snapshot(queue)?;
    let job = queue
        .get(id)?
        .filter(|job| job.spec.kind == KIND)
        .ok_or_else(|| Error::Invalid(format!("speech job {id} does not exist")))?;
    writeln!(output, "WEBVTT\n")?;
    let mut cursor = 0;
    while cursor < job.committed {
        let segments = edits::windows(queue, id, cursor, (job.committed - cursor).min(16) as u32)?;
        if segments.is_empty() {
            return Err(Error::Invalid("speech checkpoint has no output".into()));
        }
        for (ordinal, rows) in segments {
            if ordinal != cursor {
                return Err(Error::Invalid("speech checkpoint has a gap".into()));
            }
            for cue in edits::effective(rows) {
                let text = cue
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                writeln!(
                    output,
                    "{} --> {}\n{}\n",
                    time(cue.start_ms),
                    time(cue.end_ms),
                    text
                )?;
            }
            cursor += 1;
        }
    }
    Ok(())
}

fn time(ms: i64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}
