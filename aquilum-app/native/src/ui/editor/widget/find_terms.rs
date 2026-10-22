/// Ctrl+F supports literal alternatives separated by ||; \|| matches literal pipes.
pub(super) fn find_terms(query: &str) -> Vec<String> {
    let mut chars = query.chars().peekable();
    let (mut parts, mut part, mut split) = (Vec::new(), String::new(), false);
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if chars.peek() == Some(&'\\') {
                chars.next();
                part.push('\\');
                continue;
            }
            if chars.peek() == Some(&'|') {
                chars.next();
                if chars.peek() == Some(&'|') {
                    chars.next();
                    part.push_str("||");
                    continue;
                }
                part.push('\\');
                part.push('|');
                continue;
            }
        }
        if ch == '|' && chars.peek() == Some(&'|') {
            chars.next();
            parts.push(std::mem::take(&mut part));
            split = true;
        } else {
            part.push(ch);
        }
    }
    parts.push(part);
    if split {
        parts.into_iter().map(|part| part.trim().to_owned()).filter(|part| !part.is_empty()).collect()
    } else {
        parts.into_iter().filter(|part| !part.is_empty()).collect()
    }
}

