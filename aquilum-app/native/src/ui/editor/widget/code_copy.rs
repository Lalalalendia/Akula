use super::*;

impl TextEditor {
    // The popup carries only the opener offset; avoid cloning large code bodies
    // when the user merely right-clicks or opens an unrelated menu action.
    fn fenced_code_start(&self, index: usize) -> Option<usize> {
        if !self.doc.block(index).is_code() {
            return None;
        }
        let (first, _) = self.run(index, Block::is_code, |block| matches!(block, Block::Code { first: true, .. }));
        matches!(self.doc.block(first), Block::Code { first: true, fence: true, .. })
            .then_some(self.doc.paragraph(first).start)
    }

    // Indented-code normalization is not yet covered by our Markdown tests.
    // Restrict the copy action to fenced blocks where the body is unambiguous.
    fn fenced_code_body(&self, index: usize) -> Option<String> {
        if !self.doc.block(index).is_code() {
            return None;
        }
        let (first, last) = self.run(index, Block::is_code, |block| matches!(block, Block::Code { first: true, .. }));
        if !matches!(self.doc.block(first), Block::Code { first: true, fence: true, .. }) {
            return None;
        }
        let start = first + 1;
        // The last body line is included for an unclosed fence.
        let end = last + usize::from(!matches!(self.doc.block(last), Block::Code { fence: true, .. }));
        let mut body = String::new();
        for line in start..end {
            if line > start {
                body.push('\n');
            }
            body.push_str(self.doc.slice(self.doc.paragraph(line).range()));
        }
        Some(body)
    }

    pub fn fenced_code_body_from_start(&self, opener: usize) -> Option<String> {
        let index = self.doc.at_offset(opener);
        if self.doc.paragraph(index).start != opener
            || !matches!(self.doc.block(index), Block::Code { first: true, fence: true, .. })
        {
            return None;
        }
        self.fenced_code_body(index)
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
    fn code_copy_is_not_offered_for_plain_text_or_indented_code() {
        let indented = TextEditor::new("    println!(\"indented\");\n\ntext").markdown();
        assert_eq!(indented.fenced_code_body(0), None);
        assert_eq!(indented.fenced_code_body(2), None);
        let prose = TextEditor::new("before \u{60}\u{60}\u{60} inline").markdown();
        assert_eq!(prose.fenced_code_body(0), None);
    }
}
