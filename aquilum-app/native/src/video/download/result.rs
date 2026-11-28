use std::path::PathBuf;

#[derive(Debug)]
pub(in crate::video) struct Completed {
    pub(in crate::video) path: PathBuf,
    pub(in crate::video) newly_published: bool,
}

impl Completed {
    pub(in crate::video) fn cached(path: PathBuf) -> Self {
        Self {
            path,
            newly_published: false,
        }
    }

    pub(in crate::video) fn published(path: PathBuf) -> Self {
        Self {
            path,
            newly_published: true,
        }
    }
}
