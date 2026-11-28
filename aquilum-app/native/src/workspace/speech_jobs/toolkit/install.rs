use super::pins::{FFMPEG_FILES, MODEL, Pin, VAD, WHISPER_FILES};
use super::{check_path, checked, is_alias, verify_file};
use crate::video::optional_tool_transfer::Transfer;
use crate::workspace::speech_jobs::Tool;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

struct Archive {
    name: &'static str,
    url: &'static str,
    bytes: u64,
    sha256: &'static str,
    folder: &'static str,
    prefix: &'static str,
    files: &'static [Pin],
}

const FFMPEG: Archive = Archive {
    name: "ffmpeg.zip",
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip",
    bytes: 119_467_219,
    sha256: "b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd",
    folder: "media",
    prefix: "ffmpeg-9.1-essentials_build/bin",
    files: &FFMPEG_FILES,
};

const WHISPER: Archive = Archive {
    name: "whisper.zip",
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
    bytes: 8_928_640,
    sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
    folder: "engine",
    prefix: "Release",
    files: &WHISPER_FILES,
};

const MODELS: [(&Pin, &str); 2] = [
    (
        &MODEL,
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.bin",
    ),
    (
        &VAD,
        "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    ),
];

const TOTAL_BYTES: u64 = FFMPEG.bytes + WHISPER.bytes + MODEL.bytes + VAD.bytes;

pub(super) fn stage(root: &Path, progress: impl FnMut(u8)) -> Result<tempfile::TempDir, String> {
    stage_with(root, progress, |_, transfer, dest, report| {
        transfer.download(dest, report)
    })
}

fn stage_with(
    root: &Path,
    mut progress: impl FnMut(u8),
    mut download: impl FnMut(&str, Transfer<'_>, File, &mut dyn FnMut(u64)) -> Result<(), String>,
) -> Result<tempfile::TempDir, String> {
    if !root.is_absolute() {
        return Err("speech toolkit directory must be absolute".into());
    }
    ensure_directory(root)?;
    let install = tempfile::Builder::new()
        .prefix("kit-")
        .tempdir_in(root)
        .map_err(|error| error.to_string())?;
    let mut downloaded = 0;
    let mut fetch = |folder: &Path, name: &str, url: &str, bytes: u64, sha256: &str| {
        let file = folder.join(name);
        let dest = File::create_new(file).map_err(|error| error.to_string())?;
        download(name, transfer(url, bytes, sha256), dest, &mut |received| {
            progress(((downloaded + received) * 99 / TOTAL_BYTES) as u8);
        })?;
        downloaded += bytes;
        progress((downloaded * 99 / TOTAL_BYTES) as u8);
        Ok::<_, String>(())
    };
    for archive in [FFMPEG, WHISPER] {
        fetch(
            install.path(),
            archive.name,
            archive.url,
            archive.bytes,
            archive.sha256,
        )?;
        let path = install.path().join(archive.name);
        extract(&path, install.path(), &archive)?;
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    let models = install.path().join("models");
    fs::create_dir(&models).map_err(|error| error.to_string())?;
    for (pin, url) in MODELS {
        fetch(&models, pin.name, url, pin.bytes, pin.sha256)?;
    }
    for tool in Tool::ALL {
        checked(install.path(), tool)?;
    }
    Ok(install)
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_dir() || is_alias(&meta) => {
            return Err(format!(
                "invalid speech toolkit directory: {}",
                path.display()
            ));
        }
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("missing speech toolkit root")?;
            ensure_directory(parent)?;
            match fs::create_dir(path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    for ancestor in path.ancestors() {
        check_path(ancestor, true)?;
    }
    Ok(())
}

fn extract(archive_file: &Path, root: &Path, spec: &Archive) -> Result<(), String> {
    let file = File::open(archive_file).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let folder = root.join(spec.folder);
    fs::create_dir(&folder).map_err(|error| error.to_string())?;
    for pin in spec.files {
        let mut entry = archive
            .by_name(&format!("{}/{}", spec.prefix, pin.name))
            .map_err(|error| format!("speech toolkit archive is incomplete: {error}"))?;
        if !entry.is_file() || entry.size() != pin.bytes {
            return Err(format!(
                "unexpected entry in speech toolkit archive: {}",
                pin.name
            ));
        }
        let file = folder.join(pin.name);
        let mut dest = File::create_new(&file).map_err(|error| error.to_string())?;
        let bytes = io::copy(&mut entry.by_ref().take(pin.bytes + 1), &mut dest)
            .map_err(|error| error.to_string())?;
        if bytes != pin.bytes {
            return Err(format!(
                "unexpected size of extracted speech tool: {}",
                pin.name
            ));
        }
        dest.sync_all().map_err(|error| error.to_string())?;
        drop(dest);
        verify_file(&file, pin)?;
    }
    Ok(())
}

fn transfer<'a>(url: &'a str, bytes: u64, sha256: &'a str) -> Transfer<'a> {
    Transfer {
        url,
        bytes,
        sha256,
        notice_every: 1_048_576,
        errors: [
            "speech toolkit download failed",
            "speech toolkit source returned a non-200 response",
            "speech toolkit download interrupted",
            "speech toolkit download exceeded the pinned size",
            "could not save the downloaded speech toolkit file",
            "could not flush the downloaded speech toolkit file",
            "speech toolkit size or SHA-256 mismatch",
        ],
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(super) fn check_windows_runtime() -> Result<(), String> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    if !super::supported() {
        return Err("automatic speech setup requires Windows 10 or later (x64)".into());
    }
    let mut buffer = [0u16; 260];
    // SAFETY: the system API writes at most the supplied UTF-16 capacity.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err("Windows system directory is unavailable".into());
    }
    let system = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len]));
    let runtime = [
        "msvcp140.dll",
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "vcomp140.dll",
    ];
    if runtime.iter().any(|name| !system.join(name).is_file()) {
        return Err(
            "install Microsoft Visual C++ Redistributable (x64), then retry speech setup".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
