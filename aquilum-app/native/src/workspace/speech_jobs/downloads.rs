use super::{Command, Dispatcher};
use std::path::PathBuf;

impl Dispatcher {
    pub(crate) fn published_listener(&self) -> impl FnOnce(PathBuf) + Send + 'static {
        let tx = self.tx.clone();
        move |path| {
            let _ = tx.send(Command::PublishedVideo(path));
        }
    }
}
