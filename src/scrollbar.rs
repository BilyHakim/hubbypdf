use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::{Error, Result, w},
};

use crate::chrome::colorref;

pub fn create(parent: HWND, control: HWND) -> Result<HWND> {
    unsafe {
        // A plain STATIC passes pointer hit tests through to the native scrollbar.
        let skin = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            parent,
            None,
            GetModuleHandleW(None)?,
            None,
        )?;
        if !SetWindowSubclass(skin, Some(window_proc), 0, control.0 as usize).as_bool() {
            let error = Error::from_win32();
            let _ = DestroyWindow(skin);
            return Err(error);
        }
        SetWindowPos(
            skin,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )?;
        Ok(skin)
    }
}

pub fn theme(skin: HWND, dark: bool) {
    if !skin.is_invalid() {
        unsafe { SetWindowLongPtrW(skin, GWLP_USERDATA, isize::from(dark)) };
    }
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    control: usize,
) -> LRESULT {
    unsafe {
        let control = HWND(control as *mut _);
        match message {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                if GetClientRect(hwnd, &mut rect).is_ok() && !dc.is_invalid() {
                    let buffer = CreateCompatibleDC(dc);
                    let bitmap = CreateCompatibleBitmap(dc, rect.right, rect.bottom);
                    if !buffer.is_invalid() && !bitmap.is_invalid() {
                        let old = SelectObject(buffer, bitmap);
                        draw(hwnd, control, buffer, rect);
                        let _ = BitBlt(dc, 0, 0, rect.right, rect.bottom, buffer, 0, 0, SRCCOPY);
                        SelectObject(buffer, old);
                    } else {
                        draw(hwnd, control, dc, rect);
                    }
                    let _ = DeleteObject(bitmap);
                    let _ = DeleteDC(buffer);
                }
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_PRINTCLIENT => {
                let mut rect = RECT::default();
                if GetClientRect(hwnd, &mut rect).is_ok() {
                    draw(hwnd, control, HDC(wparam.0 as *mut _), rect);
                }
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }
}

fn draw(hwnd: HWND, control: HWND, dc: HDC, rect: RECT) {
    unsafe {
        let (background, rail, thumb) = if GetWindowLongPtrW(hwnd, GWLP_USERDATA) != 0 {
            (0x171e2a, 0x2b3648, 0x7588a6)
        } else {
            (0xe6ebf2, 0xd1d9e6, 0x657692)
        };
        let brush = CreateSolidBrush(colorref(background));
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush);

        let mut info = SCROLLBARINFO {
            cbSize: size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        if GetScrollBarInfo(control, OBJID_CLIENT, &mut info).is_err() {
            return;
        }
        let dpi = GetDpiForWindow(hwnd) as f32 / 96.0;
        let px = |n| (n as f32 * dpi).round() as i32;
        let center = rect.right / 2;
        let end = rect.bottom - info.dxyLineButton;
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        for (width, top, bottom, color) in [
            (px(4), info.dxyLineButton, end, rail),
            (px(8), info.xyThumbTop, info.xyThumbBottom, thumb),
        ] {
            let width = width.clamp(0, rect.right);
            if width > 0 && bottom > top {
                let brush = CreateSolidBrush(colorref(color));
                let old = SelectObject(dc, brush);
                let left = (rect.right - width) / 2;
                let _ = RoundRect(dc, left, top, left + width, bottom, width, width);
                SelectObject(dc, old);
                let _ = DeleteObject(brush);
            }
        }
        SelectObject(dc, old_pen);

        let pen = CreatePen(PS_SOLID, px(1).max(1), colorref(thumb));
        let old = SelectObject(dc, pen);
        for (y, direction) in [
            (info.dxyLineButton / 2, -1),
            (rect.bottom - info.dxyLineButton / 2, 1),
        ] {
            let _ = Polyline(
                dc,
                &[
                    POINT {
                        x: center - px(3),
                        y,
                    },
                    POINT {
                        x: center,
                        y: y + px(2) * direction,
                    },
                    POINT {
                        x: center + px(3),
                        y,
                    },
                ],
            );
        }
        SelectObject(dc, old);
        let _ = DeleteObject(pen);
    }
}

#[cfg(test)]
pub fn verify_skin(skin: HWND, control: HWND) -> Result<()> {
    unsafe {
        assert_eq!(
            SendMessageW(skin, WM_NCHITTEST, WPARAM(0), LPARAM(0)).0,
            HTTRANSPARENT as isize
        );
        let mut rect = RECT::default();
        GetClientRect(skin, &mut rect)?;
        let mut info = SCROLLBARINFO {
            cbSize: size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        GetScrollBarInfo(control, OBJID_CLIENT, &mut info)?;
        assert!(info.xyThumbBottom > info.xyThumbTop);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, rect.right, rect.bottom);
        let old = SelectObject(dc, bitmap);
        let sample = || {
            SendMessageW(skin, WM_PRINTCLIENT, WPARAM(dc.0 as usize), LPARAM(0));
            let y = (info.xyThumbTop + info.xyThumbBottom) / 2;
            (GetPixel(dc, 0, y), GetPixel(dc, rect.right / 2, y))
        };
        let light = sample();
        theme(skin, true);
        let dark = sample();
        theme(skin, false);
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        assert_eq!(light, (colorref(0xe6ebf2), colorref(0x657692)));
        assert_eq!(dark, (colorref(0x171e2a), colorref(0x7588a6)));
        Ok(())
    }
}
