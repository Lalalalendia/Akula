use super::{Button, Editor, label, preview::Preview};
use crate::i18n::t_with;
use crate::ui::dialog::header;
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::scroll::ScrollArea;
use crate::ui::text;
use crate::ui::widgets::{Leading, Rule, TextButton, Variant};
use crate::ui::{style, theme};
use masonry::core::{NewWidget, PropertySet};
use masonry::kurbo::Axis;
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::{Flex, SizedBox};

impl Editor {
    pub(super) fn card(&mut self) -> NewWidget<Flex> {
        self.buttons.clear();
        self.input = None;
        let mut top = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
        let busy = self.pending.is_some();
        let editing = self.edit.is_some();
        let close =
            (!editing).then(|| self.button(Button::Close, label("dismiss"), Variant::Ghost));
        top = top
            .with_fixed(header(&label("transcript"), close))
            .with_fixed(Rule::new(Axis::Horizontal, || {
                theme::current().border_default
            }));

        let page = self.page.as_ref();
        let source = page
            .and_then(|page| std::path::Path::new(&page.source).file_name())
            .map(|file| file.to_string_lossy().into_owned());
        let number = page.map_or(0, |page| page.number);
        let pages = page.map_or(1, |page| page.pages);
        let rows = page.map_or_else(Vec::new, |page| page.rows.clone());
        let preview = self.playback.borrow().is_open().then(|| {
            let cues = rows
                .iter()
                .filter(|row| !row.cue.text.is_empty())
                .map(|row| row.cue.clone())
                .collect();
            let preview = Preview::new(self.playback.clone(), cues);
            NewWidget::new(SizedBox::new(preview).height(Length::px(136.0)))
        });
        let count = t_with(
            "settings.system.speech.transcriptPage",
            &[
                ("number", &(number + 1).to_string()),
                ("total", &pages.to_string()),
            ],
        );
        let mut controls = Flex::row().cross_axis_alignment(CrossAxisAlignment::Center);
        if number > 0 && !busy && !editing {
            controls = controls.with_fixed(self.button(
                Button::Page(number - 1),
                label("transcriptPrevious"),
                Variant::Ghost,
            ));
        }
        controls = controls.with(message(&count), 1.0);
        if number + 1 < pages && !busy && !editing {
            controls = controls.with_fixed(self.button(
                Button::Page(number + 1),
                label("transcriptNext"),
                Variant::Ghost,
            ));
        }
        if !busy && !editing {
            controls = controls.with_fixed(self.button(
                Button::Page(number),
                label("refresh"),
                Variant::Ghost,
            ));
        }
        if self.page.is_some() && !busy {
            controls = controls.with_fixed(self.button(
                Button::Toggle,
                label("transcriptPlayPause"),
                Variant::Ghost,
            ));
        }

        let empty = rows.is_empty();
        let mut list = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
        for row in rows {
            let suffix = if row.cue.text.is_empty() {
                label("transcriptHidden")
            } else {
                row.cue.text.clone()
            };
            let time = stamp(row.cue.start_ms);
            let mut line = Flex::row().cross_axis_alignment(CrossAxisAlignment::Start);
            if busy {
                line = line.with_fixed(message(&time));
            } else {
                line = line.with_fixed(self.button(
                    Button::Jump(row.cue.start_ms),
                    time,
                    Variant::Ghost,
                ));
            }
            line = line.with(message(&suffix), 1.0);
            if !busy && !editing {
                line = line.with_fixed(self.button(
                    Button::Edit(row.id),
                    label("transcriptEdit"),
                    Variant::Ghost,
                ));
            }
            list = list.with_fixed(spaced(line));
        }
        if empty && self.page.is_some() {
            list = list.with_fixed(message(&label("transcriptEmpty")));
        }
        let list = NewWidget::new(ScrollArea::new(spaced(list)));
        self.scroll = Some(Handle::of(&list));

        let mut body = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(source) = source {
            body = body.with_fixed(message(&source));
        }
        if let Some(preview) = preview {
            body = body.with_fixed(preview);
        }
        body = body.with_fixed(spaced(controls));
        if let Some(error) = self.error.as_ref() {
            body = body.with_fixed(message(error));
        } else if busy {
            body = body.with_fixed(message(&label("transcriptLoading")));
        }
        body = body.with(list, 1.0);
        if let Some((row, draft)) = self.edit.as_ref() {
            let at = stamp(row.cue.start_ms);
            body = body.with_fixed(self.edit_form(&at, busy, draft.clone()));
        }
        NewWidget::new(top.with(spaced(body), 1.0))
    }

    fn edit_form(&mut self, at: &str, busy: bool, draft: String) -> NewWidget<Flex> {
        let heading = t_with("settings.system.speech.transcriptEditAt", &[("time", at)]);
        let mut body = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(message(&heading));
        if !busy {
            let input = NewWidget::new(TextEditor::new(&draft).styled(|| style::ui_text(20.0)))
                .with_class(style::FIELD);
            self.input = Some(Handle::of(&input));
            let input = NewWidget::new(ScrollArea::new(input));
            let input = NewWidget::new(SizedBox::new(input).height(Length::px(70.0)))
                .with_class(style::CARD)
                .with_props(PropertySet::new().with(Padding::all(Length::px(8.0))));
            let commands = Flex::row()
                .with_fixed(self.button(Button::Save, label("transcriptSave"), Variant::Primary))
                .with_fixed(self.button(
                    Button::Discard,
                    label("transcriptDiscard"),
                    Variant::Ghost,
                ));
            body = body.with_fixed(input).with_fixed(spaced(commands));
        }
        spaced(body)
    }

    fn button(&mut self, action: Button, title: String, variant: Variant) -> NewWidget<TextButton> {
        let widget = NewWidget::new(TextButton::new(title, variant));
        self.buttons.insert(widget.id(), action);
        widget
    }
}

fn spaced(flex: Flex) -> NewWidget<Flex> {
    NewWidget::new(flex).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(6.0)))
            .with(Padding::all(Length::px(6.0))),
    )
}

fn message(text: &str) -> NewWidget<crate::ui::widgets::Label> {
    text::selectable(text, |theme| theme.text_secondary, None, Leading::Px(20.0))
}

fn stamp(ms: i64) -> String {
    let seconds = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60,
        ms % 1000
    )
}
