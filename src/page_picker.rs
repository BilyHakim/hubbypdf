use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{DRAWITEMSTRUCT, ODS_COMBOBOXEDIT, ODS_FOCUS, ODS_SELECTED},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use crate::{
    PAGE_PICKER,
    chrome::{self, Language},
    pdf::{Document, Result},
    wide,
};

pub fn create(parent: HWND) -> Result<HWND> {
    Ok(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("COMBOBOX"),
            w!(""),
            WS_CHILD
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE(
                    (CBS_DROPDOWNLIST | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS | CBS_NOINTEGRALHEIGHT)
                        as u32,
                ),
            0,
            0,
            128,
            400,
            parent,
            HMENU(PAGE_PICKER as usize as *mut _),
            GetModuleHandleW(None)?,
            None,
        )?
    })
}

pub fn layout(picker: HWND, height: i32, footer: i32, dpi: f32) -> Result<()> {
    let px = |value: i32| (value as f32 * dpi).round() as i32;
    unsafe {
        for (index, size) in [(usize::MAX, px(24)), (0, px(84))] {
            if SendMessageW(picker, CB_GETITEMHEIGHT, WPARAM(index), LPARAM(0)).0 != size as isize {
                SendMessageW(
                    picker,
                    CB_SETITEMHEIGHT,
                    WPARAM(index),
                    LPARAM(size as isize),
                );
            }
        }
        MoveWindow(
            picker,
            px(16),
            (height - footer + px(5)).max(0),
            px(128),
            px(400),
            true,
        )?;
    }
    Ok(())
}

pub fn populate(picker: HWND, count: u16) -> Result<()> {
    unsafe {
        SendMessageW(picker, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
        if SendMessageW(
            picker,
            CB_INITSTORAGE,
            WPARAM(count.into()),
            LPARAM(count as isize * 12),
        )
        .0 < 0
        {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8007000e_u32 as i32,
            ))
            .into());
        }
        for number in 1..=count {
            let label = wide(&number.to_string());
            if SendMessageW(
                picker,
                CB_ADDSTRING,
                WPARAM(0),
                LPARAM(label.as_ptr() as isize),
            )
            .0 < 0
            {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x8007000e_u32 as i32,
                ))
                .into());
            }
        }
    }
    Ok(())
}

pub fn selection(picker: HWND) -> Option<u16> {
    u16::try_from(unsafe { SendMessageW(picker, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 }).ok()
}

pub fn sync(picker: HWND, doc: Option<&Document>) {
    if picker.is_invalid() {
        return;
    }
    unsafe {
        if let Some(doc) = doc {
            let _ = ShowWindow(picker, SW_SHOWNA);
            if selection(picker) != Some(doc.index) {
                SendMessageW(picker, CB_SETCURSEL, WPARAM(doc.index.into()), LPARAM(0));
            }
            let _ = InvalidateRect(picker, None, false);
        } else {
            let _ = ShowWindow(picker, SW_HIDE);
        }
    }
}

pub fn draw(
    item: &DRAWITEMSTRUCT,
    doc: &Document,
    language: Language,
    dpi: f32,
    dark: bool,
    contrast: bool,
) {
    let closed = item.itemState.0 & ODS_COMBOBOXEDIT.0 != 0;
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let px = |value: i32| (value as f32 * dpi).round() as i32;
    let mut rect = item.rcItem;
    let index = u16::try_from(item.itemID).unwrap_or(doc.index);
    unsafe {
        let highlighted = selected && !closed;
        let background = if contrast || highlighted {
            COLORREF(GetSysColor(if highlighted {
                COLOR_HIGHLIGHT
            } else {
                COLOR_WINDOW
            }))
        } else {
            chrome::colorref(if dark {
                chrome::DARK_SURFACE
            } else {
                chrome::LIGHT_SURFACE
            })
        };
        let foreground = if contrast || highlighted {
            COLORREF(GetSysColor(if highlighted {
                COLOR_HIGHLIGHTTEXT
            } else {
                COLOR_WINDOWTEXT
            }))
        } else {
            chrome::colorref(if dark {
                chrome::DARK_TEXT
            } else {
                chrome::LIGHT_TEXT
            })
        };
        let brush = CreateSolidBrush(background);
        FillRect(item.hDC, &rect, brush);
        let _ = DeleteObject(brush);
        SetBkMode(item.hDC, TRANSPARENT);
        SetTextColor(item.hDC, foreground);
        let label = if closed {
            rect.left += px(6);
            format!(
                "{} {} / {}",
                language.text(("Halaman", "Page")),
                doc.index + 1,
                doc.count()
            )
        } else if index < doc.count() {
            if let Ok(image) = doc.thumbnail(index, px(52), px(72)) {
                let bitmap = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: image.width,
                        biHeight: -image.height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                StretchDIBits(
                    item.hDC,
                    rect.left + px(6) + (px(52) - image.width) / 2,
                    rect.top + (rect.bottom - rect.top - image.height) / 2,
                    image.width,
                    image.height,
                    0,
                    0,
                    image.width,
                    image.height,
                    Some(image.bytes.as_ptr().cast()),
                    &bitmap,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
            rect.left += px(66);
            (index + 1).to_string()
        } else {
            return;
        };
        DrawTextW(
            item.hDC,
            &mut label.encode_utf16().collect::<Vec<_>>(),
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            let _ = DrawFocusRect(item.hDC, &item.rcItem);
        }
    }
}
