//! Import PDF: a PDF with several pages shows every page as a thumbnail, and the pages the user
//! picks open as documents (or, dropped on a canvas, are placed one after another), as
//! Photoshop's Import PDF dialog does. Thumbnails and pages are drawn on worker threads, so the
//! window keeps drawing; a one-page PDF opens straight away. A password-protected PDF first asks
//! for its password in the same window.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::PhotocraftApp;

/// Thumbnail size, in pixels (drawn at half that in points on a 2× screen).
const THUMB_PX: u32 = 280;
/// A page's cell in the grid, in points.
const CELL: f32 = 150.0;

/// What the picked pages are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// Each opens as its own document.
    Open,
    /// Each is placed on the active document (dropped on its canvas).
    Place,
}

/// A rendered page on its way back from the loader.
type Loaded = (usize, Result<(photocraft_doc::Document, Vec<String>), String>);

/// Picked pages being drawn at full size.
struct Loading {
    rx: Receiver<Loaded>,
    total: usize,
    done: usize,
}

/// The Import PDF window for one file.
pub(crate) struct Picker {
    name: String,
    bytes: Arc<Vec<u8>>,
    purpose: Purpose,
    /// 0 until a locked file is unlocked.
    pages: usize,
    /// Waiting for the password ([`photocraft_io::pdf_import::check`] said so).
    locked: bool,
    /// The password that opened it ("" for none), and what's typed in the password box.
    password: String,
    typed: String,
    wrong: bool,
    thumbs: Vec<Option<egui::TextureHandle>>,
    thumb_rx: Option<Receiver<(usize, egui::ColorImage)>>,
    selected: BTreeSet<usize>,
    anchor: usize,
    loading: Option<Loading>,
    /// Stops the workers (the window closed, or Cancel while loading).
    stop: Arc<AtomicBool>,
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A PDF waiting for the window: name, contents, purpose, pages (0 when locked), locked.
type Waiting = (String, Arc<Vec<u8>>, Purpose, usize, bool);

/// PDFs waiting for the window (one shows at a time).
#[derive(Default)]
pub(crate) struct PdfPages {
    pub(crate) picker: Option<Picker>,
    queue: VecDeque<Waiting>,
}

/// Whether `bytes` goes through the window: a PDF with more than one page (its count), or a
/// password-protected one (0 pages known yet, locked).
fn needs_window(bytes: &[u8]) -> Option<(usize, bool)> {
    use photocraft_io::pdf_import::{PdfError, check, is_pdf};
    if !is_pdf(bytes) {
        return None;
    }
    match check(bytes, "") {
        Ok(n) if n > 1 => Some((n, false)),
        Err(PdfError::Password) => Some((0, true)),
        // One page opens as usual; a damaged file reports its error there too.
        Ok(_) | Err(PdfError::Other(_)) => None,
    }
}

/// Shows the window for `bytes` when it is a PDF with several pages, instead of opening it as a
/// document (`purpose` Open) or placing it (Place). False when it isn't one: open it as usual.
/// Agents driving the app open the first page, as the engine's File › Open does.
pub(crate) fn intercept(app: &mut PhotocraftApp, name: &str, bytes: &[u8], purpose: Purpose) -> bool {
    if app.automation_input {
        return false;
    }
    let Some((pages, locked)) = needs_window(bytes) else { return false };
    app.pdf_pages.queue.push_back((name.to_string(), Arc::new(bytes.to_vec()), purpose, pages, locked));
    true
}

/// The file's name without `.pdf`.
fn stem(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pdf") { name.get(..name.len() - 4).unwrap_or(name) } else { name }
}

/// The name page `index` opens or is placed under: "Report - Page 3".
pub(crate) fn page_name(name: &str, index: usize) -> String {
    crate::i18n::fmt(tl!("{name} - Page {page}"), &[("name", stem(name)), ("page", &(index + 1).to_string())])
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new().name(name.into()).spawn(work).is_ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        work();
        true
    }
}

fn start(ctx: &egui::Context, name: String, bytes: Arc<Vec<u8>>, purpose: Purpose, pages: usize, locked: bool) -> Picker {
    let mut p = Picker {
        name,
        bytes,
        purpose,
        pages,
        locked,
        password: String::new(),
        typed: String::new(),
        wrong: false,
        thumbs: vec![None; pages],
        thumb_rx: None,
        selected: BTreeSet::from([0]),
        anchor: 0,
        loading: None,
        stop: Arc::new(AtomicBool::new(false)),
    };
    if !locked {
        thumbnails(ctx, &mut p);
    }
    p
}

/// Draw every page small on a worker, for the grid.
fn thumbnails(ctx: &egui::Context, p: &mut Picker) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (worker_bytes, worker_stop, repaint, password) = (p.bytes.clone(), p.stop.clone(), ctx.clone(), p.password.clone());
    let started = spawn("pdf-thumbnails", move || {
        let _ = photocraft_io::pdf_import::thumbnails(&worker_bytes, &password, THUMB_PX, |i, t| {
            let image = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.rgba);
            let sent = tx.send((i, image)).is_ok();
            repaint.request_repaint();
            sent && !worker_stop.load(Ordering::Relaxed)
        });
    });
    p.thumb_rx = started.then_some(rx);
}

/// The typed password: when it opens the file, the pages show (or a single page opens at once);
/// otherwise the box says so. False when the file can't be opened at all (reported).
fn unlock(app: &mut PhotocraftApp, ctx: &egui::Context, p: &mut Picker) -> bool {
    use photocraft_io::pdf_import::{PdfError, check};
    match check(&p.bytes, &p.typed) {
        Ok(pages) => {
            p.password = std::mem::take(&mut p.typed);
            p.locked = false;
            p.wrong = false;
            p.pages = pages;
            p.thumbs = vec![None; pages];
            p.selected = BTreeSet::from([0]);
            if pages == 1 {
                load(ctx, p);
            } else {
                thumbnails(ctx, p);
            }
            true
        }
        Err(PdfError::Password) => {
            p.wrong = true;
            true
        }
        Err(PdfError::Other(e)) => {
            app.open_failed(&p.name, &e.to_string());
            false
        }
    }
}

/// Draw `picked` pages at full size on a worker, in page order.
fn load(ctx: &egui::Context, p: &mut Picker) {
    let picked: Vec<usize> = p.selected.iter().copied().collect();
    if picked.is_empty() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let (bytes, stop, repaint, name, password) = (p.bytes.clone(), p.stop.clone(), ctx.clone(), p.name.clone(), p.password.clone());
    let total = picked.len();
    let started = spawn("pdf-pages", move || {
        for i in picked {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let r = photocraft_io::pdf_import::import_page(&page_name(&name, i), &bytes, &password, i, &photocraft_raster::Interrupt::NONE)
                .map(|r| (r.document, r.warnings))
                .map_err(|e| e.to_string());
            if tx.send((i, r)).is_err() {
                return;
            }
            repaint.request_repaint();
        }
    });
    if started {
        p.loading = Some(Loading { rx, total, done: 0 });
    }
}

/// A page drawn at full size: opened as a document, or queued to be placed.
fn arrived(app: &mut PhotocraftApp, name: &str, purpose: Purpose, index: usize, r: Result<(photocraft_doc::Document, Vec<String>), String>) {
    let title = page_name(name, index);
    let (doc, warnings) = match r {
        Ok(v) => v,
        Err(e) => return app.open_failed(&title, &e),
    };
    match purpose {
        Purpose::Open => {
            let question = app.session.color.open_question(&doc);
            crate::dialogs::open_prompt::arrive(
                app,
                crate::dialogs::open_prompt::Held {
                    doc,
                    read_only: false,
                    damage_pending: false,
                    name: title,
                    path: None,
                    warnings,
                    bytes: None,
                    slot: None,
                    question,
                    policy_applied: false,
                },
            );
        }
        Purpose::Place => match photocraft_io::export(&doc, "page.png", &Default::default()) {
            Ok(png) => app.drop_places.push_back(crate::file_open::DropPlace::Bytes(format!("{title}.png"), png.bytes)),
            Err(e) => app.open_failed(&title, &e.to_string()),
        },
    }
}

/// Per frame: the next PDF's window, its thumbnails and loaded pages, and the window itself.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.pdf_pages.picker.is_none()
        && let Some((name, bytes, purpose, pages, locked)) = app.pdf_pages.queue.pop_front()
    {
        app.pdf_pages.picker = Some(start(ctx, name, bytes, purpose, pages, locked));
    }
    let Some(mut p) = app.pdf_pages.picker.take() else { return };
    // Thumbnails as they're drawn.
    if let Some(rx) = p.thumb_rx.as_ref() {
        loop {
            match rx.try_recv() {
                Ok((i, image)) => {
                    if let Some(slot) = p.thumbs.get_mut(i) {
                        *slot = Some(ctx.load_texture(format!("pdf-page-{i}"), image, egui::TextureOptions::LINEAR));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    p.thumb_rx = None;
                    break;
                }
            }
        }
    }
    // Pages drawn at full size.
    if let Some(mut loading) = p.loading.take() {
        let mut ended = false;
        loop {
            match loading.rx.try_recv() {
                Ok((i, r)) => {
                    loading.done += 1;
                    arrived(app, &p.name, p.purpose, i, r);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        if ended || loading.done >= loading.total {
            // All in: the window closes.
            return;
        }
        p.loading = Some(loading);
    }
    if window(app, ctx, &mut p) {
        app.pdf_pages.picker = Some(p);
    }
}

/// What the window was asked to do this frame.
enum Answer {
    Stay,
    Load,
    Unlock,
    Close,
}

/// Draws the window. False when it closed (cancelled).
fn window(app: &mut PhotocraftApp, ctx: &egui::Context, p: &mut Picker) -> bool {
    let t = crate::theme::Tokens::get(ctx);
    let mut answer = Answer::Stay;
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Answer::Close;
    }
    let modifiers = ctx.input(|i| i.modifiers);
    egui::Modal::new(egui::Id::unique("pdf-pages")).show(ctx, |ui| {
        let width = (ctx.content_rect().width() - 80.0).clamp(320.0, 820.0);
        ui.set_width(width);
        ui.heading(tl!("Import PDF"));
        ui.label(egui::RichText::new(&p.name).color(t.text_dim));
        ui.separator();
        if p.locked {
            password_box(ui, p, &t, &mut answer);
            return;
        }
        if let Some(loading) = p.loading.as_ref() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                let template = "Opening page {n} of {total}…";
                ui.label(crate::i18n::fmt(tl!(template), &[("n", &(loading.done + 1).min(loading.total).to_string()), ("total", &loading.total.to_string())]));
            });
            ui.add_space(6.0);
            if ui.button(tl!("Cancel")).clicked() {
                answer = Answer::Close;
            }
            return;
        }
        let hint = if p.purpose == Purpose::Place {
            "Pick the pages to place. Ctrl-click or Shift-click picks several; double-click places one."
        } else {
            "Pick the pages to open. Ctrl-click or Shift-click picks several; double-click opens one."
        };
        ui.label(tl!(hint));
        ui.add_space(4.0);
        let height = (ctx.content_rect().height() - 260.0).clamp(180.0, 560.0);
        egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for i in 0..p.pages {
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL, CELL + 22.0), egui::Sense::click());
                    let selected = p.selected.contains(&i);
                    let painter = ui.painter_at(rect);
                    let fill = if selected { t.accent_soft } else if response.hovered() { t.hover } else { t.field };
                    painter.rect_filled(rect, t.radius, fill);
                    let stroke = if selected { egui::Stroke::new(2.5, t.accent) } else { egui::Stroke::new(1.0, t.field_border) };
                    painter.rect_stroke(rect, t.radius, stroke, egui::StrokeKind::Inside);
                    let area = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(CELL - 16.0, CELL - 16.0));
                    match p.thumbs.get(i).and_then(Option::as_ref) {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let k = (area.width() / size.x).min(area.height() / size.y);
                            let r = egui::Rect::from_center_size(area.center(), size * k);
                            painter.rect_filled(r.expand(1.0), 0.0, t.shadow);
                            painter.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                        None => {
                            painter.text(area.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(22.0), t.text_faint);
                        }
                    }
                    let label = crate::i18n::fmt(tl!("Page {n}"), &[("n", &(i + 1).to_string())]);
                    painter.text(
                        egui::pos2(rect.center().x, rect.bottom() - 12.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(13.0),
                        if selected { t.text } else { t.text_dim },
                    );
                    if response.double_clicked() {
                        p.selected = BTreeSet::from([i]);
                        p.anchor = i;
                        answer = Answer::Load;
                    } else if response.clicked() {
                        click(p, i, modifiers);
                    }
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Select All")).clicked() {
                p.selected = (0..p.pages).collect();
            }
            if ui.button(tl!("Select None")).clicked() {
                p.selected.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = p.selected.len();
                let label = match p.purpose {
                    Purpose::Open => crate::i18n::trn(crate::i18n::current(), n as u64, "Open {n} Page", "Open {n} Pages"),
                    Purpose::Place => crate::i18n::trn(crate::i18n::current(), n as u64, "Place {n} Page", "Place {n} Pages"),
                };
                if ui.add_enabled(n > 0, egui::Button::new(label)).clicked() || (n > 0 && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    answer = Answer::Load;
                }
                if ui.button(tl!("Cancel")).clicked() {
                    answer = Answer::Close;
                }
            });
        });
    });
    match answer {
        Answer::Stay => true,
        Answer::Close => false,
        Answer::Unlock => unlock(app, ctx, p),
        Answer::Load => {
            load(ctx, p);
            if p.loading.is_none() {
                app.open_failed(&p.name, "no thread to read the pages on");
                return false;
            }
            true
        }
    }
}

/// The password box of a locked file.
fn password_box(ui: &mut egui::Ui, p: &mut Picker, t: &crate::theme::Tokens, answer: &mut Answer) {
    ui.label(tl!("This PDF is password-protected. Type its password to open it."));
    ui.add_space(6.0);
    let field = ui.add(egui::TextEdit::singleline(&mut p.typed).password(true).hint_text(tl!("Password")).desired_width(f32::INFINITY));
    if !field.has_focus() && !ui.ctx().memory(|m| m.focused().is_some()) {
        field.request_focus();
    }
    if field.changed() {
        p.wrong = false;
    }
    if p.wrong {
        ui.label(egui::RichText::new(tl!("That password doesn't open this PDF. Try again.")).color(t.danger));
    }
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!p.typed.is_empty(), egui::Button::new(tl!("Unlock"))).clicked() || (enter && !p.typed.is_empty()) {
                *answer = Answer::Unlock;
            }
            if ui.button(tl!("Cancel")).clicked() {
                *answer = Answer::Close;
            }
        });
    });
}

/// A click on page `i`: alone, with Ctrl/Cmd added or taken away, with Shift a range.
fn click(p: &mut Picker, i: usize, m: egui::Modifiers) {
    if m.shift {
        let (a, b) = (p.anchor.min(i), p.anchor.max(i));
        if !m.command {
            p.selected.clear();
        }
        p.selected.extend(a..=b);
        return;
    }
    if m.command {
        if !p.selected.remove(&i) {
            p.selected.insert(i);
        }
    } else {
        p.selected = BTreeSet::from([i]);
    }
    p.anchor = i;
}

#[cfg(test)]
mod tests;
