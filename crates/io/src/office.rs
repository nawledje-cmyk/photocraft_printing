//! Word, Excel and PowerPoint files (.docx, .xlsx, .pptx), opened as their printed pages: laid out
//! as a PDF by `office2pdf` (pure Rust, Apache-2.0), then opened like any PDF ([`crate::pdf_import`]).
//! Desktop only.

use crate::IoError;

/// The extensions read here.
pub const EXTENSIONS: &[&str] = &["docx", "xlsx", "pptx"];

/// What kind of Office file `bytes` (called `name`) is: by its extension, or by the part only that
/// kind of package has.
pub fn kind(name: &str, bytes: &[u8]) -> Option<office2pdf::config::Format> {
    use office2pdf::config::Format;
    let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    let by_name = match ext.as_str() {
        "docx" => Some(Format::Docx),
        "xlsx" => Some(Format::Xlsx),
        "pptx" => Some(Format::Pptx),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    if !bytes.starts_with(b"PK\x03\x04") {
        return None;
    }
    let zip = photocraft_format::zip::ZipReader::new(bytes).ok()?;
    [("word/document.xml", Format::Docx), ("xl/workbook.xml", Format::Xlsx), ("ppt/presentation.xml", Format::Pptx)]
        .into_iter()
        .find(|(part, _)| zip.find(part).is_some())
        .map(|(_, f)| f)
}

/// Whether `name` (a file name) has one of [`EXTENSIONS`].
pub fn has_extension(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    name.contains('.') && EXTENSIONS.contains(&ext.as_str())
}

/// The Office file `bytes` laid out as a PDF, with what the layout couldn't keep.
pub fn to_pdf(name: &str, bytes: &[u8]) -> Result<(Vec<u8>, Vec<String>), IoError> {
    use office2pdf::error::ConvertError;
    let format = kind(name, bytes).ok_or_else(|| IoError::Unsupported(format!("{name}: not a Word, Excel or PowerPoint file")))?;
    let options = office2pdf::config::ConvertOptions { xlsx_ui_script: office2pdf::config::XlsxUiScript::Latin, ..Default::default() };
    // The layout engine is a large third-party crate: a panic in it is an error, never a crash.
    let converted = std::panic::catch_unwind(|| office2pdf::convert_bytes(bytes, format, &options))
        .map_err(|_| IoError::Read(format!("{name}: the document couldn't be laid out (internal error in the converter)")))?;
    let r = converted.map_err(|e| match e {
        ConvertError::UnsupportedEncryption => IoError::Unsupported(format!("{name}: the document is password-protected; save an unprotected copy and open that")),
        ConvertError::UnsupportedFormat(m) => IoError::Unsupported(format!("{name}: {m}")),
        e => IoError::Read(format!("{name}: {e}")),
    })?;
    let mut warnings: Vec<String> = r.warnings.iter().map(|w| format!("{name}: {w}")).collect();
    warnings.truncate(20);
    Ok((r.pdf, warnings))
}

#[cfg(test)]
mod tests;
