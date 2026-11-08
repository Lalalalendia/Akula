use std::ops::Range;
use masonry::accesskit::{Node, Role};
pub(super) use masonry::core::keyboard::{Key, KeyState, NamedKey};
pub(super) use masonry::core::{BrushIndex, Ime};
pub(super) use masonry::imaging::Painter;
pub(super) use masonry::kurbo::{Affine, Axis, BezPath, Cap, Circle, Join, Point, Rect, RoundedRect, Size, Stroke, Vec2};
pub(super) use masonry::parley::{Affinity, BoundingBox, Cursor, Layout, Selection};
pub(super) use super::markdown;
pub(super) use super::document::{Fold, Paragraph};
pub(super) use super::markdown::{Block, Callout};
pub(super) use super::history::Kind;
pub(super) use crate::ui::tokens::size;
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ComposeCtx, CursorIcon, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerEvent, PropertiesMut, PropertiesRef, QueryCtx, RegisterCtx, StyleProperty, StyleSet, TextEvent, Update,
    UpdateCtx, Widget, WidgetMut,
};
use masonry::layout::{LenReq, Length};
use crate::ui::editor::TextAction;
use super::{document::Document, markdown::Fonts};
use super::history::History;
use crate::ui::theme;
include!("modules.rs");
