use crate::Args;

pub(super) fn defaults() -> Args {
    Args {
        vault: None,
        data_dir: None,
        size: None,
        text: String::new(),
        threads: std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4) as u16),
        metrics: None,
        trace: false,
        paint_flashing: false,
        focus: false,
        restore: true,
        keep_position: false,
        #[cfg(not(feature = "installed"))]
        replay: None,
        open: None,
        #[cfg(feature = "dicom-preview")]
        open_dicom: None,
        #[cfg(not(feature = "installed"))]
        trace_frames: None,
        #[cfg(feature = "memory-probe")]
        memory_probe: None,
        #[cfg(feature = "memory-probe")]
        probe_notes: Vec::new(),
    }
}

pub(super) fn positional(args: &mut Args, candidate: &str) -> Result<(), String> {
    if args.open.is_some() || !std::path::Path::new(candidate).is_file() {
        return Err(format!("неизвестный аргумент {candidate}"));
    }
    args.open = Some(candidate.into());
    Ok(())
}

pub(super) fn finish(mut args: Args) -> Result<Args, String> {
    if args.vault.is_none() {
        if let Some(parent) = args.open.as_ref().and_then(|path| path.parent()).filter(|parent| parent.is_dir()) {
            args.vault = Some(parent.to_path_buf());
        }
    }
    Ok(args)
}
