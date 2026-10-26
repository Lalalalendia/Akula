//! Checked wire lengths before editor changes reach UTF-16 staging or the journal.
use super::ChangeError;
use serde_json::Value;

pub(super) fn invalid(message: &str) -> ChangeError {
    ChangeError(message.to_owned())
}

pub(super) fn section_len(value: &Value) -> Result<usize, ChangeError> {
    let length = value.as_u64().ok_or_else(|| invalid("section length is not a number"))?;
    usize::try_from(length).map_err(|_| invalid("section length exceeds the platform limit"))
}

pub(super) fn checked_end(position: usize, length: usize, doc_length: usize) -> Result<usize, ChangeError> {
    position.checked_add(length).filter(|&end| end <= doc_length)
        .ok_or_else(|| invalid("change section extends beyond the document"))
}
