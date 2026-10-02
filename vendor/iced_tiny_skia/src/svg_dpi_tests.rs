use super::*;
use crate::core::{Radians, Size, Svg};

const SCALES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
const BOUNDS: Rectangle = Rectangle {
    x: 40.0,
    y: 32.0,
    width: 24.0,
    height: 16.0,
};

fn svg() -> Svg {
    Svg::new(core::svg::Handle::from_memory(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16"><rect width="24" height="16" fill="#ffffff"/></svg>"##.as_slice(),
    ))
}

fn render(scale: f64, svg: Svg, clip: Option<Rectangle>) -> tiny_skia::Pixmap {
    let viewport = Viewport::with_physical_size(Size::new(256, 256), scale);
    let mut renderer = Renderer::new(Font::default(), Pixels(16.0));
    if let Some(clip) = clip {
        core::Renderer::start_layer(&mut renderer, clip);
    }
    core::svg::Renderer::draw_svg(&mut renderer, svg, BOUNDS);
    if clip.is_some() {
        core::Renderer::end_layer(&mut renderer);
    }
    let mut pixels = tiny_skia::Pixmap::new(256, 256).unwrap();
    let mut mask = tiny_skia::Mask::new(256, 256).unwrap();
    renderer.draw::<&str>(
        &mut pixels.as_mut(),
        &mut mask,
        &viewport,
        &[Rectangle::with_size(viewport.logical_size())],
        Color::TRANSPARENT,
        &[],
    );
    pixels
}

fn alpha_bounds(pixels: &tiny_skia::Pixmap) -> (u32, u32, u32, u32) {
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..pixels.height() {
        for x in 0..pixels.width() {
            if pixels.pixel(x, y).unwrap().alpha() > 0 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x + 1);
                bounds.3 = bounds.3.max(y + 1);
            }
        }
    }
    assert_ne!(bounds.0, u32::MAX, "SVG must produce visible pixels");
    bounds
}

#[test]
fn svg_pixels_match_physical_bounds_at_each_dpi() {
    for scale in SCALES {
        let pixels = render(scale, svg(), None);
        let scale = scale as f32;
        let expected = (
            (BOUNDS.x * scale) as u32,
            (BOUNDS.y * scale) as u32,
            ((BOUNDS.x + BOUNDS.width) * scale) as u32,
            ((BOUNDS.y + BOUNDS.height) * scale) as u32,
        );
        assert_eq!(alpha_bounds(&pixels), expected, "DPI scale {scale}");
        let area = pixels
            .pixels()
            .iter()
            .filter(|pixel| pixel.alpha() == 255)
            .count();
        assert_eq!(
            area,
            ((expected.2 - expected.0) * (expected.3 - expected.1)) as usize,
            "SVG must fill its physical rectangle at DPI scale {scale}"
        );
    }
}

#[test]
fn svg_rotation_tint_and_opacity_survive_dpi_scaling() {
    for scale in SCALES {
        let pixels = render(
            scale,
            svg()
                .rotation(Radians(std::f32::consts::FRAC_PI_2))
                .color(Color::from_rgb(1.0, 0.0, 0.0))
                .opacity(0.5_f32),
            None,
        );
        let scale = scale as f32;
        let expected = (
            (44.0 * scale) as u32,
            (28.0 * scale) as u32,
            (60.0 * scale) as u32,
            (52.0 * scale) as u32,
        );
        let actual = alpha_bounds(&pixels);
        // Rotating by pi/2 introduces floating-point edge antialiasing.
        for (actual, expected) in [actual.0, actual.1, actual.2, actual.3]
            .into_iter()
            .zip([expected.0, expected.1, expected.2, expected.3])
        {
            assert!(
                actual.abs_diff(expected) <= 1,
                "rotated SVG at {scale}: {actual} vs {expected}"
            );
        }
        let center = pixels
            .pixel((52.0 * scale) as u32, (40.0 * scale) as u32)
            .unwrap();
        assert!(center.alpha().abs_diff(128) <= 1);
        assert_eq!(center.red(), 0); // softbuffer uses BGR presentation
        assert_eq!(center.green(), 0);
        assert_eq!(center.blue(), center.alpha());
    }
}

#[test]
fn svg_layer_clip_stays_in_physical_coordinates() {
    let clip = Rectangle {
        x: 48.0,
        y: 36.0,
        width: 12.0,
        height: 8.0,
    };
    for scale in SCALES {
        let pixels = render(scale, svg(), Some(clip));
        let scale = scale as f32;
        assert_eq!(
            alpha_bounds(&pixels),
            (
                (clip.x * scale) as u32,
                (clip.y * scale) as u32,
                ((clip.x + clip.width) * scale) as u32,
                ((clip.y + clip.height) * scale) as u32
            ),
            "clipped SVG at {scale}"
        );
    }
}
