//! Visual provider catalog for Cloud & Sync. No provider connects on opening
//! this page: tiles only show the last bounded, user-initiated evidence.
use std::collections::HashMap;

use aquilum_core::settings::models::AppConfig;
use masonry::accesskit::{Action as AccessAction, Node, Role};
use masonry::app::RenderRoot;
use masonry::core::{
    AccessCtx, AccessEvent, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx,
    NewWidget, PaintCtx, PointerEvent, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget, WidgetId, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Circle, Point, RoundedRect, Size, Stroke};
use masonry::layout::{LenReq, Length};
use masonry::peniko::Color;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::core::PropertySet;
use masonry::widgets::Flex;

use super::{Binding, schema};
use crate::i18n::t;
use crate::ui::dialog::{CardSize, Modal, header};
use crate::ui::scroll::ScrollArea;
use crate::ui::text::{self, selectable};
use crate::ui::tokens::size;
use crate::ui::widgets::{IconButton, Leading, Press, Pressed, Rule, TextButton, Variant, Wrap};
use crate::ui::{icons, theme};

use schema::{Action, CloudProvider, Context, Control, Section, Setter};
mod health;
use health::{health, Health};
#[cfg(feature = "notion-offline-preview")]
mod notion;

#[cfg(feature = "notion-offline-preview")]
const NOTION_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("M4 4h16v16H4z"),
    icons::Shape::Path("M7 17V7l10 10V7"),
]);
const TILE_WIDTH: f64 = 196.0;
const TILE_HEIGHT: f64 = 148.0;
const CLOUD_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("M7 19h10a5 5 0 0 0 .8-9.9A7 7 0 0 0 4.2 11.3 4 4 0 0 0 7 19z"),
]);
const BOX_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("m12 2 9 5-9 5-9-5 9-5z"),
    icons::Shape::Path("m3 7 9 5 9-5"),
    icons::Shape::Path("M3 7v10l9 5 9-5V7"),
    icons::Shape::Path("M12 12v10"),
]);

// Small provider-specific pictograms rather than identical generic cloud
// symbols. Icons are vectors rendered by Akula's native paint engine.
const MINIO_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("m4 5 5-3 5 3-5 3-5-3"),
    icons::Shape::Path("m10 10 5-3 5 3-5 3-5-3"),
    icons::Shape::Path("m4 15 5-3 5 3-5 3-5-3"),
    icons::Shape::Path("m10 20 5-3 5 3-5 3-5-3"),
]);
const DRIVE_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("M9 3h6l7 13-3 5H5l-3-5z"),
    icons::Shape::Path("m9 3 8 13"),
    icons::Shape::Path("M2 16h20"),
    icons::Shape::Path("m15 3-8 13"),
]);
const DROPBOX_ICON: icons::Icon = icons::Icon::lucide(&[
    icons::Shape::Path("m6 2 6 4-6 4-6-4z"),
    icons::Shape::Path("m18 2 6 4-6 4-6-4z"),
    icons::Shape::Path("m6 11 6 4-6 4-6-4z"),
    icons::Shape::Path("m18 11 6 4-6 4-6-4z"),
    icons::Shape::Path("m6 20 6 3 6-3"),
]);

struct ProviderTile {
    provider: CloudProvider,
    health: Health,
    name: String,
    label: String,
    title_line: text::Line,
    status_line: text::Line,
    press: Press,
}

impl ProviderTile {
    fn new(provider: CloudProvider, health: Health) -> Self {
        Self {
            provider,
            health,
            name: provider.label(),
            label: health.label(),
            title_line: text::Line::default(),
            status_line: text::Line::default(),
            press: Press::default(),
        }
    }

    fn icon(&self) -> icons::Icon {
        match self.provider {
            CloudProvider::Yandex | CloudProvider::S3Compatible => CLOUD_ICON,
            CloudProvider::AmazonS3 => BOX_ICON,
            CloudProvider::Minio => MINIO_ICON,
            CloudProvider::GoogleDrive => DRIVE_ICON,
            CloudProvider::Dropbox => DROPBOX_ICON,
            CloudProvider::WebDav => icons::NETWORK,
            #[cfg(feature = "notion-offline-preview")]
            CloudProvider::Notion => NOTION_ICON,
        }
    }
}

impl Widget for ProviderTile {
    type Action = Pressed;

    fn on_pointer_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &PointerEvent) {
        if self.press.pointer(ctx, event) {
            ctx.submit_action::<Pressed>(Pressed);
        }
    }

    fn on_access_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &AccessEvent) {
        if event.action == AccessAction::Click {
            ctx.submit_action::<Pressed>(Pressed);
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        match event {
            Update::HoveredChanged(_) => ctx.request_paint_only(),
            Update::FontsChanged => {
                self.title_line.clear();
                self.status_line.clear();
                ctx.request_layout();
            }
            _ => {}
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(&mut self, _ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        Length::px(if axis == Axis::Horizontal { TILE_WIDTH } else { TILE_HEIGHT })
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {}

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let colors = theme::current();
        let bounds = ctx.content_box();
        let background = if ctx.is_hovered() || self.press.down {
            colors.bg_surface_hover
        } else {
            colors.bg_surface_subtle
        };
        let card = RoundedRect::from_rect(bounds, 14.0);
        painter.fill(card, background).draw();
        painter.stroke(&card, &Stroke::new(1.0), colors.border_default).draw();

        let indicator = Point::new(bounds.x1 - 16.0, bounds.y0 + 16.0);
        let color = self.health.color();
        painter.fill(Circle::new(indicator, 9.0), color.multiply_alpha(0.16)).draw();
        painter.fill(Circle::new(indicator, 5.0), color).draw();

        let icon_color = match self.provider {
            CloudProvider::Yandex => Color::new([0.08, 0.39, 0.91, 1.0]),
            CloudProvider::AmazonS3 => Color::new([0.84, 0.35, 0.14, 1.0]),
            CloudProvider::Minio => Color::new([0.85, 0.14, 0.26, 1.0]),
            CloudProvider::GoogleDrive => Color::new([0.20, 0.59, 0.33, 1.0]),
            CloudProvider::Dropbox => Color::new([0.11, 0.39, 0.87, 1.0]),
            CloudProvider::S3Compatible | CloudProvider::WebDav => colors.text_secondary,
            #[cfg(feature = "notion-offline-preview")]
            CloudProvider::Notion => colors.text_secondary,
        };
        self.icon().draw(painter, Point::new(bounds.center().x - 23.0, bounds.y0 + 23.0), 46.0, 1.6, icon_color);

        let title = self.title_line.layout(ctx, &self.name, bounds.width() - 20.0, text::Style::ui(size::SIZE_16));
        let title_x = bounds.center().x - f64::from(title.width()) / 2.0;
        render_text(painter, Affine::translate((title_x, bounds.y0 + 82.0)), title, &[colors.text_secondary.into()], true);

        let status = self.status_line.layout(ctx, &self.label, bounds.width() - 20.0, text::Style::ui(13.0));
        let status_x = bounds.center().x - f64::from(status.width()) / 2.0;
        render_text(painter, Affine::translate((status_x, bounds.y0 + 115.0)), status, &[color.into()], true);
    }

    fn accessibility_role(&self) -> Role { Role::Button }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(format!("{} — {}", self.name, self.label));
        node.add_action(AccessAction::Click);
    }

    fn children_ids(&self) -> ChildrenIds { ChildrenIds::new() }
}

pub(super) fn catalog(ctx: &Context) -> (NewWidget<Flex>, HashMap<WidgetId, Binding>) {
    let mut bindings = HashMap::new();
    let tiles = CloudProvider::ALL.into_iter().map(|provider| {
        let tile = NewWidget::new(ProviderTile::new(provider, health(provider, ctx)));
        bindings.insert(tile.id(), Binding::Button(Action::OpenCloudProvider(provider)));
        tile.erased()
    }).collect();
    let column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(selectable(&t("settings.cloud.catalogTitle"), |t| t.text_secondary, Some(600.0), Leading::Px(size::SIZE_20 as f32)))
        .with_fixed(selectable(&t("settings.cloud.catalogHint"), |t| t.text_muted, None, Leading::Times(1.3)))
        .with_fixed(Wrap::new(tiles, size::GAP_LG));
    let content = NewWidget::new(column).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::GAP_XL)))
            .with(Padding::all(Length::px(size::GAP_XL))),
    );
    (content, bindings)
}

pub struct CloudPopup {
    pub layer: WidgetId,
    pub close: WidgetId,
    pub cancel: WidgetId,
    pub save: Option<WidgetId>,
    pub provider: CloudProvider,
    pub bindings: HashMap<WidgetId, Binding>,
    // Draft public fields live only in this modal until explicit Save.
    // OS credentials and completed Configure/Import actions are independent.
    staged_texts: Vec<(Setter, String)>,
}

fn same_setter(a: Setter, b: Setter) -> bool {
    match (a, b) {
        (Setter::Config(x), Setter::Config(y)) => std::ptr::fn_addr_eq(x, y),
        (Setter::Scale, Setter::Scale) |
        (Setter::HomePage, Setter::HomePage) |
        (Setter::Autostart, Setter::Autostart) => true,
        _ => false,
    }
}

pub struct CloudCard {
    card: NewWidget<Flex>,
    close: WidgetId,
    cancel: WidgetId,
    save: Option<WidgetId>,
    bindings: HashMap<WidgetId, Binding>,
}

fn window_size(room: Size) -> Size {
    Size::new(room.width.min(840.0), room.height.min(720.0))
}

fn card(config: &AppConfig, ctx: &Context, provider: CloudProvider, staged: &[(Setter, String)]) -> CloudCard {
    let close = NewWidget::new(IconButton::new(icons::X, t("settings.cloud.close")));
    let close_id = close.id();
    let mut bindings = HashMap::new();
    let title = provider.label();
    let inner = match provider {
        CloudProvider::Yandex => {
            let mut block = schema::blocks(Section::Sync, config, ctx)
                .into_iter().next().expect("Yandex setup block");
            for row in &mut block.rows {
                if let (Some(setter), Control::Text { value, .. }) = (row.setter, &mut row.control)
                    && let Some((_, draft)) = staged.iter().find(|(key, _)| same_setter(*key, setter))
                {
                    value.clone_from(draft);
                }
            }
            super::layout::block_widget(block, &mut bindings)
        }
        #[cfg(feature = "notion-offline-preview")]
        CloudProvider::Notion => notion::body(ctx),
        _ => NewWidget::new(Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(selectable(&t("settings.cloud.unavailableHint"), |t| t.text_secondary, None, Leading::Times(1.4))))
            .with_props(PropertySet::new().with(Padding::all(Length::px(size::GAP_XL)))),
    };
    let cancel = NewWidget::new(TextButton::new(t("settings.cloud.close"), Variant::Ghost));
    let cancel_id = cancel.id();
    let mut footer = Flex::row().main_axis_alignment(MainAxisAlignment::End).with_fixed(cancel);
    let save = if provider == CloudProvider::Yandex {
        let button = NewWidget::new(TextButton::new(t("settings.cloud.save"), Variant::Primary));
        let id = button.id();
        footer = footer.with_fixed(button);
        Some(id)
    } else {
        None
    };
    let footer = NewWidget::new(footer).with_props(PropertySet::new()
        .with(Gap::new(Length::px(size::GAP_LG)))
        .with(Padding::all(Length::px(size::DIALOG_PADDING))));
    let divider = || Rule::new(Axis::Horizontal, || theme::current().border_default);
    let modal = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(header(&title, Some(close)))
        .with_fixed(divider())
        .with(NewWidget::new(ScrollArea::new(inner)), 1.0)
        .with_fixed(divider())
        .with_fixed(footer);
    CloudCard { card: NewWidget::new(modal), close: close_id, cancel: cancel_id, save, bindings }
}

pub fn provider_modal(config: &AppConfig, ctx: &Context, provider: CloudProvider) -> (NewWidget<Modal>, CloudPopup) {
    let built = card(config, ctx, provider, &[]);
    let title = provider.label();
    let layer = Modal::new(built.card, &title, CardSize::Window(window_size), Role::Dialog, false);
    let popup = CloudPopup {
        layer: layer.id(),
        close: built.close,
        cancel: built.cancel,
        save: built.save,
        provider,
        bindings: built.bindings,
        staged_texts: Vec::new(),
    };
    (layer, popup)
}

impl CloudPopup {
    pub fn remember_text(&mut self, setter: Setter, text: &str) {
        if let Some((_, existing)) = self.staged_texts.iter_mut()
            .find(|(stored, _)| same_setter(*stored, setter))
        {
            text.clone_into(existing);
        } else {
            self.staged_texts.push((setter, text.to_owned()));
        }
    }

    pub fn rebuild(&mut self, root: &mut RenderRoot, config: &AppConfig, ctx: &Context) {
        let built = card(config, ctx, self.provider, &self.staged_texts);
        root.edit_widget(self.layer, |mut w| Modal::set_card(
            &mut w.downcast(), built.card, &self.provider.label(),
        ));
        self.close = built.close;
        self.cancel = built.cancel;
        self.save = built.save;
        self.bindings = built.bindings;
        root.focus_on(Some(self.layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_tiles_never_confuse_stored_keys_with_working_sync() {
        let mut ctx = Context::new(1.0);
        assert_eq!(health(CloudProvider::Yandex, &ctx), Health::NotConnected);
        assert_eq!(health(CloudProvider::AmazonS3, &ctx), Health::Unavailable);
        ctx.yandex_keys = Some(true);
        #[cfg(not(all(target_os = "windows", feature = "s3-pair-beta")))]
        assert_eq!(health(CloudProvider::Yandex, &ctx), Health::NotConnected);
        #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
        {
            use crate::s3_pair_session::PairStatus;
            assert_eq!(health(CloudProvider::Yandex, &ctx), Health::Saved);
            ctx.s3_pair.configured = true;
            ctx.s3_pair.status = PairStatus::Configured;
            ctx.s3_pair.verified = false;
            assert_ne!(health(CloudProvider::Yandex, &ctx), Health::Verified);
            ctx.s3_pair.verified = true;
            assert_eq!(health(CloudProvider::Yandex, &ctx), Health::Verified);
            ctx.s3_pair.status = PairStatus::Transferred {
                sent: 1, received_unconfirmed: 1, needs_review: 0,
            };
            assert_eq!(health(CloudProvider::Yandex, &ctx), Health::Review);
            ctx.s3_pair.status = PairStatus::Failed;
            assert_eq!(health(CloudProvider::Yandex, &ctx), Health::Review);
        }
    }

    #[test]
    fn every_provider_is_navigable_but_only_yandex_has_a_form() {
        let config = AppConfig::default();
        let ctx = Context::new(1.0);
        let (_, bindings) = catalog(&ctx);
        let actions = bindings.values().filter_map(|binding| {
            match binding {
                Binding::Button(Action::OpenCloudProvider(provider)) => Some(*provider),
                _ => None,
            }
        }).collect::<Vec<_>>();
        assert_eq!(actions.len(), CloudProvider::ALL.len());
        for provider in CloudProvider::ALL {
            assert!(actions.contains(&provider));
            let built = card(&config, &ctx, provider, &[]);
            assert_eq!(built.save.is_some(), provider == CloudProvider::Yandex);
            if provider != CloudProvider::Yandex {
                assert!(built.bindings.is_empty());
            }
        }
    }
}
