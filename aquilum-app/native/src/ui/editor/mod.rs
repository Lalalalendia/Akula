mod change;
mod document;
mod embed;
mod highlight;
mod history;
mod markdown;
pub mod resolve;
mod table;
mod widget;

#[cfg(test)]
mod tests;

pub use table::Command as TableCommand;
pub use embed::media::media_line;
pub use widget::{Search, Smart, Status, TextEditor};
pub(crate) use document::LayoutSnapshot;
#[cfg(test)]
pub(crate) use document::take_applied as take_plain_layout_reuses;
#[cfg(test)]
pub(crate) use document::take_document_build_profile;

#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub range: std::ops::Range<usize>,
    pub insert: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextAction {
    Changed(String),
    Edited(Vec<Edit>),
    Entered(String),
    Cancelled,
    Page { down: bool, extend: bool },
    Line { down: bool },
}
