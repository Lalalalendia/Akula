mod card;
mod preview;

use crate::i18n::t;
use crate::ui::dialog::{CardSize, DialogAnswer, Modal};
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::Pressed;
use crate::video::preview::Playback;
use crate::workspace::speech_jobs::Dispatcher;
use crate::workspace::speech_jobs::transcript::{Reply, Response, Task};
use aquilum_core::background_jobs::{JobId, speech::edits, speech::playback::Transcript};
use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, NewWidget, WidgetId};
use masonry::widgets::Flex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Button {
    Close,
    Page(i64),
    Edit(edits::CueId),
    Jump(i64),
    Toggle,
    Save,
    Discard,
}

pub(crate) struct Editor {
    layer: Option<WidgetId>,
    job: JobId,
    page: Option<edits::Page>,
    edit: Option<(edits::Row, String)>,
    pending: Option<Receiver<Response>>,
    error: Option<String>,
    media: Option<Transcript>,
    pending_playback: Option<Button>,
    playback: Rc<RefCell<Playback>>,
    buttons: HashMap<WidgetId, Button>,
    input: Option<Handle<TextEditor>>,
    scroll: Option<Handle<ScrollArea<Flex>>>,
}

impl Editor {
    pub(crate) fn new(dispatcher: &Dispatcher, job: JobId) -> (NewWidget<Modal>, Self) {
        let mut editor = Self {
            layer: None,
            job,
            page: None,
            edit: None,
            pending: Some(dispatcher.transcript(job, Task::Page(0))),
            error: None,
            media: None,
            pending_playback: None,
            playback: Rc::default(),
            buttons: HashMap::new(),
            input: None,
            scroll: None,
        };
        let layer = Modal::new(
            editor.card(),
            &label("transcript"),
            CardSize::Window(super::card_size),
            Role::Dialog,
            false,
        );
        editor.layer = Some(layer.id());
        (layer, editor)
    }

    pub(crate) fn layer(&self) -> WidgetId {
        self.layer.expect("transcript modal has a layer")
    }

    pub(crate) fn action(
        &mut self,
        root: &mut RenderRoot,
        dispatcher: &Dispatcher,
        id: WidgetId,
        action: &ErasedAction,
    ) -> bool {
        let button = if id == self.layer()
            && action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Cancel)
        {
            Button::Close
        } else if let (Some(_), Some(button)) =
            (action.downcast_ref::<Pressed>(), self.buttons.get(&id))
        {
            *button
        } else {
            return false;
        };
        if button == Button::Close && self.edit.is_none() {
            return true;
        }
        if self.pending.is_some() {
            return false;
        }
        self.error = None;
        match button {
            Button::Close => {
                root.focus_on(self.input.map(Handle::id));
                return false;
            }
            Button::Page(number) if self.edit.is_none() => {
                self.pending = Some(dispatcher.transcript(self.job, Task::Page(number)));
            }
            Button::Edit(id) => {
                if let Some(row) = self
                    .page
                    .as_ref()
                    .and_then(|page| page.rows.iter().find(|row| row.id == id))
                {
                    self.edit = Some((row.clone(), row.cue.text.clone()));
                }
            }
            Button::Save => {
                if let Some((row, draft)) = self.edit.as_mut() {
                    if let Some(input) = self.input.and_then(|input| input.get(root)) {
                        *draft = input.inner().text();
                    }
                    self.pending = Some(
                        dispatcher
                            .transcript(self.job, Task::Save(row.id, row.revision, draft.clone())),
                    );
                }
            }
            Button::Discard => self.edit = None,
            Button::Jump(_) | Button::Toggle => {
                self.capture_draft(root);
                if self.media.is_some() {
                    self.play(button);
                } else {
                    self.pending_playback = Some(button);
                    self.pending = Some(dispatcher.transcript(self.job, Task::VerifyMedia));
                }
            }
            Button::Page(_) => return false,
        }
        self.render(root, false);
        false
    }

    pub(crate) fn poll(&mut self, root: &mut RenderRoot) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let reply = match pending.try_recv() {
            Ok(response) => response,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(label("transcriptUnavailable")),
        };
        self.pending = None;
        let playback = self.pending_playback.take();
        let mut reset_scroll = false;
        match reply {
            Ok(Reply::Page(page)) => {
                reset_scroll = self
                    .page
                    .as_ref()
                    .is_none_or(|old| old.number != page.number);
                self.page = Some(page);
            }
            Ok(Reply::Saved(row)) => {
                if let Some(existing) = self
                    .page
                    .as_mut()
                    .and_then(|page| page.rows.iter_mut().find(|existing| existing.id == row.id))
                {
                    *existing = row;
                }
                self.edit = None;
            }
            Ok(Reply::Media(media)) => {
                self.media = Some(media);
                if let Some(button) = playback {
                    self.play(button);
                }
            }
            Err(error) => self.error = Some(error.chars().take(300).collect()),
        }
        self.render(root, reset_scroll);
    }

    fn capture_draft(&mut self, root: &RenderRoot) {
        if let (Some((_, draft)), Some(input)) = (
            self.edit.as_mut(),
            self.input.and_then(|input| input.get(root)),
        ) {
            *draft = input.inner().text();
        }
    }

    fn play(&mut self, button: Button) {
        let Some(path) = self.media.as_ref().and_then(Transcript::current_path) else {
            self.media = None;
            *self.playback.borrow_mut() = Playback::default();
            self.error = Some(label("transcriptMediaChanged"));
            return;
        };
        let mut preview = self.playback.borrow_mut();
        if !preview.open(path) {
            self.error = Some(label("transcriptNoPlayer"));
            return;
        }
        match button {
            Button::Jump(ms) => {
                preview.seek(ms);
                if let Some(state) = crate::video::state(path).filter(|state| state.duration > 0.0)
                {
                    crate::video::seek(path, ms as f64 / 1000.0 / state.duration);
                }
            }
            Button::Toggle => preview.toggle(),
            _ => {}
        }
    }

    fn render(&mut self, root: &mut RenderRoot, reset_scroll: bool) {
        let offset = self
            .scroll
            .and_then(|handle| handle.get(root))
            .map(|area| area.inner().offset());
        let card = self.card();
        root.edit_widget(self.layer(), |mut widget| {
            Modal::set_card(&mut widget.downcast(), card, &label("transcript"))
        });
        if !reset_scroll && let (Some(scroll), Some(offset)) = (self.scroll, offset) {
            scroll.edit(root, |mut area| ScrollArea::set_offset(&mut area, offset));
        }
        root.focus_on(self.input.map(Handle::id).or(Some(self.layer())));
    }
}

fn label(key: &str) -> String {
    t(&format!("settings.system.speech.{key}"))
}

#[cfg(test)]
pub(crate) mod probe;
