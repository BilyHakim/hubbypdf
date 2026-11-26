use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{chrome::Message, forms};
use image::DynamicImage;
use pdfium_render::prelude::*;
use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

pub type Point = (f32, f32);
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub struct LinkTarget {
    pub page: u16,
    pub top: f32,
}

pub const MAX_EDGE: f32 = 4096.0;
pub const MAX_PIXELS: f32 = 9_500_000.0;
type MarkBounds = (Point, Point);

pub struct Raster {
    pub width: i32,
    pub height: i32,
    pub bytes: Vec<u8>,
}

pub enum Mark {
    Ink(Vec<Point>, Option<u64>, PdfColor),
    Highlight(Point, Point),
    Text(Point, String, PdfColor),
    Signature(Point, Point, DynamicImage),
}

impl Mark {
    fn bounds(&self) -> Option<(Point, Point)> {
        let points = match self {
            Self::Ink(points, _, _) => points.as_slice(),
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
    Erase(Vec<usize>),
    Form(usize, forms::Value),
}

struct LinkPage {
    index: u16,
    page: PdfPage<'static>,
    width: i32,
    height: i32,
    config: PdfRenderConfig,
}

struct EditablePdf {
    // Close page handles before their document.
    pages: BTreeMap<u16, PdfPage<'static>>,
    document: PdfDocument<'static>,
}

impl EditablePdf {
    fn new(document: PdfDocument<'static>) -> Self {
        Self {
            pages: BTreeMap::new(),
            document,
        }
    }

    fn with_page<T>(
        &self,
        index: u16,
        read: impl FnOnce(&PdfPage<'static>) -> Result<T>,
    ) -> Result<T> {
        if let Some(page) = self.pages.get(&index) {
            read(page)
        } else {
            read(&self.document.pages().get(index)?)
        }
    }

    fn add(&mut self, index: u16, mark: &Mark, offset: Point) -> Result<Option<MarkBounds>> {
        let page = match self.pages.entry(index) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let mut page = self.document.pages().get(index)?;
                // Serializing page contents can expand shared PDF resources into memory.
                page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
                entry.insert(page)
            }
        };
        Document::apply(&mut self.document, page, mark, offset)
    }
}

pub struct Document {
    // PDFium page handles must close before their document.
    link_page: Option<LinkPage>,
    pdf: EditablePdf,
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
            return Err(
                Message(("PDF ini tidak berisi halaman.", "This PDF has no pages.")).into(),
            );
        }
        Ok(Self {
            link_page: None,
            pdf: EditablePdf::new(pdf),
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
        self.pdf.document.pages().len()
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

    #[cfg(test)]
    pub fn size(&self) -> Result<Point> {
        self.pdf.with_page(self.index, Self::page_size)
    }

    pub fn sizes(&self) -> Result<Vec<Point>> {
        self.pdf
            .document
            .pages()
            .page_sizes()?
            .into_iter()
            .map(|rect| Self::checked_size((rect.width().value, rect.height().value)))
            .collect()
    }

    fn page_size(page: &PdfPage<'_>) -> Result<Point> {
        Self::checked_size((page.width().value, page.height().value))
    }

    fn checked_size(size: Point) -> Result<Point> {
        if !size.0.is_finite() || !size.1.is_finite() || size.0 <= 0.0 || size.1 <= 0.0 {
            return Err(Message((
                "Ukuran halaman PDF tidak valid.",
                "The PDF page size is invalid.",
            ))
            .into());
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

    #[cfg(test)]
    pub fn render(&self, width: i32, height: i32) -> Result<Raster> {
        self.render_page(self.index, width, height)
    }

    pub fn render_page(&self, index: u16, width: i32, height: i32) -> Result<Raster> {
        self.pdf
            .with_page(index, |page| Self::rasterize(page, width, height))
    }

    pub fn thumbnail(&self, index: u16, width: i32, height: i32) -> Result<Raster> {
        self.pdf.with_page(index, |page| {
            let (w, h) = Self::page_size(page)?;
            let scale = (width as f32 / w).min(height as f32 / h);
            Self::rasterize(
                page,
                (w * scale).round().max(1.0) as i32,
                (h * scale).round().max(1.0) as i32,
            )
        })
    }

    fn rasterize(page: &PdfPage<'_>, width: i32, height: i32) -> Result<Raster> {
        let bitmap = page.render_with_config(&Self::config(width, height))?;
        Ok(Raster {
            width: bitmap.width(),
            height: bitmap.height(),
            bytes: bitmap.as_raw_bytes(),
        })
    }

    pub fn highlight_band(&self, at: Point) -> Result<(f32, f32)> {
        self.pdf
            .with_page(self.index, |page| Self::page_highlight_band(page, at))
    }

    fn page_highlight_band(page: &PdfPage<'_>, at: Point) -> Result<(f32, f32)> {
        let size = Self::page_size(page)?;
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

    pub fn form_at(&self, point: Point) -> Result<Option<forms::Field>> {
        self.form_at_page(self.index, point)
    }

    fn form_at_page(&self, index: u16, point: Point) -> Result<Option<forms::Field>> {
        if !self.has_form() {
            return Ok(None);
        }
        self.pdf.with_page(index, |page| {
            let (width, height, config) = Self::edit_coordinates(Self::page_size(page)?);
            forms::hit(page, point, width, height, &config)
        })
    }

    fn with_link_destination<T>(
        &mut self,
        source: u16,
        point: Point,
        resolve: impl FnOnce(&PdfPages<'static>, u16, &PdfDestination<'_>) -> T,
    ) -> Option<T> {
        let pages = self.pdf.document.pages();
        if self
            .link_page
            .as_ref()
            .is_none_or(|page| page.index != source)
        {
            let page = pages.get(source).ok()?;
            let (width, height, config) = Self::edit_coordinates(Self::page_size(&page).ok()?);
            self.link_page = Some(LinkPage {
                index: source,
                page,
                width,
                height,
                config,
            });
        }
        let source = self.link_page.as_ref()?;
        let (x, y) = source
            .page
            .pixels_to_points(
                (point.0 * source.width as f32).round() as i32,
                (point.1 * source.height as f32).round() as i32,
                &source.config,
            )
            .ok()?;
        let link = source.page.links().link_at_point(x, y)?;
        let action = link.action();
        let dest = link.destination().or_else(|| {
            action
                .as_ref()?
                .as_local_destination_action()?
                .destination()
                .ok()
        })?;
        let index = dest
            .page_index()
            .ok()
            .filter(|&index| index < pages.len())?;
        Some(resolve(pages, index, &dest))
    }

    pub fn has_link_at(&mut self, source: u16, point: Point, tolerance: Point) -> bool {
        self.with_link_destination(source, point, |_, _, _| ())
            .is_some()
            && self
                .form_at_page(source, point)
                .is_ok_and(|field| field.is_none())
            && self.hit_mark(source, point, tolerance, false).is_none()
    }

    pub fn link_at(&mut self, point: Point) -> Option<LinkTarget> {
        self.with_link_destination(self.index, point, |pages, index, dest| {
            let (left, top) = match dest.view_settings() {
                Ok(PdfDestinationViewSettings::SpecificCoordinatesAndZoom(x, y, _)) => (x, y),
                Ok(PdfDestinationViewSettings::FitPageHorizontallyToWindow(y))
                | Ok(PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(y)) => (None, y),
                Ok(PdfDestinationViewSettings::FitPageVerticallyToWindow(x))
                | Ok(PdfDestinationViewSettings::FitBoundsVerticallyToWindow(x)) => (x, None),
                Ok(PdfDestinationViewSettings::FitPageToRectangle(rect)) => {
                    (Some(rect.left()), Some(rect.top()))
                }
                _ => (None, None),
            };
            let top = (left.is_some() || top.is_some())
                .then_some(index)
                .and_then(|index| {
                    let target = pages.get(index).ok()?;
                    let (_, height, config) =
                        Self::edit_coordinates(Self::page_size(&target).ok()?);
                    let (x, y) = target.pixels_to_points(0, 0, &config).ok()?;
                    let (_, pixels) = target
                        .points_to_pixels(
                            left.filter(|point| point.value.is_finite()).unwrap_or(x),
                            top.filter(|point| point.value.is_finite()).unwrap_or(y),
                            &config,
                        )
                        .ok()?;
                    Some((pixels as f32 / height as f32).clamp(0.0, 1.0))
                })
                .unwrap_or(0.0);
            LinkTarget { page: index, top }
        })
    }

    pub fn has_form(&self) -> bool {
        self.pdf
            .document
            .form()
            .is_some_and(|form| matches!(form.form_type(), PdfFormType::Acrobat))
    }

    pub fn fill(&mut self, index: usize, value: forms::Value) -> Result<()> {
        self.update(Edit::Form(index, value))
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
        let bounds = self.pdf.add(self.index, &mark, (0.0, 0.0))?;
        self.next_revision += 1;
        self.edits
            .push((self.index, Edit::Add(mark, bounds), self.next_revision));
        Ok(self.edits.len() - 1)
    }

    fn apply(
        pdf: &mut PdfDocument<'static>,
        page: &mut PdfPage<'static>,
        mark: &Mark,
        offset: Point,
    ) -> Result<Option<MarkBounds>> {
        let font = matches!(mark, Mark::Text(..)).then(|| pdf.fonts_mut().helvetica());
        let page_size = Self::page_size(page)?;
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
            Mark::Ink(points, _, color) => {
                let (x, y) = to_pdf(points[0])?;
                let mut path = PdfPagePathObject::new(
                    pdf,
                    x,
                    y,
                    Some(*color),
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
            Mark::Text(at, text, color) => {
                let mut object =
                    PdfPageTextObject::new(pdf, text, font.unwrap(), PdfPoints::new(14.0))?;
                object.set_fill_color(*color)?;
                let bounds = object.bounds()?;
                let width = bounds.width().value / page_size.0;
                let height = bounds.height().value / page_size.1;
                if width >= 1.0 || height >= 1.0 {
                    return Err(Message((
                        "Teks terlalu panjang untuk halaman ini. Masukkan teks yang lebih pendek.",
                        "The text is too long for this page. Enter shorter text.",
                    ))
                    .into());
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
                Edit::Add(..) | Edit::Erase(_) | Edit::Form(..) => {}
            }
        }
        offsets
    }

    fn erased(edits: &[(u16, Edit, u64)]) -> HashSet<usize> {
        edits
            .iter()
            .flat_map(|(_, edit, _)| match edit {
                Edit::Erase(ids) => ids.as_slice(),
                _ => &[],
            })
            .copied()
            .collect()
    }

    pub fn is_highlight(&self, id: usize) -> bool {
        matches!(
            self.edits.get(id),
            Some((_, Edit::Add(Mark::Highlight(..), _), _))
        )
    }

    pub fn hit(&self, point: Point, tolerance: Point) -> Option<(Vec<usize>, (Point, Point))> {
        self.hit_mark(self.index, point, tolerance, false)
    }

    pub fn select_area(&self, start: Point, end: Point) -> Option<(Vec<usize>, MarkBounds)> {
        let left = (start.0.min(end.0), start.1.min(end.1));
        let right = (start.0.max(end.0), start.1.max(end.1));
        let offsets = Self::offsets(&self.edits);
        let erased = Self::erased(&self.edits);
        let mut groups: HashMap<usize, Vec<(usize, MarkBounds)>> = HashMap::new();
        for (id, (page, edit, _)) in self.edits.iter().enumerate() {
            if *page != self.index || erased.contains(&id) {
                continue;
            }
            if let Edit::Add(mark, Some((a, b))) = edit {
                let (da, db) = offsets.get(&id).copied().unwrap_or_default();
                // Signature strokes form one selectable element.
                let key = if let Mark::Ink(_, Some(group), _) = mark {
                    self.edits
                        .iter()
                        .position(|(p, e, _)| {
                            *p == self.index
                                && matches!(e, Edit::Add(Mark::Ink(_, Some(g), _), _) if g == group)
                        })
                        .unwrap_or(id)
                } else {
                    id
                };
                groups
                    .entry(key)
                    .or_default()
                    .push((id, (translate(*a, da), translate(*b, db))));
            }
        }
        let mut ids = Vec::new();
        let mut a = (1.0_f32, 1.0_f32);
        let mut b = (0.0_f32, 0.0_f32);
        for marks in groups.values() {
            if marks.iter().all(|(_, (a, b))| {
                a.0 >= left.0 && a.1 >= left.1 && b.0 <= right.0 && b.1 <= right.1
            }) {
                for (id, (min, max)) in marks {
                    ids.push(*id);
                    a = (a.0.min(min.0), a.1.min(min.1));
                    b = (b.0.max(max.0), b.1.max(max.1));
                }
            }
        }
        ids.sort_unstable();
        (!ids.is_empty()).then_some((ids, (a, b)))
    }

    fn hit_mark(
        &self,
        index: u16,
        point: Point,
        tolerance: Point,
        precise_ink: bool,
    ) -> Option<(Vec<usize>, MarkBounds)> {
        let offsets = Self::offsets(&self.edits);
        let erased = Self::erased(&self.edits);
        let marks = self
            .edits
            .iter()
            .enumerate()
            .filter_map(|(id, (page, edit, _))| {
                if *page != index || erased.contains(&id) {
                    return None;
                }
                let Edit::Add(mark, bounds) = edit else {
                    return None;
                };
                let (top_left, bottom_right) = offsets.get(&id).copied().unwrap_or_default();
                let (a, b) = (*bounds)?;
                Some((id, mark, translate(a, top_left), translate(b, bottom_right)))
            });
        let (id, mark, mut a, mut b) = marks.clone().rev().find(|(id, mark, a, b)| {
            point.0 >= a.0 - tolerance.0
                && point.0 <= b.0 + tolerance.0
                && point.1 >= a.1 - tolerance.1
                && point.1 <= b.1 + tolerance.1
                && match mark {
                    Mark::Ink(points, _, _) if precise_ink => Self::near_stroke(
                        points,
                        translate(
                            point,
                            offsets.get(id).map_or((0.0, 0.0), |(a, _)| (-a.0, -a.1)),
                        ),
                        tolerance,
                    ),
                    _ => true,
                }
        })?;
        let mut ids = vec![id];
        if let Mark::Ink(_, Some(group), _) = mark {
            for (other, mark, left, right) in marks {
                if other != id && matches!(mark, Mark::Ink(_, Some(g), _) if g == group) {
                    ids.push(other);
                    a = (a.0.min(left.0), a.1.min(left.1));
                    b = (b.0.max(right.0), b.1.max(right.1));
                }
            }
        }
        Some((ids, (a, b)))
    }

    fn near_stroke(points: &[Point], point: Point, tolerance: Point) -> bool {
        let relative = |p: Point| ((p.0 - point.0) / tolerance.0, (p.1 - point.1) / tolerance.1);
        points.windows(2).any(|pair| {
            let (x, y) = relative(pair[0]);
            let (end_x, end_y) = relative(pair[1]);
            let (dx, dy) = (end_x - x, end_y - y);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (-(x * dx + y * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (x + t * dx).hypot(y + t * dy) <= 1.0
        })
    }

    pub fn erase_at(&mut self, point: Point, tolerance: Point) -> Result<bool> {
        let Some((ids, _)) = self.hit_mark(self.index, point, tolerance, true) else {
            return Ok(false);
        };
        self.update(Edit::Erase(ids))?;
        Ok(true)
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
                self.link_page = None;
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

    fn rebuild(&self, count: usize) -> Result<EditablePdf> {
        // PDFium 7543 can trap when removing a regenerated image object.
        let edits = &self.edits[..count];
        let mut form_changes = edits
            .iter()
            .filter_map(|(page, edit, _)| match edit {
                Edit::Form(index, value) => Some((*page, *index, value)),
                _ => None,
            })
            .peekable();
        let mut original = self.original.try_clone()?;
        let rebuilt = if form_changes.peek().is_some() {
            let mut bytes = Vec::new();
            original.seek(SeekFrom::Start(0))?;
            original.read_to_end(&mut bytes)?;
            let filled = forms::apply(self.engine.bindings(), &bytes, form_changes)?;
            self.engine.load_pdf_from_byte_vec(filled, None)?
        } else {
            self.engine.load_pdf_from_reader(original, None)?
        };
        let mut rebuilt = EditablePdf::new(rebuilt);
        let offsets = Self::offsets(edits);
        let erased = Self::erased(edits);
        for (id, (index, edit, _)) in edits.iter().enumerate() {
            if let Edit::Add(mark, _) = edit
                && !erased.contains(&id)
            {
                let (a, b) = offsets.get(&id).copied().unwrap_or_default();
                if let Mark::Highlight(start, end) = mark {
                    rebuilt.add(
                        *index,
                        &Mark::Highlight(translate(*start, a), translate(*end, b)),
                        (0.0, 0.0),
                    )?;
                } else {
                    rebuilt.add(*index, mark, a)?;
                }
            }
        }
        Ok(rebuilt)
    }

    pub fn undo(&mut self) -> Result<()> {
        if let Some((page, _, _)) = self.edits.last() {
            let pdf = self.rebuild(self.edits.len() - 1)?;
            self.link_page = None;
            self.pdf = pdf;
            self.index = *page;
            self.edits.pop();
        }
        Ok(())
    }

    pub fn save_copy(&mut self, path: &Path) -> Result<()> {
        if source_is_destination(&self.source, path) {
            return Err(Message((
                "Pilih nama lain agar PDF sumber tetap tersimpan.",
                "Choose another name to preserve the original PDF.",
            ))
            .into());
        }
        let dir = path.parent().ok_or(Message((
            "Lokasi penyimpanan tidak valid.",
            "The save location is invalid.",
        )))?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp = dir.join(format!(".hubbypdf-{}-{stamp}.tmp", std::process::id()));
        let export = if self.pdf.pages.is_empty() {
            None
        } else {
            // Drop the export document and the expanded resources once the save finishes.
            let mut export = self.rebuild(self.edits.len())?;
            for page in export.pages.values_mut() {
                page.regenerate_content()?;
            }
            Some(export)
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let saved = export
            .as_ref()
            .unwrap_or(&self.pdf)
            .document
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

    fn form_roundtrip(engine: &'static Pdfium, source: &Path, output: &Path) -> Result<()> {
        let mut doc = Document::open(engine, source.to_owned())?;
        assert!(doc.has_form());
        let (text, choice, checkbox, read_only) = {
            let page = doc.pdf.document.pages().get(0)?;
            let config = Document::edit_coordinates(Document::page_size(&page)?);
            let widgets = page.annotations();
            let mut text = None;
            let mut choice = None;
            let mut checkbox = None;
            let mut read_only = None;
            for (i, annot) in widgets.iter().enumerate() {
                let Some(field) = annot.as_form_field() else {
                    continue;
                };
                let rect = annot.bounds()?;
                let (x, y) = page.points_to_pixels(
                    PdfPoints::new((rect.left().value + rect.right().value) / 2.0),
                    PdfPoints::new((rect.top().value + rect.bottom().value) / 2.0),
                    &config.2,
                )?;
                let at = (x as f32 / config.0 as f32, y as f32 / config.1 as f32);
                if !doc.form_at(at)?.is_some_and(|found| found.index == i) {
                    continue;
                }
                if field.is_read_only() {
                    read_only.get_or_insert(at);
                } else {
                    match field {
                        PdfFormField::Text(field)
                            if !field.is_file_select() && !field.is_password() =>
                        {
                            if text.is_none() {
                                text = Some((i, at, field.value()));
                            }
                        }
                        PdfFormField::ComboBox(field) => {
                            if let Some(option) =
                                field.options().iter().find(|option| !option.is_set())
                                && choice.is_none()
                            {
                                choice = Some((i, option.index() as i32));
                            }
                        }
                        PdfFormField::Checkbox(field) if checkbox.is_none() => {
                            checkbox = Some((i, field.group_value()));
                        }
                        _ => {}
                    }
                }
            }
            (
                text.unwrap(),
                choice.unwrap(),
                checkbox.unwrap(),
                read_only.unwrap(),
            )
        };
        assert!(matches!(
            doc.form_at(read_only)?.unwrap().control,
            forms::Control::ReadOnly
        ));
        let before = doc.render(420, 594)?.bytes;
        let replacement = "Uji baru";
        doc.fill(text.0, forms::Value::Text(replacement.into()))?;
        assert_ne!(doc.render(420, 594)?.bytes, before);
        doc.undo()?;
        let forms::Control::Text { value, .. } = doc.form_at(text.1)?.unwrap().control else {
            panic!("text field");
        };
        assert_eq!(value, text.2.unwrap_or_default());
        doc.fill(text.0, forms::Value::Text(replacement.into()))?;
        doc.fill(choice.0, forms::Value::Choice(choice.1))?;
        doc.fill(checkbox.0, forms::Value::Toggle)?;
        doc.add(Mark::Ink(
            vec![(0.01, 0.01), (0.02, 0.02)],
            None,
            PdfColor::BLUE,
        ))?;
        doc.save_copy(output)?;
        let saved = Document::open(engine, output.to_owned())?;
        let page = saved.pdf.document.pages().get(0)?;
        let widgets = page.annotations();
        let a = widgets.get(text.0)?;
        let b = widgets.get(choice.0)?;
        let c = widgets.get(checkbox.0)?;
        assert_eq!(
            a.as_form_field()
                .unwrap()
                .as_text_field()
                .unwrap()
                .value()
                .as_deref(),
            Some(replacement)
        );
        let choice_field = b.as_form_field().unwrap().as_combo_box_field().unwrap();
        assert_eq!(
            choice_field
                .options()
                .iter()
                .find(|option| option.is_set())
                .unwrap()
                .index() as i32,
            choice.1
        );
        assert_ne!(
            c.as_form_field()
                .unwrap()
                .as_checkbox_field()
                .unwrap()
                .group_value(),
            checkbox.1
        );
        assert_eq!(doc.render(420, 594)?.bytes, saved.render(420, 594)?.bytes);
        Ok(())
    }

    fn form_fixture() -> Vec<u8> {
        pdf_fixture(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R 6 0 R 7 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 500] /Resources << >> /Annots [4 0 R 5 0 R 6 0 R 7 0 R] >>",
            "<< /Type /Annot /Subtype /Widget /Rect [50 420 280 449] /P 3 0 R /FT /Tx /T (Text) /V (Old) /F 4 >>",
            "<< /Type /Annot /Subtype /Widget /Rect [50 360 280 389] /P 3 0 R /FT /Ch /Ff 131072 /T (Choice) /Opt [[(one) (First)] [(two) (Second)]] /V (one) /F 4 >>",
            "<< /Type /Annot /Subtype /Widget /Rect [50 300 76 326] /P 3 0 R /FT /Btn /T (Check) /V /Off /AS /Off /AP << /N << /Off 9 0 R /Checked 10 0 R >> >> /F 4 >>",
            "<< /Type /Annot /Subtype /Widget /Rect [50 240 280 269] /P 3 0 R /FT /Tx /Ff 1 /T (ReadOnly) /V (Locked) /F 4 >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 26 26] /Length 0 >>\nstream\n\nendstream",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 26 26] /Length 12 >>\nstream\n0 0 26 26 re\nendstream",
        ])
    }

    fn link_fixture() -> Vec<u8> {
        pdf_fixture(&[
            "<< /Type /Catalog /Pages 2 0 R /Dests << /chapter [4 0 R /XYZ 0 500 0] >> >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> /Annots [5 0 R 6 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 760 240 784] /Dest /chapter >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 720 240 744] /A << /S /GoTo /D [3 0 R /XYZ 0 470 0] >> >>",
        ])
    }

    fn pdf_fixture(objects: &[&str]) -> Vec<u8> {
        let mut pdf = String::from("%PDF-1.7\n");
        let mut xref = String::from("0000000000 65535 f \n");
        use std::fmt::Write as _;
        for (index, object) in objects.iter().enumerate() {
            writeln!(xref, "{:010} 00000 n ", pdf.len()).unwrap();
            writeln!(pdf, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let start = pdf.len();
        writeln!(pdf, "xref\n0 {size}\n{xref}trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF", size = objects.len() + 1).unwrap();
        pdf.into_bytes()
    }

    #[test]
    fn edit_render_undo_and_save_copy() -> Result<()> {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = Box::leak(Box::new(crate::load_pdfium(
            &folder.join("vendor/pdfium/bin"),
            crate::chrome::Language::Indonesian,
        )?));
        let work = folder.join("target/pdf-smoke");
        fs::create_dir_all(&work)?;
        let form = work.join("acroform.pdf");
        fs::write(&form, form_fixture())?;
        let links = work.join("toc.pdf");
        fs::write(&links, link_fixture())?;
        form_roundtrip(engine, &form, &work.join("acroform-filled.pdf"))?;
        if let Some(sample) = std::env::var_os("HUBBYPDF_TEST_FORM") {
            form_roundtrip(
                engine,
                Path::new(&sample),
                &work.join("form-sample-filled.pdf"),
            )?;
        }
        let source = work.join("sumber.pdf");
        let output = work.join("salinan.pdf");
        {
            let mut blank = engine.create_new_pdf()?;
            blank
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4())?;
            blank
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4().landscape())?;
            blank.save_to_file(&source)?;
        }
        crate::verify_window_controls(engine, &source, &links)?;
        let original = fs::read(&source)?;
        let mut doc = Document::open(engine, source.clone())?;
        let rendered = doc.render(420, 594)?;
        assert_eq!(
            rendered.bytes.len(),
            (rendered.width * rendered.height * 4) as usize
        );

        doc.add(Mark::Text(
            (0.10, 0.60),
            "Uji HubbyPDF".into(),
            PdfColor::RED,
        ))?;
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
            PdfColor::BLACK,
        ))?;
        doc.add(Mark::Ink(
            vec![(0.20, 0.40), (0.25, 0.43), (0.34, 0.37)],
            Some(1),
            PdfColor::BLUE,
        ))?;
        doc.add(Mark::Highlight((0.09, 0.18), (0.25, 0.21)))?;
        let rgba = image::RgbaImage::from_pixel(10, 4, image::Rgba([20, 40, 180, 150]));
        doc.add(Mark::Signature((0.1, 0.45), (0.3, 0.5), rgba.into()))?;
        let tolerance = (0.003, 0.003);
        let before = doc.render(420, 594)?.bytes;
        let (selected, bounds) = doc.select_area((0.0, 0.0), (1.0, 1.0)).unwrap();
        assert_eq!(selected.len(), 5);
        assert_eq!(doc.select_area((1.0, 1.0), (0.0, 0.0)).unwrap().0, selected);
        assert!(doc.select_area((0.8, 0.8), (0.9, 0.9)).is_none());
        assert!(doc.select_area((0.16, 0.26), (0.20, 0.28)).is_none());
        doc.move_mark(selected.clone(), (0.1, 0.1))?;
        assert_eq!(
            doc.select_area(
                translate(bounds.0, (0.1, 0.1)),
                translate(bounds.1, (0.1, 0.1))
            )
            .unwrap()
            .0,
            selected
        );
        doc.undo()?;
        assert_eq!(doc.render(420, 594)?.bytes, before);
        assert!(!doc.erase_at((0.16, 0.29), tolerance)?);
        for point in [
            (0.12, 0.605),
            (0.15, 0.25),
            (0.25, 0.43),
            (0.15, 0.19),
            (0.18, 0.47),
        ] {
            assert!(doc.erase_at(point, tolerance)?);
            assert!(doc.hit(point, tolerance).is_none());
            assert_ne!(doc.render(420, 594)?.bytes, before);
            doc.undo()?;
            assert_eq!(doc.render(420, 594)?.bytes, before);
        }
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
        assert!(doc.erase_at((0.58, 0.47), tolerance)?);
        assert!(doc.dirty());
        assert_ne!(rendered.bytes, doc.render(420, 594)?.bytes);
        assert!(doc.save_copy(&source).is_err());
        doc.save_copy(&output)?;
        assert!(!doc.dirty());
        assert_eq!(doc.pdf.document.pages().get(0)?.objects().len(), 0);
        assert_eq!(fs::read(source)?, original);

        let mut saved = Document::open(engine, output.clone())?;
        assert!(!saved.erase_at((0.52, 0.605), tolerance)?);

        let page = saved.pdf.document.pages().get(0)?;

        assert_eq!(page.objects().len(), 4);

        assert!(page.text()?.all().contains("Uji HubbyPDF"));
        let fill = page.objects().get(0)?.fill_color()?;
        assert_eq!((fill.red(), fill.green(), fill.blue()), (255, 0, 0));
        assert!(page.objects().get(0)?.bounds()?.left().value > 280.0);
        drop(page);
        drop(saved);

        doc.undo()?;
        assert!(doc.dirty());

        let object_count = |doc: &Document| doc.pdf.with_page(0, |page| Ok(page.objects().len()));
        assert_eq!(object_count(&doc)?, 5);
        doc.undo()?;
        assert_eq!(object_count(&doc)?, 5);
        doc.undo()?;
        assert_eq!(object_count(&doc)?, 5);
        doc.undo()?;
        assert_eq!(object_count(&doc)?, 4);
        let first = doc.thumbnail(0, 52, 72)?;
        assert!(first.height > first.width);
        let second = doc.thumbnail(1, 52, 72)?;
        assert!(second.width > second.height);
        assert_eq!(doc.index, 0);
        doc.index = 1;
        let tail = (0.2, 0.4);
        let tip = (0.7, 0.4);
        let points = crate::drawing::Shape::Arrow.points(tail, tip, doc.size()?);
        assert!(points[2].0 < tip.0 && points[2].1 > tip.1 && points[4].1 < tip.1);
        doc.add(Mark::Ink(points, None, PdfColor::BLUE))?;
        let marked = doc.thumbnail(1, 52, 72)?;
        assert_ne!(marked.bytes, second.bytes);
        assert!(doc.erase_at((0.5, 0.4), tolerance)?);
        assert_eq!(doc.thumbnail(1, 52, 72)?.bytes, second.bytes);
        doc.undo()?;
        assert_eq!(doc.thumbnail(1, 52, 72)?.bytes, marked.bytes);
        doc.save_copy(&output)?;
        let saved = Document::open(engine, output)?;
        assert_eq!(object_count(&saved)?, 4);
        assert_eq!(saved.pdf.document.pages().get(1)?.objects().len(), 1);
        assert!(saved.thumbnail(1, 52, 72)?.bytes != second.bytes);
        Ok(())
    }
}
