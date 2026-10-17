use masonry::parley::{FontContext, LayoutContext};

use super::{Item, dash_width, layout_blocks, text_layout};
use crate::reader::tests::{book_file, fb2, settings};
use crate::reader::{Book, flow};

#[test]
fn a_chapter_laid_out_in_parallel_matches_one_laid_out_block_by_block() {
    let (_dir, file) = book_file(fb2("Государственное благоустройство достопримечательностей неудовлетворительно характеризуется высокопоставленными представителями администрации."));
    let book = Book::open(&file).unwrap();
    let blocks = flow::blocks(book.document(0).unwrap());
    let settings = settings(18.0, false);
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let items = layout_blocks(&blocks, &mut |_| None, &mut fcx, &mut lcx, &settings, 300.0, 800.0);
    let dash = dash_width(&mut fcx, &mut lcx, &settings);
    assert_eq!(items.len(), blocks.len());
    let mut hyphenated = 0;
    for (block, item) in blocks.iter().zip(&items) {
        let Item::Text { lines, breaks, x, .. } = item else { continue };
        let (layout, expected, indent) = text_layout(block, &mut fcx, &mut lcx, &settings, 300.0, dash).unwrap();
        let want: Vec<_> = layout.lines().map(|l| (l.text_range(), f64::from(l.metrics().block_min_coord), f64::from(l.metrics().block_max_coord))).collect();
        let got: Vec<_> = lines.iter().map(|l| (l.start..l.end, l.top, l.bottom)).collect();
        assert_eq!((got, breaks, x), (want, &expected, &indent));
        hyphenated += usize::from(breaks.iter().any(|b| b.1 == '-'));
    }
    assert!(hyphenated > 10, "в примере почти нет переносов в конце строки: {hyphenated}");
}


fn assert_same_items(old: &[Item], new: &[Item]) {
    assert_eq!(old.len(), new.len());
    for (index, (left, right)) in old.iter().zip(new).enumerate() {
        match (left, right) {
            (Item::Text { lines: a, x: ax, before: ab, after: aa, breaks: ak },
             Item::Text { lines: b, x: bx, before: bb, after: ba, breaks: bk }) => {
                assert_eq!((ax.to_bits(), ab.to_bits(), aa.to_bits(), ak), (bx.to_bits(), bb.to_bits(), ba.to_bits(), bk), "text style {index}");
                assert_eq!(a.len(), b.len(), "line count {index}");
                for (line, (a, b)) in a.iter().zip(b).enumerate() {
                    assert_eq!(
                        (a.start, a.end, a.top.to_bits(), a.bottom.to_bits()),
                        (b.start, b.end, b.top.to_bits(), b.bottom.to_bits()),
                        "block {index} line {line}",
                    );
                }
            }
            (Item::Image { size: a, before: ab, after: aa },
             Item::Image { size: b, before: bb, after: ba }) => {
                assert_eq!(
                    (a.width.to_bits(), a.height.to_bits(), ab.to_bits(), aa.to_bits()),
                    (b.width.to_bits(), b.height.to_bits(), bb.to_bits(), ba.to_bits()),
                    "image {index}",
                );
            }
            (Item::Rule, Item::Rule) => {}
            _ => panic!("item type drift at {index}"),
        }
    }
}

#[test]
fn weighted_layout_keeps_exact_lines_styles_and_item_order() {
    use super::layout::layout_blocks_fixed;
    let multipliers = [1100, 700, 45, 800, 20, 600, 30, 500, 350, 10, 400, 15];
    let mut blocks: Vec<flow::Block> = multipliers
        .into_iter()
        .map(|count| flow::Block::Text {
            text: "Контрольная строка с кириллицей и переносами. ".repeat(count),
            spans: Vec::new(),
            origins: Vec::new(),
            role: flow::Role::Paragraph,
            depth: 0,
            indent: false,
            align: None,
        })
        .collect();
    blocks.push(flow::Block::Rule);
    blocks.push(flow::Block::Image { src: "sample.png".into(), node: 0 });
    let settings = settings(18.0, false);
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let old = layout_blocks_fixed(&blocks, &mut |_| Some((200, 180)), &mut fcx, &mut lcx, &settings, 440.0, 720.0);
    let new = layout_blocks(&blocks, &mut |_| Some((200, 180)), &mut fcx, &mut lcx, &settings, 440.0, 720.0);
    assert_same_items(&old, &new);
    assert_eq!(super::paginate(old, 720.0).columns.len(), super::paginate(new, 720.0).columns.len());
}

#[test]
#[ignore = "synthetic generated EPUB data, not a CI pass/fail performance threshold"]
fn weighted_layout_abba_profile() {
    use std::hint::black_box;
    use std::path::Path;
    use std::time::Instant;
    use masonry::core::BrushIndex;
    use super::layout::layout_blocks_fixed;

    fn timed(
        blocks: &[Vec<flow::Block>],
        settings: &super::Settings,
        fcx: &mut FontContext,
        lcx: &mut LayoutContext<BrushIndex>,
        optimized: bool,
    ) -> f64 {
        let started = Instant::now();
        let mut items = 0;
        for section in blocks {
            let output = if optimized {
                layout_blocks(section, &mut |_| None, fcx, lcx, settings, 440.0, 720.0)
            } else {
                layout_blocks_fixed(section, &mut |_| None, fcx, lcx, settings, 440.0, 720.0)
            };
            items += output.len();
            black_box(output);
        }
        black_box(items);
        started.elapsed().as_secs_f64() * 1000.0
    }

    let directory = std::env::var("AQ_EPUB_SHAPE_DIR").expect("set AQ_EPUB_SHAPE_DIR");
    let settings = settings(18.0, false);
    for mode in ["single-br", "single-p", "split-br"] {
        let path = Path::new(&directory).join(format!("{mode}.epub"));
        let book = Book::open(&path).expect("open copyright-free synthetic EPUB");
        let all_blocks: Vec<Vec<flow::Block>> = (0..book.sections.len())
            .map(|index| flow::blocks(book.document(index).expect("parse synthetic section")))
            .collect();
        let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
        for section in &all_blocks {
            let old = layout_blocks_fixed(section, &mut |_| None, &mut fcx, &mut lcx, &settings, 440.0, 720.0);
            let new = layout_blocks(section, &mut |_| None, &mut fcx, &mut lcx, &settings, 440.0, 720.0);
            assert_same_items(&old, &new);
            assert_eq!(super::paginate(old, 720.0).columns.len(), super::paginate(new, 720.0).columns.len());
        }

        // Warm both versions, then ABBA in the same process. Keep raw samples.
        for _ in 0..2 {
            black_box(timed(&all_blocks, &settings, &mut fcx, &mut lcx, false));
            black_box(timed(&all_blocks, &settings, &mut fcx, &mut lcx, true));
        }
        let (mut old, mut new) = (Vec::new(), Vec::new());
        for _ in 0..7 {
            old.push(timed(&all_blocks, &settings, &mut fcx, &mut lcx, false));
            new.push(timed(&all_blocks, &settings, &mut fcx, &mut lcx, true));
            new.push(timed(&all_blocks, &settings, &mut fcx, &mut lcx, true));
            old.push(timed(&all_blocks, &settings, &mut fcx, &mut lcx, false));
        }
        println!("WEIGHTED_LAYOUT_RAW mode={mode} fixed_ms={old:?} weighted_ms={new:?}");
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        let old_med = (old[6] + old[7]) / 2.0;
        let new_med = (new[6] + new[7]) / 2.0;
        println!(
            "WEIGHTED_LAYOUT_RESULT mode={mode} sections={} fixed_ms={old_med:.3} weighted_ms={new_med:.3} speedup={:.3}",
            all_blocks.len(), old_med / new_med,
        );
    }
}
