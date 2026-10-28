use super::{title, url};
use super::ssrf::public_title_agent;
use std::io::Read;

const MAX_HEAD_BYTES: usize = 128 * 1024;
const CHUNK_BYTES: usize = 8 * 1024;

fn read_until_title(mut reader: impl Read) -> String {
    let mut buffer = Vec::with_capacity(CHUNK_BYTES);
    let mut chunk = [0u8; CHUNK_BYTES];
    while buffer.len() < MAX_HEAD_BYTES {
        let filled = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        buffer.extend_from_slice(&chunk[..filled]);
        if String::from_utf8_lossy(&buffer)
            .to_ascii_lowercase()
            .contains(title::CLOSING_TAG)
        {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

fn serves_html(content_type: &str) -> bool {
    let lowered = content_type.to_ascii_lowercase();
    lowered.is_empty() || lowered.contains("text/html") || lowered.contains("application/xhtml")
}

pub fn page_title(raw_url: &str) -> String {
    let address = match url::normalize(raw_url) {
        Ok(address) => address,
        Err(_) => return raw_url.trim().to_string(),
    };
    let fallback = url::hostname_fallback(&address);

    let response = match public_title_agent().get(&address).call() {
        Ok(response) => response,
        Err(_) => return fallback,
    };
    if !serves_html(response.header("content-type").unwrap_or("")) {
        return fallback;
    }

    title::parse(&read_until_title(response.into_reader())).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::{read_until_title, serves_html, CHUNK_BYTES, MAX_HEAD_BYTES};
    use crate::link_title::title;
    use std::io::Cursor;

    fn page(head: &str, filler_bytes: usize) -> Cursor<Vec<u8>> {
        let mut html = String::from(head);
        html.push_str(&"x".repeat(filler_bytes));
        Cursor::new(html.into_bytes())
    }

    #[test]
    fn stops_reading_at_the_closing_title_instead_of_the_whole_page() {
        let source = page("<html><head><title>Коротко</title></head><body>", 4 * 1024 * 1024);

        let head = read_until_title(source);

        assert!(head.len() <= CHUNK_BYTES, "прочитано {} байт", head.len());
        assert_eq!(title::parse(&head).as_deref(), Some("Коротко"));
    }

    #[test]
    fn a_page_without_a_title_never_reads_past_the_cap() {
        let source = page("", 4 * 1024 * 1024);

        let head = read_until_title(source);

        assert!(head.len() <= MAX_HEAD_BYTES);
        assert!(title::parse(&head).is_none());
    }

    #[test]
    fn a_title_split_across_two_chunks_still_arrives_whole() {
        let mut html = String::from("<html><head>");
        html.push_str(&"<!-- отступ -->".repeat(CHUNK_BYTES / 10));
        html.push_str("<title>Через границу чанка</title></head>");
        let source = Cursor::new(html.into_bytes());

        let head = read_until_title(source);

        assert_eq!(title::parse(&head).as_deref(), Some("Через границу чанка"));
    }

    #[test]
    fn a_response_that_is_not_html_is_not_worth_parsing() {
        assert!(serves_html(""));
        assert!(serves_html("text/html; charset=utf-8"));
        assert!(!serves_html("application/pdf"));
        assert!(!serves_html("image/png"));
    }
}
