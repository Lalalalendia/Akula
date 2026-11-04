//! Cross-platform Reader acceptance with a real EPUB 2 ZIP/container/OPF/spine.
//! All book text is deterministic and synthetic; no user EPUB is committed.

use masonry::kurbo::Size;
use masonry::parley::{FontContext, LayoutContext};

use super::{Session, Spot, Target};
use crate::reader::{Book, flow};
use crate::reader::tests::settings;

const VIEW: Size = Size::new(1200.0, 800.0);

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (if crc & 1 == 1 { 0xedb8_8320 } else { 0 });
        }
    }
    !crc
}

fn short(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_le_bytes());
}

fn long(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}

// ZIP store-only writer for tiny tests. No Python, network or extra crate is
// required; this also runs on a clean Windows Rust test worker.
pub(crate) fn write_zip(path: &std::path::Path, members: &[(String, Vec<u8>)]) {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in members {
        let name = name.as_bytes();
        let offset = u32::try_from(zip.len()).expect("small fixture");
        let size = u32::try_from(bytes.len()).expect("small fixture");
        let name_len = u16::try_from(name.len()).expect("short fixture path");
        let crc = crc32(bytes);

        long(&mut zip, 0x0403_4b50); // local header
        short(&mut zip, 20);         // version to extract
        short(&mut zip, 0);          // flags
        short(&mut zip, 0);          // stored, no compression
        short(&mut zip, 0);          // DOS time
        short(&mut zip, 0);          // DOS date
        long(&mut zip, crc);
        long(&mut zip, size);
        long(&mut zip, size);
        short(&mut zip, name_len);
        short(&mut zip, 0);          // no extra fields
        zip.extend_from_slice(name);
        zip.extend_from_slice(bytes);

        long(&mut central, 0x0201_4b50);
        short(&mut central, 20);     // version made
        short(&mut central, 20);     // version extract
        short(&mut central, 0);      // flags
        short(&mut central, 0);      // stored
        short(&mut central, 0);
        short(&mut central, 0);
        long(&mut central, crc);
        long(&mut central, size);
        long(&mut central, size);
        short(&mut central, name_len);
        short(&mut central, 0);      // extra
        short(&mut central, 0);      // comment
        short(&mut central, 0);      // disk start
        short(&mut central, 0);      // internal flags
        long(&mut central, 0);       // external attributes
        long(&mut central, offset);
        central.extend_from_slice(name);
    }
    let offset = u32::try_from(zip.len()).expect("small central directory");
    let length = u32::try_from(central.len()).expect("small central directory");
    zip.extend_from_slice(&central);
    long(&mut zip, 0x0605_4b50);
    short(&mut zip, 0); // disk
    short(&mut zip, 0); // central disk
    let count = u16::try_from(members.len()).expect("small member count");
    short(&mut zip, count);
    short(&mut zip, count);
    long(&mut zip, length);
    long(&mut zip, offset);
    short(&mut zip, 0); // comment length
    std::fs::write(path, zip).expect("write synthetic EPUB");
}

pub(crate) fn synthetic_epub() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = crate::test_dir::tempdir();
    let path = dir.path().join("reader-navigation.epub");
    let mut manifest = String::from("<item id=\"toc\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>");
    let mut spine = String::new();
    let mut members = Vec::new();
    for chapter in 0..3 {
        manifest.push_str(&format!(
            "<item id=\"ch{chapter}\" href=\"chapters/{chapter}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine.push_str(&format!("<itemref idref=\"ch{chapter}\"/>"));

        // Each section contains many pages. Section 1 uses direct-body
        // <title> (recovered in #191) and <br/>-grouped paragraphs. Other
        // sections use standard h1/p and inline Unicode-rich text.
        let mut body = if chapter == 1 {
            format!("<title><b>Глава {chapter}</b></title><div>")
        } else {
            format!("<h1>Глава {chapter}</h1>")
        };
        for line in 0..84 {
            let words = format!(
                "Метка-{chapter}-{line:03} Текстовая строка с Unicode: кириллица, emoji 🙂, \
                 диакритика café, разные слова для переноса и адресации EPUB CFI."
            );
            if chapter == 1 {
                body.push_str(&format!("{words}<br/>"));
            } else {
                body.push_str(&format!("<p id=\"p{line}\">{words} <em>курсив</em></p>"));
            }
        }
        if chapter == 1 {
            body.push_str("</div>");
        }
        let xhtml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <html xmlns=\"http://www.w3.org/1999/xhtml\">\
             <head><title>Hidden package title {chapter}</title></head>\
             <body>{body}</body></html>"
        );
        members.push((format!("OPS/chapters/{chapter}.xhtml"), xhtml.into_bytes()));
    }
    let package = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"uid\">\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
         <dc:title>Copyright-free Reader CFI fixture</dc:title>\
         <dc:identifier id=\"uid\">urn:uuid:reader-navigation-test</dc:identifier>\
         <dc:language>ru</dc:language></metadata>\
         <manifest>{manifest}</manifest><spine toc=\"toc\">{spine}</spine></package>"
    );
    let points: String = (0..3).map(|chapter| format!(
        "<navPoint id=\"nav{chapter}\" playOrder=\"{}\"><navLabel><text>Глава {chapter}</text></navLabel>\
         <content src=\"chapters/{chapter}.xhtml\"/></navPoint>", chapter + 1
    )).collect();
    let ncx = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\
         <head/><docTitle><text>Reader fixture</text></docTitle><navMap>{points}</navMap></ncx>"
    );
    members.extend([
        ("META-INF/container.xml".into(),
            b"<?xml version=\"1.0\"?><container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\
              <rootfiles><rootfile full-path=\"OPS/book.opf\" media-type=\"application/oebps-package+xml\"/>\
              </rootfiles></container>".to_vec()),
        ("OPS/book.opf".into(), package.into_bytes()),
        ("OPS/toc.ncx".into(), ncx.into_bytes()),
    ]);
    // EPUB 2 mandates the uncompressed mimetype entry as the first ZIP member.
    members.insert(0, ("mimetype".into(), b"application/epub+zip".to_vec()));
    write_zip(&path, &members);
    (dir, path)
}

fn ensure_ready(
    session: &mut Session,
    size: f32,
    scrolled: bool,
    fcx: &mut FontContext,
    lcx: &mut LayoutContext<masonry::core::BrushIndex>,
) {
    session.ensure(VIEW, &settings(size, scrolled), fcx, lcx);
    assert!(!session.needs_layout(), "unlaid visible items at section {}", session.section());
    let shown = session.on_screen();
    assert!(!shown.is_empty(), "no visible EPUB text in section {}", session.section());
    assert!(
        session.layouts.iter().all(|(item, _)| shown.contains(item)),
        "Parley layout kept outside visible page"
    );
}

#[test]
fn epub_spine_navigation_preserves_headings_forward_and_backward() {
    let (_dir, file) = synthetic_epub();
    let book = Book::open(&file).expect("valid EPUB ZIP, container, OPF and spine");
    assert_eq!(book.sections.len(), 3);
    for section in 0..3 {
        let blocks = flow::blocks(book.document(section).expect("valid XHTML"));
        let headings: Vec<_> = blocks.iter().filter_map(|block| {
            if let flow::Block::Text { text, role: flow::Role::Heading(_), .. } = block {
                Some(text.as_str())
            } else { None }
        }).collect();
        assert_eq!(headings, vec![format!("Глава {section}")], "section {section}");
        assert!(
            blocks.iter().all(|block| !matches!(block, flow::Block::Text { text, .. }
                if text.contains("Hidden package title"))),
            "HTML head metadata must never appear in reading order"
        );
    }

    for scrolled in [false, true] {
        let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
        let mut session = Session::open(&file, Target::Start).expect("Reader opens EPUB");
        ensure_ready(&mut session, 18.0, scrolled, &mut fcx, &mut lcx);
        let mut steps = 0;
        let mut last_progress = 0.0;
        while session.section() < 2 {
            let (_, progress) = session.location().expect("persisted CFI and percentage");
            assert!(
                progress >= last_progress - 1e-7,
                "reading progress moved backwards at step {steps}, scrolled={scrolled}"
            );
            last_progress = progress;
            assert!(session.next(), "next cannot advance at step {steps}");
            ensure_ready(&mut session, 18.0, scrolled, &mut fcx, &mut lcx);
            steps += 1;
            assert!(steps < 400, "stuck crossing EPUB spine");
        }
        assert!(steps > 5, "must actually paginate before crossing chapters");
        assert_eq!(session.section(), 2);
        for target in (0..2).rev() {
            let mut attempts = 0;
            while session.section() > target {
                assert!(session.prev(), "cannot return to previous EPUB section");
                ensure_ready(&mut session, 18.0, scrolled, &mut fcx, &mut lcx);
                attempts += 1;
                assert!(attempts < 400, "stuck returning across EPUB spine");
            }
            assert_eq!(session.section(), target);
        }
        // Returning to the beginning must also restore the first section.
        for _ in 0..400 {
            if !session.prev() {
                break;
            }
            ensure_ready(&mut session, 18.0, scrolled, &mut fcx, &mut lcx);
        }
        assert_eq!(session.section(), 0);
        assert!(!session.prev());
        let (_, progress) = session.location().expect("first page has CFI");
        assert!(progress < 0.05, "return-to-start progress {progress}");
    }
}

#[test]
fn epub_saved_cfi_survives_reopen_and_font_or_scroll_changes() {
    let (_dir, file) = synthetic_epub();
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let mut session = Session::open(&file, Target::Start).unwrap();
    ensure_ready(&mut session, 18.0, false, &mut fcx, &mut lcx);

    let mut attempts = 0;
    while session.section() == 0 {
        assert!(session.next());
        ensure_ready(&mut session, 18.0, false, &mut fcx, &mut lcx);
        attempts += 1;
        assert!(attempts < 200);
    }
    assert_eq!(session.section(), 1);
    for _ in 0..4 {
        assert!(session.next(), "middle chapter should occupy multiple pages");
        ensure_ready(&mut session, 18.0, false, &mut fcx, &mut lcx);
        assert_eq!(session.section(), 1, "did not test a middle-of-chapter bookmark");
    }
    let (saved_block, saved_byte) = session.start().expect("visible first line");
    assert!(saved_byte > 0, "bookmark must capture an interior point in a giant br block");
    let bookmarked = Spot { block: saved_block, byte: saved_byte };
    let (bookmark, fraction) = session.location().expect("Reader saved position");
    assert!(fraction > 0.0 && fraction < 1.0);
    assert!(bookmark.starts_with("epubcfi("), "{bookmark}");
    assert_eq!(
        session.book.resolve(&bookmark).expect("CFI resolves to original EPUB").0,
        1
    );

    for (size, scrolled) in [(18.0, false), (23.0, false), (14.0, false), (18.0, true)] {
        let mut restored = Session::open(&file, Target::Cfi(bookmark.clone()))
            .expect("reopen persisted EPUB position");
        ensure_ready(&mut restored, size, scrolled, &mut fcx, &mut lcx);
        assert_eq!(restored.section(), 1, "bookmark jumped between chapters");
        let first = restored.start().expect("visible start");
        let last = restored.end().expect("visible end");
        assert!(
            Spot { block: first.0, byte: first.1 } <= bookmarked
                && bookmarked <= Spot { block: last.0, byte: last.1 },
            "bookmarked UTF-8 byte offset {bookmarked:?} not visible at {size}pt,              scrolled={scrolled}: {first:?}..{last:?}"
        );
        let (current, ratio) = restored.location().expect("CFI after reflow");
        assert_eq!(restored.book.resolve(&current).expect("reflow CFI resolves").0, 1);
        assert!(ratio > 0.0 && ratio < 1.0, "bad progress {ratio}");
        // Repeating ensure with unchanged settings must keep exact position.
        let before = (restored.start(), restored.end());
        ensure_ready(&mut restored, size, scrolled, &mut fcx, &mut lcx);
        assert_eq!((restored.start(), restored.end()), before);
    }

    // A missing/invalid external CFI must fail closed to the start of book.
    let mut invalid = Session::open(&file, Target::Cfi("epubcfi(/999999)".into())).unwrap();
    ensure_ready(&mut invalid, 18.0, false, &mut fcx, &mut lcx);
    assert_eq!(invalid.section(), 0);
    assert_eq!(invalid.page, 0);
}
