//! Opt-in local speech projection for explicitly installed CPU tools.
mod admission;
mod command;
pub mod edits;
mod media;
mod output;
pub mod playback; pub mod search;
#[cfg(test)]
mod tests;

pub use admission::{enqueue, enqueue_batch, prepare_enqueue, MAX_BATCH_FILES};
pub use media::{Track, TrackTags};
pub use edits::read_cues; pub use output::{write_vtt, Cue};
use super::{JobId, Lease, Queue, Segment};
use command::{run, Workspace};
use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const KIND: &str = "speech";
const WINDOW_MS: i64 = 30_000;
const RECIPE_VERSION: u32 = 1;
const LEASE: Duration = Duration::from_secs(20);
const HEARTBEAT: Duration = Duration::from_millis(500);
const ASR_TIMEOUT: Duration = Duration::from_secs(180);
const JSON_LIMIT: u64 = 256 * 1024;

#[derive(Debug)]
pub enum Error {
    Queue(super::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queue(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Queue(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<super::Error> for Error {
    fn from(value: super::Error) -> Self {
        Self::Queue(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

type Result<T> = std::result::Result<T, Error>;

pub struct Tools {
    ffprobe: PathBuf,
    ffmpeg: PathBuf,
    whisper: PathBuf,
    model: PathBuf,
    vad: PathBuf,
}

impl Tools {
    pub fn new(
        ffprobe: &Path,
        ffmpeg: &Path,
        whisper: &Path,
        model: &Path,
        vad: &Path,
    ) -> Result<Self> {
        Ok(Self {
            ffprobe: local_file(ffprobe)?,
            ffmpeg: local_file(ffmpeg)?,
            whisper: local_file(whisper)?,
            model: local_file(model)?,
            vad: local_file(vad)?,
        })
    }

    fn recipe(
        &self,
        track: u32,
        language: &str,
        check: &mut impl FnMut() -> Result<()>,
    ) -> Result<Recipe> {
        if language != "auto"
            && (!(2..=3).contains(&language.len())
                || !language.bytes().all(|byte| byte.is_ascii_lowercase()))
        {
            return Err(Error::Invalid(
                "expected a Whisper language code or auto".into(),
            ));
        }
        Ok(Recipe {
            version: RECIPE_VERSION,
            track,
            language: language.to_owned(),
            tools: [
                fingerprint(&self.ffprobe, check)?.digest,
                fingerprint(&self.ffmpeg, check)?.digest,
                fingerprint(&self.whisper, check)?.digest,
                fingerprint(&self.model, check)?.digest,
                fingerprint(&self.vad, check)?.digest,
            ],
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Recipe {
    version: u32,
    track: u32,
    language: String,
    tools: [String; 5],
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Idle,
    Completed,
    Interrupted,
}

pub fn tracks(ffprobe: &Path, source: &Path) -> Result<Vec<Track>> {
    let workspace = Workspace::new()?;
    Ok(media::probe(
        &local_file(ffprobe)?,
        &local_file(source)?,
        &workspace,
        &mut || Ok(()),
    )?
    .tracks)
}

/// Process one selected job. The controller may pause or cancel from a
/// separate connection. Already committed windows are never sent to Whisper.
pub fn work(queue: &mut Queue, tools: &Tools, id: JobId, threads: u8) -> Result<Outcome> {
    work_job(queue, tools, id, threads, false)
}

/// Automatic draining claims queued jobs when no live worker owns a speech job.
pub fn work_queued(queue: &mut Queue, tools: &Tools, id: JobId, threads: u8) -> Result<Outcome> {
    work_job(queue, tools, id, threads, true)
}

fn work_job(queue: &mut Queue, tools: &Tools, id: JobId, threads: u8, queued_only: bool) -> Result<Outcome> {
    if !(1..=8).contains(&threads) {
        return Err(Error::Invalid("expected 1 to 8 CPU threads".into()));
    }
    let lease = if queued_only { queue.claim_queued(KIND, id, LEASE)? }
        else { queue.claim(KIND, Some(id), LEASE)? };
    let Some(lease) = lease else {
        return Ok(Outcome::Idle);
    };
    match process_job(queue, tools, &lease, threads) {
        Ok(()) => Ok(Outcome::Completed),
        Err(Error::Queue(super::Error::Conflict)) => Ok(Outcome::Interrupted),
        Err(error) => {
            let message = failure_message(&error);
            match queue.fail(&lease, &message) {
                Ok(()) => Err(error),
                Err(super::Error::Conflict) => Ok(Outcome::Interrupted),
                Err(failure) => Err(failure.into()),
            }
        }
    }
}

fn failure_message(error: &Error) -> String {
    let message = error.to_string().replace('\0', "\u{fffd}");
    let mut end = message.len().min(8192);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

fn process_job(queue: &mut Queue, tools: &Tools, lease: &Lease, threads: u8) -> Result<()> {
    let job = queue.get(lease.job())?.ok_or(super::Error::Conflict)?;
    let source = Path::new(&job.spec.source);
    let recipe: Recipe = serde_json::from_str(&job.spec.recipe)?;
    let temp = Workspace::new()?;
    let mut last = Instant::now();
    let (input, duration) = {
        let mut check = heartbeat(queue, lease, &mut last);
        if recipe != tools.recipe(recipe.track, &recipe.language, &mut check)? {
            return Err(Error::Invalid(
                "worker tools or recipe differ from the queued job".into(),
            ));
        }
        let input = fingerprint(source, &mut check)?;
        if input.digest != job.spec.revision {
            return Err(Error::Invalid(
                "media contents changed since enqueue".into(),
            ));
        }
        let duration =
            media::probe(&tools.ffprobe, source, &temp, &mut check)?.duration_ms(recipe.track)?;
        (input, duration)
    };
    let total = 1 + (duration - 1) / WINDOW_MS;
    queue.set_total(lease, total)?;
    for ordinal in job.committed..total {
        let start = ordinal * WINDOW_MS;
        let end = (start + WINDOW_MS).min(duration);
        input.verify(source)?;
        let cues = {
            let mut check = heartbeat(queue, lease, &mut last);
            let wav = media::window(tools, source, recipe.track, start..end, &temp, &mut check)?;
            if media::has_audio(&wav)? {
                recognize(
                    tools,
                    &wav,
                    &recipe.language,
                    threads,
                    start..end,
                    &temp,
                    &mut check,
                )?
            } else {
                Vec::new()
            }
        };
        input.verify(source)?;
        queue.append(
            lease,
            &Segment {
                ordinal,
                start_ms: start,
                end_ms: end,
                payload: serde_json::to_vec(&cues)?,
            },
        )?;
    }
    queue.finish(lease)?;
    Ok(())
}

fn heartbeat<'a>(
    queue: &'a Queue,
    lease: &'a Lease,
    at: &'a mut Instant,
) -> impl FnMut() -> Result<()> + 'a {
    move || {
        if at.elapsed() >= HEARTBEAT {
            queue.heartbeat(lease, LEASE)?;
            *at = Instant::now();
        }
        Ok(())
    }
}

fn recognize(
    tools: &Tools,
    wav: &Path,
    language: &str,
    threads: u8,
    range: std::ops::Range<i64>,
    temp: &Workspace,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Cue>> {
    let base = temp.file("result");
    let json = base.with_extension("json");
    match fs::remove_file(&json) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let mut command = Command::new(&tools.whisper);
    command
        .args(["-ng", "-np", "-oj", "-p", "1", "-t"])
        .arg(threads.to_string())
        .env("OMP_NUM_THREADS", threads.to_string())
        .args(["-l", language, "-m"])
        .arg(&tools.model)
        .args(["--vad", "-vm"])
        .arg(&tools.vad)
        .arg("-f")
        .arg(wav)
        .arg("-of")
        .arg(base);
    run(
        &mut command,
        temp,
        None,
        (&json, JSON_LIMIT),
        ASR_TIMEOUT,
        check,
    )?;
    output::decode_whisper(&fs::read(json)?, range)
}

fn local_file(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    if !path.is_file() {
        return Err(Error::Invalid(format!(
            "expected a regular local file at {}",
            path.display()
        )));
    }
    Ok(path)
}

struct Fingerprint {
    digest: String,
    metadata: Metadata,
}

impl Fingerprint {
    fn verify(&self, path: &Path) -> Result<()> {
        if same_file(&self.metadata, &fs::metadata(path)?)? {
            Ok(())
        } else {
            Err(Error::Invalid(format!(
                "file changed during processing: {}",
                path.display()
            )))
        }
    }
}

fn fingerprint(path: &Path, check: &mut impl FnMut() -> Result<()>) -> Result<Fingerprint> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::Invalid(format!(
            "expected a regular file at {}",
            path.display()
        )));
    }
    let mut digest = Context::new(&SHA256);
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let fingerprint = Fingerprint {
        metadata,
        digest: digest
            .finish()
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    };
    fingerprint.verify(path)?;
    Ok(fingerprint)
}

fn same_file(before: &Metadata, after: &Metadata) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (
            before.dev(),
            before.ino(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (after.dev(), after.ino(), after.ctime(), after.ctime_nsec())
        {
            return Ok(false);
        }
    }
    Ok(before.len() == after.len() && before.modified()? == after.modified()?)
}
