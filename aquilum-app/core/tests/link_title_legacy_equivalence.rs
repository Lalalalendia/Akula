//! Independent output-equivalence oracle for the previous link-title parser.
//! The original parser copied all HTML before looking for <title>. Do not
//! change accepted input semantics in a performance-only PR.

use aquilum_core::link_title::{entities, title};

fn old_resolve(body: &str) -> Option<char> {
    if body.is_empty() || body.len() > 8 {
        return None;
    }
    if let Some(number) = body.strip_prefix('#') {
        let code = match number.strip_prefix('x').or_else(|| number.strip_prefix('X')) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code);
    }
    match body.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        "laquo" => Some('«'),
        "raquo" => Some('»'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "lsquo" => Some('‘'),
        "rsquo" => Some('’'),
        _ => None,
    }
}

fn old_decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        match tail.find(';').and_then(|end| old_resolve(&tail[..end]).map(|character| (character, end))) {
            Some((character, end)) => {
                out.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

fn old_parse(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_open = start + "<title".len();
    let gt = lower[after_open..].find('>')?;
    let content_start = after_open + gt + 1;
    let close_rel = lower[content_start..].find("</title>")?;
    let raw = html[content_start..content_start + close_rel].trim();
    if raw.is_empty() {
        return None;
    }
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let decoded = old_decode(&collapsed);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn next(seed: &mut u64, len: usize) -> usize {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    ((*seed >> 32) as usize) % len
}

#[test]
fn title_parser_matches_legacy_for_generated_unicode_tags_and_entities() {
    let parts = [
        "<title>", "<TiTlE lang='ru'>", "<TITLE lang='en'>", "</title>",
        "</TiTlE>", "<titles>", "<titleX>", "></title >", "content",
        "<div>", "x</div>", "<title", ">", "<script>hello</script>",
        "<!--", "-->", "Заголовок", "日本語", "🦈", "\r", "\n", "\t",
        "&amp;", "&AMP;", "&nbsp;", "&MDASH;", "&unknown;", "&#39;",
        "&#x1F988;", "&#X1f988;", "&#invalid;", "&", ";", "  "
    ];
    let mut seed = 0xA11C_0220_2610_22_u64;
    for trial in 0..4096 {
        let mut html = String::new();
        for _ in 0..(2 + next(&mut seed, 32)) {
            html.push_str(parts[next(&mut seed, parts.len())]);
        }
        assert_eq!(
            title::parse(&html),
            old_parse(&html),
            "trial={trial}, html={html:?}"
        );
    }
}

#[test]
fn entity_decoder_matches_legacy_for_all_named_and_malformed_inputs() {
    let parts = [
        "&amp;", "&AMP;", "&quot;", "&Apos;", "&nbsp;", "&mdash;",
        "&ndash;", "&hellip;", "&laquo;", "&raquo;", "&ldquo;",
        "&rdquo;", "&lsquo;", "&rsquo;", "&lt;", "&gt;",
        "&#0;", "&#39;", "&#x1F988;", "&#Xabcd;", "&#-1;",
        "&toolongentity;", "&nönascii;", "&withoutend", ";", "&&",
        "大海", "\t", "🐙"
    ];
    let mut seed = 0xD0C5_2026_1022_u64;
    for trial in 0..4096 {
        let mut encoded = String::new();
        for _ in 0..(1 + next(&mut seed, 24)) {
            encoded.push_str(parts[next(&mut seed, parts.len())]);
        }
        assert_eq!(
            entities::decode(&encoded),
            old_decode(&encoded),
            "trial={trial}, text={encoded:?}"
        );
    }
}

#[test]
fn large_html_preserves_title_after_half_megabyte_and_current_corner_cases() {
    let page = format!(
        "<html><body>{}<TiTlE lang='ja'>  акула \t&amp; 海  </tItLe></body></html>",
        "🦈 <div>x</div>".repeat(32768)
    );
    assert!(page.len() > 500_000);
    assert_eq!(title::parse(&page), old_parse(&page));
    assert_eq!(title::parse(&page).as_deref(), Some("акула & 海"));
    for fragment in [
        "<titleX>accepted today</title>", "<!-- <title>in comment</title> -->",
        "<title> &nbsp; </title>", "<TITLE lang='ru'>你好</TITLE>",
        "<title>tab\n and  \t spaces</title>", "<title>&AMP;&BoGuS;</title>",
    ] {
        assert_eq!(title::parse(fragment), old_parse(fragment), "{fragment}");
    }
}

#[test]
#[ignore = "diagnostic-only paired benchmark; never an automatic merge gate"]
fn benchmark_link_title_parse_abba() {
    use std::hint::black_box;
    use std::time::Instant;

    fn p(samples: &mut [u128], percentile: usize) -> u128 {
        samples.sort_unstable();
        let index = (samples.len() * percentile).div_ceil(100).saturating_sub(1);
        samples[index]
    }

    let cases = [
        (
            "short",
            "<html><head><TiTlE>One &amp; Two</TITLE></head></html>".to_owned(),
            2_000,
        ),
        (
            "large_tail",
            format!("{}<TiTlE lang='ru'>Акула &amp; 海</tItLe>", "x".repeat(512 * 1024)),
            300,
        ),
        (
            "many_tags",
            format!(
                "{}<TITLE lang='ja'>Акула &amp; 海</title>",
                "<div>unicode 海 test</div>".repeat(8192)
            ),
            300,
        ),
    ];
    for (name, html, rounds) in cases {
        assert_eq!(old_parse(&html), title::parse(&html));
        let (mut old, mut new) = (Vec::with_capacity(rounds), Vec::with_capacity(rounds));
        let measure_old = || {
            let start = Instant::now();
            black_box(old_parse(black_box(html.as_str())));
            start.elapsed().as_nanos()
        };
        let measure_new = || {
            let start = Instant::now();
            black_box(title::parse(black_box(html.as_str())));
            start.elapsed().as_nanos()
        };
        for i in 0..rounds {
            if i % 2 == 0 {
                old.push(measure_old());
                new.push(measure_new());
            } else {
                new.push(measure_new());
                old.push(measure_old());
            }
        }
        let (old_p50, new_p50) = (p(&mut old, 50), p(&mut new, 50));
        let (old_p95, new_p95) = (p(&mut old, 95), p(&mut new, 95));
        println!(
            "BENCH-LINK-TITLE case={name} bytes={} rounds={rounds} old_p50_ns={old_p50} new_p50_ns={new_p50} old_p95_ns={old_p95} new_p95_ns={new_p95} ratio_p50={:.3} parity=PASS",
            html.len(),
            new_p50 as f64 / old_p50.max(1) as f64
        );
    }

    let encoded = "&AMP;&nbsp;&laquo;&raquo;&#39;&mdash;".repeat(4096);
    assert_eq!(old_decode(&encoded), entities::decode(&encoded));
    let rounds = 300;
    let (mut old, mut new) = (Vec::with_capacity(rounds), Vec::with_capacity(rounds));
    for i in 0..rounds {
        let legacy = || {
            let start = Instant::now();
            black_box(old_decode(black_box(encoded.as_str())));
            start.elapsed().as_nanos()
        };
        let candidate = || {
            let start = Instant::now();
            black_box(entities::decode(black_box(encoded.as_str())));
            start.elapsed().as_nanos()
        };
        if i % 2 == 0 {
            old.push(legacy());
            new.push(candidate());
        } else {
            new.push(candidate());
            old.push(legacy());
        }
    }
    let (old_p50, new_p50) = (p(&mut old, 50), p(&mut new, 50));
    let (old_p95, new_p95) = (p(&mut old, 95), p(&mut new, 95));
    println!(
        "BENCH-LINK-TITLE case=entities bytes={} rounds={rounds} old_p50_ns={old_p50} new_p50_ns={new_p50} old_p95_ns={old_p95} new_p95_ns={new_p95} ratio_p50={:.3} parity=PASS",
        encoded.len(),
        new_p50 as f64 / old_p50.max(1) as f64
    );
}
