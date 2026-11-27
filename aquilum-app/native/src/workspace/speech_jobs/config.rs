use super::{Tool, Tools};
mod storage;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TrustedFile {
    path: PathBuf,
    digest: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Config {
    files: [Option<TrustedFile>; 5],
    #[serde(default)]
    pub(super) auto_downloads: bool,
}

impl Config {
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
        next.save(profile)?;
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
