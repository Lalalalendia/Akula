use super::super::markdown::Block;
use super::Document;
use super::tests::markdown;

fn lines(n: usize) -> String {
    (0..n).map(|i| format!("строка {i}\n")).collect()
}

fn windowed(text: &str) -> Document {
    let mut doc = Document::windowed(crop::Rope::from(text));
    doc.set_markdown(markdown("").markdown.take());
    doc
}

fn agrees(doc: &Document, text: &str) {
    let (base, window) = doc.loaded();
    assert_eq!(doc.whole(), text);
    assert_eq!(window, &text[base..base + window.len()]);
}

#[test]
fn undo_of_large_replacement_remounts_a_bounded_window_before_layout() {
    let original = lines(30_000);
    let replacement = "replacement";
    let mut doc = windowed(replacement);
    assert_eq!(doc.loaded_range(), (0, replacement.len()));

    // Undo a select-all paste: the single-line mounted window must not
    // become a 30,000-line synchronous layout workload.
    doc.replace(0..replacement.len(), &original);
    agrees(&doc, &original);
    let (start, end) = doc.loaded_range();
    assert!(start > 0, "large undo must remount near the insertion end");
    assert_eq!(end, original.len(), "restored tail remains in the mounted window");
    assert!(doc.line_count() < 2_000, "unbounded restored GUI window: {} lines", doc.line_count());

    // Complete content must remain available through the persistent Rope,
    // including the originally unloaded first line.
    assert_eq!(doc.copy(0..original.find('\n').unwrap()), "строка 0");
    doc.include(0);
    assert_eq!(doc.loaded().0, 0);
    agrees(&doc, &original);
}

#[test]
fn large_few_line_replacement_is_bounded_by_bytes_too() {
    // Long unbroken lines are indivisible by design. Use ordinary lines
    // where the 512 KiB byte limit, rather than the line count, decides.
    let original = ("X".repeat(700) + "\n").repeat(1_000);
    let mut doc = windowed("tiny");
    doc.replace(0..4, &original);
    agrees(&doc, &original);
    assert!(doc.loaded().0 > 0, "byte-bounded window was not remounted");
    assert!(doc.loaded().1.len() <= super::window::SPAN_BYTES + 1_000,
        "large insertion expanded the mounted source beyond the byte budget");
}

#[test]
fn a_far_position_moves_the_window_instead_of_stretching_it() {
    let text = lines(50_000);
    let mut doc = windowed(&text);
    let size = doc.loaded().1.len();
    assert!(size < text.len() / 4);
    doc.include(text.len() - 3);
    let (base, end) = doc.loaded_range();
    assert!(base <= text.len() - 3 && text.len() - 3 <= end);
    assert!(end - base <= size * 2, "окно растянулось: {} байт", end - base);
    agrees(&doc, &text);
}

#[test]
fn edits_outside_the_window_land_in_the_text() {
    let mut text = lines(50_000);
    let mut doc = windowed(&text);
    doc.include(text.len() / 2);
    let edits: [fn(&str) -> (std::ops::Range<usize>, &'static str); 3] = [|_| (0..0, "до\n"), |t| (0..t.find('\n').unwrap_or(0) + 1, ""), |t| (t.len()..t.len(), "после")];
    for edit in edits {
        let (range, insert) = edit(&text);
        doc.replace(range.clone(), insert);
        text.replace_range(range, insert);
        agrees(&doc, &text);
    }
    let end = text.len();
    doc.replace(0..end, "всё");
    agrees(&doc, "всё");
}

#[test]
fn a_window_inside_a_long_code_block_knows_it_is_code() {
    let text = format!("```\n{}```\nтекст\n", lines(20_000));
    let mut doc = windowed(&text);
    doc.include(text.len() / 2);
    let code = (0..300).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(10));
        doc.track((0.0, 0.0));
        doc.block(0).is_code()
    });
    assert!(code, "окно в середине блока кода разобрано как текст: {:?}", doc.block(0));
    assert!(matches!(doc.block(doc.line_count() - 1), Block::Code { .. } | Block::Text));
}

#[test]
fn lines_in_view_stay_laid_when_the_window_starts_below_the_top() {
    use masonry::parley::{FontContext, LayoutContext};
    let text = lines(50_000);
    let mut doc = windowed(&text);
    let (mut fcx, mut lcx, styles) = (FontContext::default(), LayoutContext::new(), masonry::core::StyleSet::new(16.0));
    doc.layout(&mut fcx, &mut lcx, &styles, Some(600.0), None, None);
    doc.include(text.len() / 2);
    let y = doc.y_of(text.len() / 2);
    doc.keep = (y - 400.0, y + 400.0);
    doc.layout(&mut fcx, &mut lcx, &styles, Some(600.0), None, None);
    let line = doc.at_y(y);
    assert!(doc.above > 0.0, "окно не ушло от начала заметки");
    assert!(doc.laid.get(line).is_some(), "видимая строка выброшена из раскладки: keep в координатах заметки, высоты — в координатах окна");
}

#[test]
fn initial_window_is_bounded_but_far_jumps_keep_the_complete_document() {
    let text = lines(50_000);
    let mut doc = windowed(&text);
    let first = doc.line_count();
    assert!(
        (1024..=1600).contains(&first),
        "initial Markdown window should retain lookahead without parsing 3000 lines: {first}"
    );
    agrees(&doc, &text);
    for byte in [text.len() / 3, text.len() - 2000, text.len() / 2, 0, text.len() - 1] {
        doc.include(byte);
        let (start, end) = doc.loaded_range();
        assert!(start <= byte && byte <= end, "far byte {byte} outside {start}..{end}");
        agrees(&doc, &text);
    }
}

#[test]
fn long_code_fence_remains_a_code_block_after_a_jump_with_smaller_window() {
    let text = format!("```rust\n{}\n```\n# после блока\n", lines(30_000));
    let mut doc = windowed(&text);
    doc.include(text.len() / 2);
    let reached = (0..300).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(10));
        doc.track((0.0, 0.0));
        doc.block(0).is_code()
    });
    assert!(reached, "the middle of a huge code fence must classify as code");
    agrees(&doc, &text);
}

#[test]
fn windowed_code_recovers_original_opener_outside_loaded_lines() {
    let text = format!("before\n~~~rust\n{}~~~\nafter\n", lines(30_000));
    let opener = text.find("~~~rust").unwrap();
    let mut doc = windowed(&text);
    doc.include(text.len() / 2);
    assert!(doc.loaded().0 > opener, "opener must be outside the mounted window");
    let mut recovered = None;
    for _ in 0..300 {
        doc.track((0.0, 0.0));
        recovered = doc.window_fence_opener();
        if recovered.is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(recovered, Some(opener), "windowed parser must retain global fence identity");
}

#[test]
fn checkpoints_identify_the_second_windowed_fence_not_the_first() {
    let text = format!("~~~rust\n{}~~~\nprose\n~~~python\n{}~~~\n", lines(5_000), lines(7_000));
    let second = text.find("~~~python").unwrap();
    let mut doc = windowed(&text);
    doc.include(text.len() - 300);
    assert!(doc.loaded().0 > second, "second opener must be outside window");
    let mut recovered = None;
    for _ in 0..300 {
        doc.track((0.0, 0.0));
        recovered = doc.window_fence_opener();
        if recovered.is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(recovered, Some(second), "offset must never migrate to an older code block");
}


#[test]
fn stale_window_byte_offsets_never_underflow_or_split_utf8() {
    let source = lines(70_000);
    let mut doc = windowed(&source);
    doc.include(source.len());
    let (base, loaded) = doc.loaded();
    assert!(base > 0, "must reproduce a far-away mounted window");
    assert!(loaded.starts_with("строка "), "window starts on a UTF-8 line boundary");

    // The crash reported an absolute caret byte offset below Document::base.
    // Window-only readers must not wrap usize, panic, or alter the full Rope.
    assert_eq!(doc.before(0), "");
    assert_eq!(doc.after(0), "");
    assert_eq!(doc.slice(0..1), "");
    assert_eq!(doc.before(doc.end() + 1), "");
    assert_eq!(doc.after(doc.end() + 1), "");
    assert_eq!(doc.slice(base..doc.end() + 1), "");

    // A byte position inside the first Cyrillic character is not a valid
    // UTF-8 boundary, even if it lies numerically within the loaded window.
    assert_eq!(doc.before(base + 1), "");
    assert_eq!(doc.after(base + 1), "");
    assert_eq!(doc.slice(base..base + 1), "");

    assert_eq!(doc.after(base), loaded);
    assert_eq!(doc.slice(base..doc.end()), loaded);
    assert_eq!(doc.whole(), source, "an invalid window read must never mutate the source");
}
