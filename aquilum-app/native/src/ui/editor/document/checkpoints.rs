use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::Arc;

use super::super::markdown::State;
use super::Document;
use super::scan::{scan, walk};

type Edited = ((usize, usize), isize);

#[derive(Default)]
pub(super) struct Checkpoints {
    points: Vec<(usize, State)>,
    rx: Option<Receiver<Vec<(usize, State)>>>,
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
    fn start(&mut self, rope: crop::Rope, from: usize, state: State) {
        // A rescan can supersede a still-running worker. Do not leave its
        // discarded receiver as the only cancellation mechanism.
        self.stop_scan();
        let (tx, rx) = channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancelled));
        (self.rx, self.edits) = (Some(rx), Vec::new());
        std::thread::spawn(move || {
            let _zone = aq_trace::zone("фон: контрольные точки разбора");
            scan(rope, from, state, tx, &cancelled);
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
                    self.points.extend(batch.into_iter().filter_map(|(at, state)| Some((edits.iter().try_fold(at, moved)?, state))));
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
        self.rx.is_none() || self.points.last().is_some_and(|&(at, _)| at >= byte)
    }

    fn before(&self, byte: usize) -> Option<(usize, State)> {
        let at = self.points.partition_point(|(at, _)| *at <= byte);
        at.checked_sub(1).map(|i| self.points[i])
    }

    fn edited(&mut self, window: (usize, usize), delta: isize) {
        let edit = (window, delta);
        self.points.retain_mut(|(at, _)| moved(*at, &edit).map(|to| *at = to).is_some());
        if self.rx.is_some() {
            self.edits.push(edit);
        }
    }

    fn cut(&mut self, from: usize) {
        self.stop_scan();
        self.points.retain(|(at, _)| *at < from);
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
            self.points.start(self.full.clone(), 0, State::default());
        }
    }

    pub(super) fn checkpoint(&mut self, byte: usize) -> State {
        self.points.poll();
        match self.points.before(byte) {
            Some((at, state)) if self.points.reached(byte) => walk(&self.full, at, state, 0, |start, _| start < byte),
            _ => State::default(),
        }
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
            let (from, state) = self.points.before(at).unwrap_or_default();
            self.points.cut(from);
            self.points.start(self.full.clone(), from, state);
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
        points.start(text.clone(), 0, State::default());
        let first = Arc::clone(points.cancel.as_ref().unwrap());

        points.start(text.clone(), 0, State::default());
        assert!(first.load(Ordering::Acquire), "old worker should be cancelled on restart");
        let second = Arc::clone(points.cancel.as_ref().unwrap());

        points.cut(0);
        assert!(second.load(Ordering::Acquire), "cut should cancel current worker");

        points.start(text, 0, State::default());
        let third = Arc::clone(points.cancel.as_ref().unwrap());
        drop(points);
        assert!(third.load(Ordering::Acquire), "dropping a document must cancel its worker");
    }

    #[test]
    fn points_of_a_running_scan_follow_later_edits_and_the_end_is_reached() {
        let (tx, rx) = channel();
        let mut points = Checkpoints { rx: Some(rx), ..Checkpoints::default() };
        points.edited((100, 200), 5);
        tx.send([50, 150, 300].map(|at| (at, State::default())).to_vec()).unwrap();
        assert!(points.poll());
        assert_eq!(points.points.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [50, 305]);
        assert!(!points.reached(400));
        drop(tx);
        assert!(points.poll());
        assert!(points.reached(400));
    }
}
