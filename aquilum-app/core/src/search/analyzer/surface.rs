use super::filters::ExclusionFilter;
use super::morphology::StemFilter;
use std::cmp::Ordering;
use std::collections::HashMap;
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, RawTokenizer, RemoveLongFilter, TextAnalyzer, TokenStream};

fn surface_word_tokenizer() -> TextAnalyzer {
    // WordSpans already produces a single alphanumeric word. Running the
    // document sanitizer for every surface form only copies that word twice.
    TextAnalyzer::builder(RawTokenizer::default())
        .filter(RemoveLongFilter::limit(120))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(StemFilter)
        .filter(ExclusionFilter)
        .build()
}

pub fn stem_surface_labels(title: &str, body: &str) -> HashMap<String, String> {
    stem_surface_labels_with_tokenizer(title, body, surface_word_tokenizer())
}

fn stem_surface_labels_with_tokenizer(title: &str, body: &str, mut analyzer: TextAnalyzer) -> HashMap<String, String> {
    let mut surfaces = HashMap::<&str, u32>::new();
    for text in [title, body] {
        for surface in alphanumeric_words(text).filter(|surface| surface.chars().nth(1).is_some()) {
            *surfaces.entry(surface).or_insert(0) += 1;
        }
    }
    let mut best = HashMap::<String, (&str, u32)>::new();
    for (surface, count) in surfaces {
        let Some(stem) = stem_token(&mut analyzer, surface).filter(|stem| stem.chars().nth(1).is_some()) else {
            continue;
        };
        let chosen = best.entry(stem).or_insert((surface, count));
        if preference(&(surface, count), chosen) == Ordering::Greater {
            *chosen = (surface, count);
        }
    }
    best.into_iter().map(|(stem, (surface, _))| (stem, surface.to_owned())).collect()
}

fn preference((left, left_count): &(&str, u32), (right, right_count): &(&str, u32)) -> Ordering {
    left_count
        .cmp(right_count)
        .then_with(|| right.chars().count().cmp(&left.chars().count()))
        .then_with(|| right.cmp(left))
}

fn stem_token(analyzer: &mut TextAnalyzer, surface: &str) -> Option<String> {
    let mut stem = None;
    analyzer.token_stream(surface).process(&mut |token| {
        if stem.is_none() && !token.text.is_empty() {
            stem = Some(token.text.clone());
        }
    });
    stem
}

fn alphanumeric_words(text: &str) -> impl Iterator<Item = &str> {
    WordSpans { text, cursor: 0 }
}

struct WordSpans<'a> {
    text: &'a str,
    cursor: usize,
}

impl<'a> Iterator for WordSpans<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.text.len() {
            let ch = self.text[self.cursor..].chars().next()?;
            if ch.is_alphanumeric() {
                break;
            }
            self.cursor += ch.len_utf8();
        }
        if self.cursor >= self.text.len() {
            return None;
        }
        let start = self.cursor;
        while self.cursor < self.text.len() {
            let ch = self.text[self.cursor..].chars().next()?;
            if !ch.is_alphanumeric() {
                break;
            }
            self.cursor += ch.len_utf8();
        }
        Some(&self.text[start..self.cursor])
    }
}

#[cfg(test)]
mod tests {
    use super::stem_surface_labels;

    #[test]
    fn maps_russian_stems_to_surface_words() {
        let labels = stem_surface_labels("новая книга", "про новую книгу");
        assert_eq!(labels.get("книг").map(String::as_str), Some("книга"));
        assert_eq!(labels.get("нов").map(String::as_str), Some("новая"));
    }

    #[test]
    fn prefers_the_most_frequent_surface_form() {
        let labels = stem_surface_labels("", "книгу книгу книгу книга");
        assert_eq!(labels.get("книг").map(String::as_str), Some("книгу"));
    }

    #[test]
    fn maps_english_stems_to_surface_words() {
        let labels = stem_surface_labels("machines", "about machines and machine");
        assert_eq!(labels.get("machin").map(String::as_str), Some("machines"));
    }

    #[test]
    fn optimized_surface_words_match_the_full_analyzer_on_real_words() {
        let mut full = crate::search::analyzer::indexing_tokenizer();
        let mut direct = super::surface_word_tokenizer();
        for word in [
            "машина", "машины", "программирование", "architecture",
            "programming", "machines", "naïve", "Café", "строительство",
            "abc123", "API", "www", "https", "дом", "45", "東京", "École",
            "azərbaycan", "あいう", "агент2026", "Einführung",
        ] {
            assert_eq!(
                super::stem_token(&mut direct, word),
                super::stem_token(&mut full, word),
                "mismatch on {word:?}"
            );
        }
    }

    #[test]
    fn optimized_surface_labels_match_legacy_pipeline_on_documents() {
        let input = [
            ("Машины и технологии", "Мы строим новые машины и программируем машины."),
            ("Machine architecture", "Machines are programming programs with machine learning."),
            ("Наука и research", "Операционная система, quantum вычисления, Wikipedia."),
            ("Café и École", "naïve architecture 東京 программирование"),
            ("YAML fragments", "---\ntype: book\n---\ntext with [[wiki links]] and URLs https://example.org"),
            ("mixed", "code123 ### заголовки abc123 с http url www example"),
        ];
        for (title, body) in input {
            let legacy = super::stem_surface_labels_with_tokenizer(
                title, body, crate::search::analyzer::indexing_tokenizer()
            );
            let optimized = super::stem_surface_labels(title, body);
            assert_eq!(optimized, legacy, "labels changed for {title:?}");
        }
    }

    #[test]
    #[ignore = "diagnostic performance comparison; run in release mode"]
    fn benchmark_surface_tokenizer() {
        use std::hint::black_box;
        use std::time::Instant;

        let title = "Программирование, машины, архитектура и нейронные сети";
        let roots = [
            "architecture", "programming", "machines", "performance",
            "компьютеризация", "технологические", "программирование", "архитектура",
        ];
        let body = (0..512)
            .map(|i| {
                let suffix = format!(
                    "{}{}",
                    char::from(b'a' + (i / 26) as u8),
                    char::from(b'a' + (i % 26) as u8),
                );
                format!("{}{} ", roots[i % roots.len()], suffix)
            })
            .collect::<String>();

        fn sample(title: &str, body: &str, optimized: bool) -> f64 {
            let start = Instant::now();
            for _ in 0..60 {
                let analyzer = if optimized {
                    super::surface_word_tokenizer()
                } else {
                    crate::search::analyzer::indexing_tokenizer()
                };
                black_box(super::stem_surface_labels_with_tokenizer(
                    black_box(title), black_box(body), analyzer
                ));
            }
            start.elapsed().as_secs_f64() * 1_000.0
        }

        let mut before = Vec::new();
        let mut after = Vec::new();
        for _ in 0..6 {
            before.push(sample(title, &body, false));
            after.push(sample(title, &body, true));
            after.push(sample(title, &body, true));
            before.push(sample(title, &body, false));
        }
        before.sort_by(f64::total_cmp);
        after.sort_by(f64::total_cmp);
        let baseline = (before[5] + before[6]) / 2.0;
        let optimized = (after[5] + after[6]) / 2.0;
        println!(
            "BENCH-SURFACE baseline_ms={baseline:.3} optimized_ms={optimized:.3} speedup={:.3}x",
            baseline / optimized
        );
    }

}
