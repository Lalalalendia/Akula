use masonry::core::{NewWidget, PropertySet, Widget, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::{Flex, SizedBox};

use super::column::EditorColumn;
use super::metadata::metadata_widget;
use super::title::TitleField;
use crate::i18n::t;
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::icons;
use crate::ui::scroll::ScrollArea;
use crate::ui::style;
use crate::ui::tokens::size;
use crate::ui::widgets::{ButtonSize, Caption, IconButton, Slot, TextButton, Variant, Visible};

// Fine-grained editor page profiler enabled exclusively by cargo test.
#[cfg(test)]
#[derive(Clone, Copy, Default)]
pub(crate) struct EditorPageProfile {
    pub constructor: std::time::Duration,
    pub document_rescan: std::time::Duration,
    pub document_window: std::time::Duration,
    pub markdown: std::time::Duration,
    pub page_total: std::time::Duration,
    pub install: std::time::Duration,
    pub slot_pod: std::time::Duration,
    pub slot_remove: std::time::Duration,
    pub slot_notify: std::time::Duration,
    pub slot_framework: std::time::Duration,
    pub save_error_refresh: std::time::Duration,
}

#[cfg(test)]
thread_local! {
    static LAST_PAGE: std::cell::Cell<EditorPageProfile> =
        std::cell::Cell::new(EditorPageProfile::default());
}

#[cfg(test)]
pub(crate) fn take_page_profile() -> EditorPageProfile {
    LAST_PAGE.with(|slot| slot.replace(EditorPageProfile::default()))
}

#[cfg(test)]
pub(crate) fn record_page_install(
    duration: std::time::Duration,
    edit_elapsed: std::time::Duration,
    phases: Option<(std::time::Duration, std::time::Duration, std::time::Duration)>,
) {
    LAST_PAGE.with(|slot| {
        let mut profile = slot.get();
        let (pod, remove, notify) = phases.unwrap_or_default();
        profile.install = duration;
        profile.slot_pod = pod;
        profile.slot_remove = remove;
        profile.slot_notify = notify;
        // Remainder of the RenderRoot::edit call: framework work not timed
        // inside our Slot method, rather than falsely attributing it to a
        // single widget operation.
        profile.slot_framework = edit_elapsed.saturating_sub(pod + remove + notify);
        profile.save_error_refresh = duration.saturating_sub(edit_elapsed);
        slot.set(profile);
    });
}

pub struct NoteView<'a> {
    pub file_name: &'a str,
    pub title: &'a str,
    pub body: crop::Rope,
    #[cfg(test)]
    pub parley_trial_layout: Option<crate::ui::editor::LayoutTrialSnapshot>,
    pub metadata: Option<bool>,
    pub can_back: bool,
    pub can_forward: bool,
    pub focus_mode: bool,
    pub wide: bool,
    pub hero: Option<crate::ui::book::HeroView>,
    pub resolver: Option<std::sync::Arc<dyn crate::ui::editor::resolve::Resolver>>,
    pub smart: crate::ui::editor::Smart,
    pub wake: std::sync::Arc<dyn Fn() + Send + Sync>,
}

pub struct NoteIds {
    pub back: WidgetId,
    pub forward: WidgetId,
    pub focus: WidgetId,
    pub focus_button: Handle<TextButton>,
    pub focus_hidden: Vec<Handle<Visible>>,
    pub search: WidgetId,
    pub more: WidgetId,
    pub error_slot: Handle<Slot>,
    pub name: Handle<Caption>,
    pub title_box: Handle<TitleField>,
    pub title: Handle<TextEditor>,
    pub metadata: Handle<Slot>,
    pub body: Handle<crate::ui::editor::TextEditor>,
    pub hero: Option<(Handle<crate::ui::book::Hero>, crate::ui::book::HeroIds)>,
    pub hero_slot: Handle<Slot>,
    pub spacer: Handle<SizedBox>,
    pub column: Handle<EditorColumn>,
    pub scroll: Handle<ScrollArea<Flex>>,
    pub find: Handle<Slot>,
}

pub struct HeroParts {
    pub widget: NewWidget<dyn Widget>,
    pub hero: Option<(Handle<crate::ui::book::Hero>, crate::ui::book::HeroIds)>,
    pub top: f64,
    pub inset: f64,
}

pub fn hero_parts(view: Option<crate::ui::book::HeroView>) -> HeroParts {
    match view {
        Some(view) => {
            let (hero, ids) = crate::ui::book::hero(view);
            let handle = Handle::of(&hero);
            HeroParts { widget: hero.erased(), hero: Some((handle, ids)), top: size::SPACE_24, inset: size::EDITOR_PADDING_X * 2.0 }
        }
        None => HeroParts {
            widget: NewWidget::new(SizedBox::empty().height(Length::ZERO)).erased(),
            hero: None,
            top: size::EDITOR_PADDING_Y,
            inset: 0.0,
        },
    }
}

pub fn page(view: NoteView<'_>) -> (NewWidget<Flex>, NoteIds) {
    #[cfg(test)]
    let page_started = std::time::Instant::now();
    let back = NewWidget::new(IconButton::new(icons::ARROW_LEFT, t("editor.back"))).disabled(!view.can_back);
    let forward = NewWidget::new(IconButton::new(icons::ARROW_RIGHT, t("editor.forward"))).disabled(!view.can_forward);
    let (back_id, forward_id) = (back.id(), forward.id());
    let focus = NewWidget::new(TextButton::new(t("editor.focusMode"), Variant::Ghost).size(ButtonSize::Xs).pressed(view.focus_mode));
    let focus_id = focus.id();
    let focus_button = Handle::of(&focus);
    let search = NewWidget::new(IconButton::new(icons::SEARCH, t("editor.findInNote")));
    let search_id = search.id();
    let search = Visible::new(search, !view.focus_mode);
    let more = NewWidget::new(IconButton::new(icons::MORE_HORIZONTAL, t("editor.noteActions")));
    let more_id = more.id();
    let name = NewWidget::new(Caption::new(view.file_name).copyable_file_name());
    let name_handle = Handle::of(&name);
    let name = Visible::new(name, !view.focus_mode);
    let left = Flex::row().cross_axis_alignment(CrossAxisAlignment::Center).with_fixed(back).with_fixed(forward);
    let left = Visible::new(NewWidget::new(left).with_props(PropertySet::new().with(Gap::new(Length::px(size::SPACE_4)))), !view.focus_mode);
    let focus_hidden = vec![Handle::of(&left), Handle::of(&name), Handle::of(&search)];
    let right = Flex::row().cross_axis_alignment(CrossAxisAlignment::Center).with_fixed(focus).with_fixed(search).with_fixed(more);
    let right = NewWidget::new(right).with_props(PropertySet::new().with(Gap::new(Length::px(size::SPACE_4))));
    let toolbar = Flex::row().main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::Center).with_fixed(left);
    let toolbar = toolbar.with(name, 1.0).with_fixed(right);
    let toolbar = NewWidget::new(SizedBox::new(NewWidget::new(toolbar)).height(Length::px(size::SIZE_40)))
        .with_props(PropertySet::new().with(Padding::all(Length::px(size::SPACE_4))).with(Gap::new(Length::px(size::SPACE_8))))
        .with_class(style::EDITOR);

    let error_slot = Slot::fit(NewWidget::new(SizedBox::empty()).erased());
    let error_slot_handle = Handle::of(&error_slot);

    let title_area = NewWidget::new(TextEditor::single_line(view.title).styled(style::editor_title)).with_class(style::EDITOR_TEXT);
    let title = Handle::of(&title_area);
    let title_field = TitleField::new(title_area, view.title.is_empty());
    let title_box = Handle::of(&title_field);

    let metadata = Slot::fit(metadata_widget(view.metadata));
    let metadata_handle = Handle::of(&metadata);

    #[cfg(test)]
    let started = std::time::Instant::now();
    let body_area = crate::ui::editor::TextEditor::windowed(view.body);
    #[cfg(test)]
    let constructor = started.elapsed();
    #[cfg(test)]
    let doc = crate::ui::editor::take_document_build_profile();
    #[cfg(test)]
    let started = std::time::Instant::now();
    let body_area = body_area
        .styled(style::editor_text)
        .markdown()
        .report_edits()
        .with_resolver(view.resolver.clone())
        .with_wake(std::sync::Arc::clone(&view.wake))
        .with_smart(view.smart);
    #[cfg(test)]
    let body_area = match view.parley_trial_layout {
        Some(snapshot) => body_area.with_parley_trial(snapshot),
        None => body_area,
    };
    #[cfg(test)]
    let markdown = started.elapsed();
    let body_area = NewWidget::new(body_area).with_class(style::EDITOR_TEXT);
    let body = Handle::of(&body_area);

    let content = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(error_slot)
        .with_fixed(title_field)
        .with_fixed(metadata)
        .with_fixed(body_area);
    let parts = hero_parts(view.hero);
    let hero = parts.hero;
    let hero_slot = Slot::fit(parts.widget);
    let hero_slot_handle = Handle::of(&hero_slot);
    let spacer = NewWidget::new(SizedBox::empty().height(Length::px(parts.top)));
    let spacer_handle = Handle::of(&spacer);
    let column = EditorColumn::new(NewWidget::new(content), parts.inset, view.wide);
    let column_handle = Handle::of(&column);
    let wrapper = NewWidget::new(Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch).with_fixed(spacer).with_fixed(column)).with_props(
        PropertySet::new().with(Padding {
            top: Length::ZERO,
            bottom: Length::px(size::EDITOR_PADDING_Y + size::EDITOR_SCROLL_PAST_END),
            left: Length::px(size::EDITOR_PADDING_X),
            right: Length::px(size::EDITOR_PADDING_X),
        }),
    );
    let scroller = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch).with_fixed(hero_slot).with_fixed(wrapper);
    let scroller = NewWidget::new(scroller).with_class(style::EDITOR);
    let scroll = NewWidget::new(ScrollArea::new(scroller));
    let scroll_handle = Handle::of(&scroll);
    let find = Slot::fit(NewWidget::new(SizedBox::empty()).erased());
    let find_handle = Handle::of(&find);
    let page = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(toolbar)
        .with_fixed(find)
        .with(scroll, 1.0);
    let page = NewWidget::new(page).with_class(style::EDITOR);
    let ids = NoteIds {
        back: back_id,
        forward: forward_id,
        focus: focus_id,
        focus_button,
        focus_hidden,
        search: search_id,
        more: more_id,
        error_slot: error_slot_handle,
        name: name_handle,
        title_box,
        title,
        metadata: metadata_handle,
        body,
        hero,
        hero_slot: hero_slot_handle,
        spacer: spacer_handle,
        column: column_handle,
        scroll: scroll_handle,
        find: find_handle,
    };
    #[cfg(test)]
    LAST_PAGE.with(|slot| slot.set(EditorPageProfile {
        constructor,
        document_rescan: doc.rescan,
        document_window: doc.load,
        markdown,
        page_total: page_started.elapsed(),
        install: std::time::Duration::ZERO,
        slot_pod: std::time::Duration::ZERO,
        slot_remove: std::time::Duration::ZERO,
        slot_notify: std::time::Duration::ZERO,
        slot_framework: std::time::Duration::ZERO,
        save_error_refresh: std::time::Duration::ZERO,
    }));
    (page, ids)
}
