//! Связь окна с ядром `aquilum-core`: каталог данных, база знаний и события ядра.
//!
//! События приходят из потоков ядра: `EventSink` кладёт их в канал и будит цикл winit,
//! хост вычитывает канал целиком в `proxy_wake_up` (пробуждения склеиваются).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use aquilum_core::{Core, CoreEvent, EventSink};

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub const APP_ID: &str = "io.github.lalalendia.akula";
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub const APP_ID: &str = "com.dmitriy.aquilum-app";

pub struct Workspace {
    pub core: Arc<Core>,
    pub data_dir: PathBuf,
    /// Корень базы знаний; `None` — база не выбрана.
    pub root: Option<PathBuf>,
    pub wake: Arc<dyn Fn() + Send + Sync>,
    events: Receiver<CoreEvent>,
}

impl Workspace {
    /// Открывает ядро в `data_dir` (по умолчанию — каталог нативной сборки) и выбирает базу:
    /// `vault`, иначе последнюю открытую в этом каталоге данных.
    pub fn open(data_dir: Option<&Path>, vault: Option<&Path>, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let data_dir = data_dir_or_default(data_dir)?;
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        // Windows/Linux Akula never auto-import Aquilum/Quantum app data.
        // macOS is frozen and retains its previous migration behavior.
        // Any cross-product migration needs an explicit, reviewed backup tool.
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        if data_dir.ends_with(APP_ID) {
            aquilum_core::migration::migrate_legacy_data(&data_dir);
        }

        let (sender, events) = channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let waker = Arc::clone(&wake);
        let sink: Arc<dyn EventSink> = Arc::new(move |event: CoreEvent| {
            // Окно закрыто — событие некому показывать.
            if sender.send(event).is_ok() {
                waker();
            }
        });
        let core = Core::open(&data_dir, sink);

        let root = match vault {
            Some(vault) => Some(normalize(vault)),
            None => core.ui_state.list_workspaces().ok().and_then(|known| known.into_iter().next()).map(|w| PathBuf::from(w.path)),
        };
        if let Some(root) = &root {
            if let Err(e) = core.ui_state.resolve_workspace(&root.to_string_lossy(), crate::dates::now_ms()) {
                report!("база {} не запомнена: {e:?}", root.display());
            }
        }
        Ok(Workspace { core, data_dir, root, wake, events })
    }

    /// Все накопившиеся события ядра.
    pub fn drain_events(&self) -> Vec<CoreEvent> {
        self.events.try_iter().collect()
    }
}

pub fn data_dir_or_default(data_dir: Option<&Path>) -> Result<PathBuf, String> {
    match data_dir {
        Some(dir) => Ok(dir.to_path_buf()),
        None => Ok(platform_data_dir().ok_or("не найден каталог данных пользователя")?.join(APP_ID)),
    }
}

pub fn normalize(path: &Path) -> PathBuf {
    let Ok(canonical) = std::fs::canonicalize(path) else { return path.to_path_buf() };
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => canonical.clone(),
    }
}

/// Тот же корень, что `app_data_dir` у Tauri: %APPDATA%, ~/Library/Application Support,
/// $XDG_DATA_HOME или ~/.local/share.
fn platform_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    }
}

// RELEASE-LINUX-ID-01: checked without modifying user home directories.
#[cfg(all(test, target_os = "linux"))]
mod akula_linux_identity_tests {
    use super::*;

    #[test]
    fn standalone_app_id_is_independent_of_aquilum() {
        assert_eq!(APP_ID, "io.github.lalalendia.akula");
        assert_ne!(APP_ID, "com.dmitriy.aquilum-app");
    }

    #[test]
    fn explicit_test_directory_is_preserved() {
        let test_path = Path::new("/tmp/akula-linux-test-data");
        assert_eq!(data_dir_or_default(Some(test_path)).unwrap(), test_path);
    }
}

#[cfg(test)]
mod host_custody_feature_tests {
    #[test]
    fn native_compiles_the_shared_host_gate_even_without_s3_transport() {
        assert!(
            aquilum_core::sync::HOST_CUSTODY_FEATURE_ENABLED,
            "the native shell must not compile a Core without the host custody capability"
        );
    }
}


/// P0-B4a: exercise the real native Workspace::open entrypoint (the route
/// behind --data-dir), not just an internal Core fixture. This only proves
/// fail-closed behavior in an explicitly installed-host binary with NO
/// compiled host database. Successful installer provisioning and authority
/// over the DB/WAL directory remain separate release blockers.
#[cfg(all(test, feature = "sync-installed-host"))]
mod installed_host_native_tests {
    use super::*;
    use aquilum_core::files::gate;
    use aquilum_core::history::{NoteWrite, Source};
    use std::fs;
    use std::process::Command;

    const CHILD: &str = "workspace::installed_host_native_tests::child_probe";

    fn needs_missing_host_test() -> bool {
        !option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
            .is_some_and(|v| Path::new(v).is_absolute())
    }

    fn attempted_mutations(
        appdata: &Path, root: &Path,
    ) -> Result<[bool; 4], String> {
        let workspace = Workspace::open(Some(appdata), Some(root), || {})?;
        let note = root.join("Original.md");
        let created = root.join("New.md");
        let copied = root.join("Copy.md");
        let overwrite = gate::write(
            &workspace.core, &note, "forbidden", None, Source::Me, None,
        ).is_ok();
        let create = gate::create(
            &workspace.core, &created, "forbidden", Source::Me, None,
        ).is_ok();
        let copy = gate::copy(&workspace.core, &note, &copied).is_ok();
        let direct_history = workspace.core.history.try_saved(
            &|| workspace.core.known_roots(),
            &note,
            NoteWrite {
                disk_before: None,
                text: "forbidden",
                hash: "invalid-host-permit",
                source: Source::Me,
                name: None,
            },
        ).is_ok();
        Ok([overwrite, create, copy, direct_history])
    }

    #[test]
    fn child_probe() {
        if std::env::var_os("AKULA_INSTALLED_NATIVE_CHILD").is_none() {
            return;
        }
        let appdata = PathBuf::from(
            std::env::var_os("AKULA_INSTALLED_NATIVE_APPDATA").unwrap(),
        );
        let root = PathBuf::from(
            std::env::var_os("AKULA_INSTALLED_NATIVE_ROOT").unwrap(),
        );
        let report = PathBuf::from(
            std::env::var_os("AKULA_INSTALLED_NATIVE_REPORT").unwrap(),
        );
        let attempts = attempted_mutations(&appdata, &root).unwrap();
        fs::write(report, format!(
            "{}|{}|{}|{}",
            attempts[0], attempts[1], attempts[2], attempts[3],
        )).unwrap();
    }

    #[test]
    fn real_two_process_native_workspace_refuses_alternate_appdata() {
        if !needs_missing_host_test() {
            // Other CI lanes cover the positive, populated compile-time
            // source. Never overwrite or provision an operator host path.
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let app_a = temp.path().join("app-a");
        let app_b = temp.path().join("app-b");
        for dir in [&root, &app_a, &app_b] {
            fs::create_dir(dir).unwrap();
        }
        let root = fs::canonicalize(root).unwrap();
        let note = root.join("Original.md");
        fs::write(&note, "original bytes").unwrap();

        // This is the ordinary native entrypoint, with a user-supplied
        // data-dir. Its core must not silently become a legacy writer.
        let first = attempted_mutations(&app_a, &root).unwrap();
        assert_eq!(first, [false; 4]);
        assert_eq!(fs::read_to_string(&note).unwrap(), "original bytes");
        assert!(!root.join("New.md").exists());
        assert!(!root.join("Copy.md").exists());

        // A truly separate OS process opens the SAME workspace through a
        // DIFFERENT appdata. A runtime environment override must not change
        // the compile-time trusted host policy.
        let report = temp.path().join("second-process-result");
        let runtime_fake_host = temp.path().join("fake-host.sqlite");
        fs::write(&runtime_fake_host, b"untrusted").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(CHILD).arg("--nocapture")
            .env("AKULA_INSTALLED_NATIVE_CHILD", "1")
            .env("AKULA_INSTALLED_NATIVE_APPDATA", &app_b)
            .env("AKULA_INSTALLED_NATIVE_ROOT", &root)
            .env("AKULA_INSTALLED_NATIVE_REPORT", &report)
            .env("AKULA_INSTALLED_HOST_CUSTODY_DB", &runtime_fake_host)
            .output().unwrap();

        assert!(output.status.success(),
            "native child exited unsuccessfully: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains(
            "workspace::installed_host_native_tests::child_probe ... ok"
        ), "native child test did not execute: {}",
            String::from_utf8_lossy(&output.stdout));
        assert_eq!(
            fs::read_to_string(&report).unwrap(),
            "false|false|false|false",
        );
        assert_eq!(fs::read_to_string(&note).unwrap(), "original bytes");
        assert!(!root.join("New.md").exists());
        assert!(!root.join("Copy.md").exists());
        assert_eq!(fs::read(runtime_fake_host).unwrap(), b"untrusted");
    }
}


#[cfg(all(test, feature = "sync-installed-host"))]
#[path = "workspace_host_positive_tests.rs"]
mod installed_host_positive_tests;

#[cfg(all(test, feature = "sync-installed-host-witness"))]
#[path = "workspace_installed_witness_tests.rs"]
mod installed_host_witness_tests;
