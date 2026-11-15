//! Small native-only action glyph: no browser or external icon asset.
use super::{Icon, Shape};

pub const DOWNLOAD: Icon = Icon::lucide(&[
    Shape::Path("M12 3v12"),
    Shape::Path("m6 11 6 6 6-6"),
    Shape::Path("M5 21h14"),
]);
