use aquilum_core::background_jobs::{JobId, Queue, speech};
use std::fs::{self, File};
use std::path::Path;

pub(super) fn save(queue: &Queue, id: JobId, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or("choose a filename for WebVTT")?;
    let temporary = parent.join(format!(".akula-webvtt-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = File::create_new(&temporary).map_err(|error| error.to_string())?;
    let result = speech::write_vtt(queue, id, &mut file)
        .map_err(|error| error.to_string())
        .and_then(|()| file.sync_all().map_err(|error| error.to_string()));
    drop(file);
    // Publish the complete file without ever replacing an existing entry.
    let result = result
        .and_then(|()| fs::hard_link(&temporary, destination).map_err(|error| error.to_string()));
    let _ = fs::remove_file(temporary);
    result
}
