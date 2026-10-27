//! Fast lexing of quoted literals and two-byte operators.
//! The general escape parser remains the fallback for quoted strings.

use super::Symbol;

pub(super) fn read_text(rest: &str) -> Result<(String, usize), String> {
    let inside = &rest[1..];
    let Some(end) = inside.find(['"', '\\']) else {
        return Err("Строка не закрыта кавычкой".to_owned());
    };
    if inside.as_bytes()[end] == b'"' {
        // No escape before this quote: return the original UTF-8 byte length.
        return Ok((inside[..end].to_owned(), end + 2));
    }

    // Keep the prefix already scanned by find(): starting again at inside[0]
    // regresses escaped strings by making two passes through that prefix.
    let mut value = inside[..end].to_owned();
    let mut length = end + 1;
    let mut characters = inside[end..].chars();
    while let Some(symbol) = characters.next() {
        length += symbol.len_utf8();
        match symbol {
            '"' => return Ok((value, length)),
            '\\' => match characters.next() {
                Some(escaped) => {
                    length += escaped.len_utf8();
                    value.push(escaped);
                }
                None => break,
            },
            _ => value.push(symbol),
        }
    }
    Err("Строка не закрыта кавычкой".to_owned())
}

pub(super) fn read_symbol(rest: &str) -> Option<(Symbol, usize)> {
    let bytes = rest.as_bytes();
    let double = match bytes {
        [b'=', b'>', ..] => Some(Symbol::Arrow),
        [b'!', b'=', ..] => Some(Symbol::NotEqual),
        [b'>', b'=', ..] => Some(Symbol::GreaterOrEqual),
        [b'<', b'=', ..] => Some(Symbol::LessOrEqual),
        [b'&', b'&', ..] => Some(Symbol::AmpAmp),
        [b'|', b'|', ..] => Some(Symbol::PipePipe),
        _ => None,
    };
    if let Some(symbol) = double {
        return Some((symbol, 2));
    }
    let single = match bytes.first()? {
        b'(' => Symbol::OpenParen,
        b')' => Symbol::CloseParen,
        b'[' => Symbol::OpenBracket,
        b']' => Symbol::CloseBracket,
        b',' => Symbol::Comma,
        b'.' => Symbol::Dot,
        b'+' => Symbol::Plus,
        b'-' => Symbol::Minus,
        b'*' => Symbol::Star,
        b'/' => Symbol::Slash,
        b'%' => Symbol::Percent,
        b'=' => Symbol::Equal,
        b'<' => Symbol::Less,
        b'>' => Symbol::Greater,
        b'!' => Symbol::Bang,
        b'&' => Symbol::AmpAmp,
        b'|' => Symbol::PipePipe,
        _ => return None,
    };
    Some((single, 1))
}

#[cfg(test)]
mod tests;
