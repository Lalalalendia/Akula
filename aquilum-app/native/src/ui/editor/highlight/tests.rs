use super::{Lang, Token, tokens};

fn kinds(line: &str, lang: Lang) -> Vec<(&str, Token)> {
    tokens(line, lang).into_iter().map(|(r, t)| (&line[r], t)).collect()
}

#[test]
fn highlights_common_tokens() {
    let got = kinds("fn main() { let x = 42; println!(\"hi\"); } // done", Lang::Rust);
    assert!(got.contains(&("fn", Token::Keyword)));
    assert!(got.contains(&("main", Token::Function)));
    assert!(got.contains(&("42", Token::Number)));
    assert!(got.contains(&("\"hi\"", Token::String)));
    assert!(got.contains(&("// done", Token::Comment)));
    let js = kinds("document.getElementById(\"demo\").innerHTML = txt", Lang::Script);
    assert!(js.contains(&("getElementById", Token::Function)));
    assert!(js.contains(&("innerHTML", Token::Property)));
    let html = kinds("<input type=\"button\">", Lang::Markup);
    assert!(html.contains(&("input", Token::Keyword)));
    assert!(html.contains(&("type", Token::Property)));
}

#[test]
fn sql_and_dataview_keywords_ignore_ascii_case_without_matching_longer_names() {
    let sql = kinds("SeLeCt selected FROM fromage WHERE answer > 4", Lang::Sql);
    assert!(sql.contains(&("SeLeCt", Token::Keyword)));
    assert!(sql.contains(&("FROM", Token::Keyword)));
    assert!(sql.contains(&("WHERE", Token::Keyword)));
    assert!(!sql.iter().any(|(text, token)| (*text == "selected" || *text == "fromage") && *token == Token::Keyword));

    let dataview = kinds("TaBlE WITHOUT id from once sOrT title desc", Lang::Dataview);
    for expected in ["TaBlE", "WITHOUT", "id", "from", "once", "sOrT", "desc"] {
        assert!(dataview.contains(&(expected, Token::Keyword)), "missing keyword {expected}");
    }
    assert!(!dataview.iter().any(|(text, token)| *text == "title" && *token == Token::Keyword));
}

#[test]
fn comments_are_recognized_only_at_supported_start_characters() {
    let rust = kinds("let msg = \"// quoted\"; // real comment", Lang::Rust);
    assert!(rust.contains(&("\"// quoted\"", Token::String)));
    assert!(rust.contains(&("// real comment", Token::Comment)));

    let sql = kinds("SELECT 2 - 1; -- commented", Lang::Sql);
    assert!(sql.contains(&("SELECT", Token::Keyword)));
    assert!(sql.contains(&("-- commented", Token::Comment)));
    let without_comment = kinds("SELECT 2 - 1", Lang::Sql);
    assert!(without_comment.iter().all(|(_, token)| *token != Token::Comment));

    let markup = kinds("<!-- Примечание --> <b>Текст</b>", Lang::Markup);
    assert!(markup.contains(&("<!-- Примечание -->", Token::Comment)));
    let yaml = kinds("ключ: значение # комментарий", Lang::Yaml);
    assert!(yaml.contains(&("# комментарий", Token::Comment)));

    // JSON is intentionally not given comments by the highlighter.
    assert!(kinds("\"url\": \"https://example.test//path\"", Lang::Json)
        .iter()
        .all(|(_, token)| *token != Token::Comment));
}

#[test]
fn unicode_keyword_inputs_keep_byte_exact_ranges() {
    let text = "taBLE Название, İ, Key, κόσμος WHERE type = \"книга\"";
    let got = kinds(text, Lang::Dataview);
    assert!(got.contains(&("taBLE", Token::Keyword)));
    assert!(got.contains(&("WHERE", Token::Keyword)));
    assert!(got.contains(&("\"книга\"", Token::String)));
    assert!(!got.iter().any(|(word, token)| ["Название", "İ", "Key", "κόσμος"].contains(word) && *token == Token::Keyword));
}
