use std::collections::BTreeSet;

use super::*;

/// A PDF of `pages` pages, each `w`×`h` points with a red square.
fn pdf(w: u32, h: u32, pages: usize) -> Vec<u8> {
    let content = "1 0 0 rg 10 10 40 40 re f";
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for i in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + i * 2));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
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

fn app() -> PhotocraftApp {
    let services = crate::Services {
        import: Some(Box::new(|name, bytes, opts| photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    ctx.begin_pass(egui::RawInput::default());
    show(app, ctx);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
}

/// Frames until `done` (or a time limit), letting the workers run.
fn until(app: &mut PhotocraftApp, ctx: &egui::Context, done: impl Fn(&PhotocraftApp) -> bool) {
    let t0 = std::time::Instant::now();
    while !done(app) && t0.elapsed() < std::time::Duration::from_secs(30) {
        frame(app, ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn a_pdf_with_several_pages_shows_its_pages_and_opens_the_picked_ones() {
    let mut app = app();
    let ctx = egui::Context::default();
    let warnings = app.open_bytes("Report.pdf", &pdf(300, 200, 3)).unwrap();
    assert!(warnings.is_empty());
    assert!(app.session.documents().is_empty(), "nothing opens before pages are picked");
    frame(&mut app, &ctx);
    let p = app.pdf_pages.picker.as_ref().expect("the page picker shows");
    assert_eq!(p.pages, 3);
    assert_eq!(p.selected, BTreeSet::from([0]), "the first page starts picked");
    // Thumbnails arrive from the worker.
    until(&mut app, &ctx, |a| a.pdf_pages.picker.as_ref().is_some_and(|p| p.thumbs.iter().all(Option::is_some)));
    assert!(app.pdf_pages.picker.as_ref().unwrap().thumbs.iter().all(Option::is_some));
    // Pages 1 and 3, opened as two documents.
    let p = app.pdf_pages.picker.as_mut().unwrap();
    click(p, 2, egui::Modifiers::COMMAND);
    load(&ctx, p);
    until(&mut app, &ctx, |a| a.pdf_pages.picker.is_none());
    assert_eq!(names(&app), ["Report - Page 1", "Report - Page 3"]);
    let d = &app.session.documents()[0];
    assert_eq!((d.doc.size.width, d.doc.size.height), (1250, 833));
    assert_eq!(d.path, None, "a page never saves back over the PDF");
}

#[test]
fn a_one_page_pdf_opens_at_once() {
    let mut app = app();
    app.open_bytes("One.pdf", &pdf(72, 72, 1)).unwrap();
    assert_eq!(names(&app), ["One.pdf"]);
    assert!(app.pdf_pages.queue.is_empty() && app.pdf_pages.picker.is_none());
}

#[test]
fn cancel_opens_nothing_and_the_next_pdf_waits_its_turn() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.open_bytes("A.pdf", &pdf(100, 100, 2)).unwrap();
    app.open_bytes("B.pdf", &pdf(100, 100, 4)).unwrap();
    frame(&mut app, &ctx);
    assert_eq!(app.pdf_pages.picker.as_ref().unwrap().name, "A.pdf");
    // Esc closes it; B's window comes next.
    ctx.begin_pass(egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() });
    show(&mut app, &ctx);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(app.pdf_pages.picker.is_none());
    frame(&mut app, &ctx);
    assert_eq!(app.pdf_pages.picker.as_ref().unwrap().pages, 4);
    assert!(app.session.documents().is_empty());
}

#[test]
fn dropped_on_a_canvas_the_picked_pages_are_placed_one_after_another() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
    app.drop_places.push_back(crate::file_open::DropPlace::Bytes("Flyer.pdf".into(), pdf(100, 50, 2)));
    app.place_next_dropped(&ctx);
    assert!(app.drop_places.is_empty());
    frame(&mut app, &ctx);
    let p = app.pdf_pages.picker.as_mut().expect("the page picker shows");
    assert_eq!(p.purpose, Purpose::Place);
    p.selected = (0..2).collect();
    load(&ctx, p);
    until(&mut app, &ctx, |a| a.pdf_pages.picker.is_none());
    let queued: Vec<String> = app
        .drop_places
        .iter()
        .map(|d| match d {
            crate::file_open::DropPlace::Bytes(name, _) => name.clone(),
            crate::file_open::DropPlace::File(_) => String::new(),
        })
        .collect();
    assert_eq!(queued, ["Flyer - Page 1.png", "Flyer - Page 2.png"]);
    // The first is placed (in Free Transform), as any dropped image.
    let layers = app.session.active().unwrap().doc.layers.len();
    app.place_next_dropped(&ctx);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn clicks_pick_one_add_and_range() {
    let ctx = egui::Context::default();
    let mut p = start(&ctx, "x.pdf".into(), Arc::new(pdf(50, 50, 6)), Purpose::Open, 6, false);
    click(&mut p, 2, egui::Modifiers::NONE);
    assert_eq!(p.selected, BTreeSet::from([2]));
    click(&mut p, 4, egui::Modifiers::COMMAND);
    assert_eq!(p.selected, BTreeSet::from([2, 4]));
    click(&mut p, 2, egui::Modifiers::COMMAND);
    assert_eq!(p.selected, BTreeSet::from([4]));
    click(&mut p, 1, egui::Modifiers::SHIFT);
    assert_eq!(p.selected, BTreeSet::from([1, 2]), "from the last page clicked");
    click(&mut p, 5, egui::Modifiers::SHIFT);
    assert_eq!(p.selected, BTreeSet::from([2, 3, 4, 5]));
    assert_eq!(page_name("Book.PDF", 9), "Book - Page 10");
    assert_eq!(page_name("notes", 0), "notes - Page 1");
}

#[test]
fn agents_open_the_first_page_without_a_window() {
    let mut app = app();
    app.automation_input = true;
    app.open_bytes("Many.pdf", &pdf(72, 72, 3)).unwrap();
    assert_eq!(names(&app), ["Many.pdf"]);
}


/// A password-protected PDF asks for its password; a wrong one says so; the right one shows the
/// pages, and the picked ones open.
#[test]
fn a_locked_pdf_asks_for_its_password_then_shows_its_pages() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.open_bytes("Locked.pdf", &photocraft_testkit::pdf::pdf(200, 100, 3, Some("secret"))).unwrap();
    frame(&mut app, &ctx);
    let p = app.pdf_pages.picker.as_mut().expect("the window shows");
    assert!(p.locked && p.pages == 0 && p.thumb_rx.is_none(), "nothing is drawn before it's unlocked");
    p.typed = "nope".into();
    let mut p = app.pdf_pages.picker.take().unwrap();
    assert!(unlock(&mut app, &ctx, &mut p));
    assert!(p.locked && p.wrong, "a wrong password keeps it locked and says so");
    p.typed = "secret".into();
    assert!(unlock(&mut app, &ctx, &mut p));
    assert!(!p.locked && p.pages == 3 && p.typed.is_empty());
    app.pdf_pages.picker = Some(p);
    until(&mut app, &ctx, |a| a.pdf_pages.picker.as_ref().is_some_and(|p| p.thumbs.iter().all(Option::is_some)));
    let p = app.pdf_pages.picker.as_mut().unwrap();
    p.selected = (0..3).collect();
    load(&ctx, p);
    until(&mut app, &ctx, |a| a.pdf_pages.picker.is_none());
    assert_eq!(names(&app), ["Locked - Page 1", "Locked - Page 2", "Locked - Page 3"]);
}


/// A locked PDF of one page opens as soon as its password is in (no page grid); a locked one of
/// two shows its grid.
#[test]
fn a_locked_single_page_opens_once_unlocked() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.open_bytes("One.pdf", &photocraft_testkit::pdf::pdf(72, 72, 1, Some("secret"))).unwrap();
    frame(&mut app, &ctx);
    let mut p = app.pdf_pages.picker.take().expect("it asks for the password");
    p.typed = "secret".into();
    assert!(unlock(&mut app, &ctx, &mut p));
    assert!(p.loading.is_some() && p.thumb_rx.is_none(), "one page: opened at once");
    app.pdf_pages.picker = Some(p);
    until(&mut app, &ctx, |a| a.pdf_pages.picker.is_none());
    assert_eq!(names(&app), ["One - Page 1"]);
    app.open_bytes("Two.pdf", &photocraft_testkit::pdf::pdf(72, 72, 2, Some("secret"))).unwrap();
    frame(&mut app, &ctx);
    let mut p = app.pdf_pages.picker.take().unwrap();
    p.typed = "secret".into();
    assert!(unlock(&mut app, &ctx, &mut p));
    assert_eq!(p.pages, 2);
    assert!(p.loading.is_none() && p.thumb_rx.is_some(), "two pages: the grid");
}
