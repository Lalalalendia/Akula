use masonry::core::BrushIndex;
use masonry::parley::{Alignment, Layout};

use super::markdown::parse;
use super::model::{Align, DEFAULT_COL, Range2, Table};
use super::view::{TableView, letters};

pub(super) fn table(text: &str) -> Table {
    parse(&text.split('\n').collect::<Vec<_>>()).unwrap()
}

pub(super) fn blank_view(table: &Table) -> TableView {
    table.layout(500.0, None, |_: &str, _: Option<f64>, _: Alignment, _: bool| (Layout::<BrushIndex>::default(), crate::ui::editor::markdown::Decor::default()))
}

#[test]
fn round_trips_merges_and_metadata() {
    let source = "| Этап | <!--q-empty--> | Срок |\n| --- | :---: | ---: |\n| Backend |  | 12.08 |\n| ^^ | ^^ | 14.08 |\n<!--q-table:{\"merges\":[[1,0,2,1]],\"rows\":[40,52,52],\"cols\":[80,120,60]}-->";
    let t = table(source);
    assert_eq!(t.value(2, 1), "Backend");
    assert_eq!(t.aligns, [None, Some(Align::Center), Some(Align::Right)]);
    assert!(t.explicit);
    assert_eq!(t.serialize(), source);
}

#[test]
fn legacy_spans_are_inferred() {
    let t = table("| a | b |\n| --- | --- |\n| x |  |\n| ^^ | ^^ |");
    assert_eq!(t.merges, [Range2 { top: 1, left: 0, bottom: 2, right: 1 }]);
}

#[test]
fn edits_sanitize_and_structure_ops_keep_merges() {
    let mut t = table("| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |");
    t.set(1, 1, "x|y\n");
    assert_eq!(t.value(1, 1), "x|y ");
    assert!(t.merge(Range2::span((1, 0), (2, 1))));
    assert_eq!(t.value(2, 1), "1");
    t.add_col();
    assert_eq!(t.cols(), 4);
    assert!(t.move_cols(3, 3, 0));
    assert_eq!(t.merges[0].left, 1);
    assert!(t.delete_row(2));
    assert_eq!(t.merges, [Range2 { top: 1, left: 1, bottom: 1, right: 2 }]);
    assert!(t.unmerge(1, 1));
    assert_eq!(letters(27), "AB");
}

#[test]
fn a_table_without_saved_widths_fills_the_column() {
    let view = blank_view(&table("| a | b |\n| --- | --- |\n| 1 | 2 |"));
    let widths = view.widths();
    assert!(widths[0] == widths[1] && widths[0] > DEFAULT_COL, "{widths:?}");
    assert!((view.layout_w - view.scroll_w).abs() < 0.5, "таблица {} при колонке {}", view.layout_w, view.scroll_w);
    assert_eq!(view.max_scroll(), 0.0);
    assert_eq!(view.edge(), view.layout_w);
}

#[test]
fn the_last_border_stays_visible_when_a_wide_table_scrolls_to_its_end() {
    let view = blank_view(&table(&format!("|{}\n|{}\n|{}", " a |".repeat(10), " --- |".repeat(10), " 1 |".repeat(10))));
    let end = view.max_scroll();
    assert!(end > 0.0);
    assert!(view.range_rect(Range2::span((0, 9), (0, 9)), end).x1 < view.scroll_w);
}

#[test]
fn borders_are_found_where_they_are_painted_after_the_table_narrows() {
    use super::view::{Hit, Line};
    use masonry::kurbo::Point;
    let make = |_: &str, _: Option<f64>, _: Alignment, _: bool| (Layout::<BrushIndex>::default(), crate::ui::editor::markdown::Decor::default());
    let wide = table("| a | b | c | d |\n| --- | --- | --- | --- |\n| 1 | 2 | 3 | 4 |\n<!--q-table:{\"merges\":[],\"cols\":[300,300,300,300]}-->");
    let scroll = wide.layout(500.0, None, make).max_scroll();
    let mut narrow = wide.clone();
    narrow.widths = vec![300.0, 300.0, 36.0, 36.0];
    let view = narrow.layout(500.0, None, make);
    assert!(view.max_scroll() < scroll);
    let painted = view.border_line(Hit::ColumnBorder(0), view.max_scroll());
    assert_eq!(view.border_line(Hit::ColumnBorder(0), scroll), painted);
    let Line::Col(x, _) = painted else { panic!("нет линии границы") };
    assert_eq!(view.hit(Point::new(x, 60.0), scroll), Hit::ColumnBorder(0));
}

fn deletion_fixture() -> Table {
    let mut table = Table::empty(4, 4);
    for row in 0..4 {
        for col in 0..4 {
            table.cells[row][col] = format!("{row}:{col}");
        }
    }
    table.heights = vec![40.0, 45.0, 50.0, 55.0];
    table.widths = vec![60.0, 70.0, 80.0, 90.0];
    table.aligns = vec![None, Some(Align::Left), Some(Align::Center), Some(Align::Right)];
    table.explicit = true;
    table
}

// Independent oracle for a synchronized table: project the surviving cells
// and original merge rectangles onto retained row/column coordinates.
fn projected_deletion(original: &Table, row: bool, index: usize) -> Table {
    let rows: Vec<_> = (0..original.rows()).filter(|&i| !row || i != index).collect();
    let cols: Vec<_> = (0..original.cols()).filter(|&i| row || i != index).collect();
    let mut result = Table {
        cells: rows.iter().map(|&r| cols.iter().map(|&c| original.cells[r][c].clone()).collect()).collect(),
        merges: Vec::new(),
        aligns: cols.iter().map(|&c| original.aligns[c]).collect(),
        heights: rows.iter().map(|&r| original.heights[r]).collect(),
        widths: cols.iter().map(|&c| original.widths[c]).collect(),
        explicit: original.explicit,
    };
    for m in &original.merges {
        let covered_rows: Vec<_> = rows.iter().enumerate()
            .filter_map(|(new, old)| (m.top..=m.bottom).contains(old).then_some(new)).collect();
        let covered_cols: Vec<_> = cols.iter().enumerate()
            .filter_map(|(new, old)| (m.left..=m.right).contains(old).then_some(new)).collect();
        if let (Some(&top), Some(&bottom), Some(&left), Some(&right)) =
            (covered_rows.first(), covered_rows.last(), covered_cols.first(), covered_cols.last())
        {
            let projected = Range2 { top, bottom, left, right };
            if !projected.single() { result.merges.push(projected); }
        }
    }
    result
}

fn check_all_deletions(original: &Table) {
    for row in [true, false] {
        for index in 0..4 {
            let mut actual = original.clone();
            let expected = projected_deletion(original, row, index);
            assert!(if row { actual.delete_row(index) } else { actual.delete_col(index) });
            assert_eq!(actual, expected, "row={row}, index={index}, merges={:?}", original.merges);
        }
    }
}

#[test]
fn deleting_axes_preserves_every_valid_merge_projection() {
    let original = deletion_fixture();
    check_all_deletions(&original);
    let mut cases = 0;
    for top in 0..4 {
        for bottom in top..4 {
            for left in 0..4 {
                for right in left..4 {
                    let range = Range2 { top, bottom, left, right };
                    if range.single() { continue; }
                    let mut table = original.clone();
                    assert!(table.merge(range));
                    check_all_deletions(&table);
                    cases += 8;
                }
            }
        }
    }
    assert_eq!(cases, 672);
    let mut table = original;
    assert!(table.merge(Range2::span((0, 0), (1, 1))));
    assert!(table.merge(Range2::span((2, 2), (3, 3))));
    check_all_deletions(&table);
}

#[test]
fn invalid_axis_deletions_are_no_ops() {
    let mut minimum = Table::empty(1, 2);
    let before = minimum.clone();
    assert!(!minimum.delete_row(0));
    assert!(!minimum.delete_col(0));
    assert_eq!(minimum, before);
    let mut table = deletion_fixture();
    assert!(table.merge(Range2::span((0, 0), (2, 2))));
    let before = table.clone();
    for index in [4, usize::MAX] {
        assert!(!table.delete_row(index));
        assert!(!table.delete_col(index));
        assert_eq!(table, before);
    }
}
