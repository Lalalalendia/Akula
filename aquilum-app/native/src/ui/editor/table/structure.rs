use super::model::{DEFAULT_COL, DEFAULT_ROW, Range2, Table};

#[derive(Clone, Copy)]
enum DeleteAxis { Row, Column }

impl Table {
    pub fn add_row(&mut self) {
        let cols = self.cols();
        self.cells.push(vec![String::new(); cols]);
        self.heights.push(DEFAULT_ROW);
    }

    pub fn add_col(&mut self) {
        for row in &mut self.cells {
            row.push(String::new());
        }
        self.aligns.push(None);
        self.widths.push(DEFAULT_COL);
    }

    pub fn delete_row(&mut self, index: usize) -> bool {
        self.delete_axis(index, DeleteAxis::Row)
    }

    pub fn delete_col(&mut self, index: usize) -> bool {
        self.delete_axis(index, DeleteAxis::Column)
    }

    fn delete_axis(&mut self, index: usize, axis: DeleteAxis) -> bool {
        let (len, minimum) = match axis {
            DeleteAxis::Row => (self.rows(), 2),
            DeleteAxis::Column => (self.cols(), 1),
        };
        if index >= len || len <= minimum {
            return false;
        }
        let values: Vec<(Range2, String)> = self.merges.iter().map(|m| (*m, self.cells[m.top][m.left].clone())).collect();
        match axis {
            DeleteAxis::Row => { self.cells.remove(index); self.heights.remove(index); }
            DeleteAxis::Column => {
                for row in &mut self.cells { row.remove(index); }
                self.aligns.remove(index);
                self.widths.remove(index);
            }
        }
        let mut merges = Vec::new();
        for (mut r, v) in values {
            let (start, end) = match axis {
                DeleteAxis::Row => (&mut r.top, &mut r.bottom),
                DeleteAxis::Column => (&mut r.left, &mut r.right),
            };
            if index < *start {
                *start -= 1;
                *end -= 1;
            } else if index <= *end {
                if *start == *end { continue; }
                *end -= 1;
            }
            if !r.single() {
                self.cells[r.top][r.left] = v;
                merges.push(r);
            }
        }
        self.merges = merges;
        self.sync();
        true
    }

    fn order(len: usize, start: usize, end: usize, dest: usize) -> Option<Vec<usize>> {
        let size = end - start + 1;
        let dest = dest.min(len - size);
        if dest == start {
            return None;
        }
        let mut order: Vec<usize> = (0..len).collect();
        let block: Vec<usize> = order.drain(start..=end).collect();
        order.splice(dest..dest, block);
        Some(order)
    }

    pub fn move_rows(&mut self, top: usize, bottom: usize, dest: usize) -> bool {
        let Some(order) = Self::order(self.rows(), top, bottom, dest) else { return false };
        let mut back = vec![0; order.len()];
        for (new, old) in order.iter().enumerate() {
            back[*old] = new;
        }
        self.cells = order.iter().map(|o| self.cells[*o].clone()).collect();
        self.heights = order.iter().map(|o| self.heights[*o]).collect();
        self.merges = self.merges.iter().map(|m| Range2 { top: back[m.top], bottom: back[m.bottom], ..*m }).collect();
        *self = self.clone().normalized();
        true
    }

    pub fn move_cols(&mut self, left: usize, right: usize, dest: usize) -> bool {
        let Some(order) = Self::order(self.cols(), left, right, dest) else { return false };
        let mut back = vec![0; order.len()];
        for (new, old) in order.iter().enumerate() {
            back[*old] = new;
        }
        self.cells = self.cells.iter().map(|row| order.iter().map(|o| row[*o].clone()).collect()).collect();
        self.widths = order.iter().map(|o| self.widths[*o]).collect();
        self.aligns = order.iter().map(|o| self.aligns[*o]).collect();
        self.merges = self.merges.iter().map(|m| Range2 { left: back[m.left], right: back[m.right], ..*m }).collect();
        *self = self.clone().normalized();
        true
    }
}
