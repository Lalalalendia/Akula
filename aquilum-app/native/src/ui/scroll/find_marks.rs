/// One passive horizontal tick per occupied physical track row.
pub(super) fn marker_rows(fractions: &[f64], height: f64, scale: f64) -> Vec<usize> {
    if !height.is_finite() || !scale.is_finite() || height <= 0.0 || scale <= 0.0 {
        return Vec::new();
    }
    let rows = (height * scale).ceil().min(65_536.0) as usize;
    if rows == 0 {
        return Vec::new();
    }
    let mut occupied = vec![false; rows];
    for &fraction in fractions {
        if fraction.is_finite() {
            let row = (fraction.clamp(0.0, 1.0) * (rows - 1) as f64).round() as usize;
            occupied[row] = true;
        }
    }
    occupied.into_iter().enumerate().filter_map(|(row, marked)| marked.then_some(row)).collect()
}

