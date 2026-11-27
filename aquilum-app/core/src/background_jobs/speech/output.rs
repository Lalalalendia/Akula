use super::{Error, Result, KIND};
use crate::background_jobs::{JobId, Queue};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::ops::Range;

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
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

/// Read a page of committed windows with their timestamped cues.
pub fn read_cues(
    queue: &Queue,
    id: JobId,
    first: i64,
    windows: u32,
) -> Result<Vec<(i64, Vec<Cue>)>> {
    if !queue.get(id)?.is_some_and(|job| job.spec.kind == KIND) {
        return Err(Error::Invalid(format!("speech job {id} does not exist")));
    }
    queue
        .segments(id, first, windows)?
        .into_iter()
        .map(|segment| Ok((segment.ordinal, serde_json::from_slice(&segment.payload)?)))
        .collect()
}

/// Export a consistent snapshot of committed speech, including a partial job.
pub fn write_vtt(queue: &Queue, id: JobId, mut output: impl Write) -> Result<()> {
    let job = queue
        .get(id)?
        .filter(|job| job.spec.kind == KIND)
        .ok_or_else(|| Error::Invalid(format!("speech job {id} does not exist")))?;
    writeln!(output, "WEBVTT\n")?;
    let mut cursor = 0;
    while cursor < job.committed {
        let segments = queue.segments(id, cursor, (job.committed - cursor).min(16) as u32)?;
        if segments.is_empty() {
            return Err(Error::Invalid("speech checkpoint has no output".into()));
        }
        for segment in segments {
            if segment.ordinal != cursor {
                return Err(Error::Invalid("speech checkpoint has a gap".into()));
            }
            let cues: Vec<Cue> = serde_json::from_slice(&segment.payload)?;
            for cue in cues {
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
