// Isolated source-equivalent syntax-highlighting test harness. CI compiles
// actual tokens.rs and langs.rs from one pinned Akula commit into this wrapper.
use std::{hint::black_box, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Property,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust, Script, Python, Go, CLike, Shell, Sql, Css, Json,
    Yaml, Markup, Lua, Dataview, Math,
}

mod highlight {
    pub(super) use super::{Lang, Token};
    mod langs {
        include!(env!("AKULA_HIGHLIGHT_LANGS"));
    }
    mod tokens {
        include!(env!("AKULA_HIGHLIGHT_TOKENS"));
    }
    pub use tokens::tokens;
}

fn fixtures(kind: &str) -> (Lang, Vec<String>) {
    let (language, rows): (Lang, [&str; 5]) = match kind {
        "sql" => (Lang::Sql, [
            "SELECT user_id, Name, rating FROM documents WHERE rating >= 10 ORDER BY Name DESC LIMIT 50",
            "select col1, col2 from tbl left join ref on tbl.id = ref.id -- комментарий",
            "create table somethings (name text, NUMBER int, comment varchar(255));",
            "thisIsNotAKeyword SELECTED ordering /* комментарий */ SELECT 'where -- quoted';",
            "select 'abc // quote', κόσμος, İ and Key from tbl where tag = 'значение';",
        ]),
        "dataview" => (Lang::Dataview, [
            "TABLE WITHOUT ID link(file.link, title) AS Name WHERE stage = \"active\" SORT rating DESC",
            "list without id file.name WHERE length(filter(rows.file.ctime, (t) => t >= date(today))) > 3",
            "GROUP BY file.folder FLATTEN rows as item WHERE item.Название != \"\"",
            "from \"Книги\" where contains(file.tags, \"фантастика\") sort file.mtime desc",
            "Table Title, today, NOW, SORTED, группировка /* not a comment for dataview */",
        ]),
        "rust" => (Lang::Rust, [
            "pub(crate) fn result<T: Clone>(v: T) -> Option<T> { Some(v) }",
            "let greeting = \"Привет 🌍\"; println!(\"{greeting}\");",
            "let result = items.iter().map(|x| x.id).collect::<Vec<_>>(); // note",
            "for index in 0..20 { if index > 0 { continue; } }",
            "const MAX: usize = 2048; let a = self.foo.bar(1,2,3);",
        ]),
        "markup" => (Lang::Markup, [
            "<div class=\"small\"><input type=\"button\" disabled></div>",
            "<!-- TODO: русский комментарий --> <article aria-label=\"hello\">",
            "<a href='https://example.com/test?q=1'>title</a>",
            "<meta name=\"viewport\" content=\"width=device-width\">",
            "<svg viewBox=\"0 0 100 100\"><path d=\"M 0 0 L 20 20\"/></svg>",
        ]),
        "python" => (Lang::Python, [
            "def hello(name): return name.strip().upper()",
            "for i in range(100): print(i)  # comment",
            "print('quoted # not comment'); value = 3.14159",
            "if λ > 0: raise ValueError(\"пример\")",
            "items = {k: len(v) for k, v in mapping.items()}",
        ]),
        _ => unreachable!(),
    };
    let mut lines = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        lines.push(format!("{} {}", rows[i % rows.len()], i % 100));
    }
    (language, lines)
}

fn bench(kind: &str) {
    let (lang, lines) = fixtures(kind);
    let mut checksum = 0xCBF2_9CE4_8422_2325u64;
    let start = Instant::now();
    for _ in 0..4 {
        for line in &lines {
            let output = black_box(highlight::tokens(line, lang));
            checksum = checksum.wrapping_mul(0x1000_0000_01B3).wrapping_add(output.len() as u64);
            for (range, token) in output {
                checksum ^= ((range.start as u64) << 17)
                    ^ ((range.end as u64) << 5)
                    ^ (token as u64);
                checksum = checksum.wrapping_mul(0x1000_0000_01B3);
            }
        }
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    black_box(checksum);
    println!("HIGHLIGHT_AB case={kind} elapsed_ms={elapsed_ms:.6} checksum={checksum}");
}

fn main() {
    for case in ["sql", "dataview", "rust", "markup", "python"] {
        bench(case);
    }
}
