use masonry::peniko::Color;

// Use the actual painted accent background, not the light/dark theme flag.
fn linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

pub(super) fn text_on(background: Color) -> Color {
    let [r, g, b, _] = background.to_rgba8().to_u8_array();
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    if luminance >= 0.17913 { Color::BLACK } else { Color::WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme;

    #[test]
    fn light_and_dark_accents_always_get_readable_selected_text() {
        for (hex, black) in [("#FFFFFF", true), ("#FFFF00", true), ("#000000", false), ("#1471EB", false)] {
            for dark in [false, true] {
                let _lock = theme::test_lock();
                theme::apply_setting(if dark { "dark" } else { "light" }, dark, hex);
                let colors = theme::current();
                let actual = colors.text_on_accent;
                let expected = if black { Color::BLACK } else { Color::WHITE };
                assert_eq!(actual, expected, "{hex} dark={dark}");
                let [r, g, b, _] = colors.bg_accent.to_rgba8().to_u8_array();
                let l = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
                let contrast = if actual == Color::BLACK { (l + 0.05) / 0.05 } else { 1.05 / (l + 0.05) };
                assert!(contrast >= 4.5, "WCAG AA contrast {contrast} for {hex} dark={dark}");
            }
        }
    }
}
