mod apply;
mod download_guard;
mod release;
#[cfg(target_os = "linux")]
#[allow(dead_code)] // Transaction engine compiled; publisher approval remains HOLD.
mod linux_transaction;
#[cfg(target_os = "linux")]
#[allow(dead_code)] // Private stage verification does not enable updater publication.
mod linux_signed_stage;
#[cfg(target_os = "macos")]
mod tar;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod macos_transaction;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod startup_ack;

#[cfg(target_os = "linux")]
mod linux_first_gui_ack;

#[cfg(all(target_os = "linux", feature = "installed"))]
pub(crate) fn on_first_presented_frame() {
    linux_first_gui_ack::on_first_presented_frame();
}

#[cfg(all(target_os = "macos", feature = "installed"))]
pub(crate) fn on_first_presented_frame() {
    startup_ack::on_first_presented_frame();
}
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

use release::{PUBLIC_KEY, Release, find, verify};

pub(crate) use apply::apply;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    Latest,
    Available(String),
    Downloading { version: String, done: u64, total: Option<u64> },
    Ready(String),
    Installing,
    Failed(String),
}

#[derive(Clone)]
pub struct Updater {
    status: Arc<Mutex<Status>>,
    // A Windows installer may only cross the apply boundary as a sealed,
    // publisher-authenticated SignedCandidate, never arbitrary bytes.
    #[cfg(target_os = "windows")]
    payload: Arc<Mutex<Option<release::SignedCandidate>>>,
    #[cfg(not(target_os = "windows"))]
    payload: Arc<Mutex<Option<Vec<u8>>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// Until the signed Windows release metadata states a tested minimum OS,
/// a Windows 7 test build must not replace itself with the ordinary Windows
/// installer. A valid signature proves origin, not operating-system support.
#[cfg(target_os = "windows")]
fn windows_7_or_earlier() -> bool {
    // GetVersion is available on Windows 7 and does not introduce new imports.
    // On newer, unmanifested Windows it can report an *older* version, which
    // is safe here: a false legacy result only disables automatic updating.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVersion() -> u32;
    }
    let value = unsafe { GetVersion() };
    let major = value & 0xff;
    let minor = (value >> 8) & 0xff;
    major < 6 || (major == 6 && minor <= 1)
}

pub fn available() -> bool {
    // Laboratory packages are isolated from the public update channel even
    // when they are built with the "installed" feature. The launcher sets this
    // variable; real Win7 also fails closed if the EXE is launched directly.
    if std::env::var_os("AKULA_LAB_DISABLE_UPDATES").is_some() {
        return false;
    }
    #[cfg(target_os = "windows")]
    if windows_7_or_earlier() {
        return false;
    }
    // A signed public release currently means windows-x86_64, not Win7-tested.
    cfg!(feature = "installed") && apply::supported() && release::channel_ready()
}

impl Updater {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Updater {
        Updater { status: Arc::new(Mutex::new(Status::Idle)), payload: Arc::new(Mutex::new(None)), wake }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, status: Status) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
        (self.wake)();
    }

    fn busy(&self) -> bool {
        matches!(self.status(), Status::Checking | Status::Downloading { .. } | Status::Ready(_) | Status::Installing)
    }

    pub fn check(&self) {
        if !available() || self.busy() {
            return;
        }
        self.set(Status::Checking);
        let this = self.clone();
        std::thread::spawn(move || {
            this.set(match find() {
                Ok(Some(release)) => Status::Available(release.version),
                Ok(None) => Status::Latest,
                Err(error) => Status::Failed(error),
            });
        });
    }

    pub fn install(&self) {
        if !available() || self.busy() {
            return;
        }
        self.set(Status::Checking);
        let this = self.clone();
        std::thread::spawn(move || {
            let result = find().and_then(|release| match release {
                Some(release) => this.download(&release).map(|()| Some(release.version)),
                None => Ok(None),
            });
            this.set(match result {
                Ok(Some(version)) => Status::Ready(version),
                Ok(None) => Status::Latest,
                Err(error) => Status::Failed(error),
            });
        });
    }

    fn download(&self, release: &Release) -> Result<(), String> {
        let agent = aquilum_core::web_agent::web_agent("updates");
        let response = agent.get(&release.url).call().map_err(|e| e.to_string())?;
        let total = download_guard::content_length(response.header("Content-Length"))?;
        // The publisher-authenticated manifest determines the exact download
        // budget. Never allocate according to an untrusted HTTP header alone.
        let data = download_guard::read_signed_payload(
            response.into_reader(), total, release.size,
            |done, total| {
                self.set(Status::Downloading {
                    version: release.version.clone(),
                    done,
                    total,
                });
            },
        )?;
        // Confirm both independently signed package bytes AND the manifest
        // (which binds version, product, SHA-256, size and platform).
        verify(&data, &release.signature, PUBLIC_KEY)?;
        let candidate = release::admit_signed_payload(
            &release.manifest, &release.manifest_signature, data,
        )?;
        if candidate.version() != release.version
            || candidate.sha256() != release.sha256
            || candidate.bytes().len() as u64 != release.size
        {
            return Err("signed update offer changed between discovery and download".into());
        }
        #[cfg(target_os = "windows")]
        {
            *self.payload.lock().unwrap_or_else(|e| e.into_inner()) = Some(candidate);
        }
        #[cfg(not(target_os = "windows"))]
        {
            *self.payload.lock().unwrap_or_else(|e| e.into_inner()) = Some(candidate.into_bytes());
        }
        Ok(())
    }

    pub fn fail(&self, error: String) {
        self.set(Status::Failed(error));
    }

    // A single GUI call-site retains the same name on each target, but the
    // Windows type is non-forgeable outside signed manifest admission.
    #[cfg(target_os = "windows")]
    pub(crate) fn take_ready(&self) -> Option<release::SignedCandidate> {
        if !matches!(self.status(), Status::Ready(_)) {
            return None;
        }
        let candidate = self.payload.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        self.set(Status::Installing);
        Some(candidate)
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn take_ready(&self) -> Option<Vec<u8>> {
        if !matches!(self.status(), Status::Ready(_)) {
            return None;
        }
        let payload = self.payload.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        self.set(Status::Installing);
        Some(payload)
    }
}
