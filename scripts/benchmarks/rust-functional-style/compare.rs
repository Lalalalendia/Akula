//! Comparison of three Rust styles on the same deterministic book catalog.
//! Isolated research only. No external dependencies or production changes.
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone, Copy)]
struct Book {
    published: bool,
    ru: bool,
    chapters: u32,
}

// Export stable symbols so we can inspect the actual instructions in the binary.
#[no_mangle]
#[inline(never)]
fn imperative(books: &[Book]) -> u64 {
    let mut total = 0_u64;
    for book in books {
        if book.published && book.ru {
            total += u64::from(book.chapters);
        }
    }
    total
}

#[no_mangle]
#[inline(never)]
fn functional(books: &[Book]) -> u64 {
    books
        .iter()
        .filter(|book| book.published && book.ru)
        .map(|book| u64::from(book.chapters))
        .sum()
}

// No if in the source-level hot loop. Verify the actual machine code independently.
#[no_mangle]
#[inline(never)]
fn branchless(books: &[Book]) -> u64 {
    let mut total = 0_u64;
    for book in books {
        let selected = u64::from(book.published && book.ru);
        total += u64::from(book.chapters) * selected;
    }
    total
}

fn books_fixture(count: usize) -> Vec<Book> {
    let mut books = Vec::with_capacity(count);
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..count {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let x = seed as u32;
        books.push(Book {
            published: x & 3 != 0,
            ru: (x >> 2) & 3 == 0,
            chapters: 1 + ((x >> 8) % 300),
        });
    }
    books
}

fn run_many(books: &[Book], f: fn(&[Book]) -> u64, repetitions: usize) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..repetitions {
        let input = black_box(books);
        checksum = checksum.wrapping_add(black_box(f(input)));
    }
    black_box(checksum);
    (start.elapsed().as_secs_f64() * 1_000.0) / repetitions as f64
}

fn median(sorted: &[f64]) -> f64 {
    let len = sorted.len();
    if len % 2 == 0 {
        (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
    } else {
        sorted[len / 2]
    }
}

fn p95(sorted: &[f64]) -> f64 {
    sorted[((sorted.len() * 95).div_ceil(100)).saturating_sub(1)]
}

fn main() {
    const BOOKS: usize = 10_000_000;
    const REPETITIONS: usize = 32;
    const TRIALS: usize = 60;

    // Every method must return exactly the same number for exactly the same data.
    let books = books_fixture(BOOKS);
    let expected = imperative(&books);
    assert_eq!(functional(&books), expected);
    assert_eq!(branchless(&books), expected);

    let methods: [fn(&[Book]) -> u64; 3] = [imperative, functional, branchless];
    let names = ["imperative", "functional", "branchless"];

    // Identical warm-up count for each method.
    for _ in 0..3 {
        for &method in &methods {
            black_box(run_many(&books, method, REPETITIONS));
        }
    }

    let mut times: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::with_capacity(TRIALS));
    let mut csv = String::from("trial,variant,ms_per_full_scan\n");
    // Six balanced orders; each method appears first, middle and last equally often.
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2], [1, 2, 0], [2, 0, 1],
        [2, 1, 0], [1, 0, 2], [0, 2, 1],
    ];

    for trial in 0..TRIALS {
        let mut result = [0_f64; 3];
        for &method in &ORDERS[trial % ORDERS.len()] {
            result[method] = run_many(&books, methods[method], REPETITIONS);
        }
        for method in 0..3 {
            times[method].push(result[method]);
            csv.push_str(&format!("{trial},{},{:.6}\n", names[method], result[method]));
        }
    }

    for measurements in &mut times {
        measurements.sort_by(f64::total_cmp);
    }
    let a_med = median(&times[0]);
    let b_med = median(&times[1]);
    let c_med = median(&times[2]);
    let a_p95 = p95(&times[0]);
    let b_p95 = p95(&times[1]);
    let c_p95 = p95(&times[2]);

    println!("BOOKS={BOOKS} REPETITIONS={REPETITIONS} TRIALS={TRIALS}");
    println!("FULL_SCANS_PER_VARIANT={}", REPETITIONS * TRIALS);
    println!("RECORDS_PROCESSED_PER_VARIANT={}", (BOOKS as u64) * (REPETITIONS as u64) * (TRIALS as u64));
    println!("EXPECTED_CHAPTERS={expected}");
    println!("BOOK_SIZE_BYTES={}", std::mem::size_of::<Book>());
    println!("IMPERATIVE_MEDIAN_MS={a_med:.6} IMPERATIVE_P95_MS={a_p95:.6}");
    println!("FUNCTIONAL_MEDIAN_MS={b_med:.6} FUNCTIONAL_P95_MS={b_p95:.6}");
    println!("BRANCHLESS_MEDIAN_MS={c_med:.6} BRANCHLESS_P95_MS={c_p95:.6}");
    println!("FUNCTIONAL_TO_IMPERATIVE_RATIO={:.6}", b_med / a_med);
    println!("BRANCHLESS_TO_IMPERATIVE_RATIO={:.6}", c_med / a_med);
    println!("BRANCHLESS_TO_FUNCTIONAL_RATIO={:.6}", c_med / b_med);

    std::fs::create_dir_all("rust-functional-results").unwrap();
    let summary = format!(
        r#"{{
  "fixture": "deterministic xorshift",
  "books": {BOOKS},
  "repetitions_per_trial": {REPETITIONS},
  "trials": {TRIALS},
  "chapters": {expected},
  "book_size_bytes": {},
  "imperative_median_ms": {a_med:.6},
  "imperative_p95_ms": {a_p95:.6},
  "functional_median_ms": {b_med:.6},
  "functional_p95_ms": {b_p95:.6},
  "branchless_median_ms": {c_med:.6},
  "branchless_p95_ms": {c_p95:.6},
  "functional_to_imperative_ratio": {:.6},
  "branchless_to_imperative_ratio": {:.6},
  "branchless_to_functional_ratio": {:.6}
}}
"#,
        std::mem::size_of::<Book>(),
        b_med / a_med,
        c_med / a_med,
        c_med / b_med,
    );
    std::fs::write("rust-functional-results/summary.json", summary).unwrap();
    std::fs::write("rust-functional-results/trials.csv", csv).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_agree(books: &[Book], expected: u64) {
        assert_eq!(imperative(books), expected);
        assert_eq!(functional(books), expected);
        assert_eq!(branchless(books), expected);
    }

    #[test]
    fn empty_input() {
        assert_agree(&[], 0);
    }

    #[test]
    fn mixed_books_have_known_answer() {
        let books = [
            Book { published: true, ru: true, chapters: 12 },
            Book { published: false, ru: true, chapters: 90 },
            Book { published: true, ru: false, chapters: 80 },
            Book { published: true, ru: true, chapters: 31 },
        ];
        assert_agree(&books, 43);
    }

    #[test]
    fn sum_is_u64_not_u32() {
        let books = [
            Book { published: true, ru: true, chapters: u32::MAX },
            Book { published: true, ru: true, chapters: u32::MAX },
        ];
        assert_agree(&books, 2 * u64::from(u32::MAX));
    }

    #[test]
    fn large_fixture_parity() {
        let books = books_fixture(100_000);
        assert_agree(&books, imperative(&books));
    }

    #[test]
    fn all_selected_and_none_selected() {
        let all = [Book { published: true, ru: true, chapters: 7 }; 100];
        let none = [Book { published: false, ru: true, chapters: 7 }; 100];
        assert_agree(&all, 700);
        assert_agree(&none, 0);
    }
}
