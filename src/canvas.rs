use std::{collections::BTreeMap, ops::Range};

use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        },
    },
    core::{Result, w},
};

use crate::{
    chrome,
    pdf::{self, Point, Raster},
};

pub struct Canvas {
    target: ID2D1HwndRenderTarget,
    pages: BTreeMap<usize, (D2D_RECT_F, ID2D1Bitmap)>,
    mist: ID2D1LinearGradientBrush,
    night: ID2D1LinearGradientBrush,
    brush: ID2D1SolidColorBrush,
    heading: IDWriteTextFormat,
    body: IDWriteTextFormat,
}

pub struct Scene<'a> {
    pub page: D2D_RECT_F,
    pub has_document: bool,
    pub viewport: D2D_RECT_F,
    pub dpi: f32,
    pub language: chrome::Language,
    pub system_background: Option<D2D1_COLOR_F>,
    pub dark: bool,
    pub toolbar_dividers: &'a [(i32, i32)],
    pub preview: Option<Preview<'a>>,
}

pub enum Preview<'a> {
    Selection(Point, Point, bool),
    Ink(Cow<'a, [Point]>, PdfColor),
    Box(Point, Point, bool),
}

impl Canvas {
    pub fn new(hwnd: HWND, width: u32, height: u32, dpi: f32) -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateHwndRenderTarget(
                &D2D1_RENDER_TARGET_PROPERTIES {
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                },
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd,
                    pixelSize: D2D_SIZE_U { width, height },
                    ..Default::default()
                },
            )?;
            let mist = gradient(&target, [0xe0e5ed, 0xe9edf4, 0xdce3ed])?;
            let night = gradient(&target, [0x141b27, 0x19212f, 0x111723])?;
            let brush = target.CreateSolidColorBrush(&hex(0xffffff), None)?;
            let fonts: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self {
                target,
                pages: BTreeMap::new(),
                mist,
                night,
                brush,
                heading: text_format(&fonts, 31.0 * dpi, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                body: text_format(&fonts, 14.0 * dpi, DWRITE_FONT_WEIGHT_NORMAL)?,
            })
        }
    }

    pub fn resize(&self, width: u32, height: u32) -> Result<()> {
        unsafe { self.target.Resize(&D2D_SIZE_U { width, height }) }
    }

    pub fn retain_pages(&mut self, visible: Range<usize>) {
        self.pages.retain(|index, _| visible.contains(index));
    }

    pub fn set_page(
        &mut self,
        index: usize,
        rect: D2D_RECT_F,
        dimensions: (i32, i32),
        resizing: bool,
        render: impl FnOnce() -> pdf::Result<Raster>,
    ) -> pdf::Result<()> {
        if let Some((current, bitmap)) = self.pages.get_mut(&index) {
            let size = unsafe { bitmap.GetPixelSize() };
            if resizing || (size.width as i32, size.height as i32) == dimensions {
                *current = rect;
                return Ok(());
            }
        }
        let raster = render()?;
        let bitmap = unsafe {
            self.target.CreateBitmap(
                D2D_SIZE_U {
                    width: raster.width as u32,
                    height: raster.height as u32,
                },
                Some(raster.bytes.as_ptr().cast()),
                raster.width as u32 * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )?
        };
        self.pages.insert(index, (rect, bitmap));
        Ok(())
    }

    pub fn draw(&self, scene: Scene<'_>) -> Result<()> {
        let rect = scene.page;
        unsafe {
            let size = self.target.GetSize();
            self.target.BeginDraw();
            if let Some(background) = scene.system_background {
                self.target.Clear(Some(&background));
            } else {
                let backdrop = if scene.dark { &self.night } else { &self.mist };
                backdrop.SetStartPoint(D2D_POINT_2F { x: 0.0, y: 0.0 });
                backdrop.SetEndPoint(D2D_POINT_2F {
                    x: size.width,
                    y: size.height,
                });
                self.target
                    .FillRectangle(&bounds(0.0, 0.0, size.width, size.height), backdrop);
            }
            let d = scene.dpi;
            let viewport = scene.viewport;
            let brush = &self.brush;
            self.target
                .PushAxisAlignedClip(&viewport, D2D1_ANTIALIAS_MODE_ALIASED);
            for &(rect, ref bitmap) in self.pages.values() {
                if scene.system_background.is_none() {
                    brush.SetColor(&if scene.dark {
                        color(0.0, 0.0, 0.0, 0.55)
                    } else {
                        color(0.10, 0.23, 0.24, 0.19)
                    });
                    self.target.FillRectangle(
                        &bounds(
                            rect.left + 3.0 * d,
                            rect.top + 5.0 * d,
                            rect.right + 3.0 * d,
                            rect.bottom + 5.0 * d,
                        ),
                        brush,
                    );
                }
                self.target.DrawBitmap(
                    bitmap,
                    Some(&rect),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
            if !scene.has_document && scene.system_background.is_none() {
                self.draw_empty(&scene);
            }
            let to_screen = |(x, y): Point| D2D_POINT_2F {
                x: rect.left + x * (rect.right - rect.left),
                y: rect.top + y * (rect.bottom - rect.top),
            };
            if let Some(preview) = scene.preview {
                brush.SetColor(&hex(0x213347));
                match preview {
                    Preview::Ink(points, ink) => {
                        brush.SetColor(&color(
                            f32::from(ink.red()) / 255.0,
                            f32::from(ink.green()) / 255.0,
                            f32::from(ink.blue()) / 255.0,
                            1.0,
                        ));
                        for pair in points.windows(2) {
                            self.target.DrawLine(
                                to_screen(pair[0]),
                                to_screen(pair[1]),
                                brush,
                                2.0,
                                None,
                            );
                        }
                    }
                    Preview::Selection(a, b, handles) => {
                        let (a, b) = (to_screen(a), to_screen(b));
                        let pad = if handles { 0.0 } else { 4.0 * scene.dpi };
                        let selection = bounds(a.x - pad, a.y - pad, b.x + pad, b.y + pad);
                        if !handles {
                            brush.SetColor(&color(0.20, 0.31, 0.62, 0.16));
                            self.target.FillRectangle(&selection, brush);
                        }
                        brush.SetColor(&hex(0x32498c));
                        self.target
                            .DrawRectangle(&selection, brush, 2.0 * scene.dpi, None);
                        if handles {
                            let x = (a.x + b.x) / 2.0;
                            let radius = (3.5 * scene.dpi).min((b.y - a.y) * 0.45);
                            for y in [a.y, b.y] {
                                let handle = bounds(x - radius, y - radius, x + radius, y + radius);
                                self.target.FillRectangle(&handle, brush);
                            }
                        }
                    }
                    Preview::Box(a, b, highlight) => {
                        let (a, b) = (to_screen(a), to_screen(b));
                        let bounds = D2D_RECT_F {
                            left: a.x.min(b.x),
                            top: a.y.min(b.y),
                            right: a.x.max(b.x),
                            bottom: a.y.max(b.y),
                        };
                        if highlight {
                            brush.SetColor(&color(1.0, 0.85, 0.24, 0.36));
                            self.target.FillRectangle(&bounds, brush);
                        } else {
                            self.target.DrawRectangle(&bounds, brush, 2.0, None);
                        }
                    }
                }
            }
            self.target.PopAxisAlignedClip();
            if scene.system_background.is_none() {
                brush.SetColor(&hex(if scene.dark {
                    chrome::DARK_SURFACE
                } else {
                    chrome::LIGHT_SURFACE
                }));
                self.target
                    .FillRectangle(&bounds(0.0, 0.0, size.width, viewport.top), brush);
                brush.SetColor(&hex(if scene.dark { 0x46536b } else { 0xbfc8d7 }));
                for &(x, y) in scene.toolbar_dividers {
                    let (x, y) = (x as f32, y as f32);
                    self.target
                        .FillRectangle(&bounds(x, y + 7.0 * d, x + 1.0, y + 29.0 * d), brush);
                }
                self.target.FillRectangle(
                    &bounds(0.0, viewport.top - 1.0, size.width, viewport.top),
                    brush,
                );
                brush.SetColor(&hex(if scene.dark {
                    chrome::DARK_SURFACE
                } else {
                    chrome::LIGHT_SURFACE
                }));
                self.target.FillRectangle(
                    &bounds(
                        0.0,
                        size.height - chrome::FOOTER as f32 * d,
                        size.width,
                        size.height,
                    ),
                    brush,
                );
            }
            self.target.EndDraw(None, None)
        }
    }

    unsafe fn glass(&self, rect: D2D_RECT_F, radius: f32, dark: bool) {
        let shape = D2D1_ROUNDED_RECT {
            rect,
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.brush.SetColor(&if dark {
                color(0.18, 0.22, 0.31, 0.78)
            } else {
                color(1.0, 1.0, 1.0, 0.68)
            });
            self.target.FillRoundedRectangle(&shape, &self.brush);
            self.brush.SetColor(&if dark {
                hex(0x465675)
            } else {
                color(1.0, 1.0, 1.0, 0.92)
            });
            self.target
                .DrawRoundedRectangle(&shape, &self.brush, 1.0, None);
        }
    }

    unsafe fn draw_empty(&self, scene: &Scene<'_>) {
        let d = scene.dpi;
        let center = (scene.viewport.right / 2.0).floor();
        let top = ((scene.viewport.top + scene.viewport.bottom - chrome::HERO_H as f32 * d) / 2.0)
            .floor();
        let card_width = (chrome::HERO_W as f32 * d).min(scene.viewport.right - 24.0 * d);
        let left = center - card_width / 2.0;
        let right = center + card_width / 2.0;
        let bottom = top + chrome::HERO_H as f32 * d;
        unsafe {
            self.brush.SetColor(&color(0.13, 0.31, 0.30, 0.08));
            self.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: bounds(
                        left - 4.0 * d,
                        top + 8.0 * d,
                        right + 4.0 * d,
                        bottom + 8.0 * d,
                    ),
                    radiusX: 23.0 * d,
                    radiusY: 23.0 * d,
                },
                &self.brush,
            );
            self.glass(bounds(left, top, right, bottom), 22.0 * d, scene.dark);
            let tile = D2D1_ROUNDED_RECT {
                rect: bounds(
                    center - 25.0 * d,
                    top + 27.0 * d,
                    center + 25.0 * d,
                    top + 77.0 * d,
                ),
                radiusX: 13.0 * d,
                radiusY: 13.0 * d,
            };
            self.brush
                .SetColor(&hex(if scene.dark { 0x35456c } else { 0xe0e7fb }));
            self.target.FillRoundedRectangle(&tile, &self.brush);
            self.brush
                .SetColor(&hex(if scene.dark { 0xc3d0ff } else { 0x32498c }));
            self.target.DrawRectangle(
                &bounds(
                    center - 9.0 * d,
                    top + 40.0 * d,
                    center + 9.0 * d,
                    top + 64.0 * d,
                ),
                &self.brush,
                1.6 * d,
                None,
            );
            for offset in [49.0, 55.0] {
                self.target.DrawLine(
                    D2D_POINT_2F {
                        x: center - 5.0 * d,
                        y: top + offset * d,
                    },
                    D2D_POINT_2F {
                        x: center + 5.0 * d,
                        y: top + offset * d,
                    },
                    &self.brush,
                    1.5 * d,
                    None,
                );
            }

            self.brush
                .SetColor(&hex(if scene.dark { 0xeff3ff } else { 0x233048 }));
            self.target.DrawText(
                &"HubbyPDF".encode_utf16().collect::<Vec<_>>(),
                &self.heading,
                &bounds(left, top + 87.0 * d, right, top + 132.0 * d),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.brush
                .SetColor(&hex(if scene.dark { 0xc0cce3 } else { 0x465470 }));
            self.target.DrawText(
                &scene
                    .language
                    .text((
                        "Seret dan lepas PDF ke jendela ini.",
                        "Drag and drop a PDF onto this window.",
                    ))
                    .encode_utf16()
                    .collect::<Vec<_>>(),
                &self.body,
                &bounds(left, top + 136.0 * d, right, top + 164.0 * d),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

pub fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn bounds(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn hex(rgb: u32) -> D2D1_COLOR_F {
    color(
        (rgb >> 16) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
        1.0,
    )
}

fn text_format(
    factory: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = factory.CreateTextFormat(
            w!("Segoe UI"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("id-ID"),
        )?;
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        Ok(format)
    }
}

fn gradient(target: &ID2D1HwndRenderTarget, colors: [u32; 3]) -> Result<ID2D1LinearGradientBrush> {
    let stops = std::array::from_fn::<_, 3, _>(|i| D2D1_GRADIENT_STOP {
        position: [0.0, 0.56, 1.0][i],
        color: hex(colors[i]),
    });
    unsafe {
        let stops =
            target.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        target.CreateLinearGradientBrush(&Default::default(), None, &stops)
    }
}
use std::borrow::Cow;

use pdfium_render::prelude::PdfColor;
