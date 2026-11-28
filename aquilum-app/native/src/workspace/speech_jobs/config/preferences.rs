use super::Config;
use aquilum_core::background_jobs::speech::CpuMode;
use std::path::Path;

impl Config {
    pub(in crate::workspace::speech_jobs) fn set_auto_downloads(
        &mut self,
        profile: &Path,
        enabled: bool,
    ) -> Result<(), String> {
        self.modify(profile, |config| config.auto_downloads = enabled)
    }

    pub(in crate::workspace::speech_jobs) fn set_cpu_mode(
        &mut self,
        profile: &Path,
        mode: CpuMode,
    ) -> Result<(), String> {
        self.modify(profile, |config| config.cpu_mode = mode)
    }

    fn modify(&mut self, profile: &Path, edit: impl FnOnce(&mut Self)) -> Result<(), String> {
        let mut next = self.clone();
        edit(&mut next);
        next.save(profile)?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_admission_tracks_the_installed_kit_independently_of_cpu_mode() {
        let original = Config {
            managed_tools: Some("first-kit".into()),
            ..Config::default()
        };
        let mut changed = original.clone();
        changed.cpu_mode = CpuMode::Quiet;
        assert!(original.same_tools(&changed));
        changed.managed_tools = Some("replacement-kit".into());
        assert!(!original.same_tools(&changed));
    }
}
