use pdfium_render::prelude::PdfColor;

use crate::{chrome::Label, pdf::Point};

pub const COLORS: &[(Label, PdfColor)] = &[
    (("Hitam", "Black"), PdfColor::BLACK),
    (("Biru", "Blue"), PdfColor::BLUE),
    (("Merah", "Red"), PdfColor::new(215, 40, 51, 255)),
    (("Hijau", "Green"), PdfColor::new(16, 126, 73, 255)),
    (("Ungu", "Purple"), PdfColor::new(122, 60, 169, 255)),
    (("Jingga", "Orange"), PdfColor::new(198, 89, 13, 255)),
];

#[derive(Clone, Copy)]
pub enum Shape {
    Rectangle,
    Ellipse,
    Line,
    Arrow,
}

pub const SHAPES: &[(Label, Shape)] = &[
    (("Persegi panjang", "Rectangle"), Shape::Rectangle),
    (("Elips", "Ellipse"), Shape::Ellipse),
    (("Garis", "Line"), Shape::Line),
    (("Panah", "Arrow"), Shape::Arrow),
];

impl Shape {
    pub fn points(self, a: Point, b: Point, size: Point) -> Vec<Point> {
        match self {
            Self::Rectangle => vec![a, (b.0, a.1), b, (a.0, b.1), a],
            Self::Ellipse => {
                let center = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                let radius = ((b.0 - a.0) / 2.0, (b.1 - a.1) / 2.0);
                (0..=48)
                    .map(|i| {
                        let (sin, cos) = (i as f32 * std::f32::consts::TAU / 48.0).sin_cos();
                        (center.0 + radius.0 * cos, center.1 + radius.1 * sin)
                    })
                    .collect()
            }
            Self::Line => vec![a, b],
            Self::Arrow => {
                let (dx, dy) = ((b.0 - a.0) * size.0, (b.1 - a.1) * size.1);
                let length = dx.hypot(dy);
                if length == 0.0 {
                    return vec![a, b];
                }
                let head = 12.0_f32.min(length * 0.32);
                let (x, y) = (dx * head / length, dy * head / length);
                let wing = |side: f32| {
                    (
                        (b.0 + (-x - side * y * 0.55) / size.0).clamp(0.0, 1.0),
                        (b.1 + (-y + side * x * 0.55) / size.1).clamp(0.0, 1.0),
                    )
                };
                vec![a, b, wing(1.0), b, wing(-1.0)]
            }
        }
    }
}
