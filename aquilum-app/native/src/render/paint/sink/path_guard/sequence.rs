//! Validate that a nonempty Kurbo path starts with MoveTo.
//! Vello #2005: SIMD flattening debug-asserts on a leading ClosePath.

use masonry::kurbo::PathEl;

pub(super) fn within_budget(elements: &[PathEl], limit: usize) -> bool {
    elements.len() <= limit && matches!(elements.first(), None | Some(PathEl::MoveTo(_)))
}

#[cfg(test)]
mod tests {
    use super::super::{guard_geometry, PaintError, Painter};
    use super::within_budget;
    use masonry::imaging::{ClipRef, GeometryRef, PaintSink};
    use masonry::kurbo::{Affine, BezPath, PathEl, Point};
    use masonry::peniko::Fill;

    #[test]
    fn malformed_path_sequence_close_path_first_fails_before_vello() {
        // Construct a valid Kurbo path, then inject the upstream #2005 input.
        // from_vec/push would panic on invalid input in debug builds.
        let mut path = BezPath::new();
        path.move_to((4.0, 4.0));
        path.elements_mut()[0] = PathEl::ClosePath;

        assert!(!within_budget(path.elements(), 262_144));
        assert!(matches!(
            guard_geometry(&GeometryRef::Path(&path), Affine::IDENTITY),
            Err(PaintError::GeometryBudget(_))
        ));
        let mut painter = Painter::new(32, 32, crate::render::paint::settings(0));
        painter.begin(0, 0, 32, 32);
        painter.push_clip(ClipRef::Fill {
            shape: GeometryRef::Path(&path),
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
        });
        assert!(matches!(
            painter.finish(),
            Err(PaintError::GeometryBudget(_))
        ));
    }

    #[test]
    fn malformed_path_sequence_draw_commands_first_are_rejected() {
        let a = Point::new(3.0, 4.0);
        let b = Point::new(5.0, 6.0);
        let c = Point::new(7.0, 8.0);
        for first in [
            PathEl::LineTo(a),
            PathEl::QuadTo(a, b),
            PathEl::CurveTo(a, b, c),
        ] {
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.elements_mut()[0] = first;
            assert!(!within_budget(path.elements(), 262_144));
            assert!(matches!(
                guard_geometry(&GeometryRef::Path(&path), Affine::IDENTITY),
                Err(PaintError::GeometryBudget(_))
            ));
        }
    }

    #[test]
    fn malformed_path_sequence_empty_and_valid_multisubpath_remain_supported() {
        let empty = BezPath::new();
        assert!(within_budget(empty.elements(), 262_144));
        assert!(guard_geometry(&GeometryRef::Path(&empty), Affine::IDENTITY).is_ok());

        let mut path = BezPath::new();
        path.move_to((2.0, 2.0));
        path.line_to((18.0, 2.0));
        path.line_to((18.0, 18.0));
        path.close_path();
        path.move_to((20.0, 20.0));
        path.curve_to((22.0, 23.0), (24.0, 23.0), (26.0, 20.0));
        path.close_path();
        assert!(within_budget(path.elements(), 262_144));
        assert!(!within_budget(path.elements(), 1));
        assert!(guard_geometry(&GeometryRef::Path(&path), Affine::IDENTITY).is_ok());

        let mut painter = Painter::new(32, 32, crate::render::paint::settings(0));
        painter.begin(0, 0, 32, 32);
        painter.push_clip(ClipRef::Fill {
            shape: GeometryRef::Path(&path),
            transform: Affine::IDENTITY,
            fill_rule: Fill::NonZero,
        });
        painter.pop_clip();
        painter.finish().expect("ordinary multisubpath clip must render");
    }
}
