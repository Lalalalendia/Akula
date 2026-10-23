use super::index::SearchIndex;
use super::query::{analyzed_query_terms, Request};
use std::path::Path;
use tantivy::doc;

#[test]
fn normalizes_case_and_deduplicates_query_terms() {
    assert_eq!(analyzed_query_terms("Aquilum АКВИЛ aquilum"), vec!["aquilum".to_string(), "акв".to_string()]);
}

fn indexed(documents: &[(&str, &str)]) -> (tempfile::TempDir, SearchIndex) {
    let directory = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(&directory.path().join("index")).unwrap();
    let mut writer = index.index.writer(15_000_000).unwrap();
    for (name, body) in documents {
        let path = directory.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        let title = Path::new(name).file_stem().unwrap().to_string_lossy();
        writer
            .add_document(doc!(
                index.fields.id => path.to_string_lossy().as_ref(),
                index.fields.path => *name,
                index.fields.title => title.as_ref(),
                index.fields.body => *body,
            ))
            .unwrap();
    }
    writer.commit().unwrap();
    index.reload().unwrap();
    (directory, index)
}

fn search(index: &SearchIndex, text: &str) -> Vec<super::models::SearchResult> {
    index.search(&Request::new(text)).unwrap().results
}

#[test]
fn disk_index_searches_title_and_body_by_case_insensitive_prefix() {
    let (_guard, index) = indexed(&[("Квантовая заметка.md", "Внутри есть быстрый поиск и ещё один поиск.")]);

    assert_eq!(search(&index, "КВАН").len(), 1);
    let body_results = search(&index, "ПОИ");
    assert_eq!(body_results[0].match_count, 2);
    assert!(body_results[0].snippet.contains("поиск"));
}

#[test]
fn the_body_is_indexed_but_not_kept_in_the_index_store() {
    let (_guard, index) = indexed(&[("Заметка.md", "длинный текст заметки")]);
    let searcher = index.reader.searcher();
    let document = searcher.doc::<tantivy::TantivyDocument>(tantivy::DocAddress::new(0, 0)).unwrap();
    assert!(document.get_first(index.fields.body).is_none());
    assert_eq!(search(&index, "текст").len(), 1);
}

#[test]
fn the_first_match_comes_from_the_file_with_its_offset_and_section() {
    let body = "---\ntags: []\n---\n# Раздел\n\nСтрока 😀 с Тестом и ещё тесты\n";
    let (_guard, index) = indexed(&[("Заметка.md", body)]);
    let result = &search(&index, "тест")[0];
    let at = body.find("Тестом").unwrap();
    assert_eq!(result.match_offset, body[..at].encode_utf16().count());
    assert_eq!(result.heading.as_deref(), Some("Раздел"));
    assert_eq!(result.match_count, 2);
    assert!(result.snippet.contains("Тестом"));
}

#[test]
fn a_note_without_a_body_match_shows_its_beginning() {
    let (_guard, index) = indexed(&[("Тест.md", "Начало заметки без слова")]);
    let result = &search(&index, "тест")[0];
    assert_eq!((result.match_count, result.match_offset), (1, 0));
    assert_eq!(result.snippet, "Начало заметки без слова");
    assert_eq!(result.heading, None);
    assert_eq!(result.matched_terms, ["тест"]);
}

#[test]
fn a_single_letter_finds_and_counts_words_that_start_with_it() {
    let (_guard, index) = indexed(&[("Кофе.md", "кофе и какао"), ("Чай.md", "чай")]);
    let results = search(&index, "к");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].match_count, 3);
}

#[test]
fn a_folder_limits_the_search_and_more_results_are_reported() {
    let (guard, index) = indexed(&[
        ("Проекты/Кофе.md", "кофе"),
        ("Проекты/Чай.md", "кофе и чай"),
        ("Архив/Кофе.md", "кофе"),
    ]);
    let folder = guard.path().join("Проекты");
    let inside = index.search(&Request { folder: Some(&folder), ..Request::new("кофе") }).unwrap();
    assert_eq!(inside.results.len(), 2);
    assert!(inside.results.iter().all(|result| Path::new(&result.path).starts_with(&folder)));
    assert!(!inside.has_more);
    let first = index.search(&Request { limit: Some(1), ..Request::new("кофе") }).unwrap();
    assert!(first.has_more && first.results.len() == 1);
}

#[test]
fn a_cancelled_request_returns_no_results() {
    let (_guard, index) = indexed(&[("Кофе.md", "кофе")]);
    let generation = crate::text::cancel::Generation::default();
    let cancel = generation.next();
    generation.stop();
    let request = Request { cancel, ..Request::new("кофе") };
    assert!(matches!(index.search(&request), Err(super::error::SearchError::Cancelled)));
}

#[test]
fn a_multi_word_query_ranks_partial_matches_instead_of_returning_nothing() {
    let (_guard, index) = indexed(&[
        ("Камера.md", "# Ощущение камеры\n\nФизика тика влияет на кадр.\n"),
        ("Свет.md", "# Освещение\n\nОдин источник на всю сцену.\n"),
        ("Быстрота.md", "# Плавность\n\nКадр рисуется быстро.\n"),
    ]);

    let results = search(&index, "физика тика кадр плавность");
    assert_eq!(results.len(), 2, "ни одна заметка не содержит все слова, но частичные совпадения возвращаются");
    assert!(results[0].path.ends_with("Камера.md"), "заметка с большим числом совпавших слов идёт первой");
    assert_eq!(results[0].matched_terms.len(), 3);
    assert_eq!(results[1].matched_terms.len(), 2);
    assert_eq!(results[0].heading.as_deref(), Some("Ощущение камеры"), "фрагмент подписан разделом, в котором нашлось совпадение");
}

#[test]
fn a_distant_crlf_body_match_keeps_heading_and_normalized_utf16_offset() {
    let mut body = "# Заголовок\r\n".to_owned();
    body.push_str(&"😀 длинный абзац без искомого слова\r\n".repeat(500));
    body.push_str("## Нужный раздел\r\nЗдесь находится УникальныйТермин\r\nСтрока после совпадения\r\n");
    let (_guard, index) = indexed(&[("Книга.md", &body)]);
    let results = search(&index, "уникальныйтермин");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    let normalized = crate::files::document::normalize_line_endings(body);
    let byte_offset = normalized.find("УникальныйТермин").unwrap();
    assert_eq!(result.match_offset, normalized[..byte_offset].encode_utf16().count());
    assert_eq!(result.heading.as_deref(), Some("Нужный раздел"));
    assert!(result.snippet.contains("УникальныйТермин"));
}

#[test]
fn a_stale_index_match_keeps_the_opening_snippet_bounded() {
    let (guard, index) = indexed(&[("Данные.md", "originalneedle был здесь")]);
    let file = guard.path().join("Данные.md");
    std::fs::write(&file, "Нет совпадений, только начало заметки.\n".repeat(50_000)).unwrap();

    let results = search(&index, "originalneedle");
    assert_eq!(results.len(), 1, "the live index still contains the previously indexed token");
    let result = &results[0];
    assert_eq!(result.match_offset, 0);
    assert_eq!(result.heading, None);
    assert!(result.snippet.starts_with("Нет совпадений"));
    assert!(result.snippet.len() < 1024, "do not buffer or expose the entire stale file");
    assert!(result.snippet.ends_with("..."));
}

#[test]
fn unreadable_or_invalid_utf8_search_hits_do_not_crash() {
    let (guard, index) = indexed(&[("Данные.md", "originalneedle был здесь")]);
    let path = guard.path().join("Данные.md");
    std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
    let result = &search(&index, "originalneedle")[0];
    assert_eq!(result.match_offset, 0);
    assert!(result.snippet.is_empty());
    std::fs::remove_file(&path).unwrap();
    let removed_result = &search(&index, "originalneedle")[0];
    assert_eq!(removed_result.match_offset, 0);
    assert!(removed_result.snippet.is_empty());
}

#[test]
fn a_minus_word_excludes_matching_body_titles_and_paths() {
    let (_guard, index) = indexed(&[
        ("Свободная.md", "поиск отчётов"),
        ("Ошибка.md", "поиск отчётов"),
        ("Архив/Документ.md", "поиск отчётов"),
        ("Открыто/Документ.md", "поиск отчётов"),
    ]);
    let names: Vec<_> = search(&index, "поиск -ошибка -архив")
        .into_iter()
        .map(|result| result.path)
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().any(|path| path.ends_with("Свободная.md")));
    assert!(names.iter().any(|path| path.contains("Открыто")));
    assert!(names.iter().all(|path| !path.contains("Ошибка") && !path.contains("Архив")));
}

#[test]
fn an_exclusion_keeps_partial_positive_matches_and_never_lists_all_notes() {
    let (_guard, index) = indexed(&[
        ("Первый.md", "физика тика кадр"),
        ("Второй.md", "физика плавность"),
        ("Третий.md", "кадр ошибка"),
    ]);
    let results = search(&index, "физика кадр -ошибка");
    assert_eq!(results.len(), 2, "positive terms remain partial matches");
    assert!(results[0].path.ends_with("Первый.md"));
    assert!(results[1].path.ends_with("Второй.md"));
    assert!(search(&index, "-ошибка").is_empty(), "negative-only search must not enumerate the vault");
    assert!(search(&index, "физика -физика").is_empty());
    assert_eq!(search(&index, "физика-тика").len(), 2, "a hyphen inside a word is not negation");
}

#[test]
fn exclusion_stems_are_normalized_and_do_not_appear_in_result_terms() {
    let (_guard, index) = indexed(&[
        ("Свободная.md", "проверка документов"),
        ("Запрет.md", "проверка документов и ошибки"),
    ]);
    let found = index.search(&Request::new("ПРОВЕРКА -ОШИБКИ")).unwrap();
    assert_eq!(found.results.len(), 1);
    assert!(found.results[0].path.ends_with("Свободная.md"));
    assert!(!found.terms.iter().any(|term| term.contains("ошиб")));
}
