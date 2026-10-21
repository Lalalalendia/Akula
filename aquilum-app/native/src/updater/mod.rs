mod apply;
mod download_guard;
mod release;
#[cfg(target_os = "linux")]
#[allow(dead_code)] // Transaction primitives compile; publisher/GUI ACK admission remains HOLD.
mod linux_transaction;
#[cfg(target_os = "macos")]
mod tar;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod macos_transaction;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod startup_ack;

#[cfg(target_os = "linux")]
mod linux_first_gui_ack;

// One-shot kernel-authenticated receiving side, compiled but NOT yet wired to
// the installed package updater or allowed to authorize a release.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
mod linux_ack_receiver;

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

pub use apply::apply;

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
    payload: Arc<Mutex<Option<Vec<u8>>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

pub fn available() -> bool {
    // Akula has no dedicated signed release channel yet. Installed builds must
    // never offer an upstream Aquilum package as an Akula update.
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
        let data = download_guard::read_payload(response.into_reader(), total, |done, total| {
            self.set(Status::Downloading {
                version: release.version.clone(),
                done,
                total,
            });
        })?;
        verify(&data, &release.signature, PUBLIC_KEY)?;
        *self.payload.lock().unwrap_or_else(|e| e.into_inner()) = Some(data);
        Ok(())
    }

    pub fn fail(&self, error: String) {
        self.set(Status::Failed(error));
    }

    pub fn take_ready(&self) -> Option<Vec<u8>> {
        if !matches!(self.status(), Status::Ready(_)) {
            return None;
        }
        let payload = self.payload.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        self.set(Status::Installing);
        Some(payload)
    }
}
