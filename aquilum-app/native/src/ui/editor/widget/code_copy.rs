use super::*;
use aquilum_core::markdown::fence::Fence;

impl TextEditor {
    // The popup carries only the opener offset; avoid cloning large code bodies
    // when the user merely right-clicks or opens an unrelated menu action.
    pub(super) fn fenced_code_start(&self, index: usize) -> Option<usize> {
        if !self.doc.block(index).is_code() {
            return None;
        }
        let (first, _) = self.run(index, Block::is_code, |block| matches!(block, Block::Code { first: true, .. }));
        if !matches!(self.doc.block(first), Block::Code { first: true, fence: true, .. }) {
            return None;
        }
        // Math ($) is a different code-style block. Never offer an action
        // that would fail silently when clicked.
        let opener = self.doc.paragraph(first);
        Fence::open(self.doc.slice(opener.range()), usize::MAX).map(|_| opener.start)
    }

    // Indented-code normalization is not yet covered by our Markdown tests.
    // Restrict the copy action to fenced blocks where the body is unambiguous.
    fn fenced_code_body(&self, index: usize) -> Option<String> {
        if !self.doc.block(index).is_code() {
            return None;
        }
        let (first, _) = self.run(index, Block::is_code, |block| matches!(block, Block::Code { first: true, .. }));
        if !matches!(self.doc.block(first), Block::Code { first: true, fence: true, .. }) {
            return None;
        }
        let opener = self.doc.paragraph(first).start;
        self.doc.copy_fenced_body(opener)
    }

    pub fn fenced_code_body_from_start(&self, opener: usize) -> Option<String> {
        let index = self.doc.at_offset(opener);
        if self.doc.paragraph(index).start != opener
            || !matches!(self.doc.block(index), Block::Code { first: true, fence: true, .. })
        {
            return None;
        }
        self.fenced_code_start(index)?;
        self.fenced_code_body(index)
    }

    /// Resolve the whole fenced code body under the keyboard caret, even
    /// when the opening fence is outside the mounted paragraph window.
    /// This uses the same full-Rope extraction as menu and header actions.
    pub(super) fn focused_fenced_code_body(&self) -> Option<(usize, String)> {
        let line = self.doc.at_offset(self.focus);
        let opener = self.code_opener_at(line)?;
        self.doc.copy_fenced_body(opener).map(|body| (opener, body))
    }

    pub(super) fn fenced_code_start_at(&self, point: Point) -> Option<usize> {
        let index = self.index_at(point)?;
        let paragraph = self.doc.paragraph(index);
        if point.y < self.doc.top(index) || point.y >= self.doc.bottom(index)
            || !matches!(paragraph.fold, Fold::None | Fold::Above)
        {
            return None;
        }
        self.fenced_code_start(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_code_copy_keeps_body_bytes_but_not_fence_markers() {
        let source = "Вступление\n\u{60}\u{60}\u{60}rust\n    let x = \"🐱\";\n\n    println!(\"{x}\");\n\u{60}\u{60}\u{60}\nПосле";
        let editor = TextEditor::new(source).markdown();
        let expected = "    let x = \"🐱\";\n\n    println!(\"{x}\");";
        for index in 1..=5 {
            assert_eq!(editor.fenced_code_body(index).as_deref(), Some(expected));
        }
        assert_eq!(editor.fenced_code_body(0), None);
        assert_eq!(editor.fenced_code_body(6), None);
        let opener = editor.fenced_code_start(3).expect("code opener");
        assert_eq!(editor.fenced_code_body_from_start(opener).as_deref(), Some(expected));
        assert_eq!(editor.fenced_code_body_from_start(opener + 1), None);
        assert_eq!(editor.text(), source, "copying must not edit the note");
    }

    #[test]
    fn unclosed_and_empty_fences_copy_without_inventing_a_closing_marker() {
        let unclosed = TextEditor::new("~~~python\nprint('ok')").markdown();
        assert_eq!(unclosed.fenced_code_body(1).as_deref(), Some("print('ok')"));
        let empty = TextEditor::new("~~~\n~~~").markdown();
        assert_eq!(empty.fenced_code_body(0).as_deref(), Some(""));
        assert_eq!(empty.fenced_code_body(1).as_deref(), Some(""));
    }


    #[test]
    fn large_windowed_fence_copies_the_full_rope_instead_of_loaded_excerpt() {
        let lines: String = (0..5_000).map(|n| format!("println!(\"{n}\");\n")).collect();
        let source = format!("\u{60}\u{60}\u{60}rust\n{lines}\u{60}\u{60}\u{60}\nпосле");
        let editor = TextEditor::windowed(crop::Rope::from(source.as_str())).markdown();
        assert!(
            editor.doc.loaded().1.len() < source.len(),
            "regression fixture must exceed the loaded editor window"
        );
        let copied = editor.fenced_code_body_from_start(0).expect("whole block");
        assert_eq!(copied, lines.trim_end_matches('\n'));
        assert!(copied.contains("println!(\"4999\");"), "must include unloaded tail");
    }

    #[test]
    fn longer_fence_does_not_close_on_inner_shorter_fences() {
        let source = "\u{60}\u{60}\u{60}\u{60}markdown\n\u{60}\u{60}\u{60}js\ninside\n\u{60}\u{60}\u{60}\n\u{60}\u{60}\u{60}\u{60}\nafter";
        let editor = TextEditor::new(source).markdown();
        assert_eq!(
            editor.fenced_code_body_from_start(0).as_deref(),
            Some("\u{60}\u{60}\u{60}js\ninside\n\u{60}\u{60}\u{60}")
        );
    }

    #[test]
    fn crlf_code_copy_keeps_internal_line_breaks_without_orphan_cr() {
        let text = "\u{60}\u{60}\u{60}txt\r\none\r\ntwo\r\n\u{60}\u{60}\u{60}\r\nend";
        let editor = TextEditor::new(text).markdown();
        assert_eq!(editor.fenced_code_body_from_start(0).as_deref(), Some("one\r\ntwo"));
    }

    #[test]
    fn math_block_does_not_offer_the_fenced_code_command() {
        let editor = TextEditor::new("$\nx^2 + y^2\n$").markdown();
        assert_eq!(editor.fenced_code_start(0), None);
        assert_eq!(editor.fenced_code_body_from_start(0), None);
    }

    #[test]
    fn keyboard_body_resolver_recovers_opener_outside_the_windowed_paragraphs() {
        let body: String = (0..12_000)
            .map(|n| format!("println!(\"line-{n}-🐱\");\n"))
            .collect();
        let source = format!("~~~rust\n{body}~~~\nafter");
        let caret = source.find("line-7500").expect("far code body") + 6;
        let mut editor = TextEditor::windowed(crop::Rope::from(source.as_str())).markdown();
        editor.focus = caret;
        editor.doc.include(caret);
        assert!(editor.doc.loaded().0 > 0, "the opener must be outside the loaded window");

        let copied = (0..300).find_map(|_| {
            editor.doc.track((0.0, 0.0));
            editor.refresh_code_window_opener();
            let result = editor.focused_fenced_code_body();
            if result.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result
        });
        let (opener, text) = copied.expect("completed parser checkpoint should recover opening fence");
        assert_eq!(opener, 0);
        assert_eq!(text, body.trim_end_matches('\n'), "copy must include the unloaded tail");
        assert!(text.contains("line-11999-🐱"));
        assert_eq!(editor.doc.whole(), source);
    }

    #[test]
    fn code_copy_is_not_offered_for_plain_text_or_indented_code() {
        let indented = TextEditor::new("    println!(\"indented\");\n\ntext").markdown();
        assert_eq!(indented.fenced_code_body(0), None);
        assert_eq!(indented.fenced_code_body(2), None);
        let prose = TextEditor::new("before \u{60}\u{60}\u{60} inline").markdown();
        assert_eq!(prose.fenced_code_body(0), None);
    }
}
