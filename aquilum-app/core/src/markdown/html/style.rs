use super::tokens::{attr, prop};
use super::writer::{BOLD, CODE, ITALIC, STRIKE};

pub fn language(attrs: &str) -> Option<String> {
    let class = attr(attrs, "class")?;
    class.split_whitespace().find_map(|c| c.strip_prefix("language-").or_else(|| c.strip_prefix("lang-"))).map(str::to_owned)
}

pub fn marks_of(parent: u8, name: &str, css: &str, heading: bool) -> u8 {
    let mut marks = parent
        | match name {
            "b" | "strong" => BOLD,
            "i" | "em" | "cite" | "dfn" | "var" => ITALIC,
            "s" | "strike" | "del" => STRIKE,
            "code" | "kbd" | "samp" | "tt" => CODE,
            _ => 0,
        };
    let weight = prop(css, "font-weight").and_then(|w| match w.as_str() {
        "bold" | "bolder" => Some(true),
        "normal" | "lighter" => Some(false),
        number => number.parse::<u32>().ok().map(|n| n >= 600),
    });
    let slant = prop(css, "font-style").and_then(|s| match s.as_str() {
        "italic" | "oblique" => Some(true),
        "normal" => Some(false),
        _ => None,
    });
    let line = prop(css, "text-decoration-line").or_else(|| prop(css, "text-decoration")).map(|l| l.contains("line-through"));
    for (flag, on) in [(BOLD, weight), (ITALIC, slant), (STRIKE, line)] {
        match on {
            Some(true) => marks |= flag,
            Some(false) => marks &= !flag,
            None => {}
        }
    }
    if heading {
        marks &= !BOLD;
    }
    marks
}


fn decode_query_component(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            out.push(hex(*bytes.get(i + 1)?)? * 16 + hex(*bytes.get(i + 2)?)?);
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn link_url(href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    if !["http://", "https://", "mailto:", "tel:"].iter().any(|scheme| lower.starts_with(scheme)) {
        return None;
    }
    // In default local Core, avoid making an optional network crate mandatory.
    // Unwrap only the exact Google redirect and accept web URLs as targets.
    let query = href.strip_prefix("https://www.google.com/url?")
        .or_else(|| href.strip_prefix("http://www.google.com/url?"));
    let target = query.and_then(|query| query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "q").then(|| decode_query_component(value)).flatten()
    }));
    Some(target.filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .unwrap_or_else(|| href.to_owned()))
}
