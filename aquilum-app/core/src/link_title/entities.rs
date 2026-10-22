fn named(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        "laquo" => Some('«'),
        "raquo" => Some('»'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "lsquo" => Some('‘'),
        "rsquo" => Some('’'),
        _ => None,
    }
}

fn numeric(body: &str) -> Option<char> {
    let code = match body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => body.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

fn resolve(body: &str) -> Option<char> {
    if body.is_empty() || body.len() > 8 {
        return None;
    }
    match body.strip_prefix('#') {
        Some(number) => numeric(number),
        None => {
            if !body.is_ascii() {
                return None;
            }
            let mut lower = [0u8; 8];
            for (dst, byte) in lower.iter_mut().zip(body.bytes()) {
                *dst = byte.to_ascii_lowercase();
            }
            named(std::str::from_utf8(&lower[..body.len()]).ok()?)
        },
    }
}

pub fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        match tail
            .find(';')
            .and_then(|end| resolve(&tail[..end]).map(|character| (character, end)))
        {
            Some((character, end)) => {
                out.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
