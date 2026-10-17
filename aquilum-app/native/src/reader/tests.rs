use std::path::PathBuf;

use serde_json::Value;

use super::{Book, Point};

pub(crate) fn sample_fb2() -> String {
    fb2("Съешь же ещё этих мягких французских булок да выпей чаю, повторяя это снова и снова.")
}

pub(crate) fn settings(size: f32, scrolled: bool) -> super::page::Settings {
    super::page::Settings { family: "iA Writer Quattro".into(), size, weight: 400.0, line_height: 1.6, justify: true, hyphenate: true, max_width_ch: 65, margin: 48.0, scrolled }
}

pub(crate) fn book_file(content: String) -> (tempfile::TempDir, PathBuf) {
    let dir = crate::test_dir::tempdir();
    let file = dir.path().join("книга.fb2");
    std::fs::write(&file, content).unwrap();
    (dir, file)
}

pub(crate) fn fb2(sentence: &str) -> String {
    let paragraphs: String = (1..=400).map(|i| format!("<p>Абзац номер {i}. {sentence}</p>")).collect();
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\"><body><section><title><p>Глава первая</p></title>{paragraphs}</section><section><title><p>Глава вторая</p></title>{paragraphs}</section></body></FictionBook>")
}

fn after(book: &Book, index: usize, point: Point) -> String {
    let Point::At(node, offset) = point else { return String::new() };
    let Some(text) = book.document(index).and_then(|d| d.text_of(node)) else { return String::new() };
    let units: Vec<u16> = text.encode_utf16().collect();
    let from = (offset as usize).min(units.len());
    String::from_utf16_lossy(&units[from..(from + 24).min(units.len())])
}

fn prefix(text: &str, units: usize) -> String {
    let all: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16_lossy(&all[..units.min(all.len())])
}

fn text_point(point: Point) -> Option<(u32, u32)> {
    match point {
        Point::At(node, offset) => Some((node, offset)),
        _ => None,
    }
}

#[test]
#[ignore]
fn matches_foliate_golden() {
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var("AQ_READER_GOLDEN").unwrap()).unwrap()).unwrap();
    let vault = PathBuf::from(std::env::var("AQ_VAULT").unwrap());
    let mut failures = Vec::new();
    let mut checked = 0;
    for entry in golden.as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let started = std::time::Instant::now();
        let Some(book) = Book::open(&vault.join(file)) else {
            failures.push(format!("{file}: не открылась"));
            continue;
        };
        let sections = entry["sections"].as_array().unwrap();
        if sections.len() != book.sections.len() {
            failures.push(format!("{file}: глав {} вместо {}", book.sections.len(), sections.len()));
            continue;
        }
        for (index, section) in sections.iter().enumerate() {
            if book.sections[index].base != section["base"].as_str().unwrap() {
                failures.push(format!("{file} #{index}: база {} вместо {}", book.sections[index].base, section["base"]));
            }
            if book.document(index).is_none() {
                failures.push(format!("{file} #{index}: глава не разобрана"));
                continue;
            }
            for point in section["points"].as_array().unwrap() {
                checked += 1;
                let cfi = point["cfi"].as_str().unwrap();
                let Some((i, start, _)) = book.resolve(cfi) else {
                    failures.push(format!("{file}: {cfi} не разрешился"));
                    continue;
                };
                let got = after(&book, i, start);
                if got != point["after"].as_str().unwrap() {
                    failures.push(format!("{file}: {cfi} → {got:?} вместо {:?}", point["after"]));
                    continue;
                }
                let back = text_point(start).and_then(|p| book.cfi(i, p, p));
                if back.as_deref() != Some(cfi) {
                    failures.push(format!("{file}: из места собран {back:?} вместо {cfi}"));
                }
            }
            for range in section["ranges"].as_array().unwrap() {
                checked += 1;
                let cfi = range["cfi"].as_str().unwrap();
                let Some((i, start, end)) = book.resolve(cfi) else {
                    failures.push(format!("{file}: {cfi} не разрешился"));
                    continue;
                };
                let text = book.text(i, start, end).unwrap_or_default();
                let length = text.encode_utf16().count() as u64;
                if length != range["length"].as_u64().unwrap() || prefix(&text, 48) != range["text"].as_str().unwrap() {
                    failures.push(format!("{file}: {cfi} → {:?} ({length}) вместо {:?} ({})", prefix(&text, 48), range["text"], range["length"]));
                    continue;
                }
                let back = text_point(start).zip(text_point(end)).and_then(|(a, b)| book.cfi(i, a, b));
                if back.as_deref() != Some(cfi) {
                    failures.push(format!("{file}: из выделения собран {back:?} вместо {cfi}"));
                }
            }
        }
        for saved in entry["saved"].as_array().unwrap() {
            checked += 1;
            let cfi = saved["cfi"].as_str().unwrap();
            let Some((i, start, end)) = book.resolve(cfi) else {
                failures.push(format!("{file}: сохранённая {cfi} не разрешилась"));
                continue;
            };
            let text = book.text(i, start, end).unwrap_or_default();
            if i as u64 != saved["index"].as_u64().unwrap() || prefix(&text, 60) != saved["text"].as_str().unwrap() || text.encode_utf16().count() as u64 != saved["length"].as_u64().unwrap() {
                failures.push(format!("{file}: сохранённая {cfi} → #{i} {:?} вместо #{} {:?}", prefix(&text, 60), saved["index"], saved["text"]));
            }
        }
        eprintln!("{file}: глав {}, {:?}", book.sections.len(), started.elapsed());
    }
    for failure in failures.iter().take(40) {
        eprintln!("{failure}");
    }
    assert!(failures.is_empty(), "расхождений {} из {checked}", failures.len());
    eprintln!("проверено {checked}, всё совпало");
}

#[test]
#[ignore]
fn quote_links_point_at_their_text() {
    let note = PathBuf::from(std::env::var("AQ_NOTE").unwrap());
    let vault = PathBuf::from(std::env::var("AQ_VAULT").unwrap());
    let doc = std::fs::read_to_string(&note).unwrap();
    let own = crate::frontmatter::parse(&doc).and_then(|f| f.text(crate::frontmatter::BOOK_FILE).map(str::to_owned));
    let normalize = |s: &str| s.replace(['\\', '«', '»', '“', '”'], "").split_whitespace().collect::<Vec<_>>().join(" ");
    let quotes = super::quotes::find(&doc);
    assert!(!quotes.is_empty(), "цитат нет");
    let mut failures = Vec::new();
    for quote in quotes {
        let file = quote.book.clone().or_else(|| own.clone()).expect("файл книги");
        let book = Book::open(&vault.join(&file)).expect("книга открылась");
        let Some((index, start, end)) = book.resolve(&quote.cfi) else {
            failures.push(format!("[{}] {} не разрешился", quote.label, quote.cfi));
            continue;
        };
        let got = normalize(&book.text(index, start, end).unwrap_or_default());
        let want = normalize(&doc[quote.text.clone()]);
        let head: String = want.chars().take(40).collect();
        eprintln!("[{}] {} -> {:?}", quote.label, quote.cfi, got.chars().take(60).collect::<String>());
        if !got.contains(&head) {
            failures.push(format!("[{}] {}: ждали {head:?}, а там {:?}", quote.label, quote.cfi, got.chars().take(80).collect::<String>()));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}


#[test]
#[ignore = "diagnostic benchmark, requires a synthetic EPUB fixture directory"]
fn synthetic_epub_shape_pipeline_profile() {
    use super::{flow, page};
    use masonry::parley::{FontContext, LayoutContext};
    use std::time::Instant;

    let mode = std::env::var("AQ_EPUB_SHAPE_MODE").expect("AQ_EPUB_SHAPE_MODE");
    assert!(["single-br", "single-p", "split-br"].contains(&mode.as_str()));
    let folder = std::env::var("AQ_EPUB_SHAPE_DIR").expect("AQ_EPUB_SHAPE_DIR");
    let file = std::path::Path::new(&folder).join(format!("{mode}.epub"));
    let elapsed = |start: Instant| start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let book = Book::open(&file).expect("open generated EPUB");
    let open_ms = elapsed(start);
    let expected_sections = if mode == "split-br" { 18 } else { 1 };
    assert_eq!(book.sections.len(), expected_sections);
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let settings = settings(18.0, false);
    let mut parse_ms = 0.0;
    let mut flow_ms = 0.0;
    let mut layout_ms = 0.0;
    let mut paginate_ms = 0.0;
    let mut blocks_total = 0;
    let mut first_section_ms = 0.0;
    let mut max_block_bytes = 0;
    let mut headings = 0;
    let mut pages = 0;
    let mut xhtml_bytes = 0;
    let mut collected = String::new();
    for index in 0..book.sections.len() {
        let section_start = Instant::now();
        xhtml_bytes += book.size(index);
        let start = Instant::now();
        let dom = book.document(index).expect("parse generated XHTML");
        parse_ms += elapsed(start);
        let start = Instant::now();
        let blocks = flow::blocks(dom);
        flow_ms += elapsed(start);
        blocks_total += blocks.len();
        for block in &blocks {
            if let flow::Block::Text { text, role, .. } = block {
                max_block_bytes = max_block_bytes.max(text.len());
                headings += usize::from(matches!(role, flow::Role::Heading(_)));
                collected.push_str(text);
                collected.push(' ');
            }
        }
        let start = Instant::now();
        let items = page::layout_blocks(
            &blocks, &mut |_| None, &mut fcx, &mut lcx, &settings, 440.0, 720.0,
        );
        layout_ms += elapsed(start);
        assert_eq!(items.len(), blocks.len());
        let start = Instant::now();
        let laid = page::paginate(items, 720.0);
        paginate_ms += elapsed(start);
        pages += laid.columns.len();
        if index == 0 {
            first_section_ms = elapsed(section_start);
        }
    }
    assert_eq!(headings, 18, "recover all chapter headings");
    assert_eq!(collected.matches('~').count(), 2 * 5324, "retain all line tokens");
    assert!(collected.contains("~00000~") && collected.contains("~05323~"));
    let normalized = collected.split_whitespace().collect::<Vec<_>>().join(" ");
    let digest = normalized
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let peak_rss_kib = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(0);
    println!(
        "READER_EPUB_SHAPE mode={mode} sections={} xhtml_bytes={xhtml_bytes} blocks={blocks_total} headings={headings} max_block_bytes={max_block_bytes} pages={pages} first_section_ms={first_section_ms:.3} open_ms={open_ms:.3} parse_ms={parse_ms:.3} flow_ms={flow_ms:.3} layout_ms={layout_ms:.3} paginate_ms={paginate_ms:.3} rss_kib={peak_rss_kib} digest={digest:016x}",
        book.sections.len(),
    );
}
