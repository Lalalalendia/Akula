//! Alpha: one trusted host-backed sample command; NOT arbitrary plugin WASM.
//! No user package code or OS capabilities execute here. This is an opt-in
//! vertical UX proof (install -> one-shot consent -> result -> disable).
use super::{App, Effect};
use aquilum_core::app_core::Core;
use aquilum_core::plugins::{
    Capability, GrantScope, HostContext, HostPluginVaultReader, PermissionBroker, PluginManifest,
};
use aquilum_core::settings::models::PluginAlphaSettings;
use std::{fs, path::{Path, PathBuf}};

const SAMPLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"), "/../../docs/plugins/fixtures/valid-v1.json"
));
const MAX_NOTE_BYTES: u64 = 1024 * 1024;

fn sample_path(data_dir: &Path) -> PathBuf {
    data_dir.join("plugins").join("org.example.word-count").join("manifest.json")
}

/// Installs only a bundled, fixed sample manifest. Not a generic package loader.
pub(super) fn install(data_dir: &Path) -> Result<(), String> {
    let manifest = PluginManifest::parse_json(SAMPLE).map_err(|e| e.to_string())?;
    if manifest.id != "org.example.word-count" {
        return Err("Unexpected bundled sample identity".into());
    }
    let path = sample_path(data_dir);
    fs::create_dir_all(path.parent().ok_or("Invalid sample path")?)
        .map_err(|_| "Cannot prepare the local sample directory")?;
    fs::write(&path, SAMPLE).map_err(|_| "Cannot install the local sample manifest")?;
    Ok(())
}

pub(super) fn uninstall(data_dir: &Path) -> Result<(), String> {
    let path = sample_path(data_dir);
    if path.exists() {
        fs::remove_file(path).map_err(|_| "Cannot remove the local sample manifest")?;
    }
    Ok(())
}

fn preview_saved_note(
    core: &Core,
    vault: &Path,
    path: &Path,
    data_dir: &Path,
) -> Result<usize, String> {
    let bytes = fs::read(sample_path(data_dir))
        .map_err(|_| "The sample manifest is missing; reinstall it")?;
    if bytes != SAMPLE.as_bytes() {
        return Err("Sample manifest changed; reinstall the bundled example".into());
    }
    let manifest = PluginManifest::parse_json(SAMPLE).map_err(|e| e.to_string())?;
    let relative = path.strip_prefix(vault)
        .map_err(|_| "The selected note is outside the active vault")?
        .to_string_lossy().replace('\\', "/");
    let broker = PermissionBroker::new();
    let reader = HostPluginVaultReader::open_registered(core, &broker, vault)
        .map_err(|_| "The active vault is not registered")?;
    // Host, not a plugin message, chooses the active note and pins file identity.
    let note = reader.bind_host_note(&relative)
        .map_err(|_| "The active note cannot be bound safely")?;
    let digest = *blake3::hash(SAMPLE.as_bytes()).as_bytes();
    let instance = broker.register_host_package(&manifest, digest, reader.host_vault_id())
        .map_err(|_| "The trusted sample was rejected")?;
    let scope = GrantScope::selected_notes([note.host_note_id()])
        .map_err(|_| "The selected note scope is invalid")?;
    // This follows one explicit UI click; no persistent grant is created.
    broker.grant_host_approved(instance, Capability::NotesRead, scope)
        .map_err(|_| "Reading this note was denied")?;
    let context = HostContext::from_host_state(reader.host_vault_id(), Some(note.host_note_id()));
    let result = reader.read_note_bounded(instance, &note, MAX_NOTE_BYTES, || context);
    let _ = broker.revoke_all_for_host(instance);
    let snapshot = result.map_err(|_| "Read denied, file changed, or exceeds 1 MiB")?;
    let markdown = std::str::from_utf8(&snapshot.bytes)
        .map_err(|_| "The saved note is not valid UTF-8")?;
    Ok(markdown.split_whitespace().count())
}

impl App {
    pub(super) fn plugin_demo_update(
        &self,
        update: impl FnOnce(&mut PluginAlphaSettings),
    ) -> Effect {
        let mut config = self.workspace.core.settings.get_config();
        update(&mut config.plugin_alpha);
        match self.workspace.core.settings.update_config(config) {
            Ok(()) => Effect::Content,
            Err(error) => {
                report!("plugin alpha settings were not saved: {error}");
                Effect::None
            }
        }
    }

    pub(in crate::app) fn plugin_demo_run(&self) -> Result<usize, String> {
        let config = self.workspace.core.settings.get_config();
        if !config.plugin_alpha.installed || !config.plugin_alpha.enabled {
            return Err("The sample is disabled or not installed".into());
        }
        let root = self.tree.root().ok_or("Open a vault first")?;
        let note = self.note.as_ref().ok_or("Open a Markdown note first")?;
        // Do not pretend on-disk committed text is the editor's dirty buffer.
        if note.clean_cache_hash().is_none() {
            return Err("Save the current note before running the sample".into());
        }
        preview_saved_note(
            &self.workspace.core, root, &note.path, &self.workspace.data_dir,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::app_core::CoreEvent;
    use std::sync::Arc;

    #[test]
    fn installed_sample_counts_only_a_registered_saved_note() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("appdata");
        let vault = temp.path().join("vault");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&vault).unwrap();
        let note = vault.join("a.md");
        fs::write(&note, "Hello мир!\nAnother line.").unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        assert!(preview_saved_note(&core, &vault, &note, &data).is_err());
        install(&data).unwrap();
        assert_eq!(preview_saved_note(&core, &vault, &note, &data).unwrap(), 4);
        let other = temp.path().join("secret.md");
        fs::write(&other, "must not leak").unwrap();
        assert!(preview_saved_note(&core, &vault, &other, &data).is_err());
        fs::write(sample_path(&data), "{}").unwrap();
        assert!(preview_saved_note(&core, &vault, &note, &data).is_err());
        uninstall(&data).unwrap();
        assert!(preview_saved_note(&core, &vault, &note, &data).is_err());
        core.shutdown();
    }

    #[test]
    fn sample_config_is_default_off() {
        let state = PluginAlphaSettings::default();
        assert!(!state.installed);
        assert!(!state.enabled);
        assert!(state.last_words.is_none());
    }
}
