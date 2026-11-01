//! The tray icon, drawn in code so its three states share one geometry: the kith mark
//! (three devices and their two links) with solid devices when kith works, a dot beside it
//! when something needs this person, and hollow devices when kith is off. Shape alone
//! carries the state, so the macOS template image (one colour) loses nothing.

use crate::tokens::{ATTN_ON_PANEL, INK_ON_PANEL, Rgb};

/// What the icon says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconState {
    /// Connected and nothing needs this person: solid.
    Fine,
    /// Something needs this person: solid, plus a dot.
    NeedsYou,
    /// The daemon is not running or this device is in no network: hollow.
    Off,
}

/// The colours to draw with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Paint {
    /// The mark.
    pub(crate) mark: Rgb,
    /// The needs-you dot.
    pub(crate) dot: Rgb,
}

/// For a macOS template image: the OS recolours it for the menu bar, using only alpha.
pub(crate) const TEMPLATE: Paint = Paint {
    mark: Rgb(0, 0, 0),
    dot: Rgb(0, 0, 0),
};

/// For tray panels that show the icon as drawn; those are dark on every desktop kith ships.
pub(crate) const PANEL: Paint = Paint {
    mark: INK_ON_PANEL,
    dot: ATTN_ON_PANEL,
};

/// The mark's grid, as in the page's sprite.
const GRID: f64 = 24.0;
/// The devices, on that grid.
const DEVICES: [(f64, f64); 3] = [(6.0, 12.0), (18.0, 6.0), (18.0, 18.0)];
/// A device's radius to the middle of its outline.
const RADIUS: f64 = 3.0;
/// The outline and link width.
const STROKE: f64 = 1.5;
/// The two links.
const LINKS: [((f64, f64), (f64, f64)); 2] =
    [((8.7, 10.6), (15.3, 7.4)), ((8.7, 13.4), (15.3, 16.6))];
/// The needs-you dot: in the empty top-left corner, far enough from the mark to read as a
/// badge rather than a fourth device.
const DOT_CENTRE: (f64, f64) = (3.0, 3.0);
/// Its radius.
const DOT_RADIUS: f64 = 2.25;
/// Samples per pixel side, for anti-aliasing.
const SAMPLES: u32 = 4;

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / dx.mul_add(dx, dy * dy)).clamp(0.0, 1.0);
    distance(p, (t.mul_add(dx, a.0), t.mul_add(dy, a.1)))
}

/// Which part of the icon a point is on.
#[derive(Debug, Clone, Copy)]
enum Part {
    Mark,
    Dot,
}

/// What covers a point of the grid.
fn covers(state: IconState, p: (f64, f64)) -> Option<Part> {
    if state == IconState::NeedsYou && distance(p, DOT_CENTRE) <= DOT_RADIUS {
        return Some(Part::Dot);
    }
    let half = STROKE / 2.0;
    let on_link = LINKS
        .iter()
        .any(|&(a, b)| distance_to_segment(p, a, b) <= half);
    let on_device = DEVICES.iter().any(|&centre| {
        let from_centre = distance(p, centre);
        match state {
            IconState::Off => (from_centre - RADIUS).abs() <= half,
            IconState::Fine | IconState::NeedsYou => from_centre <= RADIUS + half,
        }
    });
    (on_link || on_device).then_some(Part::Mark)
}

/// The icon as `size`×`size` RGBA pixels.
pub(crate) fn render(state: IconState, size: u32, paint: Paint) -> Vec<u8> {
    let scale = GRID / f64::from(size);
    let step = 1.0 / f64::from(SAMPLES);
    let total = SAMPLES * SAMPLES;
    let mut pixels = Vec::with_capacity(
        usize::try_from(size)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(size).unwrap_or(0))
            .saturating_mul(4),
    );
    for y in 0..size {
        for x in 0..size {
            let (mut mark, mut dot) = (0_u32, 0_u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let p = (
                        (f64::from(x) + (f64::from(sx) + 0.5) * step) * scale,
                        (f64::from(y) + (f64::from(sy) + 0.5) * step) * scale,
                    );
                    match covers(state, p) {
                        Some(Part::Dot) => dot += 1,
                        Some(Part::Mark) => mark += 1,
                        None => {}
                    }
                }
            }
            let colour = if dot > mark { paint.dot } else { paint.mark };
            let alpha = u8::try_from((mark + dot) * 255 / total).unwrap_or(u8::MAX);
            pixels.extend_from_slice(&[colour.0, colour.1, colour.2, alpha]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 36;
    /// `SIZE` as an index; a grid unit is 1.5 pixels.
    const SIDE: usize = 36;

    /// The pixel at (x, y).
    fn at(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        let start = (y * SIDE + x) * 4;
        [
            pixels[start],
            pixels[start + 1],
            pixels[start + 2],
            pixels[start + 3],
        ]
    }

    #[test]
    fn the_three_states_differ_by_shape() {
        let fine = render(IconState::Fine, SIZE, TEMPLATE);
        let needs = render(IconState::NeedsYou, SIZE, TEMPLATE);
        let off = render(IconState::Off, SIZE, TEMPLATE);
        assert_eq!(fine.len(), SIDE * SIDE * 4);

        let device = (9, 18);
        assert_eq!(
            at(&fine, device.0, device.1)[3],
            255,
            "devices are solid when fine"
        );
        assert_eq!(
            at(&off, device.0, device.1)[3],
            0,
            "devices are hollow when off"
        );
        assert_eq!(at(&off, 13, 18)[3], 255, "the outline stays");

        let dot = (4, 4);
        assert_eq!(at(&fine, dot.0, dot.1)[3], 0, "no dot when fine");
        assert_eq!(
            at(&needs, dot.0, dot.1)[3],
            255,
            "a dot when something needs you"
        );
        assert_eq!(at(&fine, 0, 0)[3], 0, "the corners stay clear");
    }

    #[test]
    fn a_template_is_one_colour_and_a_panel_icon_marks_the_dot() {
        let template = render(IconState::NeedsYou, SIZE, TEMPLATE);
        assert!(template.chunks(4).all(|px| px[..3] == [0, 0, 0]));

        let panel = render(IconState::NeedsYou, SIZE, PANEL);
        let dot = ATTN_ON_PANEL;
        let mark = INK_ON_PANEL;
        assert_eq!(at(&panel, 4, 4), [dot.0, dot.1, dot.2, 255]);
        assert_eq!(at(&panel, 27, 9), [mark.0, mark.1, mark.2, 255]);
    }
}
