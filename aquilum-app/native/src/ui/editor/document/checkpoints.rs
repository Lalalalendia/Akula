use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::Arc;

use super::super::markdown::State;
use super::Document;
use super::scan::{active_fence_at, scan, walk, FenceCheckpoint};

type Edited = ((usize, usize), isize);

fn rebase_opener(at: usize, edit: &Edited) -> Option<usize> {
    let (from, to) = edit.0;
    // A replaced fence or an insertion exactly at the opening byte makes
    // old scroll state ambiguous; prefer losing the offset to misidentifying.
    if at == from || (from < at && at <= to) {
        None
    } else {
        moved(at, edit)
    }
}

#[derive(Default)]
pub(super) struct Checkpoints {
    points: Vec<FenceCheckpoint>,
    rx: Option<Receiver<Vec<FenceCheckpoint>>>,
    cancel: Option<Arc<AtomicBool>>,
    edits: Vec<Edited>,
}

fn moved(at: usize, &((from, to), delta): &Edited) -> Option<usize> {
    match at {
        at if at <= from => Some(at),
        at if at > to => Some(at.wrapping_add_signed(delta)),
        _ => None,
    }
}

impl Checkpoints {
    fn start(&mut self, rope: crop::Rope, from: usize, state: State, opener: Option<usize>) {
        // A rescan can supersede a still-running worker. Do not leave its
        // discarded receiver as the only cancellation mechanism.
        self.stop_scan();
        let (tx, rx) = channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancelled));
        (self.rx, self.edits) = (Some(rx), Vec::new());
        std::thread::spawn(move || {
            let _zone = aq_trace::zone("фон: контрольные точки разбора");
            scan(rope, from, state, opener, tx, &cancelled);
        });
    }

    fn stop_scan(&mut self) {
        if let Some(cancelled) = self.cancel.take() {
            cancelled.store(true, Ordering::Release);
        }
        self.rx = None;
    }

    fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let before = self.points.len();
        let finished = loop {
            match rx.try_recv() {
                Ok(batch) => {
                    let edits = &self.edits;
                    self.points.extend(batch.into_iter().filter_map(|(at, state, opener)| {
                        let at = edits.iter().try_fold(at, moved)?;
                        let opener = opener.and_then(|mut current| {
                            for edit in edits {
                                current = rebase_opener(current, edit)?;
                            }
                            Some(current)
                        });
                        Some((at, state, opener))
                    }));
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if finished {
            (self.rx, self.edits) = (None, Vec::new());
            self.cancel = None;
        }
        finished || self.points.len() != before
    }

    fn reached(&self, byte: usize) -> bool {
        self.rx.is_none() || self.points.last().is_some_and(|&(at, _, _)| at >= byte)
    }

    fn before(&self, byte: usize) -> Option<FenceCheckpoint> {
        let at = self.points.partition_point(|(at, _, _)| *at <= byte);
        at.checked_sub(1).map(|i| self.points[i])
    }

    fn edited(&mut self, window: (usize, usize), delta: isize) {
        let edit = (window, delta);
        self.points.retain_mut(|(at, _, opener)| {
            let Some(to) = moved(*at, &edit) else { return false };
            *at = to;
            *opener = opener.and_then(|current| rebase_opener(current, &edit));
            true
        });
        if self.rx.is_some() {
            self.edits.push(edit);
        }
    }

    fn cut(&mut self, from: usize) {
        self.stop_scan();
        self.points.retain(|(at, _, _)| *at < from);
    }
}

impl Drop for Checkpoints {
    fn drop(&mut self) {
        if let Some(cancelled) = self.cancel.as_ref() {
            cancelled.store(true, Ordering::Release);
        }
    }
}

impl Document {
    pub(super) fn rescan(&mut self) {
        self.points = Checkpoints::default();
        if self.windowed {
            self.points.start(self.full.clone(), 0, State::default(), None);
        }
    }

    pub(super) fn checkpoint(&mut self, byte: usize) -> State {
        self.points.poll();
        match self.points.before(byte) {
            Some((at, state, _)) if self.points.reached(byte) => walk(&self.full, at, state, 0, |start, _| start < byte),
            _ => State::default(),
        }
    }

    /// Resolve the original opening fence if the mounted window begins
    /// inside a longer block. No completed checkpoint => no guessed identity.
    pub(in crate::ui::editor) fn window_fence_opener(&mut self) -> Option<usize> {
        let base = self.loaded().0;
        if base == 0 || self.markdown.is_none() || !self.fenced_code(0) {
            return None;
        }
        self.points.poll();
        if !self.points.reached(base) {
            return None;
        }
        let (start, state, opener) = self.points.before(base)?;
        active_fence_at(&self.full, start, state, opener, base)
    }

    pub(super) fn refresh_entry(&mut self) {
        if self.base > 0 && self.points.poll() && self.points.reached(self.base) {
            let entry = self.checkpoint(self.base);
            if entry != self.entry {
                self.entry = entry;
                self.classify();
            }
        }
    }

    pub(super) fn rescan_from(&mut self, at: usize) {
        if self.windowed {
            let (from, state, opener) = self.points.before(at).unwrap_or_default();
            self.points.cut(from);
            self.points.start(self.full.clone(), from, state, opener);
        }
    }

    pub(super) fn checkpoints_edited(&mut self, window: (usize, usize), delta: isize, exit_changed: bool) {
        self.points.edited(window, delta);
        if exit_changed {
            self.rescan_from(window.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Checkpoints, State, channel};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    #[test]
    fn superseded_checkpoint_workers_receive_cancellation() {
        let text = crop::Rope::from("content\n".repeat(64));
        let mut points = Checkpoints::default();
        points.start(text.clone(), 0, State::default(), None);
        let first = Arc::clone(points.cancel.as_ref().unwrap());

        points.start(text.clone(), 0, State::default(), None);
        assert!(first.load(Ordering::Acquire), "old worker should be cancelled on restart");
        let second = Arc::clone(points.cancel.as_ref().unwrap());

        points.cut(0);
        assert!(second.load(Ordering::Acquire), "cut should cancel current worker");

        points.start(text, 0, State::default(), None);
        let third = Arc::clone(points.cancel.as_ref().unwrap());
        drop(points);
        assert!(third.load(Ordering::Acquire), "dropping a document must cancel its worker");
    }

    #[test]
    fn points_of_a_running_scan_follow_later_edits_and_the_end_is_reached() {
        let (tx, rx) = channel();
        let mut points = Checkpoints::default();
        points.rx = Some(rx);
        points.edited((100, 200), 5);
        tx.send([50, 150, 300].map(|at| (at, State::default(), None)).to_vec()).unwrap();
        assert!(points.poll());
        assert_eq!(points.points.iter().map(|(at, _, _)| *at).collect::<Vec<_>>(), [50, 305]);
        assert!(!points.reached(400));
        drop(tx);
        assert!(points.poll());
        assert!(points.reached(400));
    }
}
