// Standalone, dependency-free, paired benchmark of Akula's actual Markdown
// inline-mark source files. Compile with BENCH_BASE_DIR and BENCH_CAND_DIR set
// to folders containing atoms.rs and delimited.rs.
//
// This is a parsing-kernel benchmark, NOT a whole-editor latency benchmark.
use std::hint::black_box;
use std::time::Instant;

macro_rules! parser {
    ($module:ident, $root:literal) => {
        #[allow(dead_code)]
        mod $module {
            use std::ops::Range;

            #[derive(Clone, Copy, Debug, Eq, PartialEq)]
            #[repr(u8)]
            pub enum Kind {
                Sub, Sup, Highlight, Code, Underline, Strike, Strong,
                Emphasis, Comment, Escape, Footnote, BlockId,
            }

            #[derive(Debug)]
            pub struct Span {
                pub kind: Kind,
                pub whole: Range<usize>,
                pub inner: Range<usize>,
                pub target: Option<String>,
            }

            mod marks {
                use super::{Kind, Span};
                use std::ops::Range;

                fn span(kind: Kind, whole: Range<usize>, inner: Range<usize>) -> Span {
                    Span { kind, whole, inner, target: None }
                }

                mod atoms {
                    include!(concat!(env!($root), "/atoms.rs"));
                }
                mod delimited {
                    include!(concat!(env!($root), "/delimited.rs"));
                }

                pub(super) fn signature(text: &str) -> Vec<(u8, usize, usize, usize, usize)> {
                    let mut result = Vec::new();
                    for span in atoms::html_pairs(text).into_iter().chain(atoms::html_comments(text)) {
                        result.push((
                            span.kind as u8, span.whole.start, span.whole.end,
                            span.inner.start, span.inner.end
                        ));
                    }
                    for symbol in ['=', '%', 'ж'] {
                        for (open, close) in delimited::paired(text, symbol, symbol == '=') {
                            result.push((if symbol == '=' { 60 } else if symbol == '%' { 61 } else { 62 },
                                open, close, open + 2 * symbol.len_utf8(), close));
                        }
                    }
                    result
                }

                pub(super) fn evaluate(text: &str) -> u64 {
                    let mut sum = 0_u64;
                    // Benchmark precisely the hot functions changed in #85.
                    for span in atoms::html_pairs(text).into_iter().chain(atoms::html_comments(text)) {
                        sum = sum.rotate_left(7)
                            .wrapping_add((span.kind as u64) << 48)
                            .wrapping_add((span.whole.start as u64) << 24)
                            .wrapping_add(span.whole.end as u64);
                    }
                    for symbol in ['=', '%'] {
                        for (open, close) in delimited::paired(text, symbol, symbol == '=') {
                            sum = sum.rotate_left(7)
                                .wrapping_add((symbol as u64) << 48)
                                .wrapping_add((open as u64) << 24)
                                .wrapping_add(close as u64);
                        }
                    }
                    sum
                }
            }

            pub fn signature(text: &str) -> Vec<(u8, usize, usize, usize, usize)> {
                marks::signature(text)
            }
            pub fn evaluate(text: &str) -> u64 {
                marks::evaluate(text)
            }
        }
    };
}

parser!(baseline, "BENCH_BASE_DIR");
parser!(candidate, "BENCH_CAND_DIR");

fn sample(parse: fn(&str) -> u64, text: &str, iterations: usize) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for i in 0..iterations {
        checksum ^= black_box(parse(black_box(text))).rotate_left((i % 63) as u32);
    }
    black_box(checksum);
    start.elapsed().as_nanos() as f64 / iterations as f64
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    (samples[samples.len() / 2 - 1] + samples[samples.len() / 2]) / 2.0
}

fn main() {
    let cases = [
        ("plain", "Обычная заметка без HTML и выделения. This is plain text. 😀\n".repeat(12), 20_000_usize),
        ("html", "<mark>важно</mark> и <kbd>Ctrl</kbd> с <strong>жирным</strong> текстом.\n".repeat(12), 12_000),
        ("delimiters", "==подсветка== и %%комментарий%% с русским текстом и emoji 🐱.\n".repeat(12), 12_000),
        ("long_plain", "Длинный обычный абзац. ".repeat(250), 6_000),
        ("mixed", "Текст <sub>индекс</sub> ==важное== <!-- заметка --> %%скрыто%%.\n".repeat(12), 12_000),
    ];
    // Verify complete span positions (including UTF-8 boundaries) before timing.
    let edge_cases = [
        "😀<mark>текст</mark> <kbd>Ctrl</kbd>",
        "<kbd></kbd> <b>нет закрытия",
        "а %%комментарий%% и ==подсветка==",
        "===не пара===, ==текст==, %%не закрыто",
        "жжтекстжж, <sup>x</sup>, <!-- скрыто -->",
        "обычная заметка",
        "😀<!-- comment --> and <ins>text</ins>",
    ];
    for text in edge_cases.into_iter().chain(cases.iter().map(|(_, text, _)| text.as_str())) {
        assert_eq!(
            baseline::signature(text), candidate::signature(text),
            "span parity failed for {text:?}"
        );
    }
    println!("case,baseline_ns_per_call,candidate_ns_per_call,baseline_over_candidate");
    // Warm up both implementations before ABBA sampling.
    for (_, input, _) in &cases {
        for _ in 0..200 {
            black_box(baseline::evaluate(black_box(input)));
            black_box(candidate::evaluate(black_box(input)));
        }
    }
    for (name, input, iterations) in &cases {
        let mut a = Vec::new();
        let mut b = Vec::new();
        // Six ABBA rounds on the same runner reduce temperature/order bias.
        for _ in 0..6 {
            a.push(sample(baseline::evaluate, input, *iterations));
            b.push(sample(candidate::evaluate, input, *iterations));
            b.push(sample(candidate::evaluate, input, *iterations));
            a.push(sample(baseline::evaluate, input, *iterations));
        }
        let original = median(&mut a);
        let changed = median(&mut b);
        println!("{name},{original:.1},{changed:.1},{:.3}", original / changed);
    }
}
