use super::Config;
use aquilum_core::files::document::write_file_atomic_impl;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_CONFIG: u64 = 64 * 1024;

impl Config {
    pub(in crate::workspace::speech_jobs) fn load(profile: &Path) -> Result<Self, String> {
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

    pub(super) fn save(&self, profile: &Path) -> Result<(), String> {
        let content = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        if content.len() as u64 > MAX_CONFIG {
            return Err("speech tools configuration exceeds the size limit".into());
        }
        write_file_atomic_impl(&config_path(profile), &content, None)
            .map_err(|error| format!("{error:?}"))?;
        Ok(())
    }

    pub(in crate::workspace::speech_jobs) fn set_auto_downloads(
        &mut self,
        profile: &Path,
        enabled: bool,
    ) -> Result<(), String> {
        let mut next = self.clone();
        next.auto_downloads = enabled;
        next.save(profile)?;
        *self = next;
        Ok(())
    }
}

fn config_path(profile: &Path) -> PathBuf {
    profile.join("speech-tools.json")
}
