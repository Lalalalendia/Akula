use super::{Tool, Tools, toolkit};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    managed_tools: Option<PathBuf>,
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
        Tool::ALL.map(|tool| {
            self.files[tool as usize]
                .as_ref()
                .map(|file| file.path.clone())
                .or_else(|| self.managed_tools.as_ref().map(|root| toolkit::path(root, tool)))
        })
    }

    pub(super) fn use_installed(
        &mut self,
        profile: &Path,
        install: tempfile::TempDir,
    ) -> Result<(), String> {
        let mut next = self.clone();
        next.files = Default::default();
        next.managed_tools = Some(install.path().to_owned());
        next.save(profile)?;
        *self = next;
        let _ = install.keep();
        Ok(())
    }

    pub(super) fn verify(&self) -> Result<Tools, String> {
        let [probe, ffmpeg, whisper, model, vad] = [
            self.checked(Tool::Probe)?,
            self.checked(Tool::Ffmpeg)?,
            self.checked(Tool::Whisper)?,
            self.checked(Tool::Model)?,
            self.checked(Tool::Vad)?,
        ];
        Tools::new(&probe, &ffmpeg, &whisper, &model, &vad).map_err(|error| error.to_string())
    }

    pub(super) fn checked(&self, tool: Tool) -> Result<PathBuf, String> {
        match &self.files[tool as usize] {
            Some(file) => {
                if digest(&file.path)? != file.digest {
                    return Err("speech tool or model changed. Select the file again in Settings".into());
                }
                Ok(file.path.clone())
            }
            None => toolkit::checked(
                self.managed_tools
                    .as_deref()
                    .ok_or("speech tool is not configured")?,
                tool,
            ),
        }
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
