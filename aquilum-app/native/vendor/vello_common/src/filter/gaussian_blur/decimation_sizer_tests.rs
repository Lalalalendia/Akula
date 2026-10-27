//! Upstream linebender/vello#2005 regression for DecimationSizer::upscale
//! with empty downscale history and u16 boundary restoration.

use super::DecimationSizer;

#[test]
fn vello_decimation_sizer_empty_history_is_idempotent() {
    let mut zero = DecimationSizer::default();
    assert_eq!(zero.upscale(), (0, 0));
    assert_eq!(zero.upscale(), (0, 0));
    let mut initial = DecimationSizer::new(3840, 2160);
    assert_eq!(initial.upscale(), (3840, 2160));
    assert_eq!(initial.current(), (3840, 2160));
    assert_eq!(initial.downscale(), (1920, 1080));
    assert_eq!(initial.upscale(), (3840, 2160));
    assert_eq!(initial.upscale(), (3840, 2160));
}

#[test]
fn vello_decimation_sizer_u16_boundary_roundtrip_does_not_overflow() {
    for (width, height) in [
        (0, 0), (1, 1), (5, 7), (3840, 2160), (7680, 4320), (8192, 8192),
        (u16::MAX, 0), (0, u16::MAX), (u16::MAX, u16::MAX),
        (u16::MAX - 1, u16::MAX - 2), (u16::MAX - 3, 2),
    ] {
        let mut sizer = DecimationSizer::new(width, height);
        let expected_downscale = (width.div_ceil(2), height.div_ceil(2));
        assert_eq!(sizer.downscale(), expected_downscale);
        assert_eq!(sizer.upscale(), (width, height),
            "roundtrip lost original {width}x{height}");
        assert_eq!(sizer.upscale(), (width, height));
    }
}

#[test]
fn vello_decimation_sizer_nested_roundtrip_restores_each_exact_level() {
    let (w, h) = (u16::MAX, u16::MAX - 2);
    let mut sizer = DecimationSizer::new(w, h);
    let first = sizer.downscale();
    let second = sizer.downscale();
    let third = sizer.downscale();
    assert_eq!(sizer.upscale(), second);
    assert_eq!(sizer.upscale(), first);
    assert_eq!(sizer.upscale(), (w, h));
    assert_eq!(sizer.upscale(), (w, h));
    let mut reset = DecimationSizer::new(1, 1);
    reset.downscale();
    reset.reset(w, h);
    assert_eq!(reset.upscale(), (w, h));
    assert_eq!(reset.downscale(), first);
    assert_ne!(third, first);
}
