//! Reject ambiguous HTTP framing before parsing any request body.
use super::{refused, Failure, Head};

pub(super) fn validate(head: &Head) -> Result<(), Failure> {
    let transfer = head.header("transfer-encoding").is_some();
    let length = head.header("content-length").is_some();
    if head.header_count("transfer-encoding") > 1
        || head.header_count("content-length") > 1
        || (transfer && length)
    {
        return Err(refused(400, "Неоднозначное HTTP-обрамление запроса"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::request::Body;

    fn parsed(raw: &[u8]) -> Head {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);
        assert!(matches!(request.parse(raw), Ok(httparse::Status::Complete(_))));
        Head::from_parsed(&request)
    }

    #[test]
    fn rejects_all_duplicate_and_conflicting_body_lengths() {
        for request in [
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n",
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\ncontent-length: 3\r\n\r\n",
            "POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n",
            "POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\ntransfer-encoding: chunked\r\n\r\n",
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2, 2\r\n\r\n",
        ] {
            assert!(parsed(request.as_bytes()).body().is_err(), "{request}");
        }
    }

    #[test]
    fn accepts_unambiguous_sized_chunked_and_empty_requests() {
        assert!(matches!(parsed(b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n").body(), Ok(Body::Sized(2))));
        assert!(matches!(parsed(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").body(), Ok(Body::Chunked)));
        assert!(matches!(parsed(b"POST / HTTP/1.1\r\n\r\n").body(), Ok(Body::Sized(0))));
    }
}
