use super::{Kind, extra};

fn kinds(text: &str) -> Vec<(Kind, &str)> {
    extra(text, &[]).into_iter().map(|s| (s.kind, &text[s.inner])).collect()
}

#[test]
fn highlight_needs_flanking_double_marks() {
    assert_eq!(kinds("==Выделенный текст== тут"), [(Kind::Highlight, "Выделенный текст")]);
    assert_eq!(kinds("a == b == c"), []);
    assert_eq!(kinds("==a == b=="), [(Kind::Highlight, "a == b")]);
    assert_eq!(kinds("===x==="), []);
    assert_eq!(kinds("==один== и ==два=="), [(Kind::Highlight, "один"), (Kind::Highlight, "два")]);
    assert_eq!(kinds("==не закрыто"), []);
}

#[test]
fn a_comment_keeps_its_markers_in_the_inner_range() {
    assert_eq!(kinds("до %%скрыто%% после"), [(Kind::Comment, "%%скрыто%%")]);
    assert_eq!(kinds("100%% не пара"), []);
}

#[test]
fn footnote_references_and_block_ids_are_found() {
    assert_eq!(kinds("Текст[^1] и ещё[^note-2]."), [(Kind::Footnote, "[^1]"), (Kind::Footnote, "[^note-2]")]);
    assert_eq!(kinds("[^ нет] и [^] тоже нет"), []);
    assert_eq!(kinds("Абзац ^block-id"), [(Kind::BlockId, "^block-id")]);
    assert_eq!(kinds("^abc"), [(Kind::BlockId, "^abc")]);
    assert_eq!(kinds("E = mc^2 и дальше"), []);
    assert_eq!(kinds("x^2^"), [(Kind::Sup, "2")]);
}

#[test]
fn scripts_work_inside_a_word_and_never_swallow_spaces() {
    assert_eq!(kinds("H~2~O и E = mc^2^."), [(Kind::Sub, "2"), (Kind::Sup, "2")]);
    assert_eq!(kinds("a ~ b ~ c, cd ~/foo и ~/bar, 2^10 и x^"), []);
    assert_eq!(kinds("~~зачёркнуто~~"), []);
    assert_eq!(kinds("[^1][^2] и ^id"), [(Kind::Footnote, "[^1]"), (Kind::Footnote, "[^2]"), (Kind::BlockId, "^id")]);
    assert_eq!(kinds("x^(n+1)^"), [(Kind::Sup, "(n+1)")]);
}

#[test]
fn html_tags_and_backslash_escapes_become_spans() {
    let text = r"<kbd>Ctrl</kbd> + <mark>да</mark> и <u>под</u>, \*не курсив\*, \\ и C:\Users";
    let found: Vec<(Kind, &str)> = extra(text, &[]).into_iter().map(|s| (s.kind, &text[s.inner])).collect();
    assert!(found.contains(&(Kind::Code, "Ctrl")), "{found:?}");
    assert!(found.contains(&(Kind::Highlight, "да")), "{found:?}");
    assert!(found.contains(&(Kind::Underline, "под")), "{found:?}");
    assert_eq!(found.iter().filter(|(kind, _)| *kind == Kind::Escape).map(|(_, s)| *s).collect::<Vec<_>>(), ["*", "*", r"\"]);
    assert!(extra("<kbd></kbd> <b>без закрытия", &[]).is_empty());
}

#[test]
fn marks_inside_taken_ranges_are_dropped() {
    let text = "`==код==` и ==да==";
    let found = extra(text, &[0..text.find(" и").unwrap()]);
    assert_eq!(found.len(), 1);
    assert_eq!(&text[found[0].inner.clone()], "да");
}


#[test]
fn html_fast_path_preserves_offsets_and_does_not_match_plain_text() {
    use super::atoms::{html_comments, html_pairs};

    assert!(html_pairs("Обычный текст без HTML и с emoji 😀").is_empty());
    assert!(html_comments("Обычный текст без HTML и с emoji 😀").is_empty());

    let text = "😀<mark>кириллица</mark> и <kbd>Ctrl</kbd>";
    let spans = html_pairs(text);
    assert_eq!(spans.len(), 2);
    assert_eq!(&text[spans[0].inner.clone()], "кириллица");
    assert_eq!(&text[spans[1].inner.clone()], "Ctrl");
    assert_eq!(spans[0].whole.start, "😀".len());

    let comments = html_comments("до <!-- скрыто --> после");
    assert_eq!(comments.len(), 1);
    assert_eq!(&"до <!-- скрыто --> после"[comments[0].whole.clone()], "<!-- скрыто -->");
}

#[test]
fn paired_fast_path_preserves_unclosed_and_adjacent_delimiters() {
    use super::delimited::paired;

    assert_eq!(paired("без выделения", '=', true), Vec::new());
    assert_eq!(paired("==текст== %%комментарий%%", '=', true), vec![(0, "==текст".len())]);
    assert_eq!(paired("==не закрыто", '=', true), Vec::new());
    assert_eq!(paired("===не пара===", '=', true), Vec::new());
    assert_eq!(paired("%%заметка%%", '%', false), vec![(0, "%%заметка".len())]);
    assert_eq!(paired("жжтекстжж", 'ж', false), vec![(0, "жжтекст".len())]);
}

/*
 * Baseline is the pre-optimization html_pairs algorithm. Keeping it here
 * allows a same-process reference comparison without relying on CI hosts
 * with different CPUs or Rust toolchains.
 */
fn reference_html_pairs(text: &str) -> Vec<super::Span> {
    use super::{Kind, Span};

    const TAGS: [(&str, Kind); 12] = [
        ("sub", Kind::Sub),
        ("sup", Kind::Sup),
        ("mark", Kind::Highlight),
        ("kbd", Kind::Code),
        ("u", Kind::Underline),
        ("ins", Kind::Underline),
        ("del", Kind::Strike),
        ("s", Kind::Strike),
        ("b", Kind::Strong),
        ("strong", Kind::Strong),
        ("i", Kind::Emphasis),
        ("em", Kind::Emphasis),
    ];

    let mut found = Vec::new();
    for (name, kind) in TAGS {
        let (open, close) = (format!("<{name}>"), format!("</{name}>"));
        let mut from = 0;
        while let Some(start) = text[from..].find(&open).map(|i| from + i) {
            let inner = start + open.len();
            match text[inner..].find(&close).map(|i| inner + i) {
                Some(end) if end > inner => {
                    found.push(Span { kind, whole: start..end + close.len(), inner: inner..end, target: None });
                    from = end + close.len();
                }
                _ => from = inner,
            }
        }
    }
    found
}

#[test]
fn html_fast_path_is_equivalent_to_previous_algorithm() {
    use super::atoms::html_pairs;

    let cases = [
        "",
        "Text without markup",
        "Обычный абзац и emoji 😀 без тегов",
        "<sub>1</sub><sup>2</sup>",
        "<strong>bold</strong> and <b>bold</b> and <em>italic</em>",
        "<mark>да</mark><kbd>Ctrl</kbd><u>подчёркнутый</u>",
        "<mark></mark> <mark>незакрытый",
        "<<b>inside</b>",
        "<s>delete</s><del>struck</del>",
        "<!-- unrelated comment -->",
        "<i>nest <b>bold</b> here</i>",
        "😀<mark>русский 😀</mark> and <mark>second</mark>",
        "<strong>same</strong><strong>twice</strong>",
        "<MARK>case-sensitive</MARK>",
    ];
    for case in cases {
        assert_eq!(html_pairs(case), reference_html_pairs(case), "{case:?}");
    }
}

#[test]
#[ignore = "Release-only, diagnostic microbenchmark; run with --ignored --nocapture"]
fn benchmark_inline_html_pairs() {
    use std::hint::black_box;
    use std::time::Instant;
    use super::atoms::html_pairs;

    fn run(corpus: &[String], parser: fn(&str) -> Vec<super::Span>) -> f64 {
        let start = Instant::now();
        for _ in 0..200 {
            for input in corpus {
                black_box(parser(black_box(input.as_str())));
            }
        }
        start.elapsed().as_secs_f64() * 1_000.0
    }

    fn compare(label: &str, corpus: &[String]) {
        let mut before = Vec::new();
        let mut after = Vec::new();
        for pass in 0..6 {
            if pass % 2 == 0 {
                before.push(run(corpus, reference_html_pairs));
                after.push(run(corpus, html_pairs));
            } else {
                after.push(run(corpus, html_pairs));
                before.push(run(corpus, reference_html_pairs));
            }
        }
        before.sort_by(f64::total_cmp);
        after.sort_by(f64::total_cmp);
        let base_ms = (before[2] + before[3]) / 2.0;
        let optimized_ms = (after[2] + after[3]) / 2.0;
        eprintln!(
            "BENCH-INLINE case={label} baseline_ms={base_ms:.3} optimized_ms={optimized_ms:.3} speedup={:.3}x",
            base_ms / optimized_ms
        );
    }

    let plain: Vec<_> = (0..64)
        .map(|i| format!("Заметка {i}: Unicode 😀 и Markdown без HTML — обычный текст для редактора."))
        .collect();
    let rich: Vec<_> = (0..64)
        .map(|i| format!("Часть {i} <mark>важное</mark>, <kbd>Ctrl</kbd>, <sub>2</sub> и <em>курсив</em>."))
        .collect();
    compare("plain", &plain);
    compare("html", &rich);
}
