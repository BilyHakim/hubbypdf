use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::DynamicImage;
use pdfium_render::prelude::*;
use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

pub type Point = (f32, f32);
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const MAX_EDGE: f32 = 4096.0;
pub const MAX_PIXELS: f32 = 9_500_000.0;
type MarkBounds = (Point, Point);

pub struct Raster {
    pub width: i32,
    pub height: i32,
    pub bytes: Vec<u8>,
}

pub enum Mark {
    Ink(Vec<Point>, Option<u64>),
    Highlight(Point, Point),
    Text(Point, String),
    Signature(Point, Point, DynamicImage),
}

impl Mark {
    fn bounds(&self) -> Option<(Point, Point)> {
        let points = match self {
            Self::Ink(points, _) => points.as_slice(),
            Self::Highlight(a, b) | Self::Signature(a, b, _) => {
                return Some(((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1))));
            }
            Self::Text(..) => return None,
        };
        let first = *points.first()?;
        Some(points.iter().fold((first, first), |(a, b), p| {
            ((a.0.min(p.0), a.1.min(p.1)), (b.0.max(p.0), b.1.max(p.1)))
        }))
    }
}

pub fn translate(point: Point, offset: Point) -> Point {
    (point.0 + offset.0, point.1 + offset.1)
}

enum Edit {
    Add(Mark, Option<MarkBounds>),
    Move(Vec<usize>, Point),
    ResizeHighlight(usize, (f32, f32)),
}

pub struct Document {
    pdf: PdfDocument<'static>,
    pub source: PathBuf,
    pub index: u16,
    engine: &'static Pdfium,
    original: File,
    edits: Vec<(u16, Edit, u64)>,
    next_revision: u64,
    saved_revision: u64,
}

impl Document {
    pub fn open(engine: &'static Pdfium, source: PathBuf) -> Result<Self> {
        let original = OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
            .open(&source)?;
        let pdf = engine.load_pdf_from_reader(original.try_clone()?, None)?;
        if pdf.pages().is_empty() {
            return Err("PDF ini tidak berisi halaman.".into());
        }
        Ok(Self {
            pdf,
            engine,
            original,
            source,
            index: 0,
            edits: Vec::new(),
            next_revision: 0,
            saved_revision: 0,
        })
    }

    pub fn count(&self) -> u16 {
        self.pdf.pages().len()
    }

    fn revision(&self) -> u64 {
        self.edits.last().map_or(0, |edit| edit.2)
    }

    pub fn dirty(&self) -> bool {
        self.revision() != self.saved_revision
    }

    pub fn can_undo(&self) -> bool {
        !self.edits.is_empty()
    }

    pub fn size(&self) -> Result<Point> {
        let page = self.pdf.pages().get(self.index)?;
        Self::page_size(&page)
    }

    fn page_size(page: &PdfPage<'_>) -> Result<Point> {
        let size = (page.width().value, page.height().value);
        if !size.0.is_finite() || !size.1.is_finite() || size.0 <= 0.0 || size.1 <= 0.0 {
            return Err("Ukuran halaman PDF tidak valid.".into());
        }
        Ok(size)
    }

    fn config(width: i32, height: i32) -> PdfRenderConfig {
        PdfRenderConfig::new()
            .set_fixed_size(width, height)
            .set_format(PdfBitmapFormat::BGRx)
            .set_reverse_byte_order(false)
            .use_lcd_text_rendering(true)
    }

    pub fn render(&self, width: i32, height: i32) -> Result<Raster> {
        let page = self.pdf.pages().get(self.index)?;
        let bitmap = page.render_with_config(&Self::config(width, height))?;
        Ok(Raster {
            width: bitmap.width(),
            height: bitmap.height(),
            bytes: bitmap.as_raw_bytes(),
        })
    }

    pub fn highlight_band(&self, at: Point) -> Result<(f32, f32)> {
        let page = self.pdf.pages().get(self.index)?;
        let size = Self::page_size(&page)?;
        let (w, h, config) = Self::edit_coordinates(size);
        let (x, y) = page.pixels_to_points(
            (at.0 * w as f32).round() as i32,
            (at.1 * h as f32).round() as i32,
            &config,
        )?;
        let text = page.text()?;
        let chars = text.chars();
        if let Some(glyph) =
            chars.get_char_near_point(x, PdfPoints::new(8.0), y, PdfPoints::new(3.0))
            && let Ok(bounds) = glyph.loose_bounds()
        {
            let (_, a) = page.points_to_pixels(bounds.left(), bounds.top(), &config)?;
            let (_, b) = page.points_to_pixels(bounds.right(), bounds.bottom(), &config)?;
            let center = (a + b) as f32 / (2.0 * h as f32);
            let font_height = glyph.scaled_font_size().value / size.1;
            let half = if font_height.is_finite() && font_height > 0.0 {
                font_height / 2.0
            } else {
                a.abs_diff(b) as f32 / (2.0 * h as f32)
            };
            let band = ((center - half).max(0.0), (center + half).min(1.0));
            if band.0 < band.1 {
                return Ok(band);
            }
        }
        let half = 6.0 / size.1;
        Ok(((at.1 - half).max(0.0), (at.1 + half).min(1.0)))
    }

    fn edit_coordinates((w, h): Point) -> (i32, i32, PdfRenderConfig) {
        let scale = 4096.0 / w.max(h);
        let w = (w * scale).round().max(1.0) as i32;
        let h = (h * scale).round().max(1.0) as i32;
        (w, h, Self::config(w, h))
    }

    pub fn add(&mut self, mark: Mark) -> Result<usize> {
        let mark = match mark {
            Mark::Highlight(a, b) => {
                Mark::Highlight((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)))
            }
            mark => mark,
        };
        let bounds = Self::apply(&mut self.pdf, self.index, &mark, (0.0, 0.0))?;
        self.next_revision += 1;
        self.edits
            .push((self.index, Edit::Add(mark, bounds), self.next_revision));
        Ok(self.edits.len() - 1)
    }

    fn apply(
        pdf: &mut PdfDocument<'static>,
        index: u16,
        mark: &Mark,
        offset: Point,
    ) -> Result<Option<MarkBounds>> {
        let font = matches!(mark, Mark::Text(..)).then(|| pdf.fonts_mut().helvetica());
        let mut page = pdf.pages().get(index)?;
        let page_size = Self::page_size(&page)?;
        let (w, h, config) = Self::edit_coordinates(page_size);
        let to_pdf = |(x, y): Point| {
            page.pixels_to_points(
                ((x + offset.0) * w as f32).round() as i32,
                ((y + offset.1) * h as f32).round() as i32,
                &config,
            )
        };
        let mut selection_bounds = mark.bounds();
        let object: PdfPageObject = match mark {
            Mark::Ink(points, signature) => {
                let (x, y) = to_pdf(points[0])?;
                let color = if signature.is_some() {
                    PdfColor::BLUE
                } else {
                    PdfColor::BLACK
                };
                let mut path = PdfPagePathObject::new(
                    pdf,
                    x,
                    y,
                    Some(color),
                    Some(PdfPoints::new(1.8)),
                    None,
                )?;
                for point in points.iter().copied().skip(1) {
                    let (x, y) = to_pdf(point)?;
                    path.line_to(x, y)?;
                }
                path.into()
            }
            Mark::Highlight(a, b) => {
                let (x1, y1) = to_pdf(*a)?;
                let (x2, y2) = to_pdf(*b)?;
                let bounds = PdfRect::new(
                    PdfPoints::new(y1.value.min(y2.value)),
                    PdfPoints::new(x1.value.min(x2.value)),
                    PdfPoints::new(y1.value.max(y2.value)),
                    PdfPoints::new(x1.value.max(x2.value)),
                );
                PdfPagePathObject::new_rect(
                    pdf,
                    bounds,
                    None,
                    None,
                    Some(PdfColor::new(255, 218, 61, 92)),
                )?
                .into()
            }
            Mark::Text(at, text) => {
                let mut object =
                    PdfPageTextObject::new(pdf, text, font.unwrap(), PdfPoints::new(14.0))?;
                let bounds = object.bounds()?;
                let width = bounds.width().value / page_size.0;
                let height = bounds.height().value / page_size.1;
                if width >= 1.0 || height >= 1.0 {
                    return Err(
                        "Teks terlalu panjang untuk halaman ini. Masukkan teks yang lebih pendek."
                            .into(),
                    );
                }
                let left = at.0.min(1.0 - width);
                let top = at.1.min(1.0 - height);
                selection_bounds = Some(((left, top), (left + width, top + height)));
                let baseline = top + bounds.top().value / page_size.1;
                let (x, y) = to_pdf((left - bounds.left().value / page_size.0, baseline))?;
                let (right_x, right_y) =
                    to_pdf((left - bounds.left().value / page_size.0 + 0.05, baseline))?;
                let angle = (right_y.value - y.value).atan2(right_x.value - x.value);
                let (sin, cos) = angle.sin_cos();
                object.apply_matrix(PdfMatrix::new(cos, sin, -sin, cos, x.value, y.value))?;
                object.into()
            }
            Mark::Signature(a, b, image) => {
                let left = a.0.min(b.0);
                let right = a.0.max(b.0);
                let top = a.1.min(b.1);
                let bottom = a.1.max(b.1);
                let (x, y) = to_pdf((left, bottom))?;
                let (right_x, right_y) = to_pdf((right, bottom))?;
                let (up_x, up_y) = to_pdf((left, top))?;
                let mut object = PdfPageImageObject::new(pdf, image)?;
                object.apply_matrix(PdfMatrix::new(
                    right_x.value - x.value,
                    right_y.value - y.value,
                    up_x.value - x.value,
                    up_y.value - y.value,
                    x.value,
                    y.value,
                ))?;
                object.into()
            }
        };
        page.objects_mut().add_object(object)?;
        Ok(selection_bounds)
    }

    fn offsets(edits: &[(u16, Edit, u64)]) -> HashMap<usize, MarkBounds> {
        let mut offsets = HashMap::new();
        for (_, edit, _) in edits {
            match edit {
                Edit::Move(ids, delta) => {
                    for id in ids {
                        let (a, b) = offsets.entry(*id).or_default();
                        *a = translate(*a, *delta);
                        *b = translate(*b, *delta);
                    }
                }
                Edit::ResizeHighlight(id, (top, bottom)) => {
                    let (a, b) = offsets.entry(*id).or_default();
                    a.1 += top;
                    b.1 += bottom;
                }
                Edit::Add(..) => {}
            }
        }
        offsets
    }

    pub fn is_highlight(&self, id: usize) -> bool {
        matches!(
            self.edits.get(id),
            Some((_, Edit::Add(Mark::Highlight(..), _), _))
        )
    }

    pub fn hit(&self, point: Point, tolerance: Point) -> Option<(Vec<usize>, (Point, Point))> {
        let offsets = Self::offsets(&self.edits);
        let marks = self
            .edits
            .iter()
            .enumerate()
            .filter_map(|(id, (page, edit, _))| {
                if *page != self.index {
                    return None;
                }
                let Edit::Add(mark, bounds) = edit else {
                    return None;
                };
                let (top_left, bottom_right) = offsets.get(&id).copied().unwrap_or_default();
                let (a, b) = (*bounds)?;
                Some((id, mark, translate(a, top_left), translate(b, bottom_right)))
            });
        let (id, mark, mut a, mut b) = marks.clone().rev().find(|(_, _, a, b)| {
            point.0 >= a.0 - tolerance.0
                && point.0 <= b.0 + tolerance.0
                && point.1 >= a.1 - tolerance.1
                && point.1 <= b.1 + tolerance.1
        })?;
        let mut ids = vec![id];
        if let Mark::Ink(_, Some(group)) = mark {
            for (other, mark, left, right) in marks {
                if other != id && matches!(mark, Mark::Ink(_, Some(g)) if g == group) {
                    ids.push(other);
                    a = (a.0.min(left.0), a.1.min(left.1));
                    b = (b.0.max(right.0), b.1.max(right.1));
                }
            }
        }
        Some((ids, (a, b)))
    }

    pub fn move_mark(&mut self, ids: Vec<usize>, delta: Point) -> Result<()> {
        if delta == (0.0, 0.0) {
            return Ok(());
        }
        self.update(Edit::Move(ids, delta))
    }

    pub fn resize_highlight(&mut self, id: usize, delta: (f32, f32)) -> Result<()> {
        if delta == (0.0, 0.0) {
            return Ok(());
        }
        self.update(Edit::ResizeHighlight(id, delta))
    }

    fn update(&mut self, edit: Edit) -> Result<()> {
        let revision = self.next_revision + 1;
        self.edits.push((self.index, edit, revision));
        match self.rebuild(self.edits.len()) {
            Ok(pdf) => {
                self.pdf = pdf;
                self.next_revision = revision;
                Ok(())
            }
            Err(error) => {
                self.edits.pop();
                Err(error)
            }
        }
    }

    fn rebuild(&self, count: usize) -> Result<PdfDocument<'static>> {
        // PDFium 7543 can trap when removing a regenerated image object.
        let mut rebuilt = self
            .engine
            .load_pdf_from_reader(self.original.try_clone()?, None)?;
        let edits = &self.edits[..count];
        let offsets = Self::offsets(edits);
        for (id, (index, edit, _)) in edits.iter().enumerate() {
            if let Edit::Add(mark, _) = edit {
                let (a, b) = offsets.get(&id).copied().unwrap_or_default();
                if let Mark::Highlight(start, end) = mark {
                    Self::apply(
                        &mut rebuilt,
                        *index,
                        &Mark::Highlight(translate(*start, a), translate(*end, b)),
                        (0.0, 0.0),
                    )?;
                } else {
                    Self::apply(&mut rebuilt, *index, mark, a)?;
                }
            }
        }
        Ok(rebuilt)
    }

    pub fn undo(&mut self) -> Result<()> {
        if let Some((page, _, _)) = self.edits.last() {
            self.pdf = self.rebuild(self.edits.len() - 1)?;
            self.index = *page;
            self.edits.pop();
        }
        Ok(())
    }

    pub fn save_copy(&mut self, path: &Path) -> Result<()> {
        if source_is_destination(&self.source, path) {
            return Err("Pilih nama lain agar PDF sumber tetap tersimpan.".into());
        }
        let dir = path.parent().ok_or("Lokasi penyimpanan tidak valid.")?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp = dir.join(format!(".hubbypdf-{}-{stamp}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let saved = self
            .pdf
            .save_to_writer(&mut file)
            .map_err(Box::<dyn std::error::Error>::from)
            .and_then(|()| Ok(file.sync_all()?));
        drop(file);
        let saved = saved.and_then(|()| Ok(fs::rename(&temp, path)?));
        if saved.is_err() {
            let _ = fs::remove_file(&temp);
        }
        saved?;
        self.saved_revision = self.revision();
        Ok(())
    }
}

fn source_is_destination(source: &Path, destination: &Path) -> bool {
    let source = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let destination = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    source
        .to_string_lossy()
        .eq_ignore_ascii_case(&destination.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_render_undo_and_save_copy() -> Result<()> {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = Box::leak(Box::new(crate::load_pdfium(
            &folder.join("vendor/pdfium/bin"),
        )?));
        crate::verify_window_controls(engine)?;
        let work = folder.join("target/pdf-smoke");
        fs::create_dir_all(&work)?;
        let source = work.join("sumber.pdf");
        let output = work.join("salinan.pdf");
        {
            let mut blank = engine.create_new_pdf()?;
            blank
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4())?;
            blank.save_to_file(&source)?;
        }
        let original = fs::read(&source)?;
        let mut doc = Document::open(engine, source.clone())?;
        let rendered = doc.render(420, 594)?;
        assert_eq!(
            rendered.bytes.len(),
            (rendered.width * rendered.height * 4) as usize
        );

        doc.add(Mark::Text((0.10, 0.60), "Uji HubbyPDF".into()))?;
        let (top, bottom) = doc.highlight_band((0.12, 0.605))?;
        let (_, page_height) = doc.size()?;
        assert!(top < 0.605 && bottom > 0.605);
        let height = (bottom - top) * page_height;
        assert!((13.9..17.0).contains(&height), "14 pt text band: {height}");
        let (top, bottom) = doc.highlight_band((0.8, 0.605))?;
        assert!(((bottom - top) * page_height - 12.0).abs() < 0.01);
        doc.add(Mark::Ink(
            vec![(0.15, 0.25), (0.3, 0.30), (0.4, 0.26)],
            None,
        ))?;
        doc.add(Mark::Ink(
            vec![(0.20, 0.40), (0.25, 0.43), (0.34, 0.37)],
            Some(1),
        ))?;
        doc.add(Mark::Highlight((0.09, 0.18), (0.25, 0.21)))?;
        let rgba = image::RgbaImage::from_pixel(10, 4, image::Rgba([20, 40, 180, 150]));
        doc.add(Mark::Signature((0.1, 0.45), (0.3, 0.5), rgba.into()))?;
        for (start, delta) in [
            ((0.12, 0.605), (0.4, 0.0)),
            ((0.2, 0.28), (0.4, 0.0)),
            ((0.24, 0.40), (0.4, 0.0)),
            ((0.15, 0.19), (0.4, 0.0)),
            ((0.18, 0.47), (0.4, 0.0)),
        ] {
            let (ids, _) = doc
                .hit(start, (0.001, 0.001))
                .expect("mark at original position");
            let before = doc.render(420, 594)?.bytes;
            doc.move_mark(ids.clone(), delta)?;
            assert!(doc.hit(start, (0.001, 0.001)).is_none());
            assert_eq!(
                doc.hit(translate(start, delta), (0.001, 0.001)).unwrap().0,
                ids
            );
            assert_ne!(doc.render(420, 594)?.bytes, before);
            doc.undo()?;
            assert_eq!(doc.render(420, 594)?.bytes, before);
        }
        let (ids, _) = doc.hit((0.15, 0.19), (0.0, 0.0)).unwrap();
        let before = doc.render(420, 594)?.bytes;
        doc.resize_highlight(ids[0], (-0.02, 0.0))?;
        doc.resize_highlight(ids[0], (0.0, 0.03))?;
        assert_eq!(doc.hit((0.15, 0.17), (0.0, 0.0)).unwrap().0, ids);
        assert_eq!(doc.hit((0.15, 0.23), (0.0, 0.0)).unwrap().0, ids);
        assert_ne!(doc.render(420, 594)?.bytes, before);
        doc.undo()?;
        assert!(doc.hit((0.15, 0.23), (0.0, 0.0)).is_none());
        doc.undo()?;
        assert!(doc.hit((0.15, 0.17), (0.0, 0.0)).is_none());
        assert_eq!(doc.render(420, 594)?.bytes, before);
        let (ids, _) = doc.hit((0.18, 0.47), (0.001, 0.001)).unwrap();
        doc.move_mark(ids, (0.4, 0.0))?;
        let (ids, _) = doc.hit((0.12, 0.605), (0.001, 0.001)).unwrap();
        doc.move_mark(ids, (0.4, 0.0))?;
        assert!(doc.dirty());
        assert_ne!(rendered.bytes, doc.render(420, 594)?.bytes);
        assert!(doc.save_copy(&source).is_err());
        doc.save_copy(&output)?;
        assert!(!doc.dirty());
        assert_eq!(fs::read(source)?, original);

        let saved = Document::open(engine, output)?;

        let page = saved.pdf.pages().get(0)?;

        assert_eq!(page.objects().len(), 5);

        assert!(page.text()?.all().contains("Uji HubbyPDF"));
        assert!(page.objects().get(0)?.bounds()?.left().value > 280.0);

        doc.undo()?;
        assert!(doc.dirty());

        assert_eq!(doc.pdf.pages().get(0)?.objects().len(), 5);
        doc.undo()?;
        assert_eq!(doc.pdf.pages().get(0)?.objects().len(), 5);
        doc.undo()?;
        assert_eq!(doc.pdf.pages().get(0)?.objects().len(), 4);
        Ok(())
    }
}
