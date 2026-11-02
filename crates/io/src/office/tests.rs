use photocraft_format::zip::ZipWriter;

/// A minimal Word document with `paragraphs` paragraphs and a page break after each but the last.
fn docx(paragraphs: &[&str]) -> Vec<u8> {
    let mut body = String::new();
    for (i, p) in paragraphs.iter().enumerate() {
        body.push_str(&format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"));
        if i + 1 < paragraphs.len() {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
    }
    let mut z = ZipWriter::new();
    z.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
    )
    .unwrap();
    z.add(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
    )
    .unwrap();
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>"#
    );
    z.add("word/document.xml", doc.as_bytes()).unwrap();
    z.finish().unwrap()
}

#[test]
fn a_word_document_becomes_its_pages() {
    let bytes = docx(&["Hello from page one", "And page two"]);
    assert!(super::kind("letter.docx", &bytes).is_some());
    assert!(super::kind("renamed.bin", &bytes).is_some(), "found by its parts");
    let (pdf, _) = super::to_pdf("letter.docx", &bytes).unwrap();
    assert!(crate::pdf_import::is_pdf(&pdf));
    assert_eq!(crate::pdf_import::page_count(&pdf).unwrap(), 2);
    // Opened (an agent's File › Open): the first page, a Letter page at 300 ppi.
    let r = crate::import_with_options("letter.docx", &bytes, &Default::default()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (2550, 3300));
    // Something is printed on it (the text), not a blank page.
    let s = r.document.layers[0].surface().unwrap();
    let dark = (300..1500).step_by(3).flat_map(|x| (300..420).step_by(3).map(move |y| (x, y))).any(|(x, y)| s.pixel(x, y)[0] < 0.5);
    assert!(dark, "the first line of text is drawn");
}

#[test]
fn broken_office_files_are_errors_not_panics() {
    assert!(super::to_pdf("x.docx", b"").is_err());
    assert!(super::to_pdf("x.docx", b"PK\x03\x04 broken").is_err());
    assert!(super::to_pdf("x.xlsx", b"not a zip").is_err());
    assert!(super::to_pdf("x.txt", b"hello").is_err());
    assert!(!super::has_extension("docx"));
    assert!(super::has_extension("Report.DOCX"));
}
