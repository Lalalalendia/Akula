use std::borrow::Cow;

fn run_is_double(text: &str, at: usize, symbol: char) -> bool {
    text[..at].chars().next_back() != Some(symbol) && text[at + 2 * symbol.len_utf8()..].chars().next() != Some(symbol)
}

pub(super) fn paired(text: &str, symbol: char, flanking: bool) -> Vec<(usize, usize)> {
    let delimiter: Cow<'_, str> = match symbol {
        '=' => Cow::Borrowed("=="),
        '%' => Cow::Borrowed("%%"),
        _ => Cow::Owned([symbol; 2].iter().collect()),
    };
    if !text.contains(delimiter.as_ref()) {
        return Vec::new();
    }
    let width = delimiter.len();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find(delimiter.as_ref()).map(|i| from + i) {
        let after = open + width;
        let opens = run_is_double(text, open, symbol) && (!flanking || text[after..].chars().next().is_some_and(|c| !c.is_whitespace()));
        let close = opens.then(|| {
            let mut search = after;
            while let Some(at) = text[search..].find(delimiter.as_ref()).map(|i| search + i) {
                if run_is_double(text, at, symbol) && (!flanking || text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace())) {
                    return Some(at);
                }
                search = at + width;
            }
            None
        });
        match close.flatten() {
            Some(close) if close > after => {
                found.push((open, close));
                from = close + width;
            }
            _ => from = after,
        }
    }
    found
}
