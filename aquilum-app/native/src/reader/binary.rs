use std::cell::{OnceCell, RefCell};

use base64::Engine;

/// Each FB2 payload changes from encoded text to decoded bytes on first use.
/// Failed decoding is cached too; neither representation grows on repeated reads.
pub(super) struct Binary {
    encoded: RefCell<Option<String>>,
    decoded: OnceCell<Option<Vec<u8>>>,
}

impl Binary {
    pub(super) fn new(encoded: String) -> Self {
        Self { encoded: RefCell::new(Some(encoded)), decoded: OnceCell::new() }
    }

    pub(super) fn bytes(&self) -> Option<&[u8]> {
        self.decoded.get_or_init(|| {
            let raw = self.encoded.borrow_mut().take()?;
            let engine = base64::engine::general_purpose::STANDARD;
            if raw.chars().any(char::is_whitespace) {
                // Keep the existing Unicode whitespace semantics, not only ASCII.
                let clean: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                engine.decode(clean).ok()
            } else {
                engine.decode(raw.as_bytes()).ok()
            }
        }).as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::Binary;

    #[test]
    fn binary_decodes_lazily_and_reuses_bytes_after_releasing_encoded_text() {
        let binary = Binary::new("aGVsbG8=".into());
        assert!(binary.decoded.get().is_none());
        assert!(binary.encoded.borrow().is_some());
        let first = binary.bytes().unwrap();
        assert_eq!(first, b"hello");
        let pointer = first.as_ptr();
        assert!(binary.encoded.borrow().is_none());
        for _ in 0..10 {
            assert_eq!(binary.bytes().unwrap().as_ptr(), pointer);
        }
    }

    #[test]
    fn binary_preserves_unicode_whitespace_and_caches_failure() {
        let binary = Binary::new(" aG\nVs\t\u{00a0}bG8=\u{2003}".into());
        assert_eq!(binary.bytes(), Some(b"hello".as_slice()));
        let invalid = Binary::new("not-base64!".into());
        assert!(invalid.bytes().is_none());
        assert!(matches!(invalid.decoded.get(), Some(None)));
        assert!(invalid.encoded.borrow().is_none());
        assert!(invalid.bytes().is_none());
        assert_eq!(Binary::new(String::new()).bytes(), Some(b"".as_slice()));
    }
}
