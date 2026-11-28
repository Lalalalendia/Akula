use super::{Tool, Tools, toolkit};
mod preferences;
mod storage;
use serde::{Deserialize, Serialize};
mod trust;
use trust::digest;
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
    #[serde(default)]
    pub(super) cpu_mode: aquilum_core::background_jobs::speech::CpuMode,
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

    pub(super) fn same_tools(&self, other: &Self) -> bool {
        self.files == other.files && self.managed_tools == other.managed_tools
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
