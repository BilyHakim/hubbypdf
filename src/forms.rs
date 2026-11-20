use std::{ffi::c_void, slice};

use pdfium_render::prelude::*;

use crate::{
    chrome::Message,
    pdf::{Point, Result},
};

pub enum Value {
    Text(String),
    Choice(i32),
    Toggle,
}

pub enum Control {
    Text {
        value: String,
        multiline: bool,
        password: bool,
    },
    Choice(Vec<PdfFormFieldOption>),
    Checkbox,
    ReadOnly,
}

pub struct Field {
    pub index: usize,
    pub bounds: (Point, Point),
    pub control: Control,
}

pub fn hit(
    page: &PdfPage<'_>,
    at: Point,
    width: i32,
    height: i32,
    config: &PdfRenderConfig,
) -> Result<Option<Field>> {
    let annotations = page.annotations();
    for index in (0..annotations.len()).rev() {
        let annotation = annotations.get(index)?;
        let Some(field) = annotation.as_form_field() else {
            continue;
        };
        if annotation.is_hidden() || annotation.is_printable_but_not_viewable() {
            continue;
        }
        let rect = annotation.bounds()?;
        let (x1, y1) = page.points_to_pixels(rect.left(), rect.top(), config)?;
        let (x2, y2) = page.points_to_pixels(rect.right(), rect.bottom(), config)?;
        let bounds = (
            (
                x1.min(x2) as f32 / width as f32,
                y1.min(y2) as f32 / height as f32,
            ),
            (
                x1.max(x2) as f32 / width as f32,
                y1.max(y2) as f32 / height as f32,
            ),
        );
        let (a, b) = bounds;
        if !(a.0..=b.0).contains(&at.0) || !(a.1..=b.1).contains(&at.1) {
            continue;
        }
        let control = if field.is_read_only() {
            Control::ReadOnly
        } else {
            match field {
                PdfFormField::Text(text) if !text.is_file_select() => Control::Text {
                    value: text.value().unwrap_or_default(),
                    multiline: text.is_multiline() && !text.is_password(),
                    password: text.is_password(),
                },
                PdfFormField::ComboBox(combo) => Control::Choice(combo.options().iter().collect()),
                PdfFormField::Checkbox(_) => Control::Checkbox,
                _ => continue,
            }
        };
        return Ok(Some(Field {
            index,
            bounds,
            control,
        }));
    }
    Ok(None)
}

fn edit_error() -> Message {
    Message((
        "Kolom PDF ini gagal diperbarui.",
        "This PDF field could not be updated.",
    ))
}

// pdfium-render exposes read-only widget metadata but keeps the form handle private.
pub fn apply(
    bindings: &dyn PdfiumLibraryBindings,
    source: &[u8],
    changes: impl Iterator<Item = (u16, usize, impl std::borrow::Borrow<Value>)>,
) -> Result<Vec<u8>> {
    let document = bindings.FPDF_LoadMemDocument64(source, None);
    if document.is_null() {
        return Err(edit_error().into());
    }
    let mut info: FPDF_FORMFILLINFO = unsafe { std::mem::zeroed() };
    info.version = 1;
    let form = bindings.FPDFDOC_InitFormFillEnvironment(document, &mut info);
    let result = (|| -> Result<Vec<u8>> {
        if form.is_null() {
            return Err(edit_error().into());
        }
        for (index, widget, value) in changes {
            let page = bindings.FPDF_LoadPage(document, i32::from(index));
            if page.is_null() {
                return Err(edit_error().into());
            }
            bindings.FORM_OnAfterLoadPage(page, form);
            let annot = bindings.FPDFPage_GetAnnot(page, i32::try_from(widget).unwrap_or(-1));
            let changed = if annot.is_null() {
                false
            } else {
                let changed = bindings.FPDFAnnot_GetFormFieldFlags(form, annot) & 1 == 0
                    && apply_value(bindings, page, form, annot, value.borrow());
                bindings.FPDFPage_CloseAnnot(annot);
                changed
            };
            bindings.FORM_OnBeforeClosePage(page, form);
            bindings.FPDF_ClosePage(page);
            if !changed {
                return Err(edit_error().into());
            }
        }
        let mut writer = FormWriter {
            base: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            bytes: Vec::new(),
        };
        if !bindings.is_true(bindings.FPDF_SaveAsCopy(document, &mut writer.base, 0)) {
            return Err(edit_error().into());
        }
        Ok(writer.bytes)
    })();
    if !form.is_null() {
        bindings.FPDFDOC_ExitFormFillEnvironment(form);
    }
    bindings.FPDF_CloseDocument(document);
    result
}

fn apply_value(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    form: FPDF_FORMHANDLE,
    annot: FPDF_ANNOTATION,
    value: &Value,
) -> bool {
    let field_type = bindings.FPDFAnnot_GetFormFieldType(form, annot);
    if !bindings.is_true(bindings.FORM_SetFocusedAnnot(form, annot)) {
        return false;
    }
    let was_checked =
        field_type == 2 && bindings.is_true(bindings.FPDFAnnot_IsChecked(form, annot));
    let changed = match (field_type, value) {
        (6, Value::Text(text)) if bindings.is_true(bindings.FORM_SelectAllText(form, page)) => {
            let wide: Vec<_> = text.encode_utf16().chain([0]).collect();
            bindings.FORM_ReplaceSelection(form, page, wide.as_ptr());
            true
        }
        (4, Value::Choice(index)) => {
            bindings.is_true(bindings.FORM_SetIndexSelected(form, page, *index, 1))
        }
        (2, Value::Toggle) => {
            let mut rect = FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            if !bindings.is_true(bindings.FPDFAnnot_GetRect(annot, &mut rect)) {
                return false;
            }
            let x = f64::from(rect.left + rect.right) / 2.0;
            let y = f64::from(rect.top + rect.bottom) / 2.0;
            bindings.FORM_OnLButtonDown(form, page, 0, x, y);
            bindings.is_true(bindings.FORM_OnLButtonUp(form, page, 0, x, y))
        }
        _ => false,
    };
    if !changed || !bindings.is_true(bindings.FORM_ForceToKillFocus(form)) {
        return false;
    }
    match value {
        Value::Text(text) => {
            let length = bindings.FPDFAnnot_GetFormFieldValue(form, annot, std::ptr::null_mut(), 0);
            let mut actual = vec![0; length as usize / 2];
            if length > 0 {
                bindings.FPDFAnnot_GetFormFieldValue(form, annot, actual.as_mut_ptr(), length);
            }
            let end = actual
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(actual.len());
            normalize_lines(&String::from_utf16_lossy(&actual[..end])) == normalize_lines(text)
        }
        Value::Choice(index) => {
            bindings.is_true(bindings.FPDFAnnot_IsOptionSelected(form, annot, *index))
        }
        Value::Toggle => bindings.is_true(bindings.FPDFAnnot_IsChecked(form, annot)) != was_checked,
    }
}

pub fn normalize_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[repr(C)]
struct FormWriter {
    base: FPDF_FILEWRITE,
    bytes: Vec<u8>,
}

unsafe extern "C" fn write_block(
    writer: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: u32,
) -> i32 {
    if size == 0 {
        return 1;
    }
    // PDFium supplies a readable block for the duration of this callback.
    let writer = unsafe { &mut *writer.cast::<FormWriter>() };
    if writer.bytes.try_reserve(size as usize).is_err() {
        return 0;
    }
    writer
        .bytes
        .extend_from_slice(unsafe { slice::from_raw_parts(data.cast(), size as usize) });
    1
}
