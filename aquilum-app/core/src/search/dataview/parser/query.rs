use super::super::ast::{Clause, Column, Direction, Expr, Query, Shape, SortKey, Source};
use super::super::duration;
use super::super::lexer::{Piece, Symbol};
use super::expr::{comma_separated, expression, is_keyword, left_chain};
use super::Cursor;

pub fn query(cursor: &mut Cursor<'_>) -> Result<Query, String> {
    let shape = shape(cursor)?;
    let mut source = None;
    let mut title = None;
    let mut refresh = None;
    let mut clauses = Vec::new();

    loop {
        if at_title(cursor) {
            if title.is_some() {
                return Err("«TITLE» указан дважды".to_owned());
            }
            title = Some(title_text(cursor)?);
        } else if at_every(cursor) {
            if refresh.is_some() {
                return Err("«EVERY» указан дважды".to_owned());
            }
            refresh = Some(every_seconds(cursor)?);
        } else if cursor.take_word("from") {
            if source.is_some() {
                return Err("«FROM» указан дважды".to_owned());
            }
            source = Some(source_expression(cursor)?);
        } else if cursor.take_word("where") {
            clauses.push(Clause::Where(expression(cursor)?));
        } else if cursor.take_word("sort") {
            clauses.push(Clause::Sort(sort_keys(cursor)?));
        } else if cursor.take_word("limit") {
            clauses.push(Clause::Limit(limit(cursor)?));
        } else if cursor.take_word("group") {
            if !cursor.take_word("by") {
                return Err("После «GROUP» ожидается «BY»".to_owned());
            }
            let (value, name) = named_expression(cursor, "key")?;
            clauses.push(Clause::GroupBy { value, name });
        } else if cursor.take_word("flatten") {
            let (value, name) = named_expression(cursor, "flat")?;
            clauses.push(Clause::Flatten { value, name });
        } else {
            break;
        }
    }

    Ok(Query {
        shape,
        source,
        clauses,
        title,
        refresh,
    })
}

const MIN_REFRESH_SECONDS: u32 = 1;

fn at_every(cursor: &Cursor<'_>) -> bool {
    cursor.at_word("every")
        && matches!(cursor.peek_after().map(|token| &token.piece), Some(Piece::Number(_)))
}

fn every_seconds(cursor: &mut Cursor<'_>) -> Result<u32, String> {
    cursor.advance();
    let from = cursor.offset();
    cursor.advance();
    if cursor.word().is_none() {
        return Err("После «EVERY» ожидается промежуток вроде «30s» или «5 minutes»".to_owned());
    }
    cursor.advance();
    let text = cursor.slice(from, cursor.previous_end());
    let Some(length) = duration::parse(text) else {
        return Err(format!("Не похоже на промежуток: «{text}»"));
    };
    let seconds = length.approximate_nanos() / 1_000_000_000;
    if seconds < i64::from(MIN_REFRESH_SECONDS) {
        return Err("«EVERY» не бывает чаще секунды".to_owned());
    }
    Ok(seconds.min(i64::from(u32::MAX)) as u32)
}

fn at_title(cursor: &Cursor<'_>) -> bool {
    cursor.at_word("title")
        && matches!(cursor.peek_after().map(|token| &token.piece), Some(Piece::Text(_)))
}

fn title_text(cursor: &mut Cursor<'_>) -> Result<String, String> {
    cursor.advance();
    match cursor.advance().map(|token| &token.piece) {
        Some(Piece::Text(name)) => Ok(name.clone()),
        _ => Err("После «TITLE» ожидается название в кавычках".to_owned()),
    }
}

fn shape(cursor: &mut Cursor<'_>) -> Result<Shape, String> {
    if cursor.take_word("table") {
        let show_path = !without_id(cursor)?;
        return Ok(Shape::Table {
            columns: columns(cursor)?,
            show_path,
        });
    }
    if cursor.take_word("task") {
        let show_path = !without_id(cursor)?;
        return Ok(Shape::Tasks { show_path });
    }
    if cursor.take_word("list") {
        let show_path = !without_id(cursor)?;
        let value = if at_clause(cursor) {
            None
        } else {
            Some(expression(cursor)?)
        };
        return Ok(Shape::List { value, show_path });
    }
    Err("Запрос начинается словом «TABLE», «LIST» или «TASK»".to_owned())
}

fn without_id(cursor: &mut Cursor<'_>) -> Result<bool, String> {
    if !cursor.take_word("without") {
        return Ok(false);
    }
    if !cursor.take_word("id") {
        return Err("После «WITHOUT» ожидается «ID»".to_owned());
    }
    Ok(true)
}

fn columns(cursor: &mut Cursor<'_>) -> Result<Vec<Column>, String> {
    if at_clause(cursor) {
        return Ok(Vec::new());
    }
    comma_separated(cursor, column)
}

fn column(cursor: &mut Cursor<'_>) -> Result<Column, String> {
    let from = cursor.offset();
    let value = expression(cursor)?;
    let to = cursor.previous_end();
    let title = if cursor.take_word("as") {
        match cursor.advance().map(|token| &token.piece) {
            Some(Piece::Text(name)) | Some(Piece::Word(name)) => name.clone(),
            _ => return Err("После «AS» ожидается название колонки".to_owned()),
        }
    } else {
        cursor.slice(from, to).to_owned()
    };
    Ok(Column { title, value })
}

fn named_expression(
    cursor: &mut Cursor<'_>,
    fallback: &str,
) -> Result<(Expr, String), String> {
    let from = cursor.offset();
    let value = expression(cursor)?;
    let to = cursor.previous_end();
    if cursor.take_word("as") {
        return match cursor.advance().map(|token| &token.piece) {
            Some(Piece::Text(name)) | Some(Piece::Word(name)) => Ok((value, name.clone())),
            _ => Err("После «AS» ожидается имя".to_owned()),
        };
    }
    let written = cursor.slice(from, to);
    let name = if written.is_empty() || !written.chars().all(is_name_symbol) {
        fallback.to_owned()
    } else {
        written.to_owned()
    };
    Ok((value, name))
}

fn is_name_symbol(symbol: char) -> bool {
    symbol.is_alphanumeric() || symbol == '_' || symbol == '-'
}

fn sort_keys(cursor: &mut Cursor<'_>) -> Result<Vec<SortKey>, String> {
    comma_separated(cursor, |cursor| {
        let value = expression(cursor)?;
        let direction = if cursor.take_word("desc") {
            Direction::Descending
        } else {
            cursor.take_word("asc");
            Direction::Ascending
        };
        Ok(SortKey { value, direction })
    })
}

fn limit(cursor: &mut Cursor<'_>) -> Result<usize, String> {
    match cursor.advance().map(|token| &token.piece) {
        Some(Piece::Number(value)) if *value >= 0.0 => Ok(*value as usize),
        _ => Err("После «LIMIT» ожидается неотрицательное число".to_owned()),
    }
}

fn at_clause(cursor: &Cursor<'_>) -> bool {
    cursor.peek().is_none()
        || cursor.word().is_some_and(is_keyword)
        || at_title(cursor)
        || at_every(cursor)
}

fn source_binary(op: fn(Box<Source>, Box<Source>) -> Source, left: Source, right: Source) -> Source {
    op(Box::new(left), Box::new(right))
}

fn source_expression(cursor: &mut Cursor<'_>) -> Result<Source, String> {
    left_chain(cursor, source_and, &[(Symbol::PipePipe, Some("or"), Source::Or as fn(_, _) -> _)], source_binary)
}

fn source_and(cursor: &mut Cursor<'_>) -> Result<Source, String> {
    left_chain(cursor, source_atom, &[(Symbol::AmpAmp, Some("and"), Source::And as fn(_, _) -> _)], source_binary)
}

fn source_atom(cursor: &mut Cursor<'_>) -> Result<Source, String> {
    if cursor.take_symbol(Symbol::Bang)
        || cursor.take_symbol(Symbol::Minus)
        || cursor.take_word("not")
    {
        return Ok(Source::Not(Box::new(source_atom(cursor)?)));
    }
    if cursor.take_symbol(Symbol::OpenParen) {
        let inner = source_expression(cursor)?;
        cursor.expect_symbol(Symbol::CloseParen, "закрывающая «)»")?;
        return Ok(inner);
    }
    if cursor.take_word("once") {
        return Ok(Source::Once);
    }
    if cursor.take_word("outgoing") {
        cursor.expect_symbol(Symbol::OpenParen, "«(» после «outgoing»")?;
        let target = wiki_target(cursor)?;
        cursor.expect_symbol(Symbol::CloseParen, "закрывающая «)»")?;
        return Ok(Source::LinksFrom(target));
    }
    match cursor.advance().map(|token| &token.piece) {
        Some(Piece::Text(folder)) => Ok(Source::Folder(folder.clone())),
        Some(Piece::Tag(tag)) => Ok(Source::Tag(tag.clone())),
        Some(Piece::WikiLink(target)) => Ok(Source::LinksTo(target.clone())),
        _ => Err("После «FROM» ожидается папка в кавычках, #тег или [[ссылка]]".to_owned()),
    }
}

fn wiki_target(cursor: &mut Cursor<'_>) -> Result<String, String> {
    match cursor.advance().map(|token| &token.piece) {
        Some(Piece::WikiLink(target)) => Ok(target.clone()),
        _ => Err("Ожидается «[[ссылка на заметку]]»".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::super::super::ast::{Clause, Direction, Expr, Shape, Source};

    const BACKLINKS: &str = r#"TABLE WITHOUT ID
link(file.link, title) AS "Ссылающиеся заметки"
FROM [[📖 5 пороков команды]]
SORT file.ctime DESC"#;

    #[test]
    fn reads_the_backlinks_query_whole() {
        let query = parse(BACKLINKS).expect("запрос разобран");

        let Shape::Table { columns, show_path } = &query.shape else {
            panic!("это таблица");
        };
        assert!(!show_path, "«WITHOUT ID» убирает колонку со ссылкой");
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].title, "Ссылающиеся заметки");
        assert!(matches!(&columns[0].value, Expr::Call { name, arguments }
            if name == "link" && arguments.len() == 2));

        assert_eq!(
            query.source,
            Some(Source::LinksTo("📖 5 пороков команды".to_owned()))
        );

        let [Clause::Sort(keys)] = query.clauses.as_slice() else {
            panic!("одна сортировка");
        };
        assert_eq!(keys[0].direction, Direction::Descending);
        assert!(matches!(&keys[0].value, Expr::Property { name, .. } if name == "ctime"));
    }

    #[test]
    fn a_column_without_as_is_named_by_its_own_source_text() {
        let query = parse("TABLE file.ctime, rating * 2 FROM \"Книги\"").expect("разобран");
        let Shape::Table { columns, show_path } = &query.shape else {
            panic!("это таблица");
        };
        assert!(show_path, "без «WITHOUT ID» первая колонка — ссылка на заметку");
        assert_eq!(columns[0].title, "file.ctime");
        assert_eq!(columns[1].title, "rating * 2");
    }

    #[test]
    fn a_bare_table_takes_no_columns_and_the_whole_vault() {
        let query = parse("TABLE").expect("разобран");
        assert!(matches!(&query.shape, Shape::Table { columns, .. } if columns.is_empty()));
        assert_eq!(query.source, None);
    }

    #[test]
    fn a_keyword_right_after_the_header_is_not_read_as_a_column() {
        let query = parse("TABLE FROM \"Книги\"").expect("разобран");
        assert!(matches!(&query.shape, Shape::Table { columns, .. } if columns.is_empty()));
        assert_eq!(query.source, Some(Source::Folder("Книги".to_owned())));
    }

    #[test]
    fn clauses_keep_the_order_they_were_written_in() {
        let query = parse("LIST FROM \"Книги\" WHERE rating > 3 SORT title LIMIT 10").expect("разобран");
        assert!(matches!(query.clauses[0], Clause::Where(_)));
        assert!(matches!(query.clauses[1], Clause::Sort(_)));
        assert!(matches!(query.clauses[2], Clause::Limit(10)));
    }

    #[test]
    fn reads_every_kind_of_source_and_combines_them() {
        assert_eq!(
            parse("LIST FROM #книги").unwrap().source,
            Some(Source::Tag("книги".to_owned()))
        );
        assert_eq!(
            parse("LIST FROM outgoing([[Пороки]])").unwrap().source,
            Some(Source::LinksFrom("Пороки".to_owned()))
        );
        let combined = parse("LIST FROM \"Книги\" and -#черновик").unwrap().source;
        assert!(matches!(combined, Some(Source::And(_, _))));
    }

    #[test]
    fn list_takes_an_optional_value() {
        assert!(matches!(
            parse("LIST").unwrap().shape,
            Shape::List { value: None, show_path: true }
        ));
        assert!(matches!(
            parse("LIST WITHOUT ID title").unwrap().shape,
            Shape::List { value: Some(_), show_path: false }
        ));
    }

    #[test]
    fn explains_what_it_expected_instead_of_failing_silently() {
        assert!(parse("SELECT * FROM x").unwrap_err().contains("TABLE"));
        assert!(parse("TABLE WITHOUT title").unwrap_err().contains("ID"));
        assert!(parse("LIST LIMIT много").unwrap_err().contains("число"));
        assert!(parse("LIST FROM Книги").unwrap_err().contains("кавычках"));
        assert!(parse("TABLE link(file.link").unwrap_err().contains(")"));
    }
}
