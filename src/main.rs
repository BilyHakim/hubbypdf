#![windows_subsystem = "windows"]

mod canvas;
mod chrome;
mod pdf;

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use canvas::{Canvas, Preview, Scene};
use chrome::{Label, Language, Message};
use image::{DynamicImage, ImageReader};
use pdf::{Document, Mark, Point, Result};
use pdfium_render::prelude::Pdfium;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Direct2D::Common::D2D_RECT_F, Dwm::*, Gdi::*},
        System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_CENTERIMAGE},
        UI::{
            Controls::{DRAWITEMSTRUCT, Dialogs::*, EM_SETCUEBANNER, EM_SETLIMITTEXT},
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

const OPEN: u16 = 101;
const SAVE: u16 = 102;
const EXIT: u16 = 103;
const PREVIOUS: u16 = 110;
const NEXT: u16 = 111;
const ZOOM_OUT: u16 = 112;
const ZOOM_IN: u16 = 113;
const FIT: u16 = 114;
const READ: u16 = 120;
const TEXT: u16 = 121;
const HIGHLIGHT: u16 = 122;
const SKETCH: u16 = 123;
const SIGN: u16 = 124;
const PNG: u16 = 125;
const UNDO: u16 = 126;
const ERASE: u16 = 127;
const ZOOM_PICKER: u16 = 130;
const THEME: u16 = 131;
const MENU_FILE: u16 = 140;
const MENU_VIEW: u16 = 141;
const MENU_TOOLS: u16 = 142;
const MENU_LANGUAGE: u16 = 143;
const INDONESIAN: u16 = 144;
const ENGLISH: u16 = 145;
const MENU_ROW: i32 = 28;
const MENU_BUTTONS: &[(u16, Label)] = &[
    (MENU_FILE, ("File", "File")),
    (MENU_VIEW, ("Tampilan", "View")),
    (MENU_TOOLS, ("Alat", "Tools")),
    (MENU_LANGUAGE, ("Bahasa", "Language")),
];
const FIT_LABEL: Label = ("Pas jendela", "Fit window");
const OPEN_LABEL: Label = ("Buka PDF…", "Open PDF…");

#[derive(Clone, Copy, Default)]
struct PaintColors {
    status: HBRUSH,
    editor: HBRUSH,
    background: COLORREF,
    editor_background: COLORREF,
    foreground: COLORREF,
}

impl PaintColors {
    fn delete(self) {
        unsafe {
            let _ = DeleteObject(self.status);
            let _ = DeleteObject(self.editor);
        }
    }
}
const WM_SYNC_ZOOM_LABEL: u32 = WM_APP + 1;
const ZOOM_LEVELS: &[u16] = &[
    25, 40, 50, 60, 75, 80, 90, 100, 110, 120, 125, 140, 150, 175, 200, 250, 300, 400, 500,
];

struct Drag {
    ids: Vec<usize>,
    start: Point,
    bounds: (Point, Point),
    delta: Point,
    edge: Option<HighlightEdge>,
}

#[derive(Clone, Copy)]
enum HighlightEdge {
    Top,
    Bottom,
}

impl Drag {
    fn update(&mut self, point: Point) {
        let (a, b) = self.bounds;
        let dy = point.1 - self.start.1;
        let room = (b.1 - a.1 - 0.001).max(0.0);
        self.delta = match self.edge {
            Some(HighlightEdge::Top) => (0.0, dy.clamp(-a.1, room)),
            Some(HighlightEdge::Bottom) => (0.0, dy.clamp(-room, 1.0 - b.1)),
            None => (
                (point.0 - self.start.0).clamp(-a.0, 1.0 - b.0),
                dy.clamp(-a.1, 1.0 - b.1),
            ),
        };
    }

    fn translated(&self) -> (Point, Point) {
        (
            match self.edge {
                Some(HighlightEdge::Bottom) => self.bounds.0,
                _ => pdf::translate(self.bounds.0, self.delta),
            },
            match self.edge {
                Some(HighlightEdge::Top) => self.bounds.1,
                _ => pdf::translate(self.bounds.1, self.delta),
            },
        )
    }
}

const FILE_COMMANDS: &[(u16, Label)] = &[
    (OPEN, ("&Buka PDF…\tCtrl+O", "&Open PDF…\tCtrl+O")),
    (
        SAVE,
        (
            "&Simpan salinan…\tCtrl+Shift+S",
            "&Save a copy…\tCtrl+Shift+S",
        ),
    ),
    (EXIT, ("&Keluar", "E&xit")),
];
const VIEW_COMMANDS: &[(u16, Label)] = &[
    (
        PREVIOUS,
        ("&Halaman sebelumnya\tPage Up", "&Previous page\tPage Up"),
    ),
    (
        NEXT,
        ("Halaman &berikutnya\tPage Down", "&Next page\tPage Down"),
    ),
    (ZOOM_IN, ("&Perbesar\tCtrl++", "Zoom &in\tCtrl++")),
    (ZOOM_OUT, ("Per&kecil\tCtrl+-", "Zoom &out\tCtrl+-")),
    (FIT, ("&Pas ke jendela\tCtrl+0", "&Fit window\tCtrl+0")),
    (THEME, ("Tema &gelap", "&Dark theme")),
];
const TOOLS: &[(u16, Label)] = &[
    (READ, ("&Pilih / geser\tEsc", "&Select / move\tEsc")),
    (TEXT, ("&Tambah teks…", "Add &text…")),
    (HIGHLIGHT, ("So&rot", "&Highlight")),
    (SKETCH, ("&Sketsa", "S&ketch")),
    (SIGN, ("&Gambar tanda tangan", "&Draw signature")),
    (PNG, ("I&mpor tanda tangan PNG…", "&Import PNG signature…")),
    (ERASE, ("&Hapus objek", "&Erase object")),
    (
        UNDO,
        (
            "&Batalkan perubahan terakhir\tCtrl+Z",
            "&Undo last edit\tCtrl+Z",
        ),
    ),
];
const LANGUAGES: &[(u16, Label)] = &[
    (INDONESIAN, ("Bahasa &Indonesia", "Bahasa &Indonesia")),
    (ENGLISH, ("&English", "&English")),
];

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static PAINT_COLORS: Cell<PaintColors> = Cell::new(PaintColors::default());
}

struct App {
    engine: &'static Pdfium,
    doc: Option<Document>,
    canvas: Option<Canvas>,
    status: HWND,
    open_button: HWND,
    toolbar: Vec<(u16, HWND)>,
    menu: HMENU,
    menu_buttons: Vec<(u16, HWND)>,
    font: HFONT,
    action_font: HFONT,
    icon_font: HFONT,
    dark: bool,
    language: Language,
    toolbar_dividers: Vec<(i32, i32)>,
    dpi: f32,
    contrast: bool,
    editor: HWND,
    width: u32,
    height: u32,
    footer: i32,
    header: i32,
    resizing: bool,
    zoom: Option<u16>,
    zoom_picker: HWND,
    drag: Option<Drag>,
    selected_highlight: Option<(usize, (Point, Point))>,
    highlight_band: (f32, f32),
    text_at: Option<Point>,
    signature_group: u64,
    pan: Point,
    size: Point,
    page_rect: D2D_RECT_F,
    tool: u16,
    gesture: Vec<Point>,
    signature: Option<DynamicImage>,
    notice: Option<String>,
}

impl App {
    fn new(engine: &'static Pdfium, language: Language) -> Self {
        Self {
            engine,
            doc: None,
            canvas: None,
            status: HWND::default(),
            open_button: HWND::default(),
            toolbar: Vec::new(),
            menu: HMENU::default(),
            menu_buttons: Vec::new(),
            font: HFONT::default(),
            action_font: HFONT::default(),
            icon_font: HFONT::default(),
            dark: chrome::load_dark(),
            language,
            toolbar_dividers: Vec::new(),
            dpi: 1.0,
            contrast: chrome::high_contrast(),
            editor: HWND::default(),
            width: 0,
            height: 0,
            footer: 28,
            header: chrome::HEADER,
            resizing: false,
            zoom: None,
            zoom_picker: HWND::default(),
            drag: None,
            selected_highlight: None,
            highlight_band: (0.0, 0.0),
            text_at: None,
            signature_group: 0,
            pan: (0.0, 0.0),
            size: (0.0, 0.0),
            page_rect: D2D_RECT_F::default(),
            tool: READ,
            gesture: Vec::new(),
            signature: None,
            notice: None,
        }
    }

    fn resize(&mut self, hwnd: HWND) -> Result<()> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rect)? };
        self.width = rect.right.max(0) as u32;
        self.height = rect.bottom.max(0) as u32;
        let dpi = self.dpi;
        let px = |n: i32| (n as f32 * dpi).round() as i32;
        self.footer = px(chrome::FOOTER);
        let (w, h) = (self.width as i32, self.height as i32);
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(windows::core::Error::from_win32().into());
            }
            let previous_font = SelectObject(dc, self.font);
            let text_width = |label| {
                chrome::text_width(
                    dc,
                    &mut self.language.text(label).encode_utf16().collect::<Vec<_>>(),
                )
            };
            let menu_widths: Vec<_> = MENU_BUTTONS
                .iter()
                .map(|&(_, label)| text_width(label) + 2 * px(12))
                .collect();
            let tool_widths: Vec<_> = chrome::TOOLS
                .iter()
                .map(|&(_, label, _)| {
                    text_width(label) + px(chrome::ICON_SIZE) + px(chrome::ICON_GAP) + 2 * px(10)
                })
                .collect();
            SelectObject(dc, previous_font);
            let _ = DeleteDC(dc);

            MoveWindow(
                self.status,
                px(22),
                (h - self.footer).max(0),
                (w - px(186)).max(0),
                self.footer,
                true,
            )?;
            MoveWindow(
                self.zoom_picker,
                (w - px(145)).max(0),
                (h - self.footer + px(4)).max(0),
                px(129),
                px(chrome::FOOTER - 8),
                true,
            )?;
            let mut menu_left = px(12);
            for ((_, button), width) in self.menu_buttons.iter().zip(menu_widths) {
                MoveWindow(*button, menu_left, px(2), width, px(MENU_ROW - 4), true)?;
                menu_left += width + px(2);
            }
            let mut left = px(16);
            let mut row = 0;
            self.toolbar_dividers.clear();
            let theme_width = *tool_widths.last().unwrap();
            for (index, (&(id, button), &width)) in
                self.toolbar.iter().zip(&tool_widths).enumerate()
            {
                if id == THEME {
                    MoveWindow(
                        button,
                        (w - width - px(16)).max(0),
                        px(MENU_ROW + 10),
                        width,
                        px(36),
                        true,
                    )?;
                    continue;
                }
                let mut limit = if row == 0 {
                    w - theme_width - px(28)
                } else {
                    w - px(16)
                };
                if left + width > limit && left > px(16) {
                    left = px(16);
                    row += 1;
                    limit = w - px(16);
                }
                let top = px(MENU_ROW + 10 + row * 42);
                MoveWindow(button, left, top, width, px(36), true)?;
                if matches!(id, SAVE | PNG)
                    && left + width + px(19) + tool_widths[index + 1] <= limit
                {
                    self.toolbar_dividers.push((left + width + px(9), top));
                    left += width + px(19);
                } else {
                    left += width + px(3);
                }
            }
            self.header = px(MENU_ROW + chrome::HEADER + row * 42);
            let hero_top = (self.header + h - self.footer - px(chrome::HERO_H)) / 2;
            MoveWindow(
                self.open_button,
                (w - px(150)) / 2,
                hero_top + px(180),
                px(150),
                px(42),
                true,
            )?;
        }
        if w > 0 && h > self.footer {
            if let Some(canvas) = &self.canvas
                && canvas.resize(self.width, self.height).is_err()
            {
                self.canvas = None;
            }
            if self.resizing && self.canvas.is_some() {
                self.layout();
                repaint(hwnd);
            } else {
                self.refresh(hwnd)?;
            }
        }
        Ok(())
    }

    fn viewport(&self) -> D2D_RECT_F {
        let top = self.header as f32;
        D2D_RECT_F {
            left: 0.0,
            top,
            right: self.width as f32,
            bottom: (self.height.saturating_sub(self.footer as u32) as f32).max(top),
        }
    }

    fn fit_scale(&self) -> f32 {
        let area = self.viewport();
        ((area.right - 32.0).max(1.0) / self.size.0)
            .min((area.bottom - area.top - 32.0).max(1.0) / self.size.1)
    }

    fn actual_scale(&self) -> f32 {
        self.dpi * 96.0 / 72.0
    }

    fn can_pan(&self) -> bool {
        let area = self.viewport();
        self.page_rect.right - self.page_rect.left + 32.0 > area.right + 0.5
            || self.page_rect.bottom - self.page_rect.top + 32.0 > area.bottom - area.top + 0.5
    }

    fn zoom_to(&mut self, hwnd: HWND, zoom: Option<u16>) -> Result<()> {
        self.zoom = zoom;
        self.drag = None;
        self.gesture.clear();
        if zoom.is_none() {
            self.pan = (0.0, 0.0);
        }
        self.refresh(hwnd)
    }

    fn show_menu(&mut self, hwnd: HWND, id: u16) -> Result<()> {
        let index = usize::from(id - MENU_FILE);
        let picked = unsafe {
            let mut anchor = RECT::default();
            GetWindowRect(self.menu_buttons[index].1, &mut anchor)?;
            let popup = GetSubMenu(self.menu, index as i32);
            TrackPopupMenuEx(
                popup,
                (TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN).0,
                anchor.left,
                anchor.bottom,
                hwnd,
                None,
            )
            .0
        };
        if picked != 0 {
            self.command(hwnd, picked as u16)?;
        }
        Ok(())
    }

    fn show_zoom_menu(&mut self, hwnd: HWND) -> Result<()> {
        if self.doc.is_none() {
            return Ok(());
        }
        let picked = unsafe {
            let mut anchor = RECT::default();
            GetWindowRect(self.zoom_picker, &mut anchor)?;
            let menu = CreatePopupMenu()?;
            let result = (|| -> windows::core::Result<i32> {
                let flag = |selected| MF_STRING | if selected { MF_CHECKED } else { MF_UNCHECKED };
                AppendMenuW(
                    menu,
                    flag(self.zoom.is_none()),
                    1,
                    PCWSTR(wide(self.language.text(FIT_LABEL)).as_ptr()),
                )?;
                AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
                for (i, level) in ZOOM_LEVELS.iter().enumerate() {
                    let label = wide(&format!("{level}%"));
                    AppendMenuW(
                        menu,
                        flag(self.zoom == Some(*level)),
                        i + 2,
                        PCWSTR(label.as_ptr()),
                    )?;
                }
                Ok(TrackPopupMenuEx(
                    menu,
                    (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTALIGN | TPM_BOTTOMALIGN).0,
                    anchor.right,
                    anchor.top,
                    hwnd,
                    None,
                )
                .0)
            })();
            let _ = DestroyMenu(menu);
            result?
        };
        match picked {
            0 => Ok(()),
            1 => self.zoom_to(hwnd, None),
            n => self.zoom_to(hwnd, ZOOM_LEVELS.get((n - 2) as usize).copied()),
        }
    }

    fn update_fonts(&mut self, hwnd: HWND) {
        self.dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
        let font = chrome::font((14.0 * self.dpi).round() as i32, 400);
        let action = chrome::font((15.0 * self.dpi).round() as i32, 600);
        let icons = chrome::icon_font((chrome::ICON_SIZE as f32 * self.dpi).round() as i32);
        unsafe {
            for control in [self.status, self.editor, self.zoom_picker]
                .into_iter()
                .chain(
                    self.toolbar
                        .iter()
                        .chain(&self.menu_buttons)
                        .map(|(_, button)| *button),
                )
            {
                SendMessageW(control, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
            SendMessageW(
                self.open_button,
                WM_SETFONT,
                WPARAM(action.0 as usize),
                LPARAM(1),
            );
            let _ = DeleteObject(self.font);
            let _ = DeleteObject(self.action_font);
            let _ = DeleteObject(self.icon_font);
        }
        self.font = font;
        self.action_font = action;
        self.icon_font = icons;
    }

    fn position_editor(&self) {
        if let Some(at) = self.text_at {
            let r = self.page_rect;
            let area = self.viewport();
            let editor_width = (260.0 * self.dpi).round().min(area.right);
            let editor_height = (29.0 * self.dpi).round();
            let x =
                (r.left + at.0 * (r.right - r.left) - 3.0).clamp(0.0, area.right - editor_width);
            let y = (r.top + at.1 * (r.bottom - r.top) - 4.0)
                .clamp(area.top, area.top.max(area.bottom - editor_height));
            unsafe {
                let _ = MoveWindow(
                    self.editor,
                    x.round() as i32,
                    y.round() as i32,
                    editor_width as i32,
                    editor_height as i32,
                    true,
                );
            }
        }
    }

    fn layout(&mut self) {
        let (w, h) = self.size;
        if w <= 0.0 || h <= 0.0 {
            self.page_rect = D2D_RECT_F::default();
            return;
        }
        let area = self.viewport();
        let viewport = (area.right, area.bottom - area.top);
        let scale = self.zoom.map_or_else(
            || self.fit_scale(),
            |percent| percent as f32 * self.actual_scale() / 100.0,
        );
        let (w, h) = ((w * scale).round().max(1.0), (h * scale).round().max(1.0));
        let overflow = (
            ((w - viewport.0 + 32.0) / 2.0).max(0.0),
            ((h - viewport.1 + 32.0) / 2.0).max(0.0),
        );
        self.pan.0 = self.pan.0.clamp(-overflow.0, overflow.0);
        self.pan.1 = self.pan.1.clamp(-overflow.1, overflow.1);
        let left = ((viewport.0 - w) / 2.0 + self.pan.0).round();
        let top = (area.top + (viewport.1 - h) / 2.0 + self.pan.1).round();
        self.page_rect = D2D_RECT_F {
            left,
            top,
            right: left + w,
            bottom: top + h,
        };
        self.position_editor();
    }

    fn refresh(&mut self, hwnd: HWND) -> Result<()> {
        if self.width == 0 || self.height <= self.footer as u32 {
            return Ok(());
        }
        self.size = self
            .doc
            .as_ref()
            .map(Document::size)
            .transpose()?
            .unwrap_or_default();
        self.layout();
        if self.canvas.is_none() {
            self.canvas = Some(Canvas::new(hwnd, self.width, self.height, self.dpi)?);
        }
        let canvas = self.canvas.as_mut().unwrap();
        let raster = if let Some(doc) = &self.doc {
            let rect = self.page_rect;
            let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
            let scale = (pdf::MAX_EDGE / w.max(h))
                .min((pdf::MAX_PIXELS / (w * h)).sqrt())
                .min(1.0);
            Some(doc.render(
                (w * scale).round().max(1.0) as i32,
                (h * scale).round().max(1.0) as i32,
            )?)
        } else {
            None
        };
        canvas.set_page(raster)?;
        repaint(hwnd);
        Ok(())
    }

    fn apply_theme(&self, hwnd: HWND) {
        unsafe {
            let dark = self.dark && !self.contrast;
            let (background, editor_background, foreground) = if self.contrast {
                let background = COLORREF(GetSysColor(COLOR_BTNFACE));
                (
                    background,
                    COLORREF(GetSysColor(COLOR_WINDOW)),
                    COLORREF(GetSysColor(COLOR_BTNTEXT)),
                )
            } else if dark {
                (
                    chrome::colorref(chrome::DARK_SURFACE),
                    chrome::colorref(0x141b27),
                    chrome::colorref(chrome::DARK_TEXT),
                )
            } else {
                (
                    chrome::colorref(chrome::LIGHT_SURFACE),
                    COLORREF(GetSysColor(COLOR_WINDOW)),
                    chrome::colorref(chrome::LIGHT_TEXT),
                )
            };
            PAINT_COLORS.with(|colors| {
                colors
                    .replace(PaintColors {
                        status: CreateSolidBrush(background),
                        editor: CreateSolidBrush(editor_background),
                        background,
                        editor_background,
                        foreground,
                    })
                    .delete()
            });
            let default = COLORREF(u32::MAX);
            let caption = if dark {
                chrome::colorref(chrome::DARK_SURFACE)
            } else {
                default
            };
            let text = if dark {
                chrome::colorref(chrome::DARK_TEXT)
            } else {
                default
            };
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                (&caption as *const COLORREF).cast(),
                std::mem::size_of::<COLORREF>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TEXT_COLOR,
                (&text as *const COLORREF).cast(),
                std::mem::size_of::<COLORREF>() as u32,
            );
            let _ = InvalidateRect(self.status, None, true);
            let _ = InvalidateRect(self.editor, None, true);
        }
        for &(_, button) in &self.menu_buttons {
            repaint(button);
        }
        repaint(self.open_button);
        repaint(self.zoom_picker);
        repaint(hwnd);
    }

    fn set_language(&mut self, language: Language) -> Result<()> {
        let menu = make_menu(language)?;
        let old = std::mem::replace(&mut self.menu, menu);
        self.language = language;
        unsafe {
            if !old.is_invalid() {
                let _ = DestroyMenu(old);
            }
            for ((_, button), (_, label)) in self.menu_buttons.iter().zip(MENU_BUTTONS) {
                let _ = SetWindowTextW(*button, PCWSTR(wide(language.text(*label)).as_ptr()));
                repaint(*button);
            }
            for ((_, button), (_, label, _)) in self.toolbar.iter().zip(chrome::TOOLS) {
                let _ = SetWindowTextW(*button, PCWSTR(wide(language.text(*label)).as_ptr()));
                repaint(*button);
            }
            let _ = SetWindowTextW(
                self.open_button,
                PCWSTR(wide(language.text(OPEN_LABEL)).as_ptr()),
            );
            let cue = wide(language.text(("Teks untuk ditambahkan", "Text to add")));
            SendMessageW(
                self.editor,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(cue.as_ptr() as isize),
            );
        }
        Ok(())
    }

    fn sync(&self, hwnd: HWND) {
        let language = self.language;
        let (title, page, dirty) = match &self.doc {
            Some(doc) => {
                let name = doc.source.file_name().unwrap_or_default().to_string_lossy();
                let dirty = doc.dirty();
                (
                    format!("{}{name} | HubbyPDF", if dirty { "* " } else { "" }),
                    format!(
                        "{} {} / {}",
                        language.text(("Halaman", "Page")),
                        doc.index + 1,
                        doc.count()
                    ),
                    dirty,
                )
            }
            None => ("HubbyPDF".into(), String::new(), false),
        };
        let hint = language.text(match self.tool {
            TEXT if self.text_at.is_some() => (
                "Ketik teks · Enter untuk menempatkan, Esc untuk batal.",
                "Type text · Enter to place, Esc to cancel.",
            ),
            TEXT => (
                "Klik posisi teks pada halaman.",
                "Click where you want to add text.",
            ),
            READ if self.doc.is_some() && !dirty => {
                ("Seret objek untuk menggeser.", "Drag an object to move it.")
            }
            HIGHLIGHT => (
                "Sorot · Seret mendatar, lalu tarik pegangan untuk mengubah tinggi.",
                "Highlight · Drag horizontally, then pull a handle to adjust the height.",
            ),
            SKETCH => ("Sketsa · Gambar di halaman.", "Sketch · Draw on the page."),
            SIGN => (
                "Tanda tangan · Gambar di halaman.",
                "Signature · Draw on the page.",
            ),
            PNG => (
                "Tanda tangan PNG · Seret untuk menentukan ukuran, atau tekan Enter.",
                "PNG signature · Drag to size it, or press Enter.",
            ),
            ERASE => (
                "Klik objek tambahan untuk menghapusnya.",
                "Click an object you added to erase it.",
            ),
            _ if dirty => ("Perubahan belum disimpan", "Unsaved changes"),
            _ => ("", ""),
        });
        let right = self.notice.as_deref().unwrap_or(hint);
        let status = if page.is_empty() || right.is_empty() {
            format!("{page}{right}")
        } else {
            format!("{page}    |    {right}")
        };
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(wide(&title).as_ptr()));
            let _ = SetWindowTextW(self.status, PCWSTR(wide(&status).as_ptr()));
            let _ = ShowWindow(
                self.open_button,
                if self.doc.is_some() { SW_HIDE } else { SW_SHOW },
            );
            let _ = ShowWindow(
                self.editor,
                if self.text_at.is_some() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let _ = EnableWindow(self.zoom_picker, self.doc.is_some());
            // Owner-drawn buttons can request a synchronous repaint during WM_SETTEXT.
            let _ = PostMessageW(hwnd, WM_SYNC_ZOOM_LABEL, WPARAM(0), LPARAM(0));
            for &(id, button) in &self.toolbar {
                let enabled = matches!(id, OPEN | THEME)
                    || self
                        .doc
                        .as_ref()
                        .is_some_and(|doc| id != UNDO || doc.can_undo());
                let _ = EnableWindow(button, enabled);
                repaint(button);
            }
            let menu = self.menu;
            for &(id, _) in VIEW_COMMANDS
                .iter()
                .chain(TOOLS)
                .chain(&FILE_COMMANDS[1..2])
            {
                let enabled = id == THEME
                    || self.doc.as_ref().is_some_and(|doc| match id {
                        PREVIOUS => doc.index > 0,
                        NEXT => doc.index + 1 < doc.count(),
                        UNDO => doc.can_undo(),
                        _ => true,
                    });
                let _ = EnableMenuItem(
                    menu,
                    id as u32,
                    MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED },
                );
            }
            let _ = CheckMenuItem(
                menu,
                THEME as u32,
                (MF_BYCOMMAND | if self.dark { MF_CHECKED } else { MF_UNCHECKED }).0,
            );
            for &(id, _) in TOOLS.iter().filter(|(id, _)| *id != UNDO) {
                let _ = CheckMenuItem(
                    menu,
                    id as u32,
                    (MF_BYCOMMAND
                        | if id == self.tool && self.doc.is_some() {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        })
                    .0,
                );
            }
            let _ = CheckMenuRadioItem(
                menu,
                INDONESIAN as u32,
                ENGLISH as u32,
                if language == Language::English {
                    ENGLISH
                } else {
                    INDONESIAN
                } as u32,
                MF_BYCOMMAND.0,
            );
        }
    }

    fn open(&mut self, hwnd: HWND, path: PathBuf) -> Result<()> {
        let doc = Document::open(self.engine, path)?;
        doc.size()?;
        self.doc = Some(doc);
        unsafe {
            let _ = SetFocus(hwnd);
        }
        self.tool = READ;
        self.signature = None;
        self.gesture.clear();
        self.zoom = None;
        self.drag = None;
        self.selected_highlight = None;
        self.text_at = None;
        self.pan = (0.0, 0.0);
        self.refresh(hwnd)
    }

    fn discard_allowed(&self, hwnd: HWND) -> bool {
        !self.doc.as_ref().is_some_and(Document::dirty)
            || unsafe {
                MessageBoxW(
                    hwnd,
                    PCWSTR(
                        wide(self.language.text((
                            "Buang perubahan yang belum disimpan?",
                            "Discard unsaved changes?",
                        )))
                        .as_ptr(),
                    ),
                    w!("HubbyPDF"),
                    MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
                ) == IDYES
            }
    }

    fn command(&mut self, hwnd: HWND, id: u16) -> Result<()> {
        self.notice = None;
        if (MENU_FILE..=MENU_LANGUAGE).contains(&id) {
            return self.show_menu(hwnd, id);
        }
        if matches!(id, INDONESIAN | ENGLISH) {
            let language = if id == ENGLISH {
                Language::English
            } else {
                Language::Indonesian
            };
            if language != self.language {
                self.set_language(language)?;
                self.resize(hwnd)?;
                if language.save().is_err() {
                    self.notice = Some(
                        language
                            .text((
                                "Bahasa diubah. Preferensi gagal disimpan.",
                                "Language changed. Could not save the preference.",
                            ))
                            .into(),
                    );
                }
                repaint(hwnd);
            }
            return Ok(());
        }
        if id == THEME {
            self.dark = !self.dark;
            self.apply_theme(hwnd);
            if chrome::save_theme(self.dark).is_err() {
                self.notice = Some(
                    self.language
                        .text((
                            "Tema diubah. Preferensi gagal disimpan.",
                            "Theme changed. Could not save the preference.",
                        ))
                        .into(),
                );
            }
            return Ok(());
        }
        if id == OPEN {
            if let Some(path) = choose_file(hwnd, FileKind::Pdf, None, self.language)?
                && self.discard_allowed(hwnd)
            {
                self.open(hwnd, path)?;
            }
            return Ok(());
        }
        if id == EXIT {
            if self.discard_allowed(hwnd) {
                unsafe { DestroyWindow(hwnd)? };
            }
            return Ok(());
        }
        let Some(doc) = self.doc.as_mut() else {
            return Ok(());
        };
        self.gesture.clear();
        self.drag = None;
        if matches!(
            id,
            PREVIOUS | NEXT | UNDO | READ | TEXT | HIGHLIGHT | SKETCH | SIGN | ERASE
        ) {
            self.selected_highlight = None;
            self.text_at = None;
            unsafe {
                let _ = SetWindowTextW(self.editor, w!(""));
            }
        }
        match id {
            SAVE => {
                let name = format!(
                    "{} - {}.pdf",
                    doc.source.file_stem().unwrap_or_default().to_string_lossy(),
                    self.language.text(("salinan", "copy"))
                );
                if let Some(path) = choose_file(hwnd, FileKind::Save, Some(&name), self.language)? {
                    doc.save_copy(&path)?;
                    self.notice = Some(
                        self.language
                            .text(("Salinan PDF disimpan.", "PDF copy saved."))
                            .into(),
                    );
                }
            }
            PREVIOUS | NEXT => {
                doc.index = if id == PREVIOUS {
                    doc.index.saturating_sub(1)
                } else {
                    (doc.index + 1).min(doc.count() - 1)
                };
                self.pan = (0.0, 0.0);
                self.refresh(hwnd)?;
            }
            ZOOM_OUT | ZOOM_IN | FIT => {
                let current = self
                    .zoom
                    .map_or_else(|| self.fit_scale() / self.actual_scale() * 100.0, f32::from);
                let level = match id {
                    ZOOM_IN => Some(
                        ZOOM_LEVELS
                            .iter()
                            .copied()
                            .find(|n| f32::from(*n) > current + 0.1)
                            .unwrap_or(500),
                    ),
                    ZOOM_OUT => Some(
                        ZOOM_LEVELS
                            .iter()
                            .rev()
                            .copied()
                            .find(|n| f32::from(*n) < current - 0.1)
                            .unwrap_or(25),
                    ),
                    _ => None,
                };
                self.zoom_to(hwnd, level)?;
            }
            UNDO => {
                doc.undo()?;
                self.refresh(hwnd)?;
            }
            PNG => {
                if let Some(path) = choose_file(hwnd, FileKind::Png, None, self.language)? {
                    let mut reader = ImageReader::with_format(
                        std::io::BufReader::new(std::fs::File::open(path)?),
                        image::ImageFormat::Png,
                    );
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(4096);
                    limits.max_image_height = Some(4096);
                    limits.max_alloc = Some(80 * 1024 * 1024);
                    reader.limits(limits);
                    let image = reader.decode()?;
                    self.signature = Some(if image.width().max(image.height()) > 1024 {
                        image.resize(1024, 1024, image::imageops::FilterType::Triangle)
                    } else {
                        image
                    });
                    self.text_at = None;
                    self.selected_highlight = None;
                    self.tool = PNG;
                    unsafe {
                        let _ = SetFocus(hwnd);
                    }
                }
            }
            READ | TEXT | HIGHLIGHT | SKETCH | SIGN | ERASE => {
                if id == SIGN && self.tool != SIGN {
                    self.signature_group += 1;
                }
                self.tool = id;
                self.signature = None;
                self.sync(hwnd);
                unsafe {
                    let _ = SetFocus(hwnd);
                }
            }
            _ => {}
        }
        self.layout();
        repaint(hwnd);
        Ok(())
    }

    fn point(&self, lparam: LPARAM, clamp: bool) -> Option<Point> {
        let (x, y) = ((lparam.0 as i16) as f32, ((lparam.0 >> 16) as i16) as f32);
        let r = self.page_rect;
        if self.doc.is_none() || r.right <= r.left || r.bottom <= r.top {
            return None;
        }
        let point = (
            (x - r.left) / (r.right - r.left),
            (y - r.top) / (r.bottom - r.top),
        );
        if clamp {
            Some((point.0.clamp(0.0, 1.0), point.1.clamp(0.0, 1.0)))
        } else if y >= self.viewport().top
            && y < self.viewport().bottom
            && (0.0..=1.0).contains(&point.0)
            && (0.0..=1.0).contains(&point.1)
        {
            Some(point)
        } else {
            None
        }
    }

    fn highlight_handle(&self, point: Point) -> Option<HighlightEdge> {
        let (_, (a, b)) = self.selected_highlight?;
        let rect = self.page_rect;
        let radius = 6.0 * self.dpi;
        let dx = (point.0 - (a.0 + b.0) / 2.0).abs() * (rect.right - rect.left);
        let top = (point.1 - a.1).abs() * (rect.bottom - rect.top);
        let bottom = (point.1 - b.1).abs() * (rect.bottom - rect.top);
        (dx <= radius && top.min(bottom) <= radius).then_some(if top <= bottom {
            HighlightEdge::Top
        } else {
            HighlightEdge::Bottom
        })
    }

    fn hit_tolerance(&self) -> Point {
        let r = self.page_rect;
        (
            7.0 * self.dpi / (r.right - r.left),
            7.0 * self.dpi / (r.bottom - r.top),
        )
    }

    fn highlight_bounds(&self, start: Point, end: Point) -> (Point, Point) {
        let (top, bottom) = self.highlight_band;
        ((start.0.min(end.0), top), (start.0.max(end.0), bottom))
    }

    fn preview(&self) -> Option<Preview<'_>> {
        if let Some(drag) = &self.drag {
            let (a, b) = drag.translated();
            return Some(Preview::Selection(a, b, self.selected_highlight.is_some()));
        }
        if self.gesture.len() < 2 {
            return self
                .selected_highlight
                .map(|(_, (a, b))| Preview::Selection(a, b, true));
        }
        let (a, b) = (self.gesture[0], *self.gesture.last()?);
        match self.tool {
            SKETCH | SIGN => Some(Preview::Ink(&self.gesture, self.tool == SIGN)),
            HIGHLIGHT => {
                let (a, b) = self.highlight_bounds(a, b);
                Some(Preview::Box(a, b, true))
            }
            PNG => Some(Preview::Box(a, self.signature_end(a, b), false)),
            _ => None,
        }
    }

    fn signature_end(&self, a: Point, b: Point) -> Point {
        let Some(image) = self.signature.as_ref().filter(|_| self.tool == PNG) else {
            return b;
        };
        let ratio = image.height() as f32 * self.size.0 / (image.width() as f32 * self.size.1);
        let limit_x = if b.0 >= a.0 { 1.0 - a.0 } else { a.0 };
        let limit_y = if b.1 >= a.1 { 1.0 - a.1 } else { a.1 };
        let w = (b.0 - a.0)
            .abs()
            .min((b.1 - a.1).abs() / ratio)
            .min(limit_x)
            .min(limit_y / ratio);
        (
            a.0 + w.copysign(b.0 - a.0),
            a.1 + (w * ratio).copysign(b.1 - a.1),
        )
    }

    fn begin_text(&mut self, hwnd: HWND, point: Point) {
        self.text_at = Some(point);
        self.position_editor();
        self.sync(hwnd);
        unsafe {
            let _ = SetFocus(self.editor);
        }
    }

    fn place_text(&mut self, hwnd: HWND) -> Result<()> {
        let Some(point) = self.text_at else {
            return Ok(());
        };
        let mut buffer = [0; 513];
        let len = unsafe { GetWindowTextW(self.editor, &mut buffer) } as usize;
        let text = String::from_utf16_lossy(&buffer[..len]);
        if text.trim().is_empty() {
            unsafe {
                let _ = SetFocus(self.editor);
            }
            return Ok(());
        }
        if !text.chars().all(|c| matches!(c, ' '..='~' | ' '..='ÿ')) {
            return Err(Message((
                "Gunakan karakter Latin dasar untuk teks PDF.",
                "Use basic Latin characters for PDF text.",
            ))
            .into());
        }
        self.add(hwnd, Mark::Text(point, text))?;
        unsafe { SetWindowTextW(self.editor, w!(""))? };
        self.command(hwnd, READ)
    }

    fn add(&mut self, hwnd: HWND, mark: Mark) -> Result<()> {
        self.notice = None;
        if let Some(doc) = self.doc.as_mut() {
            let bounds = match &mark {
                Mark::Highlight(a, b) => Some((*a, *b)),
                _ => None,
            };
            let id = doc.add(mark)?;
            if let Some(bounds) = bounds {
                self.selected_highlight = Some((id, bounds));
            }
            self.refresh(hwnd)?;
        }
        Ok(())
    }

    fn finish_gesture(&mut self, hwnd: HWND, end: Point) -> Result<()> {
        let mut points = std::mem::take(&mut self.gesture);
        let Some(&start) = points.first() else {
            return Ok(());
        };
        points.push(end);
        match self.tool {
            SKETCH | SIGN if points.windows(2).any(|pair| pair[0] != pair[1]) => self.add(
                hwnd,
                Mark::Ink(points, (self.tool == SIGN).then_some(self.signature_group)),
            )?,
            HIGHLIGHT if (start.0 - end.0).abs() * self.size.0 >= 1.0 => {
                let (a, b) = self.highlight_bounds(start, end);
                self.add(hwnd, Mark::Highlight(a, b))?;
            }
            PNG => self.place_signature(hwnd, start, end)?,
            _ => {}
        }
        repaint(hwnd);
        Ok(())
    }

    fn place_signature(&mut self, hwnd: HWND, start: Point, end: Point) -> Result<()> {
        let end = self.signature_end(start, end);
        if (end.0 - start.0).abs() < 0.002 || (end.1 - start.1).abs() < 0.002 {
            return Ok(());
        }
        if let Some(image) = self.signature.take() {
            self.tool = READ;
            self.add(hwnd, Mark::Signature(start, end, image))?;
        }
        Ok(())
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn repaint(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

enum FileKind {
    Pdf,
    Save,
    Png,
}

fn choose_file(
    hwnd: HWND,
    kind: FileKind,
    name: Option<&str>,
    language: Language,
) -> Result<Option<PathBuf>> {
    let mut buffer = vec![0_u16; 32768];
    if let Some(name) = name {
        let name = wide(name);
        buffer[..name.len()].copy_from_slice(&name);
    }
    let (label, extension) = match kind {
        FileKind::Pdf | FileKind::Save => (
            (
                "Dokumen PDF (*.pdf)\0*.pdf\0",
                "PDF documents (*.pdf)\0*.pdf\0",
            ),
            w!("pdf"),
        ),
        FileKind::Png => (
            ("Gambar PNG (*.png)\0*.png\0", "PNG images (*.png)\0*.png\0"),
            w!("png"),
        ),
    };
    let filter = wide(language.text(label));
    let save = matches!(kind, FileKind::Save);
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrDefExt: extension,
        Flags: OFN_EXPLORER
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | if save {
                OFN_OVERWRITEPROMPT | OFN_NOREADONLYRETURN
            } else {
                OFN_FILEMUSTEXIST
            },
        ..Default::default()
    };
    unsafe {
        let selected = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if !selected.as_bool() {
            let error = CommDlgExtendedError();
            return if error.0 == 0 {
                Ok(None)
            } else {
                let message = language.text(("Dialog file gagal", "File dialog failed"));
                Err(format!("{message} (0x{:X}).", error.0).into())
            };
        }
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&buffer[..end]))))
}

fn error_text(language: Language, error: &(dyn std::error::Error + 'static)) -> String {
    match error.downcast_ref::<Message>() {
        Some(message) => language.text(message.0).into(),
        None => error.to_string(),
    }
}

fn show_error(hwnd: HWND, language: Language, error: &(dyn std::error::Error + 'static)) {
    let message = wide(&error_text(language, error));
    unsafe {
        MessageBoxW(
            hwnd,
            PCWSTR(message.as_ptr()),
            w!("HubbyPDF"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn process_message(
    app: &mut App,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Result<bool> {
    match message {
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
            app.contrast = chrome::high_contrast();
            app.apply_theme(hwnd);
        }
        WM_ENTERSIZEMOVE => {
            app.resizing = true;
            return Ok(true);
        }
        WM_EXITSIZEMOVE => {
            app.resizing = false;
            app.refresh(hwnd)?;
        }
        WM_DPICHANGED => {
            let rect = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
            app.canvas = None;
            app.update_fonts(hwnd);
            app.resize(hwnd)?;
        }
        WM_SIZE if !app.status.is_invalid() => {
            app.resize(hwnd)?;
            return Ok(true);
        }
        WM_COMMAND if wparam.0 as u16 == ZOOM_PICKER => app.show_zoom_menu(hwnd)?,
        WM_COMMAND if wparam.0 as u16 >= OPEN => app.command(hwnd, wparam.0 as u16)?,
        WM_CLOSE => app.command(hwnd, EXIT)?,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut paint);
            }
            if let Some(canvas) = &app.canvas {
                let system_background = app.contrast.then(|| {
                    let sys = unsafe { GetSysColor(COLOR_BTNFACE) };
                    canvas::color(
                        (sys & 255) as f32 / 255.0,
                        ((sys >> 8) & 255) as f32 / 255.0,
                        ((sys >> 16) & 255) as f32 / 255.0,
                        1.0,
                    )
                });
                if canvas
                    .draw(Scene {
                        page: app.page_rect,
                        viewport: app.viewport(),
                        dpi: app.dpi,
                        language: app.language,
                        system_background,
                        dark: app.dark,
                        toolbar_dividers: &app.toolbar_dividers,
                        preview: app.preview(),
                    })
                    .is_err()
                {
                    app.canvas = None;
                    if let Err(error) = app.refresh(hwnd) {
                        app.notice = Some(error_text(app.language, error.as_ref()));
                        app.sync(hwnd);
                    }
                }
            }
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            }
            return Ok(true);
        }
        WM_LBUTTONDOWN => {
            if let Some(point) = app.point(lparam, false) {
                if app.tool == ERASE {
                    let tolerance = app.hit_tolerance();
                    if let Some(doc) = app.doc.as_mut()
                        && doc.erase_at(point, tolerance)?
                    {
                        app.notice = None;
                        app.refresh(hwnd)?;
                    }
                } else if let Some(edge) = app.highlight_handle(point)
                    && let Some((id, bounds)) = app.selected_highlight
                {
                    app.drag = Some(Drag {
                        ids: vec![id],
                        start: point,
                        bounds,
                        delta: (0.0, 0.0),
                        edge: Some(edge),
                    });
                    unsafe { SetCapture(hwnd) };
                } else if app.tool == TEXT {
                    app.begin_text(hwnd, point);
                } else if app.tool == READ {
                    app.selected_highlight = None;
                    let tolerance = app.hit_tolerance();
                    if let Some((ids, bounds)) =
                        app.doc.as_ref().and_then(|doc| doc.hit(point, tolerance))
                    {
                        if app.doc.as_ref().is_some_and(|doc| doc.is_highlight(ids[0])) {
                            app.selected_highlight = Some((ids[0], bounds));
                        }
                        app.drag = Some(Drag {
                            ids,
                            start: point,
                            bounds,
                            delta: (0.0, 0.0),
                            edge: None,
                        });
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                    repaint(hwnd);
                } else {
                    if app.tool == HIGHLIGHT
                        && let Some(doc) = &app.doc
                    {
                        app.highlight_band = doc.highlight_band(point)?;
                    }
                    app.selected_highlight = None;
                    app.gesture.clear();
                    app.gesture.push(point);
                    unsafe {
                        SetCapture(hwnd);
                    }
                    repaint(hwnd);
                }
            }
        }
        WM_MOUSEMOVE if app.drag.is_some() => {
            if let Some(point) = app.point(lparam, true)
                && let Some(drag) = app.drag.as_mut()
            {
                drag.update(point);
                repaint(hwnd);
            }
            return Ok(true);
        }
        WM_LBUTTONUP if app.drag.is_some() => {
            let point = app.point(lparam, true);
            if let Some(mut drag) = app.drag.take() {
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(point) = point
                    && let Some(doc) = app.doc.as_mut()
                {
                    drag.update(point);
                    let bounds = drag.translated();
                    match drag.edge {
                        Some(HighlightEdge::Top) => {
                            doc.resize_highlight(drag.ids[0], (drag.delta.1, 0.0))?
                        }
                        Some(HighlightEdge::Bottom) => {
                            doc.resize_highlight(drag.ids[0], (0.0, drag.delta.1))?
                        }
                        None => doc.move_mark(drag.ids, drag.delta)?,
                    }
                    if let Some((_, selected)) = &mut app.selected_highlight {
                        *selected = bounds;
                    }
                    app.refresh(hwnd)?;
                }
            }
            repaint(hwnd);
        }
        WM_MOUSEMOVE if !app.gesture.is_empty() => {
            if let Some(point) = app.point(lparam, true) {
                if app.tool == HIGHLIGHT || app.tool == PNG {
                    app.gesture.truncate(1);
                }
                if app.gesture.last() != Some(&point) {
                    app.gesture.push(point);
                }
                repaint(hwnd);
            }
            return Ok(true);
        }
        WM_LBUTTONUP if !app.gesture.is_empty() => {
            let result = app
                .point(lparam, true)
                .map_or(Ok(()), |end| app.finish_gesture(hwnd, end));
            unsafe {
                let _ = ReleaseCapture();
            }
            result?;
        }
        WM_CAPTURECHANGED => {
            app.gesture.clear();
            app.drag = None;
            repaint(hwnd);
            return Ok(true);
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL if app.doc.is_some() => {
            let delta = ((wparam.0 >> 16) as i16) as f32 / 120.0;
            if key_down(VK_CONTROL) {
                app.command(hwnd, if delta > 0.0 { ZOOM_IN } else { ZOOM_OUT })?;
            } else if message == WM_MOUSEHWHEEL || key_down(VK_SHIFT) {
                app.pan.0 += if message == WM_MOUSEHWHEEL {
                    -delta
                } else {
                    delta
                } * 64.0;
                app.layout();
                repaint(hwnd);
            } else if app.can_pan() {
                app.pan.1 += delta * 64.0;
                app.layout();
                repaint(hwnd);
            } else {
                app.command(hwnd, if delta > 0.0 { PREVIOUS } else { NEXT })?;
            }
        }
        _ => return Ok(false),
    }
    if unsafe { IsWindow(hwnd) }.as_bool() {
        app.sync(hwnd);
    }
    Ok(true)
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_SYNC_ZOOM_LABEL {
        let current = APP.with(|state| {
            state.try_borrow().ok().and_then(|state| {
                state
                    .as_ref()
                    .map(|app| (app.zoom_picker, app.zoom, app.language))
            })
        });
        if let Some((button, zoom, language)) = current {
            let label = zoom.map_or_else(
                || format!("{}  ▾", language.text(FIT_LABEL)),
                |level| format!("{level}%  ▾"),
            );
            unsafe {
                let _ = SetWindowTextW(button, PCWSTR(wide(&label).as_ptr()));
            }
            repaint(button);
        }
        return LRESULT(0);
    }
    if message == WM_GETMINMAXINFO {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
        let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
        info.ptMinTrackSize.x = 435 * dpi / 96;
        info.ptMinTrackSize.y = 360 * dpi / 96;
        return LRESULT(0);
    }
    if message == WM_DRAWITEM {
        let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
        APP.with(|state| {
            if let Ok(state) = state.try_borrow()
                && let Some(app) = state.as_ref()
            {
                chrome::draw_button(
                    item,
                    app.tool == item.CtlID as u16 || (item.CtlID == THEME as u32 && app.dark),
                    item.hwndItem == app.open_button,
                    app.dpi,
                    app.contrast,
                    app.dark,
                    app.icon_font,
                );
            }
        });
        return LRESULT(1);
    }
    if matches!(message, WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT) {
        let dc = HDC(wparam.0 as *mut _);
        // WM_CTLCOLOR can arrive during another update while App is already borrowed.
        let colors = PAINT_COLORS.with(Cell::get);
        let (brush, background) = if message == WM_CTLCOLOREDIT {
            (colors.editor, colors.editor_background)
        } else {
            (colors.status, colors.background)
        };
        unsafe {
            if brush.is_invalid() {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            SetTextColor(dc, colors.foreground);
            SetBkColor(dc, background);
            return LRESULT(brush.0 as isize);
        }
    }
    if message == WM_DESTROY {
        unsafe {
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }
    let handled = APP.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return false;
        };
        let Some(app) = state.as_mut() else {
            return false;
        };
        match process_message(app, hwnd, message, wparam, lparam) {
            Ok(handled) => handled,
            Err(error) => {
                app.notice = Some(error_text(app.language, error.as_ref()));
                app.sync(hwnd);
                repaint(hwnd);
                if message != WM_SIZE {
                    show_error(hwnd, app.language, error.as_ref());
                }
                true
            }
        }
    });
    if handled {
        LRESULT(0)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

fn keyboard(hwnd: HWND, message: &MSG) -> bool {
    if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return false;
    }
    APP.with(|state| {
        let mut state = state.borrow_mut();
        let Some(app) = state.as_mut() else {
            return false;
        };
        let ctrl = key_down(VK_CONTROL);
        let editing = message.hwnd == app.editor;
        let key = VIRTUAL_KEY(message.wParam.0 as u16);
        if message.message == WM_SYSKEYDOWN {
            let id = match key.0 {
                0x46 => MENU_FILE,
                0x54 if app.language == Language::English => MENU_TOOLS,
                0x54 | 0x56 => MENU_VIEW,
                0x41 => MENU_TOOLS,
                0x42 | 0x4C => MENU_LANGUAGE,
                _ => return false,
            };
            if let Err(error) = app.show_menu(hwnd, id) {
                show_error(hwnd, app.language, error.as_ref());
            }
            if unsafe { IsWindow(hwnd) }.as_bool() {
                app.sync(hwnd);
            }
            return true;
        }
        let toolbar_command = app
            .toolbar
            .iter()
            .chain(&app.menu_buttons)
            .find(|(_, button)| *button == message.hwnd)
            .map(|(id, _)| *id);
        if key == VK_TAB && !ctrl {
            let order: Vec<_> = app
                .menu_buttons
                .iter()
                .chain(&app.toolbar)
                .map(|(_, button)| *button)
                .chain([app.editor, app.open_button, app.zoom_picker])
                .filter(|control| unsafe {
                    IsWindowVisible(*control).as_bool() && IsWindowEnabled(*control).as_bool()
                })
                .collect();
            if !order.is_empty() {
                let current = order.iter().position(|control| *control == message.hwnd);
                let next = if key_down(VK_SHIFT) {
                    current.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
                } else {
                    current.map_or(0, |i| (i + 1) % order.len())
                };
                unsafe {
                    let _ = SetFocus(order[next]);
                }
            }
            return true;
        }
        if message.hwnd == app.zoom_picker && key == VK_RETURN && !ctrl {
            if let Err(error) = app.show_zoom_menu(hwnd) {
                show_error(hwnd, app.language, error.as_ref());
            }
            app.sync(hwnd);
            return true;
        }
        let command = match (key, ctrl, editing) {
            (VIRTUAL_KEY(0x4F), true, _) => OPEN,
            (VIRTUAL_KEY(0x53), true, _) if key_down(VK_SHIFT) => SAVE,
            (VIRTUAL_KEY(0x5A), true, false) => UNDO,
            (VIRTUAL_KEY(0x30) | VK_NUMPAD0, true, false) => FIT,
            (VK_OEM_PLUS | VK_ADD, true, false) => ZOOM_IN,
            (VK_OEM_MINUS | VK_SUBTRACT, true, false) => ZOOM_OUT,
            (VK_PRIOR, false, false) => PREVIOUS,
            (VK_NEXT, false, false) => NEXT,
            (VK_ESCAPE, false, _) => READ,
            (VK_RETURN, false, _)
                if toolbar_command.is_some() || message.hwnd == app.open_button =>
            {
                toolbar_command.unwrap_or(OPEN)
            }
            (VK_RETURN, false, _) if app.tool == TEXT || app.tool == PNG => {
                let rect = app.page_rect;
                let at = (
                    ((app.width as f32 / 2.0 - rect.left) / (rect.right - rect.left))
                        .clamp(0.0, 1.0),
                    (((app.viewport().top + app.viewport().bottom) / 2.0 - rect.top)
                        / (rect.bottom - rect.top))
                        .clamp(0.0, 1.0),
                );
                let action = if app.tool == TEXT {
                    if app.text_at.is_none() {
                        app.begin_text(hwnd, at);
                        Ok(())
                    } else {
                        app.place_text(hwnd)
                    }
                } else {
                    app.place_signature(hwnd, at, (at.0 + 0.25, at.1 + 0.25))
                };
                if let Err(error) = action {
                    show_error(hwnd, app.language, error.as_ref());
                }
                app.sync(hwnd);
                return true;
            }
            (VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT, false, false) if app.can_pan() => {
                match key {
                    VK_UP => app.pan.1 += 50.0,
                    VK_DOWN => app.pan.1 -= 50.0,
                    VK_LEFT => app.pan.0 += 50.0,
                    _ => app.pan.0 -= 50.0,
                }
                app.layout();
                repaint(hwnd);
                return true;
            }
            _ => return false,
        };
        if let Err(error) = app.command(hwnd, command) {
            show_error(hwnd, app.language, error.as_ref());
        }
        if unsafe { IsWindow(hwnd) }.as_bool() {
            app.sync(hwnd);
        }
        true
    })
}

fn load_pdfium(folder: &Path, language: Language) -> Result<Pdfium> {
    let dll = folder.join("pdfium.dll");
    Ok(Pdfium::new(Pdfium::bind_to_library(&dll).map_err(
        |error| {
            format!(
                "{} {}. {error}",
                language.text(("PDFium gagal dimuat dari", "Could not load PDFium from")),
                dll.display()
            )
        },
    )?))
}

fn make_menu(language: Language) -> Result<HMENU> {
    unsafe {
        let menu = CreateMenu()?;
        for ((_, label), commands) in
            MENU_BUTTONS
                .iter()
                .zip([FILE_COMMANDS, VIEW_COMMANDS, TOOLS, LANGUAGES])
        {
            let child = CreatePopupMenu()?;
            for &(id, name) in commands {
                AppendMenuW(
                    child,
                    MF_STRING,
                    id as usize,
                    PCWSTR(wide(language.text(name)).as_ptr()),
                )?;
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                child.0 as usize,
                PCWSTR(wide(language.text(*label)).as_ptr()),
            )?;
        }
        Ok(menu)
    }
}

fn run(language: Language) -> Result<()> {
    let exe = std::env::current_exe()?;
    let engine = Box::leak(Box::new(load_pdfium(
        exe.parent().ok_or(Message((
            "Lokasi aplikasi tidak tersedia.",
            "The application location is unavailable.",
        )))?,
        language,
    )?));
    let instance = unsafe { GetModuleHandleW(None)? };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("HubbyPdfWindow"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.lpszClassName,
            w!("HubbyPDF"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            800,
            None,
            None,
            instance,
            None,
        )?;
        let mut app = App::new(engine, language);
        for &(id, _) in MENU_BUTTONS {
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                0,
                0,
                hwnd,
                HMENU(id as usize as *mut _),
                instance,
                None,
            )?;
            app.menu_buttons.push((id, button));
        }
        app.status = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0),
            0,
            0,
            0,
            0,
            hwnd,
            None,
            instance,
            None,
        )?;
        app.open_button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!(""),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            0,
            0,
            0,
            0,
            hwnd,
            HMENU(OPEN as usize as *mut _),
            instance,
            None,
        )?;
        app.editor = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            0,
            0,
            0,
            0,
            hwnd,
            None,
            instance,
            None,
        )?;
        app.zoom_picker = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            0,
            0,
            0,
            0,
            hwnd,
            HMENU(ZOOM_PICKER as usize as *mut _),
            instance,
            None,
        )?;
        for &(id, _, _) in chrome::TOOLS {
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                0,
                0,
                hwnd,
                HMENU(id as usize as *mut _),
                instance,
                None,
            )?;
            app.toolbar.push((id, button));
        }
        app.update_fonts(hwnd);
        SendMessageW(app.editor, EM_SETLIMITTEXT, WPARAM(512), LPARAM(0));
        app.set_language(language)?;
        app.resize(hwnd)?;
        if let Some(path) = std::env::args_os().nth(1) {
            if let Err(error) = app.open(hwnd, PathBuf::from(path)) {
                app.notice = Some(error_text(language, error.as_ref()));
            }
        }
        app.sync(hwnd);
        app.apply_theme(hwnd);
        let focus = if app.doc.is_none() {
            app.open_button
        } else {
            hwnd
        };
        APP.with(|state| *state.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(focus);
        let mut message = MSG::default();
        loop {
            match GetMessageW(&mut message, None, 0, 0).0 {
                -1 => return Err(windows::core::Error::from_win32().into()),
                0 => break,
                _ if keyboard(hwnd, &message) => {}
                _ => {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
    APP.with(|state| {
        if let Some(app) = state.borrow_mut().take() {
            unsafe {
                let _ = DeleteObject(app.font);
                let _ = DeleteObject(app.action_font);
                let _ = DeleteObject(app.icon_font);
                let _ = DestroyMenu(app.menu);
                PAINT_COLORS.with(|colors| colors.take().delete());
            }
        }
    });
    Ok(())
}

fn main() {
    let language = Language::load();
    if let Err(error) = run(language) {
        show_error(HWND::default(), language, error.as_ref());
    }
}

#[cfg(test)]
fn verify_window_controls(engine: &'static Pdfium) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let parent = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            1100,
            30,
            None,
            None,
            instance,
            None,
        )?;
        let button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!(""),
            WS_CHILD | WINDOW_STYLE(BS_OWNERDRAW as u32),
            0,
            0,
            100,
            30,
            parent,
            HMENU(ZOOM_PICKER as usize as *mut _),
            instance,
            None,
        )?;
        let mut app = App::new(engine, Language::Indonesian);
        app.zoom_picker = button;
        app.status = button;
        app.editor = button;
        app.open_button = button;
        for &(id, _, _) in chrome::TOOLS {
            let tool = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!(""),
                WS_CHILD | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                0,
                0,
                parent,
                HMENU(id as usize as *mut _),
                instance,
                None,
            )?;
            app.toolbar.push((id, tool));
        }
        app.update_fonts(parent);
        app.set_language(Language::Indonesian)?;
        app.resize(parent)?;
        APP.with(|state| *state.borrow_mut() = Some(app));
        let mut indonesian_sign_width = 0;
        for (language, zoom, expected) in [
            (Language::Indonesian, None, "Pas jendela  ▾"),
            (Language::Indonesian, Some(120), "120%  ▾"),
            (Language::English, Some(75), "75%  ▾"),
            (Language::English, None, "Fit window  ▾"),
            (Language::Indonesian, None, "Pas jendela  ▾"),
        ] {
            APP.with(|state| {
                let mut state = state.borrow_mut();
                let app = state.as_mut().unwrap();
                if app.language != language {
                    app.set_language(language).unwrap();
                    app.resize(parent).unwrap();
                }
                let tool_rect = |id| {
                    let (_, tool) = app
                        .toolbar
                        .iter()
                        .find(|(tool_id, _)| *tool_id == id)
                        .unwrap();
                    let mut rect = RECT::default();
                    GetWindowRect(*tool, &mut rect).unwrap();
                    rect
                };
                let sign = tool_rect(SIGN);
                let sketch = tool_rect(SKETCH);
                let png = tool_rect(PNG);
                let gap = (3.0 * app.dpi).round() as i32;
                assert_eq!(sign.top, png.top);
                assert_eq!(sign.left - sketch.right, gap);
                assert_eq!(png.left - sign.right, gap);
                if language == Language::Indonesian {
                    indonesian_sign_width = sign.right - sign.left;
                } else {
                    assert!(sign.right - sign.left < indonesian_sign_width);
                }
                app.zoom_to(parent, zoom).unwrap();
                app.sync(parent);
                let tools = GetSubMenu(app.menu, 2);
                let mut label = [0; 64];
                let len =
                    GetMenuStringW(tools, ERASE as u32, Some(&mut label), MF_BYCOMMAND) as usize;
                assert_eq!(
                    String::from_utf16_lossy(&label[..len]),
                    language.text(("&Hapus objek", "&Erase object"))
                );
                let languages = GetSubMenu(app.menu, 3);
                let selected = if language == Language::English {
                    ENGLISH
                } else {
                    INDONESIAN
                };
                assert_ne!(
                    GetMenuState(languages, selected as u32, MF_BYCOMMAND) & MF_CHECKED.0,
                    0
                );
            });
            let mut event = MSG::default();
            assert!(
                PeekMessageW(
                    &mut event,
                    parent,
                    WM_SYNC_ZOOM_LABEL,
                    WM_SYNC_ZOOM_LABEL,
                    PM_REMOVE
                )
                .as_bool()
            );
            window_proc(parent, event.message, event.wParam, event.lParam);
            let mut label = [0; 64];
            let count = GetWindowTextW(button, &mut label) as usize;
            assert_eq!(String::from_utf16_lossy(&label[..count]), expected);
        }
        let dc = CreateCompatibleDC(None);
        assert!(!dc.is_invalid());
        APP.with(|state| {
            let mut state = state.borrow_mut();
            let app = state.as_mut().unwrap();
            app.contrast = false;
            for (dark, expected) in [
                (true, chrome::DARK_SURFACE),
                (false, chrome::LIGHT_SURFACE),
                (true, chrome::DARK_SURFACE),
            ] {
                app.dark = dark;
                app.apply_theme(parent);
                let result = window_proc(
                    parent,
                    WM_CTLCOLORSTATIC,
                    WPARAM(dc.0 as usize),
                    LPARAM(button.0 as isize),
                );
                let mut brush = LOGBRUSH::default();
                assert_ne!(
                    GetObjectW(
                        HBRUSH(result.0 as *mut _),
                        std::mem::size_of::<LOGBRUSH>() as i32,
                        Some((&mut brush as *mut LOGBRUSH).cast())
                    ),
                    0
                );
                assert_eq!(brush.lbColor, chrome::colorref(expected));
            }
            assert_eq!(
                GetMenuItemCount(GetSubMenu(app.menu, 0)),
                FILE_COMMANDS.len() as i32
            );
            assert_eq!(
                GetMenuItemCount(GetSubMenu(app.menu, 1)),
                VIEW_COMMANDS.len() as i32
            );
            assert_eq!(
                GetMenuItemCount(GetSubMenu(app.menu, 2)),
                TOOLS.len() as i32
            );
            DestroyMenu(app.menu).unwrap();
            let _ = DeleteObject(app.font);
            let _ = DeleteObject(app.action_font);
            let _ = DeleteObject(app.icon_font);
        });
        let _ = DeleteDC(dc);
        PAINT_COLORS.with(|colors| colors.take().delete());
        APP.with(|state| state.borrow_mut().take());
        DestroyWindow(parent)?;
    }
    Ok(())
}
