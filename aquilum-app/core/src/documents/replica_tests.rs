use super::*;

fn edit(from: usize, to: usize, insert: &str) -> TextEdit {
    TextEdit { from, to, insert: insert.into() }
}

#[test]
fn applies_edits_at_utf16_positions() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "😀 мир\n")], "test");
    replica.apply_edits(&[edit(2, 2, "!"), edit(6, 6, " вокруг")], "test");
    assert_eq!(replica.text(), "😀! мир вокруг\n");
}

#[test]
fn applies_a_batch_of_ordered_edits_against_the_original_text() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "a\nb\nc\n")], "test");
    replica.apply_edits(&[edit(0, 1, "A"), edit(4, 5, "C")], "test");
    assert_eq!(replica.text(), "A\nb\nC\n");
}

#[test]
fn restores_from_a_snapshot_plus_later_updates() {
    let original = Replica::new();
    original.apply_edits(&[edit(0, 0, "первый\n")], "test");
    let snapshot = original.state();
    let update = original.apply_edits(&[edit(7, 7, "второй\n")], "test");

    let restored = Replica::restore(&[snapshot, update]).expect("restore");
    assert_eq!(restored.text(), "первый\nвторой\n");
}

#[test]
fn concurrent_replicas_converge_by_exchanging_state() {
    let left = Replica::new();
    left.apply_edits(&[edit(0, 0, "общий текст\n")], "test");
    let right = Replica::restore(&[left.state()]).expect("restore");

    left.apply_edits(&[edit(0, 0, "слева ")], "test");
    right.apply_edits(&[edit(12, 12, "справа\n")], "test");

    let (from_left, from_right) = (left.state(), right.state());
    right.apply_update(&from_left, "remote").expect("apply");
    left.apply_update(&from_right, "remote").expect("apply");

    assert_eq!(left.text(), right.text());
    assert_eq!(left.text(), "слева общий текст\nсправа\n");
}

#[test]
fn rejects_garbage_instead_of_corrupting_the_document() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "текст")], "test");
    assert!(replica.apply_update(&[0xff, 0x00, 0x13], "remote").is_err());
    assert_eq!(replica.text(), "текст");
}

const YJS_UPDATE: &str = "0105cf8bf2ee080004010a636f64656d6972726f7204f09f988084cf8bf2ee08010d20d09fd180d0b8d0b2d0b5d18281cf8bf2ee08080284cf8bf2ee080a0ad0b8d18020f09d849e0ac4cf8bf2ee0801cf8bf2ee0802012101cf8bf2ee08010902";

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex")).collect()
}

#[test]
fn reads_an_update_written_by_yjs_with_the_same_positions() {
    let replica = Replica::restore(&[from_hex(YJS_UPDATE)]).expect("yjs update");
    assert_eq!(replica.text(), "😀! Приветир 𝄞\n");
    replica.apply_edits(&[edit(2, 3, "?")], "test");
    assert_eq!(replica.text(), "😀? Приветир 𝄞\n");
}

const YJS_DOC_WITH_POSITION: &str = "0102e8f4fae1040004010a636f64656d6972726f7219f09f988020d09fd180d0b8d0b2d0b5d1820ad0bcd0b8d1800a44e8f4fae104000dd0bdd0b0d187d0b0d0bbd0be2000";
const YJS_POSITION: &str = "00e8f4fae1040500";

#[test]
fn resolves_a_relative_position_saved_by_yjs_after_later_edits() {
    let replica = Replica::restore(&[from_hex(YJS_DOC_WITH_POSITION)]).expect("yjs update");
    assert_eq!(replica.resolve_position(&from_hex(YJS_POSITION)), Some(12));
}

#[test]
fn encodes_positions_in_the_same_bytes_as_yjs() {
    let replica = Replica::restore(&[from_hex(YJS_DOC_WITH_POSITION)]).expect("yjs update");
    assert_eq!(replica.encode_position(12), Some(from_hex(YJS_POSITION)));
}

#[test]
fn a_position_follows_text_inserted_before_it() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "абв где")], "test");
    let position = replica.encode_position(4).expect("position");
    replica.apply_edits(&[edit(0, 0, "😀 ")], "test");
    assert_eq!(replica.resolve_position(&position), Some(7));
}

#[test]
fn garbage_positions_do_not_resolve() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "текст")], "test");
    assert_eq!(replica.resolve_position(&[0xff, 0xff]), None);
}


#[test]
fn stage_rejects_surrogate_split_without_mutating_the_live_replica() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "a😀b")], "setup");
    let original = replica.state();

    for bad in [edit(2, 2, "X"), edit(2, 3, ""), edit(1, 2, ""), edit(2, 4, "")] {
        let outcome = replica.stage_edits(&[vec![bad]]);
        assert!(outcome.is_err(), "surrogate split must be rejected");
        assert_eq!(replica.text(), "a😀b");
        assert_eq!(replica.state(), original);
    }
}

#[test]
fn stage_accepts_edits_on_both_sides_of_a_surrogate_pair() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "a😀b")], "setup");
    let delta = replica.stage_edits(&[vec![edit(1, 3, "🎉")]]).expect("scalar-aligned");
    replica.apply_update(&delta, "test").expect("apply");
    assert_eq!(replica.text(), "a🎉b");
}

#[test]
fn stage_rejects_a_late_invalid_batch_atomically() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "a😀b")], "setup");
    let original = replica.state();
    assert!(replica.stage_edits(&[
        vec![edit(0, 1, "A")],
        vec![edit(2, 2, "X")],
    ]).is_err());
    assert_eq!(replica.state(), original);
}

#[test]
fn stage_rejects_out_of_bounds_and_overlapping_edits() {
    let replica = Replica::new();
    replica.apply_edits(&[edit(0, 0, "abc")], "setup");
    for edits in [
        vec![edit(2, 4, "")],
        vec![edit(2, 1, "")],
        vec![edit(1, 2, ""), edit(0, 0, "!")],
        vec![edit(0, 2, ""), edit(1, 3, "")],
        vec![edit(u32::MAX as usize + 1, u32::MAX as usize + 1, "!")],
    ] {
        assert!(replica.stage_edits(&[edits]).is_err());
        assert_eq!(replica.text(), "abc");
    }
}

/// Diagnostic only. Compare staging against an ordinary Yrs edit on the same
/// document size. Do not use a brittle elapsed-time threshold on shared CI.
#[test]
#[ignore]
fn benchmark_editor_staging() {
    use std::hint::black_box;
    use std::time::Instant;

    for size in [1_024, 16_384, 65_536] {
        let text = "a".repeat(size);
        let base = Replica::new();
        base.apply_edits(&[edit(0, 0, &text)], "setup");
        let batches = vec![vec![edit(size, size, "x")]];
        let rounds = if size == 65_536 { 12 } else { 40 };
        let mut staging_times = Vec::new();
        for _ in 0..rounds {
            let start = Instant::now();
            black_box(base.stage_edits(&batches).expect("stage"));
            staging_times.push(start.elapsed().as_micros() as u64);
        }

        let direct = Replica::new();
        direct.apply_edits(&[edit(0, 0, &text)], "setup");
        let mut direct_times = Vec::new();
        for _ in 0..rounds {
            let start = Instant::now();
            black_box(direct.apply_edits(&[edit(direct.len(), direct.len(), "x")], "direct"));
            direct_times.push(start.elapsed().as_micros() as u64);
        }
        staging_times.sort_unstable();
        direct_times.sort_unstable();
        let staged_us = staging_times[staging_times.len() / 2];
        let direct_us = direct_times[direct_times.len() / 2];
        println!(
            "BENCH-EDITOR size_bytes={size} staging_median_us={staged_us} direct_median_us={direct_us} staging_overhead_x={:.2}",
            staged_us as f64 / (direct_us.max(1) as f64)
        );
    }
}
