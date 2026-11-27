use super::command::{run, Workspace};
use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const CONTAINERS: &str = "matroska,webm,mov,mp4,m4a,3gp,3g2,mj2,mp3,wav,flac,ogg,aac,mpegts";
const PROBE_LIMIT: u64 = 256 * 1024;
const MAX_WINDOW_WAV: u64 = 1_024 * 1_024;
const IO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize, Serialize)]
pub struct Track {
    pub index: u32,
    pub codec_name: Option<String>,
    pub channels: Option<u32>,
    pub tags: Option<TrackTags>,
    duration: Option<String>,
    start_time: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TrackTags {
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Probe {
    pub tracks: Vec<Track>,
    format: Format,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
    start_time: Option<String>,
}

impl Probe {
    pub(super) fn duration_ms(&self, track: u32) -> Result<i64> {
        let track = self
            .tracks
            .iter()
            .find(|item| item.index == track)
            .ok_or_else(|| Error::Invalid(format!("audio track {track} does not exist")))?;
        let format_start = millis(self.format.start_time.as_deref()).unwrap_or(0);
        let delay = millis(track.start_time.as_deref())
            .and_then(|start| start.checked_sub(format_start))
            .unwrap_or(0)
            .max(0);
        let duration = millis(self.format.duration.as_deref())
            .or_else(|| millis(track.duration.as_deref())?.checked_add(delay));
        duration
            .filter(|ms| *ms > 0)
            .ok_or_else(|| Error::Invalid("audio timeline has no known duration".into()))
    }
}

fn millis(value: Option<&str>) -> Option<i64> {
    let seconds: f64 = value?.parse().ok()?;
    let ms = (seconds * 1_000.0).ceil();
    (ms.is_finite() && ms.abs() < i64::MAX as f64).then_some(ms as i64)
}

fn input_tools(tool: &Path) -> Command {
    let mut command = Command::new(tool);
    command.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-format_whitelist",
        CONTAINERS,
    ]);
    command
}

pub(super) fn probe(
    tool: &Path,
    source: &Path,
    temp: &Workspace,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Probe> {
    #[derive(Deserialize)]
    struct Raw {
        streams: Vec<Track>,
        format: Format,
    }
    let output = temp.file("probe.json");
    let mut command = input_tools(tool);
    command.args([
        "-select_streams", "a",
        "-show_entries", "format=duration,start_time:stream=index,codec_name,channels,start_time,duration:stream_tags=language,title",
        "-of", "json",
    ]).arg(source);
    run(
        &mut command,
        temp,
        Some(&output),
        (&output, PROBE_LIMIT),
        IO_TIMEOUT,
        check,
    )?;
    let raw: Raw = serde_json::from_slice(&fs::read(output)?)?;
    Ok(Probe {
        tracks: raw.streams,
        format: raw.format,
    })
}

pub(super) fn window(
    tools: &super::Tools,
    source: &Path,
    track: u32,
    range: std::ops::Range<i64>,
    temp: &Workspace,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<std::path::PathBuf> {
    let path = temp.file("window.wav");
    let start_samples = range.start * 16;
    let mut command = input_tools(&tools.ffmpeg);
    command
        .args([
            "-nostdin",
            "-y",
            "-copyts",
            "-start_at_zero",
            "-threads",
            "1",
        ])
        .arg("-ss")
        .arg(format!("{}ms", range.start))
        .arg("-i")
        .arg(source)
        .arg("-map")
        .arg(format!("0:{track}"))
        .args(["-vn", "-sn", "-dn", "-filter_threads", "1"])
        .arg("-af")
        .arg(format!(
            "aresample=16000:async=1:first_pts={start_samples},asetpts=PTS-{start_samples}"
        ))
        .arg("-t")
        .arg(format!("{}ms", range.end - range.start))
        .args(["-c:a", "pcm_s16le", "-ac", "1", "-ar", "16000", "-f", "wav"])
        .arg(&path);
    run(
        &mut command,
        temp,
        None,
        (&path, MAX_WINDOW_WAV),
        IO_TIMEOUT,
        check,
    )?;
    Ok(path)
}

pub(super) fn has_audio(path: &Path) -> Result<bool> {
    // FFmpeg writes a RIFF WAV. Locate the data chunk instead of depending on
    // one fixed header size (metadata chunks and their padding can vary).
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err(Error::Invalid("decoder did not produce a RIFF WAV".into()));
    }
    let end = file.metadata()?.len();
    while file.stream_position()? + 8 <= end {
        let mut name = [0; 4];
        let mut size = [0; 4];
        file.read_exact(&mut name)?;
        file.read_exact(&mut size)?;
        let size = u64::from(u32::from_le_bytes(size));
        if file.stream_position()? + size > end {
            return Err(Error::Invalid("decoder produced truncated WAV data".into()));
        }
        if &name == b"data" {
            return Ok(size > 0);
        }
        file.seek(SeekFrom::Current((size + size % 2) as i64))?;
    }
    Err(Error::Invalid("decoder WAV has no data chunk".into()))
}
