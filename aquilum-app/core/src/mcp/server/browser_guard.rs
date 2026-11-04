//! Browser-origin gate for the token-authenticated loopback MCP endpoint.
use super::{is_loopback_origin, refused, Failure, Head};

pub(super) fn validate(head: &Head) -> Result<(), Failure> {
    if head.header_count("origin") > 1 || head.header_count("sec-fetch-site") > 1 {
        return Err(refused(400, "Дублирующиеся браузерные заголовки запрещены"));
    }
    if head.header("origin").is_some_and(|value| !is_loopback_origin(value)) {
        return Err(refused(403, "Источник запроса запрещён"));
    }
    if head.header("sec-fetch-site").is_some_and(|value| {
        !["none", "same-origin", "same-site"]
            .iter().any(|allowed| value.trim().eq_ignore_ascii_case(allowed))
    }) {
        return Err(refused(403, "Межсайтовый запрос запрещён"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(extra: &str) -> Head {
        let raw = format!("POST /mcp HTTP/1.1\r\nHost: localhost:1420\r\n{extra}\r\n");
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);
        assert!(matches!(request.parse(raw.as_bytes()), Ok(httparse::Status::Complete(_))));
        Head::from_parsed(&request)
    }

    #[test]
    fn denies_duplicate_origins_and_missing_origin_cross_site_fetches() {
        for headers in [
            "Origin: http://localhost\r\nOrigin: http://localhost\r\n",
            "Origin: http://localhost\r\nOrigin: https://evil.example\r\n",
            "Origin: null\r\n",
            "Sec-Fetch-Site: cross-site\r\n",
            "Origin: http://localhost\r\nSec-Fetch-Site: cross-site\r\n",
            "Sec-Fetch-Site: none\r\nSec-Fetch-Site: cross-site\r\n",
            "Sec-Fetch-Site: invalid\r\n",
        ] {
            assert!(validate(&parsed(headers)).is_err(), "accepted {headers:?}");
        }
    }

    #[test]
    fn permits_native_requests_and_explicit_local_browser_metadata() {
        for headers in [
            "",
            "Origin: http://localhost:1420\r\nSec-Fetch-Site: same-origin\r\n",
            "Origin: http://127.0.0.1:1420\r\nSec-Fetch-Site: same-site\r\n",
            "Sec-Fetch-Site: none\r\n",
        ] {
            assert!(validate(&parsed(headers)).is_ok(), "rejected {headers:?}");
        }
    }
}
