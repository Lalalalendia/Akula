use super::{
    CpuMode, Error, JobId, KIND, LEASE, Limits, Outcome, Queue, Result, Tools, process_job,
    resources::MAX_THREADS,
};

/// Process one selected job. The controller may pause or cancel from a
/// separate connection. Already committed windows are never sent to Whisper.
pub fn work(queue: &mut Queue, tools: &Tools, id: JobId, threads: u8) -> Result<Outcome> {
    work_job(queue, tools, id, threads, false)
}

/// Automatic draining claims queued jobs when no live worker owns a speech job.
pub fn work_queued(queue: &mut Queue, tools: &Tools, id: JobId, threads: u8) -> Result<Outcome> {
    work_job(queue, tools, id, threads, true)
}

fn work_job(
    queue: &mut Queue,
    tools: &Tools,
    id: JobId,
    threads: u8,
    queued_only: bool,
) -> Result<Outcome> {
    if !(1..=MAX_THREADS).contains(&threads) {
        return Err(Error::Invalid("expected 1 to 8 CPU threads".into()));
    }
    let threads = threads.min(CpuMode::Fast.threads());
    Limits::Recognition(threads).memory()?;
    let lease = if queued_only {
        queue.claim_queued(KIND, id, LEASE)?
    } else {
        queue.claim(KIND, Some(id), LEASE)?
    };
    let Some(lease) = lease else {
        return Ok(Outcome::Idle);
    };
    match process_job(queue, tools, &lease, threads) {
        Ok(()) => Ok(Outcome::Completed),
        Err(Error::Queue(crate::background_jobs::Error::Conflict)) => Ok(Outcome::Interrupted),
        Err(Error::ResourcePressure) => match queue.park_for_memory(&lease) {
            Ok(()) => Err(Error::ResourcePressure),
            Err(crate::background_jobs::Error::Conflict) => Ok(Outcome::Interrupted),
            Err(failure) => Err(failure.into()),
        },
        Err(error) => {
            let message = failure_message(&error);
            match queue.fail(&lease, &message) {
                Ok(()) => Err(error),
                Err(crate::background_jobs::Error::Conflict) => Ok(Outcome::Interrupted),
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
