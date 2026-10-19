use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use super::super::markdown::State;

const EVERY: usize = 1024;
const BATCH: usize = 64;

/// A stable byte offset for an active fenced-block opener, stored beside
/// the async parser's checkpoint to avoid rescanning the full document.
pub(super) type FenceCheckpoint = (usize, State, Option<usize>);

fn update_opener(at: usize, state: &State, previous: &mut Option<usize>, was_open: &mut bool, active: &mut Option<usize>) {
    let open = state.has_open_fence();
    if !open {
        *active = None;
    } else if !*was_open {
        // The preceding line opened the fence: state is observed before "at".
        *active = *previous;
    }
    *was_open = open;
    *previous = Some(at);
}

/// Resolves the active opener by replaying only the interval between the
/// latest available checkpoint and the beginning of the loaded window.
pub(super) fn active_fence_at(rope: &crop::Rope, from: usize, state: State, opener: Option<usize>, target: usize) -> Option<usize> {
    if from > target {
        return None;
    }
    let (mut previous, mut was_open, mut active) = (None, state.has_open_fence(), opener);
    walk(rope, from, state, 0, |at, before| {
        if at > target { return false; }
        update_opener(at, before, &mut previous, &mut was_open, &mut active);
        at < target
    });
    active
}

pub(super) fn walk(text: &crop::Rope, from: usize, mut state: State, front: usize, mut visit: impl FnMut(usize, &State) -> bool) -> State {
    let (mut prev, mut start, mut at, mut go) = (String::new(), None, from, true);
    crate::rope::each_line(text.byte_slice(from..), |line| {
        if let Some(begin) = start.filter(|begin| *begin >= front) {
            go = visit(begin, &state);
            if !go {
                return false;
            }
            state.step(&prev, Some(line));
        }
        prev.clear();
        prev.push_str(line);
        start = Some(at);
        at += line.len() + 1;
        true
    });
    if go && start.is_some_and(|begin| begin >= front && visit(begin, &state)) {
        state.step(&prev, None);
    }
    state
}

const CANCEL_EVERY: usize = 256;

// The document can be dropped or rescanned while this background walk is
// still running. Check cancellation at bounded intervals instead of waiting
// for the next 64 * 1024-line checkpoint batch to discover a closed receiver.
pub(super) fn scan(rope: crop::Rope, from: usize, state: State, opener: Option<usize>, tx: Sender<Vec<FenceCheckpoint>>, cancelled: &AtomicBool) {
    if cancelled.load(Ordering::Acquire) {
        return;
    }
    let front = if from == 0 { crate::frontmatter::end_of(&rope).map_or(0, |end| end + 1) } else { 0 };
    let (mut batch, mut lines) = (Vec::new(), 0usize);
    let (mut previous, mut was_open, mut active) = (None, state.has_open_fence(), opener);
    walk(&rope, from, state, front, |start, state| {
        if lines % CANCEL_EVERY == 0 && cancelled.load(Ordering::Acquire) {
            return false;
        }
        update_opener(start, state, &mut previous, &mut was_open, &mut active);
        if lines % EVERY == 0 {
            batch.push((start, *state, active));
            if batch.len() == BATCH && tx.send(std::mem::take(&mut batch)).is_err() {
                return false;
            }
        }
        lines += 1;
        true
    });
    if !cancelled.load(Ordering::Acquire) {
        let _ = tx.send(batch);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::{channel, TryRecvError};

    use super::{scan, State};

    #[test]
    fn cancelled_scan_produces_no_checkpoint_batches() {
        let (tx, rx) = channel();
        let cancelled = AtomicBool::new(true);
        let text = crop::Rope::from("отменить\n".repeat(100_000));
        scan(text, 0, State::default(), None, tx, &cancelled);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn ordinary_scan_still_reports_checkpoints() {
        let (tx, rx) = channel();
        let cancelled = AtomicBool::new(false);
        let text = crop::Rope::from("первый\nвторой\n");
        scan(text, 0, State::default(), None, tx, &cancelled);
        let points: Vec<_> = rx.try_iter().flatten().collect();
        assert_eq!(points.first().map(|point| point.0), Some(0));
    }
}
