//! `just icon`: draws the app icon, 1024 px square, into
//! `crates/viewer/bundle/icon.png`, which `just bundle` turns into the
//! bundle's `.icns`. Not a test suite: `cargo test` and nextest skip it.
//!
//! The picture is the app itself, reduced: an explorer column of checked
//! rows beside a stage showing a picture, on macOS's rounded-square icon
//! body (824 px on the 1024 px grid), in brainstrike's dark tokens. Every
//! shape is drawn with 4 x 4 samples per pixel for smooth edges.

use std::path::Path;

use image::{Rgba, RgbaImage};

/// The icon's side, in pixels.
const SIDE: u32 = 1024;

/// Samples per pixel along each axis.
const SAMPLES: u32 = 4;

/// A colour from HSL degrees and percents, as the theme writes them.
fn hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let (saturation, lightness) = (saturation / 100., lightness / 100.);
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let sector = hue / 60.;
    let second = chroma * (1. - (sector % 2. - 1.).abs());
    let (red, green, blue) = match sector {
        s if s < 1. => (chroma, second, 0.),
        s if s < 2. => (second, chroma, 0.),
        s if s < 3. => (0., chroma, second),
        s if s < 4. => (0., second, chroma),
        s if s < 5. => (second, 0., chroma),
        _ => (chroma, 0., second),
    };
    let base = lightness - chroma / 2.;
    [red + base, green + base, blue + base]
}

/// A shape: whether a point, in icon pixels, lies inside it.
type Shape = Box<dyn Fn(f32, f32) -> bool>;

/// A rectangle with rounded corners.
fn rounded(left: f32, top: f32, width: f32, height: f32, radius: f32) -> Shape {
    Box::new(move |x, y| {
        let dx = (left + radius - x).max(x - (left + width - radius)).max(0.);
        let dy = (top + radius - y).max(y - (top + height - radius)).max(0.);
        x >= left
            && x <= left + width
            && y >= top
            && y <= top + height
            && dx * dx + dy * dy <= radius * radius
    })
}

fn circle(cx: f32, cy: f32, r: f32) -> Shape {
    Box::new(move |x, y| (x - cx).powi(2) + (y - cy).powi(2) <= r * r)
}

/// The region under the line through the points, within `clip`.
fn below(points: Vec<(f32, f32)>, clip: Shape) -> Shape {
    Box::new(move |x, y| {
        let line = points
            .windows(2)
            .find(|pair| x >= pair[0].0 && x <= pair[1].0);
        line.is_some_and(|pair| {
            let t = (x - pair[0].0) / (pair[1].0 - pair[0].0);
            y >= pair[0].1 + t * (pair[1].1 - pair[0].1)
        }) && clip(x, y)
    })
}

/// A checkmark stroke inside a box at `(left, top)` of side `side`.
fn check(left: f32, top: f32, side: f32) -> Shape {
    let points =
        [(0.25, 0.52), (0.43, 0.70), (0.76, 0.32)].map(|(x, y)| (left + x * side, top + y * side));
    let width = side * 0.11;
    Box::new(move |x, y| {
        points.windows(2).any(|pair| {
            let (ax, ay, bx, by) = (pair[0].0, pair[0].1, pair[1].0, pair[1].1);
            let t = (((x - ax) * (bx - ax) + (y - ay) * (by - ay))
                / ((bx - ax).powi(2) + (by - ay).powi(2)))
            .clamp(0., 1.);
            (x - ax - t * (bx - ax)).powi(2) + (y - ay - t * (by - ay)).powi(2) <= width * width
        })
    })
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates are under 1024 and channel values lie in 0..=255"
)]
#[expect(clippy::print_stdout, reason = "the tool reports what it wrote")]
fn main() {
    let body = || rounded(100., 100., 824., 824., 185.);
    let background = hsl(30., 8., 11.);
    let explorer = hsl(30., 5., 17.);
    let primary = hsl(215., 50., 58.);
    let text = hsl(30., 10., 72.);
    let sky = hsl(215., 55., 45.);
    let sun = hsl(38., 90., 68.);
    let hills = hsl(215., 45., 24.);
    let white = [1., 1., 1.];

    // Painted in order; later shapes cover earlier ones.
    let mut layers: Vec<(Shape, [f32; 3])> = vec![(body(), background)];
    layers.push((Box::new(move |x, y| x < 392. && body()(x, y)), explorer));
    for (row, width) in [(0., 108.), (1., 84.), (2., 120.), (3., 72.)] {
        let top = 262. + row * 128.;
        layers.push((rounded(156., top, 60., 60., 14.), primary));
        layers.push((check(156., top, 60.), white));
        layers.push((rounded(240., top + 18., width, 24., 12.), text));
    }
    let stage = || rounded(448., 236., 404., 552., 36.);
    layers.push((stage(), sky));
    layers.push((circle(742., 372., 58.), sun));
    layers.push((
        below(
            vec![
                (440., 640.),
                (560., 520.),
                (650., 610.),
                (730., 548.),
                (860., 660.),
            ],
            stage(),
        ),
        hills,
    ));

    let mut icon = RgbaImage::new(SIDE, SIDE);
    let step = 1. / SAMPLES as f32;
    for (px, py, pixel) in icon.enumerate_pixels_mut() {
        let mut sum = [0_f32; 4];
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                let (x, y) = (
                    px as f32 + (sx as f32 + 0.5) * step,
                    py as f32 + (sy as f32 + 0.5) * step,
                );
                if let Some((_, colour)) = layers.iter().rev().find(|(shape, _)| shape(x, y)) {
                    for (channel, value) in sum.iter_mut().zip(colour) {
                        *channel += value;
                    }
                    sum[3] += 1.;
                }
            }
        }
        let samples = (SAMPLES * SAMPLES) as f32;
        let alpha = sum[3] / samples;
        let channel = |i: usize| {
            let value = if sum[3] > 0. { sum[i] / sum[3] } else { 0. };
            (value * 255.).round() as u8
        };
        *pixel = Rgba([
            channel(0),
            channel(1),
            channel(2),
            (alpha * 255.).round() as u8,
        ]);
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundle/icon.png");
    icon.save(&path).expect("write the icon");
    println!("wrote {}", path.display());
}
