use masonry::core::keyboard::{Key, Modifiers, NamedKey};

use super::support::{command_key, markdown_root, named, press, text};

#[test]
fn ctrl_e_wraps_the_word_under_the_caret_or_the_selection_in_backticks() {
    let mut root = markdown_root("вызов функции здесь");
    for _ in 0..8 {
        named(&mut root, NamedKey::ArrowRight);
    }
    command_key(&mut root, "e");
    assert_eq!(text(&root), "вызов `функции` здесь");
    command_key(&mut root, "e");
    assert_eq!(text(&root), "вызов функции здесь", "повторное Ctrl+E снимает код");
    command_key(&mut root, "a");
    command_key(&mut root, "e");
    assert_eq!(text(&root), "`вызов функции здесь`");
}

#[test]
fn shortcuts_toggle_bold_italic_and_strike() {
    let mut root = markdown_root("слово");
    command_key(&mut root, "a");
    command_key(&mut root, "b");
    assert_eq!(text(&root), "**слово**");
    command_key(&mut root, "b");
    assert_eq!(text(&root), "слово");
    command_key(&mut root, "i");
    assert_eq!(text(&root), "*слово*");
    command_key(&mut root, "b");
    assert_eq!(text(&root), "***слово***");
    command_key(&mut root, "i");
    assert_eq!(text(&root), "**слово**");
    press(&mut root, Key::Character("s".into()), crate::test_dir::command() | Modifiers::SHIFT);
    assert_eq!(text(&root), "**~~слово~~**");
    named(&mut root, NamedKey::End);
    command_key(&mut root, "b");
    assert_eq!(text(&root), "**~~слово~~**", "каретка вне разметки и без выделения ничего не меняет");
    for _ in 0..5 {
        named(&mut root, NamedKey::ArrowLeft);
    }
    command_key(&mut root, "b");
    assert_eq!(text(&root), "~~слово~~", "каретка внутри жирного снимает его");
}

#[test]
fn three_backticks_open_a_fence_with_its_closing_backticks() {
    let mut root = markdown_root("");
    for _ in 0..3 {
        super::support::typing(&mut root, "`");
    }
    assert_eq!(text(&root), "```\n```", "closing fence must be on the next line");
    super::support::typing(&mut root, "js");
    assert_eq!(text(&root), "```js\n```", "typing a language keeps the opening fence on the first line");
    named(&mut root, NamedKey::Enter);
    assert_eq!(text(&root), "```js\n\n```", "Enter places the caret in the code body");
    let mut root = markdown_root("слово");
    named(&mut root, NamedKey::End);
    super::support::typing(&mut root, "``");
    super::support::typing(&mut root, "`");
    assert!(!text(&root).ends_with("``````"), "после буквы ограждение не открывается: {}", text(&root));
}

#[test]
fn triple_backticks_inside_prose_do_not_create_a_multiline_fence() {
    let mut root = markdown_root("префикс ");
    named(&mut root, NamedKey::End);
    super::support::typing(&mut root, "```");
    assert!(!text(&root).contains('\n'), "inline backticks must stay on one line");
}

#[test]
fn single_line_input_never_expands_backticks_to_multiple_lines() {
    let mut root = super::support::focused(crate::ui::editor::TextEditor::single_line("").markdown(), 400, 300);
    super::support::typing(&mut root, "```");
    assert!(!text(&root).contains('\n'), "single-line widgets must not insert newlines");
}
