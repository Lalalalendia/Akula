use super::{Tool, Tools};
use aquilum_core::files::document::write_file_atomic_impl;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TrustedFile {
    path: PathBuf,
    digest: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Config {
    files: [Option<TrustedFile>; 5],
}

const MAX_CONFIG: u64 = 64 * 1024;

impl Config {
    pub(super) fn load(profile: &Path) -> Result<Self, String> {
        let path = config_path(profile);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.to_string()),
        };
        let mut data = Vec::new();
        file.take(MAX_CONFIG + 1)
            .read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        if data.len() as u64 > MAX_CONFIG {
            return Err("speech tools configuration exceeds the size limit".into());
        }
        serde_json::from_slice(&data).map_err(|error| error.to_string())
    }

    pub(super) fn choose(
        &mut self,
        profile: &Path,
        tool: Tool,
        path: PathBuf,
    ) -> Result<(), String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let digest = digest(&path)?;
        let mut next = self.clone();
        next.files[tool as usize] = Some(TrustedFile { path, digest });
        let content = serde_json::to_string_pretty(&next).map_err(|error| error.to_string())?;
        if content.len() as u64 > MAX_CONFIG {
            return Err("speech tools configuration exceeds the size limit".into());
        }
        write_file_atomic_impl(&config_path(profile), &content, None)
            .map_err(|error| format!("{error:?}"))?;
        *self = next;
        Ok(())
    }

    pub(super) fn paths(&self) -> [Option<PathBuf>; 5] {
        self.files.clone().map(|file| file.map(|file| file.path))
    }

    pub(super) fn verify(&self) -> Result<Tools, String> {
        let [probe, ffmpeg, whisper, model, vad] = [
            self.checked(Tool::Probe)?,
            self.checked(Tool::Ffmpeg)?,
            self.checked(Tool::Whisper)?,
            self.checked(Tool::Model)?,
            self.checked(Tool::Vad)?,
        ];
        Tools::new(probe, ffmpeg, whisper, model, vad).map_err(|error| error.to_string())
    }

    pub(super) fn checked(&self, tool: Tool) -> Result<&Path, String> {
        let file = self.files[tool as usize]
            .as_ref()
            .ok_or_else(|| "speech tool is not configured".to_string())?;
        if digest(&file.path)? != file.digest {
            return Err("speech tool or model changed. Select the file again in Settings".into());
        }
        Ok(&file.path)
    }
}

fn config_path(profile: &Path) -> PathBuf {
    profile.join("speech-tools.json")
}

fn digest(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("expected a regular local file".into());
    }
    let mut reader = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = blake3::Hasher::new();
    let mut block = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut block).map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(hash.finalize().to_hex().to_string());
        }
        hash.update(&block[..count]);
    }
}
