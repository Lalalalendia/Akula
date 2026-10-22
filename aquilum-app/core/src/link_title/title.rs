use super::entities;

pub const CLOSING_TAG: &str = "</title>";

// Scan only ASCII '<' positions, without making a lowercased copy of HTML.
fn ascii_tag(text: &str, tag: &[u8]) -> Option<usize> {
    text.match_indices('<').find_map(|(pos, _)| {
        text.as_bytes()
            .get(pos..pos + tag.len())
            .and_then(|part| part.eq_ignore_ascii_case(tag).then_some(pos))
    })
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

pub fn parse(html: &str) -> Option<String> {
    let start = ascii_tag(html, b"<title")?;
    let after_open = start + "<title".len();
    let gt = html[after_open..].find('>')?;
    let content_start = after_open + gt + 1;
    let close_rel = ascii_tag(&html[content_start..], CLOSING_TAG.as_bytes())?;
    let raw = html[content_start..content_start + close_rel].trim();
    if raw.is_empty() {
        return None;
    }
    let decoded = entities::decode(&collapse_ws(raw));
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests;
