use super::*;

fn legacy_text(rest: &str) -> Result<(String, usize), String> {
    let mut value = String::new();
    let mut length = 1;
    let mut characters = rest[1..].chars();
    while let Some(symbol) = characters.next() {
        length += symbol.len_utf8();
        match symbol {
            '"' => return Ok((value, length)),
            '\\' => match characters.next() {
                Some(escaped) => {
                    length += escaped.len_utf8();
                    value.push(escaped);
                }
                None => break,
            },
            _ => value.push(symbol),
        }
    }
    Err("Строка не закрыта кавычкой".to_owned())
}

fn legacy_symbol(rest: &str) -> Option<(Symbol, usize)> {
    for (text, symbol) in [
        ("=>", Symbol::Arrow), ("!=", Symbol::NotEqual), (">=", Symbol::GreaterOrEqual),
        ("<=", Symbol::LessOrEqual), ("&&", Symbol::AmpAmp), ("||", Symbol::PipePipe),
    ] {
        if rest.starts_with(text) {
            return Some((symbol, 2));
        }
    }
    let single = match rest.as_bytes().first()? {
        b'(' => Symbol::OpenParen, b')' => Symbol::CloseParen,
        b'[' => Symbol::OpenBracket, b']' => Symbol::CloseBracket,
        b',' => Symbol::Comma, b'.' => Symbol::Dot,
        b'+' => Symbol::Plus, b'-' => Symbol::Minus,
        b'*' => Symbol::Star, b'/' => Symbol::Slash,
        b'%' => Symbol::Percent, b'=' => Symbol::Equal,
        b'<' => Symbol::Less, b'>' => Symbol::Greater,
        b'!' => Symbol::Bang, b'&' => Symbol::AmpAmp,
        b'|' => Symbol::PipePipe,
        _ => return None,
    };
    Some((single, 1))
}

#[test]
fn full_unicode_quote_and_error_parity() {
    for source in [
        "\"\"", "\"one two\"", "\"Привет, мир!\"", "\"😀漢字 中文\"",
        "\"hello\\\"world\"", "\"a\\\\b\"", "\"a\\бета\"", "\"trailing slash\\",
        "\"missing quote", "\"😀\\\"", "\"\\", "\"a\nb\"", "\"a\r\nb\"",
        "\"literal [[wiki]] and // comments\"",
    ] {
        assert_eq!(read_text(source), legacy_text(source), "{source:?}");
    }
    let fragments = ["a", "\"", "\\", "é", "漢字", "😀", "\n", "🔥", "📖", "123", "||", "  "];
    for first in fragments {
        for second in fragments {
            for suffix in ["\"", "", "\\\"", "\\"] {
                let source = format!("\"{first}{second}{suffix}");
                assert_eq!(read_text(&source), legacy_text(&source), "{source:?}");
            }
        }
    }
}

#[test]
fn all_ascii_operator_pairs_and_unicode_match_old_symbol_behavior() {
    for first in 0u8..=127 {
        for second in 0u8..=127 {
            let source = String::from_utf8(vec![first, second]).unwrap();
            assert_eq!(read_symbol(&source), legacy_symbol(&source), "{first} {second}");
        }
    }
    for symbol in ["😀", "é", "я", "漢", "a", "#", "!", "|", "&", "", "=>x", "...", "-1"] {
        assert_eq!(read_symbol(symbol), legacy_symbol(symbol), "{symbol:?}");
    }
}

#[test]
fn complete_token_offsets_keep_utf8_boundaries() {
    let cases = [
        "TABLE file.name AS \"Имя\", length AS \"Длина\" FROM #книги WHERE rating >= 4.5",
        "LIST FROM \"Мои книги\" WHERE name != \"Рыцарь \\\"Маске\\\"\" && status = \"готово\"",
        "WHERE (x <= 2 || y >= 3) && value != \"🔥 привет\"",
    ];
    for source in cases {
        let tokens = super::super::tokenize(source).expect("valid Dataview query");
        for token in tokens {
            assert!(token.from <= token.to && token.to <= source.len());
            assert!(source.is_char_boundary(token.from) && source.is_char_boundary(token.to));
        }
    }
}

#[test]
#[ignore = "diagnostic Release ABBA, not end-to-end Dataview latency"]
fn benchmark_search_pipeline_lexer_fast_abba() {
    use std::hint::black_box;
    use std::time::Instant;
    const ITERATIONS: usize = 40_000;
    let samples = [
        ("ascii-quoted", "\"This is a long unescaped title about a novel with multiple words\"", true),
        ("unicode-quoted", "\"Русское название романа — 漢字 и emoji 📖✨, длинная строка\"", true),
        ("escaped-quoted", "\"Рыцарь \\\"в Маске\\\" / 文件\"", true),
        ("double-op", ">=", false),
        ("single-op", "(", false),
    ];
    for (case, input, text_case) in samples {
        for cycle in 0..8 {
            for (phase, candidate) in [false, true, true, false].into_iter().enumerate() {
                let started = Instant::now();
                let mut work = 0usize;
                for _ in 0..ITERATIONS {
                    if text_case {
                        let parsed = if candidate { read_text(black_box(input)) } else { legacy_text(black_box(input)) };
                        let (value, used) = parsed.expect("closed source");
                        work = work.wrapping_add(value.len() + used);
                    } else {
                        let parsed = if candidate { read_symbol(black_box(input)) } else { legacy_symbol(black_box(input)) };
                        work = work.wrapping_add(parsed.expect("symbol source").1);
                    }
                }
                black_box(work);
                println!(
                    "LEXER_ABBA case={case} route={} cycle={} phase={} iterations={ITERATIONS} elapsed_us={} parity=PASS",
                    if candidate { "fast" } else { "legacy" },
                    cycle + 1, phase + 1, started.elapsed().as_micros()
                );
            }
        }
    }
}
