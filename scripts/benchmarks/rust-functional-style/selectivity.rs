//! Balanced Rust loop comparison at exactly 0/25/50/75/100% matches.
//! Isolated research, identical input per method, no production changes.
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

// Both booleans are evaluated before the branch, unlike short-circuit &&.
#[no_mangle]
#[inline(never)]
fn combined_cond(books: &[Book]) -> u64 {
    let mut total = 0_u64;
    for book in books {
        if book.published & book.ru {
            total += u64::from(book.chapters);
        }
    }
    total
}

// Two independently maintained sums for adjacent records.
#[no_mangle]
#[inline(never)]
fn dual_accum(books: &[Book]) -> u64 {
    let mut even = 0_u64;
    let mut odd = 0_u64;
    let mut pairs = books.chunks_exact(2);
    for pair in &mut pairs {
        let a = &pair[0];
        let b = &pair[1];
        if a.published && a.ru {
            even += u64::from(a.chapters);
        }
        if b.published && b.ru {
            odd += u64::from(b.chapters);
        }
    }
    for book in pairs.remainder() {
        if book.published && book.ru {
            even += u64::from(book.chapters);
        }
    }
    even + odd
}


// Generate exactly quota/4 matching records in every group of four.
// The rotation changes per group, so passing rows are not one simple periodic
// pattern. Chapter counts are deterministic and shared across scenarios.
fn books_fixture(count: usize, quota: usize) -> (Vec<Book>, u64, usize) {
    assert!(quota <= 4);
    assert_eq!(count % 4, 0);
    let mut books = Vec::with_capacity(count);
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rotation = 0_usize;
    let mut expected_chapters = 0_u64;
    let mut selected_count = 0_usize;

    for index in 0..count {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        if index % 4 == 0 {
            rotation = ((seed >> 19) & 3) as usize;
        }
        let selected = (((index & 3) + rotation) & 3) < quota;
        // Nonmatching records can fail at either flag, not just one.
        let published = selected || (seed & 1) != 0;
        let ru = selected || (!published && ((seed >> 5) & 1) != 0);
        let chapters = 1 + ((seed >> 8) % 300) as u32;
        if selected {
            selected_count += 1;
            expected_chapters += u64::from(chapters);
        }
        books.push(Book { published, ru, chapters });
    }
    assert_eq!(selected_count, count / 4 * quota);
    (books, expected_chapters, selected_count)
}

fn run_many(books: &[Book], f: fn(&[Book]) -> u64, repetitions: usize) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..repetitions {
        checksum = checksum.wrapping_add(black_box(f(black_box(books))));
    }
    black_box(checksum);
    (start.elapsed().as_secs_f64() * 1_000.0) / repetitions as f64
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

fn p95(sorted: &[f64]) -> f64 {
    sorted[((sorted.len() * 95).div_ceil(100)).saturating_sub(1)]
}

fn main() {
    const BOOKS: usize = 10_000_000;
    const TRIALS: usize = 40;
    const REPETITIONS: usize = 8;
    const NAMES: [&str; 5] = [
        "imperative", "functional", "branchless", "combined_cond", "dual_accum",
    ];
    let methods: [fn(&[Book]) -> u64; 5] = [
        imperative, functional, branchless, combined_cond, dual_accum,
    ];
    // Every method occupies every position equally often in 40 trials.
    const ORDERS: [[usize; 5]; 10] = [
        [0, 1, 2, 3, 4], [1, 2, 3, 4, 0], [2, 3, 4, 0, 1],
        [3, 4, 0, 1, 2], [4, 0, 1, 2, 3],
        [4, 3, 2, 1, 0], [3, 2, 1, 0, 4], [2, 1, 0, 4, 3],
        [1, 0, 4, 3, 2], [0, 4, 3, 2, 1],
    ];
    std::fs::create_dir_all("rust-selectivity-results").unwrap();
    let mut csv = String::from("selected_percent,trial,order_position,variant,ms_per_full_scan\n");
    let mut summary_csv = String::from("selected_percent,selected_books,chapters,variant,median_ms,p95_ms,min_ms,max_ms,ratio_to_imperative\n");
    let mut json = format!(
        "{{\"books\":{BOOKS},\"trials\":{TRIALS},\"repetitions_per_trial\":{REPETITIONS},\"scenarios\":["
    );

    // Mix scenario order to avoid a monotonic 0..100% clock-temperature trend.
    for (scenario_index, quota) in [2, 0, 4, 1, 3].into_iter().enumerate() {
        let pct = quota * 25;
        let (books, expected, selected) = books_fixture(BOOKS, quota);
        for (name, &method) in NAMES.iter().zip(methods.iter()) {
            assert_eq!(method(&books), expected, "Incorrect result: {name} at {pct}%");
        }

        for _ in 0..3 {
            for &method in &methods {
                black_box(run_many(&books, method, REPETITIONS));
            }
        }

        let mut times: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::with_capacity(TRIALS));
        for trial in 0..TRIALS {
            for (position, &method) in ORDERS[trial % ORDERS.len()].iter().enumerate() {
                let ms = run_many(&books, methods[method], REPETITIONS);
                times[method].push(ms);
                csv.push_str(&format!("{pct},{trial},{position},{},{ms:.6}\n", NAMES[method]));
            }
        }

        for measurements in &mut times {
            measurements.sort_by(f64::total_cmp);
        }
        let baseline = median(&times[0]);
        println!("SCENARIO selected_percent={pct} selected_books={selected} expected_chapters={expected}");
        if scenario_index > 0 { json.push(','); }
        json.push_str(&format!(
            "{{\"selected_percent\":{pct},\"selected_books\":{selected},\"expected_chapters\":{expected},\"methods\":["
        ));
        for (index, measurements) in times.iter().enumerate() {
            let med = median(measurements);
            let tail = p95(measurements);
            let min = measurements[0];
            let max = measurements[TRIALS - 1];
            let ratio = med / baseline;
            println!(
                "RESULT selected_percent={pct} variant={} median_ms={med:.6} p95_ms={tail:.6} ratio_to_imperative={ratio:.6}",
                NAMES[index]
            );
            summary_csv.push_str(&format!(
                "{pct},{selected},{expected},{},{med:.6},{tail:.6},{min:.6},{max:.6},{ratio:.6}\n",
                NAMES[index]
            ));
            if index > 0 { json.push(','); }
            json.push_str(&format!(
                "{{\"variant\":\"{}\",\"median_ms\":{med:.6},\"p95_ms\":{tail:.6},\"min_ms\":{min:.6},\"max_ms\":{max:.6},\"ratio_to_imperative\":{ratio:.6}}}",
                NAMES[index]
            ));
        }
        json.push_str("]}");
        drop(books);
    }

    json.push_str("]}\n");
    std::fs::write("rust-selectivity-results/trials.csv", csv).unwrap();
    std::fs::write("rust-selectivity-results/summary.csv", summary_csv).unwrap();
    std::fs::write("rust-selectivity-results/summary.json", json).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_agree(books: &[Book], expected: u64) {
        assert_eq!(imperative(books), expected);
        assert_eq!(functional(books), expected);
        assert_eq!(branchless(books), expected);
        assert_eq!(combined_cond(books), expected);
        assert_eq!(dual_accum(books), expected);
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
    fn odd_length_and_one_record() {
        let books = [
            Book { published: true, ru: true, chapters: 8 },
            Book { published: false, ru: true, chapters: 7 },
            Book { published: true, ru: true, chapters: 9 },
        ];
        assert_agree(&books[..1], 8);
        assert_agree(&books, 17);
    }

    #[test]
    fn exact_selectivity_quarters() {
        for quota in 0..=4 {
            let (books, expected, selected) = books_fixture(400, quota);
            assert_eq!(selected, quota * 100);
            assert_eq!(books.iter().filter(|book| book.published && book.ru).count(), selected);
            assert_agree(&books, expected);
        }
    }

    #[test]
    fn all_selected_and_none_selected() {
        let all = [Book { published: true, ru: true, chapters: 7 }; 100];
        let none = [Book { published: false, ru: true, chapters: 7 }; 100];
        assert_agree(&all, 700);
        assert_agree(&none, 0);
    }
}
