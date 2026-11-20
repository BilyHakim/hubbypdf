use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::*,
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

pub type Label = (&'static str, &'static str);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Indonesian,
    English,
}

impl Language {
    pub fn text(self, (indonesian, english): Label) -> &'static str {
        match self {
            Self::Indonesian => indonesian,
            Self::English => english,
        }
    }

    pub fn load() -> Self {
        if preference_file("language")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|value| value.trim() == "en")
        {
            Self::English
        } else {
            Self::Indonesian
        }
    }

    pub fn save(self) -> std::io::Result<()> {
        save_preference("language", self.text(("id", "en")))
    }
}

#[derive(Debug)]
pub struct Message(pub Label);

impl std::fmt::Display for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.0)
    }
}

impl std::error::Error for Message {}

pub const TOOLS: &[(u16, Label, u16)] = &[
    (super::OPEN, ("Buka", "Open"), 0xe8e5),
    (super::SAVE, ("Simpan", "Save"), 0xe74e),
    (super::READ, ("Pilih", "Select"), 0xe762),
    (super::TEXT, ("Teks", "Text"), 0xe8d2),
    (super::HIGHLIGHT, ("Sorot", "Highlight"), 0xe7e6),
    (super::SKETCH, ("Sketsa", "Sketch"), 0xed63),
    (super::SIGN, ("Tanda tangan", "Sign"), 0xe70f),
    (super::PNG, ("Impor PNG", "Import PNG"), 0xe8b9),
    (super::ERASE, ("Hapus", "Erase"), 0xe75c),
    (super::UNDO, ("Undo", "Undo"), 0xe7a7),
    (super::THEME, ("Tema", "Theme"), 0xe708),
];
pub const ICON_SIZE: i32 = 16;
pub const ICON_GAP: i32 = 6;
pub const HEADER: i32 = 56;
pub const LIGHT_SURFACE: u32 = 0xf8f9fc;
pub const DARK_SURFACE: u32 = 0x202734;
pub const LIGHT_TEXT: u32 = 0x34405a;
pub const DARK_TEXT: u32 = 0xe3eafa;

pub fn colorref(hex: u32) -> COLORREF {
    rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn preference_file(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| std::path::PathBuf::from(dir).join("HubbyPDF").join(name))
}

pub fn load_dark() -> bool {
    preference_file("theme")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|theme| theme.trim() == "dark")
}

pub fn save_theme(dark: bool) -> std::io::Result<()> {
    save_preference("theme", if dark { "dark" } else { "light" })
}

fn save_preference(name: &str, value: &str) -> std::io::Result<()> {
    if let Some(path) = preference_file(name)
        && let Some(dir) = path.parent()
    {
        std::fs::create_dir_all(dir)?;
        std::fs::write(path, value)?;
    }
    Ok(())
}

pub const FOOTER: i32 = 39;
pub const HERO_W: i32 = 440;
pub const HERO_H: i32 = 256;

pub fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub fn high_contrast() -> bool {
    let mut settings = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.cbSize,
            Some((&mut settings as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && settings.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0
    }
}

pub fn text_width(dc: HDC, label: &mut [u16]) -> i32 {
    let mut rect = RECT::default();
    unsafe {
        DrawTextW(
            dc,
            label,
            &mut rect,
            DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT,
        )
    };
    rect.right
}

pub fn draw_button(
    item: &DRAWITEMSTRUCT,
    active: bool,
    primary: bool,
    dpi: f32,
    contrast: bool,
    dark: bool,
    icon_font: HFONT,
) {
    let base_color = colorref(if dark { DARK_SURFACE } else { LIGHT_SURFACE });
    let id = item.CtlID as u16;
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    let (fill, border, text) = unsafe {
        if contrast {
            let bg = if (active || pressed) && !disabled {
                COLOR_HIGHLIGHT
            } else {
                COLOR_BTNFACE
            };
            let fg = if disabled {
                COLOR_GRAYTEXT
            } else if bg == COLOR_HIGHLIGHT {
                COLOR_HIGHLIGHTTEXT
            } else {
                COLOR_BTNTEXT
            };
            (
                COLORREF(GetSysColor(bg)),
                COLORREF(GetSysColor(fg)),
                COLORREF(GetSysColor(fg)),
            )
        } else if dark {
            if disabled {
                (base_color, base_color, colorref(0x8793aa))
            } else if primary {
                (
                    colorref(if pressed { 0x9aaff3 } else { 0xb2c3ff }),
                    colorref(0xb2c3ff),
                    colorref(0x182541),
                )
            } else if active || pressed {
                (colorref(0x35456c), colorref(0x657cba), colorref(0xf1f5ff))
            } else if matches!(id, super::ZOOM_PICKER | super::THEME) {
                (colorref(0x2a3345), colorref(0x54647e), colorref(DARK_TEXT))
            } else {
                (base_color, base_color, colorref(DARK_TEXT))
            }
        } else if disabled {
            (rgb(248, 249, 252), rgb(248, 249, 252), rgb(100, 110, 128))
        } else if primary {
            (
                if pressed {
                    rgb(37, 55, 107)
                } else if dark {
                    colorref(0xb2c3ff)
                } else {
                    rgb(50, 73, 140)
                },
                rgb(50, 73, 140),
                rgb(255, 255, 255),
            )
        } else if active || pressed {
            (rgb(224, 231, 251), rgb(131, 150, 200), rgb(34, 55, 110))
        } else if matches!(id, super::ZOOM_PICKER | super::THEME) {
            (rgb(255, 255, 255), rgb(130, 143, 166), rgb(42, 53, 76))
        } else {
            (rgb(248, 249, 252), rgb(248, 249, 252), rgb(42, 53, 76))
        }
    };
    unsafe {
        let r = item.rcItem;
        let dc = item.hDC;
        let radius = (10.0 * dpi).round() as i32;
        let base = CreateSolidBrush(if contrast {
            COLORREF(GetSysColor(COLOR_BTNFACE))
        } else {
            base_color
        });
        FillRect(dc, &r, base);
        let _ = DeleteObject(base);
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(pen);
        let _ = DeleteObject(brush);
        let font = SendMessageW(item.hwndItem, WM_GETFONT, WPARAM(0), LPARAM(0));
        let old_font = SelectObject(dc, HGDIOBJ(font.0 as *mut _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, text);
        let mut label = [0; 64];
        let len = GetWindowTextW(item.hwndItem, &mut label) as usize;
        let mut label_rect = r;
        if !primary && let Some(&(_, _, glyph)) = TOOLS.iter().find(|tool| tool.0 == id) {
            let icon_size = (ICON_SIZE as f32 * dpi).round() as i32;
            let gap = (ICON_GAP as f32 * dpi).round() as i32;
            let label_width = text_width(dc, &mut label[..len]);
            let left = r.left + ((r.right - r.left) - (icon_size + gap + label_width)).max(0) / 2;
            let mut icon_rect = RECT {
                left,
                right: left + icon_size,
                ..r
            };
            let previous = SelectObject(dc, icon_font);
            let mut icon = [if id == super::THEME && dark {
                0xe706
            } else {
                glyph
            }];
            DrawTextW(
                dc,
                &mut icon,
                &mut icon_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
            label_rect.left = icon_rect.right + gap;
            label_rect.right = (label_rect.left + label_width).min(r.right);
        }
        DrawTextW(
            dc,
            &mut label[..len],
            &mut label_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, old_font);
        if focused {
            let focus_pen = CreatePen(
                PS_SOLID,
                2,
                if primary || contrast {
                    text
                } else if dark {
                    colorref(0xb2c3ff)
                } else {
                    rgb(50, 73, 140)
                },
            );
            let old_pen = SelectObject(dc, focus_pen);
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let _ = RoundRect(
                dc,
                r.left + 3,
                r.top + 3,
                r.right - 3,
                r.bottom - 3,
                radius,
                radius,
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(focus_pen);
        }
    }
}

pub fn font(size: i32, weight: i32) -> HFONT {
    create_font(size, weight, w!("Segoe UI"))
}

pub fn icon_font(size: i32) -> HFONT {
    create_font(size, 400, w!("Segoe MDL2 Assets"))
}

fn create_font(size: i32, weight: i32, family: windows::core::PCWSTR) -> HFONT {
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            0,
            0,
            CLEARTYPE_QUALITY.0 as u32,
            0,
            family,
        )
    }
}
