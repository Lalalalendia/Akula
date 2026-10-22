mod method_ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Backlinks,
    Outgoing,
    Analysis,
    History,
    #[cfg(feature = "extension-view-lab")]
    Tasks,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Backlinks => "backlinks",
            Mode::Outgoing => "outgoing",
            Mode::Analysis => "analysis",
            Mode::History => "history",
            #[cfg(feature = "extension-view-lab")]
            Mode::Tasks => "tasks",
        }
    }

    pub fn from_key(key: &str) -> Option<Mode> {
        [
            Mode::Backlinks,
            Mode::Outgoing,
            Mode::Analysis,
            Mode::History,
            #[cfg(feature = "extension-view-lab")]
            Mode::Tasks,
        ]
        .into_iter()
        .find(|mode| mode.key() == key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Bm25f,
    AdamicAdar,
    Wikixiv,
}
