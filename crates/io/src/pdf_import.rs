//! PDF import: a page rasterised by `hayro` (pure Rust), as Photoshop's Import PDF opens a page as
//! pixels. The first page opens, at 300 ppi on white (Photoshop's defaults), capped so a huge page
//! stays within a sane pixel count. Encrypted PDFs are refused with a message.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

use crate::{ImportResult, IoError};

/// The resolution a page opens at (Photoshop's Import PDF default).
pub const PPI: f32 = 300.0;
/// The largest page opened, in pixels (and per side: the renderer works in 16-bit sizes).
const MAX_PIXELS: f32 = 100_000_000.0;
const MAX_SIDE: f32 = 30_000.0;

/// Whether `bytes` is a PDF: `%PDF-` within the first KiB (some writers put junk before it).
pub fn is_pdf(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or_default();
    head.windows(5).any(|w| w == b"%PDF-")
}

/// The pixel scale (pixels per PDF point) for a page `w`×`h` points: 300 ppi, less for a page that
/// would be too large. `None` for an empty or invalid page.
fn scale_for(w: f32, h: f32) -> Option<f32> {
    if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0) {
        return None;
    }
    let mut scale = PPI / 72.0;
    scale = scale.min(MAX_SIDE / w).min(MAX_SIDE / h);
    let px = w * h * scale * scale;
    if px > MAX_PIXELS {
        scale *= (MAX_PIXELS / px).sqrt();
    }
    (scale > 0.0 && (w * scale) >= 1.0 && (h * scale) >= 1.0).then_some(scale)
}

/// `page` (`w`×`h` points) drawn at `scale` pixels per point on white. The size is rounded, not
/// truncated (`hayro::render` truncates, so 8.5 × 11 in came out 2550 × 3299 at 300 ppi).
fn render(page: &hayro::hayro_syntax::page::Page<'_>, w: f32, h: f32, scale: f32) -> hayro::vello_cpu::Pixmap {
    use hayro::hayro_interpret::util::TransformExt as _;
    use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
    let side = |v: f32| (v * scale).round().clamp(1.0, MAX_SIDE) as u16;
    let mut ctx = RenderContext::new(side(w), side(h));
    let transform = hayro::kurbo::Affine::scale(f64::from(scale)) * page.initial_transform(true).to_kurbo();
    hayro::render_into(page, &hayro::RenderCache::new(), &Default::default(), &hayro::RenderSettings::default(), &mut ctx, transform);
    ctx.flush();
    let mut pixmap = Pixmap::new(ctx.width(), ctx.height());
    let settings = RasterizerSettings { target_init: TargetInit::Clear(hayro::vello_cpu::color::palette::css::WHITE), ..Default::default() };
    ctx.render_with(&mut pixmap, &mut Resources::default(), settings);
    pixmap
}

/// Open the first page of the PDF `bytes` as a one-layer RGB document called `name`.
pub fn import(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    use hayro::hayro_syntax::{LoadPdfError, Pdf};
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| match e {
        LoadPdfError::Decryption(_) => IoError::Unsupported("PDF: the file is password-protected or encrypted; save an unprotected copy and open that".into()),
        LoadPdfError::Invalid => IoError::Read("PDF: the file is damaged or not a PDF".into()),
    })?;
    let pages = pdf.pages();
    let page = pages.first().ok_or_else(|| IoError::Read("PDF: the file has no pages".into()))?;
    let (w, h) = page.render_dimensions();
    let scale = scale_for(w, h).ok_or_else(|| IoError::Read(format!("PDF: the first page has no usable size ({w} × {h} points)")))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let pixmap = render(page, w, h, scale);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let (pw, ph) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    if pw == 0 || ph == 0 {
        return Err(IoError::Read("PDF: the first page rendered empty".into()));
    }
    let mut doc = Document::new(name, Size::new(pw, ph), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 72.0 * scale;
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; (pw as usize).saturating_mul(ph as usize).saturating_mul(n)];
    for (p, out) in pixmap.data().iter().zip(data.chunks_exact_mut(n)) {
        // Premultiplied over white, so nearly always opaque; undo it where it isn't.
        let a = f32::from(p.a) / 255.0;
        let un = |c: u8| if a > 0.0 { (f32::from(c) / 255.0 / a).min(1.0) } else { 0.0 };
        photocraft_raster::from_rgba_into(&fmt, [un(p.r), un(p.g), un(p.b), a], out);
    }
    let mut surface = Surface::new(fmt);
    surface.write_region(Rect::new(0, 0, pw as i32, ph as i32), &data);
    surface.prune();
    // Rendered on white, so opaque: the locked Background layer, as a flat image opens.
    let mut layer = Layer::new("Background", LayerContent::Raster(surface));
    layer.locks.transparency = true;
    layer.locks.position = true;
    doc.layers.push(layer);
    let mut warnings = Vec::new();
    if pages.len() > 1 {
        warnings.push(format!("PDF: opened page 1 of {}; PhotoCraft opens the first page of a PDF", pages.len()));
    }
    if (scale - PPI / 72.0).abs() > 1e-3 {
        warnings.push(format!("PDF: the page is very large, so it opened at {:.0} ppi instead of {PPI:.0}", 72.0 * scale));
    }
    Ok(ImportResult { document: doc, warnings, source_read_only: false, preview_only: false })
}

#[cfg(test)]
mod tests;
