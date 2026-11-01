use super::*;

/// A one-page PDF `w`×`h` points with a red square 100 points from its lower-left corner, as a
/// minimal hand-written file (xref offsets computed).
fn pdf(w: u32, h: u32, pages: usize) -> Vec<u8> {
    let content = b"1 0 0 rg 100 100 72 72 re f";
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for i in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + i * 2));
        objs.push(format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), std::str::from_utf8(content).unwrap()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

#[test]
fn a_page_opens_at_300_ppi_on_white_with_its_drawing() {
    let bytes = pdf(612, 792, 1);
    assert!(is_pdf(&bytes));
    let r = import("letter.pdf", &bytes, &photocraft_raster::Interrupt::NONE).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (2550, 3300), "8.5 × 11 in at 300 ppi");
    assert!((d.resolution_dpi - 300.0).abs() < 0.01);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let s = d.layers[0].surface().unwrap();
    let white = s.pixel(10, 10);
    assert!(white[0] > 0.99 && white[1] > 0.99 && white[2] > 0.99, "{white:?}");
    // The square: 100..172 points from the bottom-left, so y from the top is 792-172..792-100.
    let (x, y) = ((136.0 * 300.0 / 72.0) as i32, ((792.0 - 136.0) * 300.0 / 72.0) as i32);
    let red = s.pixel(x, y);
    assert!(red[0] > 0.99 && red[1] < 0.01 && red[2] < 0.01, "{red:?}");
}

#[test]
fn several_pages_open_the_first_and_say_so() {
    let r = import("two.pdf", &pdf(200, 100, 2), &photocraft_raster::Interrupt::NONE).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("page 1 of 2")), "{:?}", r.warnings);
}

#[test]
fn a_huge_page_opens_smaller() {
    let r = import("poster.pdf", &pdf(14400, 14400, 1), &photocraft_raster::Interrupt::NONE).unwrap();
    let d = &r.document;
    assert!(u64::from(d.size.width) * u64::from(d.size.height) <= 100_000_000 && d.size.width <= 30_000);
    assert!(r.warnings.iter().any(|w| w.contains("ppi")), "{:?}", r.warnings);
}

#[test]
fn damaged_or_empty_input_is_an_error_not_a_panic() {
    for bad in [&b"%PDF-1.4\n"[..], b"%PDF-1.7\ngarbage garbage", b"", b"%PDF"] {
        assert!(import("bad.pdf", bad, &photocraft_raster::Interrupt::NONE).is_err());
    }
    // Truncated real files at every length near the end.
    let good = pdf(100, 100, 1);
    for cut in (good.len().saturating_sub(40)..good.len()).step_by(7) {
        let _ = import("cut.pdf", &good[..cut], &photocraft_raster::Interrupt::NONE);
    }
    assert_eq!(scale_for(0.0, 10.0), None);
    assert_eq!(scale_for(f32::NAN, 10.0), None);
}

#[test]
fn open_reaches_the_pdf_importer_by_magic_and_by_extension() {
    let bytes = pdf(72, 72, 1);
    let r = crate::import_with_options("x.pdf", &bytes, &Default::default()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (300, 300));
    let r = crate::import_with_options("renamed.bin", &bytes, &Default::default()).unwrap();
    assert_eq!(r.document.size.width, 300);
}
