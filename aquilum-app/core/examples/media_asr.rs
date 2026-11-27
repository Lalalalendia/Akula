//! Developer entry point for the opt-in local Whisper pipeline.

use aquilum_core::background_jobs::{speech, JobId, Queue};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::path::Path;

const USAGE: &str = "\
media_asr tracks <ffprobe> <source>
media_asr <queue.sqlite> add <source> <stream-index> <language|auto> <ffprobe> <ffmpeg> <whisper-cli> <model> <vad-model>
media_asr <queue.sqlite> work <job-id> <cpu-threads> <ffprobe> <ffmpeg> <whisper-cli> <model> <vad-model>
media_asr <queue.sqlite> list [before-id]
media_asr <queue.sqlite> {status,pause,resume,cancel,vtt} <job-id>";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let [first, rest @ ..] = args.as_slice() else {
        return Err(USAGE.into());
    };
    match (first.to_str(), rest) {
        (Some("--help" | "-h"), []) => {
            println!("{USAGE}");
            return Ok(());
        }
        (Some("tracks"), [ffprobe, source]) => {
            print_json(&speech::tracks(Path::new(ffprobe), Path::new(source))?)?;
            return Ok(());
        }
        (_, []) => return Err(USAGE.into()),
        _ => {}
    }
    let [command, args @ ..] = rest else {
        return Err(USAGE.into());
    };
    let mut queue = Queue::open(Path::new(first))?;
    match (command.to_str(), args) {
        (Some("add"), [source, track, language, binaries @ ..]) => {
            let id = speech::enqueue(
                &queue,
                &tools(binaries)?,
                Path::new(source),
                text(track)?.parse()?,
                text(language)?,
            )?;
            println!("{id}");
        }
        (Some("work"), [id, threads, binaries @ ..]) => {
            let outcome = speech::work(
                &mut queue,
                &tools(binaries)?,
                job_id(id)?,
                text(threads)?.parse()?,
            )?;
            print_json(&outcome)?;
        }
        (Some("list"), [] | [_]) => {
            let before = args.first().map(|id| job_id(id)).transpose()?;
            print_json(&queue.list(speech::KIND, before, 100)?)?;
        }
        (Some("status"), [id]) => {
            let id = job_id(id)?;
            let job = queue
                .get(id)?
                .filter(|job| job.spec.kind == speech::KIND)
                .ok_or_else(|| format!("speech job {id} does not exist"))?;
            print_json(&job)?;
        }
        (Some("pause" | "resume" | "cancel" | "vtt"), [id]) => {
            let id = job_id(id)?;
            if !queue
                .get(id)?
                .is_some_and(|job| job.spec.kind == speech::KIND)
            {
                return Err(format!("speech job {id} does not exist").into());
            }
            match text(command)? {
                "pause" => queue.pause(id)?,
                "resume" => queue.resume(id)?,
                "cancel" => queue.cancel(id)?,
                "vtt" => speech::write_vtt(&queue, id, std::io::stdout().lock())?,
                _ => unreachable!(),
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn job_id(value: &OsStr) -> Result<JobId, Box<dyn Error>> {
    Ok(text(value)?.parse()?)
}

fn text(value: &OsStr) -> Result<&str, Box<dyn Error>> {
    value
        .to_str()
        .ok_or_else(|| "expected a UTF-8 argument".into())
}

fn print_json(value: &impl serde::Serialize) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn tools(args: &[OsString]) -> Result<speech::Tools, Box<dyn Error>> {
    let [probe, ffmpeg, whisper, model, vad] = args else {
        return Err(USAGE.into());
    };
    Ok(speech::Tools::new(
        Path::new(probe),
        Path::new(ffmpeg),
        Path::new(whisper),
        Path::new(model),
        Path::new(vad),
    )?)
}
