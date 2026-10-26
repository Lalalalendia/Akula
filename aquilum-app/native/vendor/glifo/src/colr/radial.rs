//! COLRv1 radial color-line clipping for PAD gradients (Vello #1982).
//! skrifa may normalize stops so one endpoint has a negative radius. The
//! radius-zero tip is the actual domain boundary, not a solid-color fallback.

use crate::color::{ColorSpaceTag, HueDirection};
use crate::kurbo::Point;
use crate::peniko::{ColorStop, ColorStops};
use smallvec::SmallVec;

/// Return a nonnegative-radius radial and its reparameterized PAD color line.
/// None means both radii are nonpositive, so there is no paintable interval.
pub(super) fn clip_pad(
    p0: Point, r0: f32, p1: Point, r1: f32, stops: ColorStops,
) -> Option<(Point, f32, Point, f32, ColorStops)> {
    if stops.0.is_empty() || !r0.is_finite() || !r1.is_finite() {
        return None;
    }
    if r0 >= 0.0 && r1 >= 0.0 {
        return Some((p0, r0, p1, r1, stops));
    }
    if r0 <= 0.0 && r1 <= 0.0 {
        return None;
    }

    let cut = -r0 / (r1 - r0);
    if !(0.0..1.0).contains(&cut) {
        return None;
    }
    let boundary = stop_color(&stops, cut);
    let mut clipped = SmallVec::<[ColorStop; 4]>::new();
    if r0 < 0.0 {
        // Keep t in [cut, 1], mapping t= cut + (1-cut)*t_new.
        clipped.push(ColorStop { offset: 0.0, color: boundary });
        for mut stop in stops.0 {
            if stop.offset > cut {
                stop.offset = (stop.offset - cut) / (1.0 - cut);
                clipped.push(stop);
            }
        }
        let tip = p0 + (p1 - p0) * f64::from(cut);
        Some((tip, 0.0, p1, r1, ColorStops(clipped)))
    } else {
        // Keep t in [0,cut], mapping t= cut*t_new.
        for mut stop in stops.0 {
            if stop.offset < cut {
                stop.offset /= cut;
                clipped.push(stop);
            }
        }
        clipped.push(ColorStop { offset: 1.0, color: boundary });
        let tip = p0 + (p1 - p0) * f64::from(cut);
        Some((p0, r0, tip, 0.0, ColorStops(clipped)))
    }
}

/// Evaluate the original, already normalized sRGB gradient at the tip.
/// All color channels use the same premultiplied-alpha interpolation as the
/// default peniko gradient, including semitransparent COLR palette entries.
fn stop_color(stops: &ColorStops, t: f32) -> crate::color::DynamicColor {
    let mut previous = stops.0[0];
    for next in stops.0.iter().copied().skip(1) {
        if t < next.offset {
            let span = next.offset - previous.offset;
            if span > 0.0 {
                return previous.color
                    .interpolate(next.color, ColorSpaceTag::Srgb, HueDirection::Shorter)
                    .eval((t - previous.offset) / span);
            }
        }
        previous = next;
    }
    previous.color
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaColor, DynamicColor, Srgb};

    fn stops() -> ColorStops {
        let red = DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([1.0, 0.0, 0.0, 1.0]));
        let blue = DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([0.0, 0.0, 1.0, 1.0]));
        ColorStops(SmallVec::from_vec(vec![
            ColorStop { offset: 0.0, color: red },
            ColorStop { offset: 1.0, color: blue },
        ]))
    }

    fn assert_red_blue(line: &ColorStops, t: f32, red: f32, blue: f32) {
        let rgb = stop_color(line, t).to_alpha_color::<Srgb>().components;
        assert!((rgb[0] - red).abs() <= 0.005, "red mismatch: {rgb:?}");
        assert!(rgb[1].abs() <= 0.005, "green mismatch: {rgb:?}");
        assert!((rgb[2] - blue).abs() <= 0.005, "blue mismatch: {rgb:?}");
        assert!((rgb[3] - 1.0).abs() <= 0.005, "alpha mismatch: {rgb:?}");
    }

    #[test]
    fn colr_radial_negative_radius_start_tip_matches_spec() {
        let a = Point::new(10.0, 20.0);
        let b = Point::new(70.0, 80.0);
        let (p0, r0, p1, r1, line) =
            clip_pad(a, -250.0, b, 500.0, stops()).expect("gradient A");
        assert_eq!(r0, 0.0);
        assert_eq!(r1, 500.0);
        assert_eq!(p1, b);
        assert!((p0.x - 30.0).abs() < 1e-5);
        assert!((p0.y - 40.0).abs() < 1e-5);
        // Spec case A: center is 2/3 red, radius 200 is 40% red.
        assert_red_blue(&line, 0.0, 2.0 / 3.0, 1.0 / 3.0);
        assert_red_blue(&line, 0.4, 0.4, 0.6);
        assert_red_blue(&line, 1.0, 0.0, 1.0);
    }

    #[test]
    fn colr_radial_negative_radius_end_tip_matches_spec() {
        let a = Point::new(10.0, 20.0);
        let b = Point::new(70.0, 80.0);
        let (p0, r0, p1, r1, line) =
            clip_pad(a, 500.0, b, -250.0, stops()).expect("gradient B");
        assert_eq!(p0, a);
        assert_eq!(r0, 500.0);
        assert_eq!(r1, 0.0);
        assert!((p1.x - 50.0).abs() < 1e-5);
        assert!((p1.y - 60.0).abs() < 1e-5);
        // Spec case B: center is 1/3 red, radius 200 is 60% red.
        assert_red_blue(&line, 1.0, 1.0 / 3.0, 2.0 / 3.0);
        assert_red_blue(&line, 0.6, 0.6, 0.4);
        assert_red_blue(&line, 0.0, 1.0, 0.0);
    }

    #[test]
    fn colr_radial_nonnegative_radii_preserve_normalized_stops() {
        let a = Point::new(10.0, 20.0);
        let b = Point::new(70.0, 80.0);
        let original = stops();
        let (p0, r0, p1, r1, line) =
            clip_pad(a, 50.0, b, 500.0, original.clone()).expect("control C");
        assert_eq!((p0, r0, p1, r1), (a, 50.0, b, 500.0));
        assert_eq!(line, original);
        assert!(clip_pad(a, -50.0, b, -1.0, stops()).is_none());
    }
}
