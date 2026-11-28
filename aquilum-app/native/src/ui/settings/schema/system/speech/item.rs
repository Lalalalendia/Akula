use super::*;

pub(super) fn buttons(items: Vec<Button>) -> Control {
    Control::Buttons(items)
}

pub(super) fn button(key: &str, action: Speech) -> Button {
    Button::new(label(key), Action::Speech(action))
}

pub(super) fn label(key: &str) -> String {
    t(&format!("settings.system.speech.{key}"))
}

pub(super) fn status(key: &str, value: String, tone: Tone) -> Row {
    info(label(key), None, Control::Status { text: value, tone })
}

pub(super) fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .chars()
        .take(120)
        .collect()
}
