mod fast;
use fast::{read_symbol, read_text};

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Word(String),
    Number(f64),
    Text(String),
    WikiLink(String),
    Tag(String),
    Symbol(Symbol),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Symbol {
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    Comma,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Bang,
    AmpAmp,
    PipePipe,
    Arrow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub piece: Piece,
    pub from: usize,
    pub to: usize,
}

pub fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        let start = cursor;
        let symbol = bytes[cursor];

        if symbol.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if text[cursor..].starts_with("//") {
            cursor += text[cursor..].find('\n').unwrap_or(text.len() - cursor);
            continue;
        }
        if text[cursor..].starts_with("[[") {
            let rest = &text[cursor + 2..];
            let end = rest
                .find("]]")
                .ok_or_else(|| "Ссылка не закрыта: не хватает «]]»".to_owned())?;
            tokens.push(Token {
                piece: Piece::WikiLink(rest[..end].trim().to_owned()),
                from: start,
                to: cursor + 2 + end + 2,
            });
            cursor += 2 + end + 2;
            continue;
        }
        if symbol == b'"' {
            let (value, length) = read_text(&text[cursor..])?;
            tokens.push(Token {
                piece: Piece::Text(value),
                from: start,
                to: cursor + length,
            });
            cursor += length;
            continue;
        }
        if symbol == b'#' {
            let length = tag_length(&text[cursor + 1..]);
            if length == 0 {
                return Err("После «#» ожидается имя тега".to_owned());
            }
            tokens.push(Token {
                piece: Piece::Tag(text[cursor + 1..cursor + 1 + length].to_owned()),
                from: start,
                to: cursor + 1 + length,
            });
            cursor += 1 + length;
            continue;
        }
        if symbol.is_ascii_digit() {
            let length = number_length(&text[cursor..]);
            let raw = &text[cursor..cursor + length];
            let value = raw
                .parse::<f64>()
                .map_err(|_| format!("Не похоже на число: «{raw}»"))?;
            tokens.push(Token {
                piece: Piece::Number(value),
                from: start,
                to: cursor + length,
            });
            cursor += length;
            continue;
        }
        if let Some((found, length)) = read_symbol(&text[cursor..]) {
            tokens.push(Token {
                piece: Piece::Symbol(found),
                from: start,
                to: cursor + length,
            });
            cursor += length;
            continue;
        }

        let length = word_length(&text[cursor..]);
        if length == 0 {
            let found = text[cursor..].chars().next().unwrap_or(' ');
            return Err(format!("Непонятный символ «{found}»"));
        }
        tokens.push(Token {
            piece: Piece::Word(text[cursor..cursor + length].to_owned()),
            from: start,
            to: cursor + length,
        });
        cursor += length;
    }

    Ok(tokens)
}

fn number_length(rest: &str) -> usize {
    let mut length = 0;
    let mut seen_dot = false;
    for symbol in rest.chars() {
        if symbol.is_ascii_digit() {
            length += 1;
        } else if symbol == '.' && !seen_dot && rest[length + 1..].starts_with(|next: char| next.is_ascii_digit()) {
            seen_dot = true;
            length += 1;
        } else {
            break;
        }
    }
    length
}

fn word_length(rest: &str) -> usize {
    rest.chars()
        .take_while(|symbol| symbol.is_alphanumeric() || *symbol == '_' || *symbol == '-')
        .map(char::len_utf8)
        .sum()
}

fn tag_length(rest: &str) -> usize {
    rest.chars()
        .take_while(|symbol| symbol.is_alphanumeric() || matches!(symbol, '_' | '-' | '/'))
        .map(char::len_utf8)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{tokenize, Piece, Symbol};

    fn pieces(text: &str) -> Vec<Piece> {
        tokenize(text)
            .expect("разбор удался")
            .into_iter()
            .map(|token| token.piece)
            .collect()
    }

    #[test]
    fn reads_words_numbers_and_symbols() {
        assert_eq!(
            pieces("rating >= 4.5"),
            vec![
                Piece::Word("rating".to_owned()),
                Piece::Symbol(Symbol::GreaterOrEqual),
                Piece::Number(4.5),
            ]
        );
    }

    #[test]
    fn a_dot_after_a_number_is_not_swallowed_as_a_decimal_point() {
        assert_eq!(
            pieces("5.name"),
            vec![
                Piece::Number(5.0),
                Piece::Symbol(Symbol::Dot),
                Piece::Word("name".to_owned()),
            ]
        );
    }

    #[test]
    fn reads_a_wiki_link_with_spaces_and_emoji() {
        assert_eq!(
            pieces("[[📖 5 пороков команды]]"),
            vec![Piece::WikiLink("📖 5 пороков команды".to_owned())]
        );
    }

    #[test]
    fn reads_a_quoted_string_with_escapes() {
        assert_eq!(
            pieces(r#""Ссылающиеся \"заметки\"""#),
            vec![Piece::Text("Ссылающиеся \"заметки\"".to_owned())]
        );
    }

    #[test]
    fn reads_a_nested_tag() {
        assert_eq!(pieces("#книги/фантастика"), vec![Piece::Tag("книги/фантастика".to_owned())]);
    }

    #[test]
    fn skips_line_comments() {
        assert_eq!(
            pieces("// пояснение\nrating"),
            vec![Piece::Word("rating".to_owned())]
        );
    }

    #[test]
    fn cyrillic_field_names_are_single_words() {
        assert_eq!(pieces("Путь"), vec![Piece::Word("Путь".to_owned())]);
    }

    #[test]
    fn keeps_source_bounds_for_column_titles() {
        let tokens = tokenize("file.ctime DESC").expect("разбор удался");
        assert_eq!(tokens[0].from, 0);
        assert_eq!(tokens[2].to, "file.ctime".len());
    }

    #[test]
    fn refuses_an_unclosed_link_or_string() {
        assert!(tokenize("[[Заметка").is_err());
        assert!(tokenize("\"текст").is_err());
        assert!(tokenize("#").is_err());
    }
}
