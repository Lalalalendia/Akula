use super::find_terms::find_terms;
use super::matching::{FIND, Search};
use aquilum_core::text::matcher::Matcher;

#[test]
fn search_query_parses_or_and_literal_pipes() {
    assert_eq!(find_terms(" перевод || переводчик ||  "), ["перевод", "переводчик"]);
    assert_eq!(find_terms(r"\||"), ["||"]);
    assert_eq!(find_terms(" фраза "), [" фраза "]);
    assert_eq!(find_terms(" || "), Vec::<String>::new());
    assert_eq!(find_terms("один || || два"), ["один", "два"]);
}

#[test]
fn or_search_is_leftmost_longest_and_shared_with_embeds() {
    let text = "переводчик перевод Переводчик";
    let query = "перевод || переводчик";
    let expected = ["переводчик", "перевод", "Переводчик"];
    let found = Matcher::any(&find_terms(query), FIND).find_all(text);
    assert_eq!(found.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), expected);
    let mut seen = 0;
    assert_eq!(Search { query: query.into(), only: None }.ranges(text, &mut seen), found);
    let repeated = Matcher::any(&find_terms("aa || a"), FIND).find_all("aaaa");
    assert_eq!(repeated, [0..2, 2..4]);
}
