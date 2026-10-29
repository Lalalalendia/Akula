use std::{borrow::Cow, ops::Range};

// The OS preedit is visual, never a mutation of the durable document.
// Only a selection wholly inside this one paragraph is visually replaced;
// the general multiline composition layout remains a separate acceptance gate.
pub(super) fn preview_preedit(source: &str, span: Range<usize>, at: usize, selection: Option<&Range<usize>>, preedit: &str) -> Option<(String, Range<usize>)> {
    let local = at.checked_sub(span.start)?;
    let end = selection
        .filter(|selected| selected.start == at && selected.end <= span.end)
        .and_then(|selected| selected.end.checked_sub(span.start))
        .unwrap_or(local);
    let prefix = source.get(..local)?;
    let suffix = source.get(end..)?;
    let mut shown = String::with_capacity(prefix.len() + preedit.len() + suffix.len());
    shown.push_str(prefix);
    shown.push_str(preedit);
    shown.push_str(suffix);
    Some((shown, local..local + preedit.len()))
}

pub(super) fn render<'a>(source: &'a str, span: Range<usize>, composing: Option<(usize, &str)>, selection: Option<&Range<usize>>) -> (Cow<'a, str>, Option<Range<usize>>) {
    match composing.and_then(|(at, preedit)| preview_preedit(source, span, at, selection, preedit)) {
        Some((shown, underline)) => (Cow::Owned(shown), Some(underline)),
        None => (Cow::Borrowed(source), None),
    }
}
