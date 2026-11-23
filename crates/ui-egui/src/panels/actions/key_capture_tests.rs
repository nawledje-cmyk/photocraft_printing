

use super::key_capture;
use crate::PhotocraftApp;

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// One pass of the Set Key… window with `events`.
fn pass(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, ..Default::default() };
    ctx.run_ui(raw, |ui| key_capture::window(app, ui.ctx())).drop_without_applying_deltas();
}

/// Set Key… takes any key with its modifiers, held over passes, and Enter assigns it; Backspace
/// then removes it. While it listens, the application's shortcuts wait.
#[test]
fn set_key_assigns_any_combination_and_removes_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action::new("Stroke it", Vec::new())).unwrap();
    key_capture::begin(&mut app, &ctx, "Stroke it");
    assert!(key_capture::capturing(&mut app, &ctx));
    // Shift is pressed in one pass, Q in the next.
    pass(&mut app, &ctx, vec![key(egui::Key::ShiftLeft, egui::Modifiers::NONE)]);
    pass(&mut app, &ctx, vec![key(egui::Key::Q, egui::Modifiers::NONE)]);
    let release = egui::Event::Key { key: egui::Key::ShiftLeft, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE };
    pass(&mut app, &ctx, vec![release, key(egui::Key::Enter, egui::Modifiers::NONE)]);
    let saved = app.session.prefs().shortcuts.get(&super::shortcut_id("Stroke it")).cloned().unwrap_or_default();
    assert!(saved.contains("Shift") && saved.ends_with('Q'), "{saved}");
    assert!(!key_capture::capturing(&mut app, &ctx), "the window closed");
    key_capture::begin(&mut app, &ctx, "Stroke it");
    pass(&mut app, &ctx, vec![key(egui::Key::Backspace, egui::Modifiers::NONE)]);
    pass(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert!(app.session.prefs().shortcuts.get(&super::shortcut_id("Stroke it")).is_none_or(String::is_empty));
}

/// Esc cancels and leaves the key as it was.
#[test]
fn set_key_escape_keeps_the_shortcut() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action::new("A", Vec::new())).unwrap();
    key_capture::begin(&mut app, &ctx, "A");
    pass(&mut app, &ctx, vec![key(egui::Key::K, egui::Modifiers::CTRL)]);
    pass(&mut app, &ctx, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(app.session.prefs().shortcuts.get(&super::shortcut_id("A")).is_none_or(String::is_empty));
}
