use super::super::ast::{BinaryOp, Expr, UnaryOp};
use super::super::lexer::{Piece, Symbol};
use super::super::value::{Link, Value};
use super::Cursor;

const KEYWORDS: [&str; 10] = [
    "from", "where", "sort", "limit", "as", "asc", "desc", "group", "by", "flatten",
];

pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|keyword| word.eq_ignore_ascii_case(keyword))
}

pub fn expression(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    or_level(cursor)
}

// All four repeating precedence levels fold left. Comparison deliberately
// stays separate: its grammar accepts only a single comparison operator.
fn left_chain(
    cursor: &mut Cursor<'_>,
    operand: impl Fn(&mut Cursor<'_>) -> Result<Expr, String>,
    operators: &[(Symbol, Option<&str>, BinaryOp)],
) -> Result<Expr, String> {
    let mut left = operand(cursor)?;
    while let Some(operator) = operators.iter().find_map(|&(symbol, word, op)| {
        (cursor.take_symbol(symbol) || word.is_some_and(|w| cursor.take_word(w))).then_some(op)
    }) {
        left = binary(operator, left, operand(cursor)?);
    }
    Ok(left)
}

fn or_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    left_chain(cursor, and_level, &[(Symbol::PipePipe, Some("or"), BinaryOp::Or)])
}

fn and_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    left_chain(cursor, compare_level, &[(Symbol::AmpAmp, Some("and"), BinaryOp::And)])
}

fn compare_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    let left = additive_level(cursor)?;
    let operator = if cursor.take_symbol(Symbol::Equal) {
        BinaryOp::Equal
    } else if cursor.take_symbol(Symbol::NotEqual) {
        BinaryOp::NotEqual
    } else if cursor.take_symbol(Symbol::LessOrEqual) {
        BinaryOp::LessOrEqual
    } else if cursor.take_symbol(Symbol::GreaterOrEqual) {
        BinaryOp::GreaterOrEqual
    } else if cursor.take_symbol(Symbol::Less) {
        BinaryOp::Less
    } else if cursor.take_symbol(Symbol::Greater) {
        BinaryOp::Greater
    } else {
        return Ok(left);
    };
    Ok(binary(operator, left, additive_level(cursor)?))
}

fn additive_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    left_chain(cursor, multiplicative_level, &[
        (Symbol::Plus, None, BinaryOp::Add),
        (Symbol::Minus, None, BinaryOp::Subtract),
    ])
}

fn multiplicative_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    left_chain(cursor, unary_level, &[
        (Symbol::Star, None, BinaryOp::Multiply),
        (Symbol::Slash, None, BinaryOp::Divide),
        (Symbol::Percent, None, BinaryOp::Modulo),
    ])
}

fn unary_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    if cursor.take_symbol(Symbol::Bang) || cursor.take_word("not") {
        return Ok(unary(UnaryOp::Not, unary_level(cursor)?));
    }
    if cursor.take_symbol(Symbol::Minus) {
        return Ok(unary(UnaryOp::Negate, unary_level(cursor)?));
    }
    postfix_level(cursor)
}

fn postfix_level(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    let mut base = primary(cursor)?;
    loop {
        if cursor.take_symbol(Symbol::Dot) {
            let name = match cursor.advance().map(|token| &token.piece) {
                Some(Piece::Word(name)) => name.clone(),
                _ => return Err("После точки ожидается имя поля".to_owned()),
            };
            base = Expr::Property {
                base: Box::new(base),
                name,
            };
            continue;
        }
        if cursor.take_symbol(Symbol::OpenBracket) {
            let index = expression(cursor)?;
            cursor.expect_symbol(Symbol::CloseBracket, "закрывающая «]»")?;
            base = Expr::Index {
                base: Box::new(base),
                index: Box::new(index),
            };
            continue;
        }
        return Ok(base);
    }
}

// Parse a nonempty comma-separated sequence; callers decide when empty is legal.
pub(super) fn comma_separated<T>(
    cursor: &mut Cursor<'_>, mut item: impl FnMut(&mut Cursor<'_>) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let mut items = vec![item(cursor)?];
    while cursor.take_symbol(Symbol::Comma) {
        items.push(item(cursor)?);
    }
    Ok(items)
}

fn delimited_expressions(cursor: &mut Cursor<'_>, end: Symbol, expected: &str) -> Result<Vec<Expr>, String> {
    let items = if cursor.at_symbol(end) { Vec::new() } else { comma_separated(cursor, expression)? };
    cursor.expect_symbol(end, expected)?;
    Ok(items)
}

fn primary(cursor: &mut Cursor<'_>) -> Result<Expr, String> {
    if let Some(lambda) = lambda(cursor)? {
        return Ok(lambda);
    }
    if cursor.take_symbol(Symbol::OpenParen) {
        let inner = expression(cursor)?;
        cursor.expect_symbol(Symbol::CloseParen, "закрывающая «)»")?;
        return Ok(inner);
    }
    if cursor.take_symbol(Symbol::OpenBracket) {
        let items = delimited_expressions(cursor, Symbol::CloseBracket, "закрывающая «]»")?;
        return Ok(Expr::Call {
            name: "list".to_owned(),
            arguments: items,
        });
    }

    let Some(token) = cursor.advance() else {
        return Err("Запрос кончился там, где ожидается значение".to_owned());
    };
    match &token.piece {
        Piece::Number(value) => Ok(Expr::Literal(Value::Number(*value))),
        Piece::Text(value) => Ok(Expr::Literal(Value::Text(value.clone()))),
        Piece::Tag(value) => Ok(Expr::Literal(Value::Text(value.clone()))),
        Piece::WikiLink(target) => Ok(Expr::Literal(Value::Link(Link {
            target: target.clone(),
            display: None,
        }))),
        Piece::Word(word) => {
            if word.eq_ignore_ascii_case("true") {
                return Ok(Expr::Literal(Value::Bool(true)));
            }
            if word.eq_ignore_ascii_case("false") {
                return Ok(Expr::Literal(Value::Bool(false)));
            }
            if word.eq_ignore_ascii_case("null") {
                return Ok(Expr::Literal(Value::Null));
            }
            if word.eq_ignore_ascii_case("dur") && cursor.at_symbol(Symbol::OpenParen) {
                return Ok(Expr::Call {
                    name: "dur".to_owned(),
                    arguments: vec![Expr::Literal(Value::Text(raw_call_text(cursor)?))],
                });
            }
            if cursor.take_symbol(Symbol::OpenParen) {
                let arguments = delimited_expressions(cursor, Symbol::CloseParen, "закрывающая «)»")?;
                return Ok(Expr::Call {
                    name: word.to_lowercase(),
                    arguments,
                });
            }
            Ok(Expr::Variable(word.clone()))
        }
        Piece::Symbol(_) => Err(format!(
            "Здесь ожидается значение, а не «{}»",
            cursor.text_of(token)
        )),
    }
}

fn lambda(cursor: &mut Cursor<'_>) -> Result<Option<Expr>, String> {
    let start = cursor.mark();
    let mut parameters = Vec::new();

    if cursor.take_symbol(Symbol::OpenParen) {
        if !cursor.at_symbol(Symbol::CloseParen) {
            loop {
                match cursor.word() {
                    Some(name) => {
                        parameters.push(name.to_owned());
                        cursor.advance();
                    }
                    None => {
                        cursor.rewind(start);
                        return Ok(None);
                    }
                }
                if !cursor.take_symbol(Symbol::Comma) {
                    break;
                }
            }
        }
        if !cursor.take_symbol(Symbol::CloseParen) || !cursor.take_symbol(Symbol::Arrow) {
            cursor.rewind(start);
            return Ok(None);
        }
    } else {
        match cursor.word() {
            Some(name) => {
                parameters.push(name.to_owned());
                cursor.advance();
                if !cursor.take_symbol(Symbol::Arrow) {
                    cursor.rewind(start);
                    return Ok(None);
                }
            }
            None => return Ok(None),
        }
    }

    Ok(Some(Expr::Lambda {
        parameters,
        body: Box::new(expression(cursor)?),
    }))
}

fn raw_call_text(cursor: &mut Cursor<'_>) -> Result<String, String> {
    cursor.expect_symbol(Symbol::OpenParen, "«(» после «dur»")?;
    let from = cursor.offset();
    let mut depth = 1usize;
    loop {
        if cursor.at_symbol(Symbol::OpenParen) {
            depth += 1;
        } else if cursor.at_symbol(Symbol::CloseParen) {
            depth -= 1;
            if depth == 0 {
                let to = cursor.previous_end().max(from);
                let text = cursor.slice(from, to).to_owned();
                cursor.advance();
                return Ok(text.trim_matches('"').to_owned());
            }
        }
        if cursor.advance().is_none() {
            return Err("Ожидается закрывающая «)» после «dur»".to_owned());
        }
    }
}

fn binary(operator: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        operator,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn unary(operator: UnaryOp, operand: Expr) -> Expr {
    Expr::Unary {
        operator,
        operand: Box::new(operand),
    }
}
