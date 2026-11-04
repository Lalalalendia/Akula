use super::{Fold, Matcher, Mode, Options, EXACT};

const PREFIX: Options = Options { fold: Fold::Ascii, mode: Mode::WordPrefix, ..EXACT };

#[test]
fn ascii_fast_path_matches_reference_scanner() {
    use super::scan;
    let mut seed = 0x1234_5678_u64;
    let alphabet = b"abcABC01 2-_/.,\\n\\t";
    let random: String = (0..16_384)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            alphabet[(seed as usize) % alphabet.len()] as char
        })
        .collect();
    let all_ascii: String = (0u8..=127).map(char::from).collect();
    for text in [
        "aaaaa abcd ABcd a-b c\\r\\nAA aa",
        "Simple note with a WORD followed by punctuation.",
        "ss STRASSE Strasse A_a 9a",
        &all_ascii,
        &random,
    ] {
        for fold in [Fold::Exact, Fold::Lower, Fold::Ascii] {
            for mode in [Mode::Substring, Mode::WordPrefix, Mode::WholeWord] {
                for query in ["a", "A", "aa", "abc", "AB", "ss", "9", "-", "_", "ß", "0", "\\n"] {
                    let matcher = Matcher::new(query, Options { fold, mode, ..EXACT });
                    let mut reference = Vec::new();
                    scan::unicode_each(&matcher, text, &mut |found| {
                        reference.push(found);
                        true
                    });
                    assert_eq!(
                        matcher.find_all(text),
                        reference,
                        "ASCII parity failed: {fold:?} {mode:?}, query={query:?}",
                    );
                    assert_eq!(
                        matcher.first(text),
                        reference.into_iter().next(),
                        "first-hit parity failed: {fold:?} {mode:?}, query={query:?}",
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "opt-in release-mode benchmark; verify on the same runner before merge"]
fn benchmark_ascii_memchr_candidate_search() {
    use super::scan;
    use std::hint::black_box;
    use std::time::Instant;
    let matcher = Matcher::new("unique-needle", PREFIX);
    for target in [1usize, 8, 64] {
        let repeated = "ordinary text in a long note with no matching keyword.\\n";
        let note = repeated.repeat(target * 1024 * 1024 / repeated.len() + 1);
        let sample = |optimized: bool| {
            let started = Instant::now();
            for _ in 0..3 {
                let count = if optimized {
                    black_box(matcher.find_all(&note).len())
                } else {
                    let mut hits = 0;
                    scan::unicode_each(&matcher, &note, &mut |_| {
                        hits += 1;
                        true
                    });
                    black_box(hits)
                };
                assert_eq!(count, 0);
            }
            started.elapsed().as_secs_f64() * 1000.0 / 3.0
        };
        let mut old = Vec::new();
        let mut fast = Vec::new();
        for _ in 0..3 {
            old.push(sample(false));
            fast.push(sample(true));
            fast.push(sample(true));
            old.push(sample(false));
        }
        old.sort_by(f64::total_cmp);
        fast.sort_by(f64::total_cmp);
        println!(
            "MATCHER_ASCII size_mb={target} baseline_ms={:.3} candidate_ms={:.3} speedup={:.2}x",
            old[3], fast[3], old[3] / fast[3]
        );
    }
}
