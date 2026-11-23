//! Set Key…: any key with any modifiers as an action's shortcut, not only the F1–F12 of the
//! panel's context menu. The key is saved in the preferences as `actions.play:<name>`, which
//! the shortcut dispatch already runs; a key an action holds is the action's alone.

use egui::Key;
use photocraft_engine::prefs::{normalize_shortcut, reserved_shortcut};

use super::{SHORTCUT_PREFIX, assign_shortcut, shortcut_id};
use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The open Set Key… window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeyCapture {
    /// The action the key is for.
    name: String,
    /// Modifiers held so far, carried across passes (they and the key often land in different ones).
    mods: egui::Modifiers,
    /// The pressed combination in shortcut notation; `Some("")` removes the shortcut.
    key: Option<String>,
    error: Option<String>,
    /// The pass the window was last drawn in: a capture whose panel stopped drawing it (the panel
    /// was closed) must not keep the application's shortcuts blocked.
    drawn: u64,
}

/// Open Set Key… for the action `name`.
pub(crate) fn begin(app: &mut PhotocraftApp, ctx: &egui::Context, name: &str) {
    app.ui.actions.key_capture = Some(KeyCapture { name: name.to_string(), mods: egui::Modifiers::NONE, key: None, error: None, drawn: ctx.cumulative_pass_nr() });
}

/// Is Set Key… listening? The application's shortcuts wait until it is done, so the keys pressed
/// are the new shortcut instead of running commands.
pub(crate) fn capturing(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let live = app.ui.actions.key_capture.as_ref().is_some_and(|c| ctx.cumulative_pass_nr().saturating_sub(c.drawn) <= 2);
    if !live {
        app.ui.actions.key_capture = None;
    }
    live
}

/// A modifier key on its own: egui reports them as key events too, before the key pressed with them.
fn is_modifier_key(key: Key) -> bool {
    matches!(key, Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight)
}

/// The modifier a modifier key's press stands for, when egui leaves it out of the event.
fn modifier_of(key: Key) -> egui::Modifiers {
    match key {
        Key::ShiftLeft | Key::ShiftRight => egui::Modifiers::SHIFT,
        Key::ControlLeft | Key::ControlRight => egui::Modifiers::CTRL,
        Key::AltLeft | Key::AltRight => egui::Modifiers::ALT,
        Key::SuperLeft | Key::SuperRight => egui::Modifiers::COMMAND,
        _ => egui::Modifiers::NONE,
    }
}

/// The modifiers held now, and the first non-modifier key pressed this pass with the modifiers
/// held with it.
pub(crate) fn key_press(held: egui::Modifiers, i: &egui::InputState) -> (egui::Modifiers, Option<(Key, egui::Modifiers)>) {
    let mut mods = held | i.modifiers;
    for e in &i.events {
        match e {
            egui::Event::ModifiersChanged(m) => mods = *m,
            egui::Event::Key { key, pressed: true, modifiers, .. } if is_modifier_key(*key) => mods = *modifiers | modifier_of(*key),
            egui::Event::Key { key, pressed: true, modifiers, .. } => return (mods, Some((*key, mods | *modifiers))),
            egui::Event::Key { key, pressed: false, modifiers, .. } if is_modifier_key(*key) => mods = *modifiers,
            _ => {}
        }
    }
    (mods, None)
}

/// A key press in shortcut notation (`Cmd+Shift+K`, where `Cmd` is Ctrl off the Mac); `None` for a
/// bare modifier.
pub(crate) fn shortcut_text(key: Key, m: egui::Modifiers) -> Option<String> {
    if is_modifier_key(key) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if m.command || m.mac_cmd {
        parts.push("Cmd");
    }
    if m.ctrl && !m.command {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    let name = match key {
        Key::Equals => "=",
        Key::Minus => "-",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Semicolon => ";",
        Key::Quote => "'",
        k => k.name(),
    };
    parts.push(name);
    normalize_shortcut(&parts.join("+"))
}

/// What else `shortcut` runs now (another action or a menu command), to say so before the action
/// takes the key over.
fn owner(app: &PhotocraftApp, name: &str, shortcut: &str) -> Option<String> {
    let key = normalize_shortcut(shortcut)?;
    let same = |s: &str| normalize_shortcut(s).as_deref() == Some(key.as_str());
    let own = shortcut_id(name);
    let prefs = app.session.prefs();
    if let Some(other) = prefs.shortcuts.iter().find(|(id, k)| id.starts_with(SHORTCUT_PREFIX) && **id != own && same(k)).and_then(|(id, _)| id.strip_prefix(SHORTCUT_PREFIX)) {
        return Some(crate::i18n::fmt(tl!("the action \"{name}\""), &[("name", other)]));
    }
    crate::menu_catalog::CATALOG.iter().find_map(|(_, label, _, id)| {
        let main = crate::shortcuts::effective_shortcut(app, id, crate::shortcuts::default_shortcut(id).as_deref());
        main.is_some_and(|m| same(&m)).then(|| label.trim_end_matches('…').to_string())
    })
}

/// The selected action's shortcut and its Set Key… button, under the panel.
pub(crate) fn row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(name) = super::selected_action(app).map(|a| a.name.clone()) else { return };
    let current = app.session.prefs().shortcuts.get(&shortcut_id(&name)).cloned().unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    let mut open = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Shortcut:")).color(t.text_dim));
        let shown = if current.is_empty() { tl!("None").to_string() } else { crate::shortcuts::pretty(&current) };
        ui.label(egui::RichText::new(shown).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            open = ui.button(tl!("Set Key…")).on_hover_text(tl!("Assign any key or key combination to play this action")).clicked();
        });
    });
    if open {
        begin(app, ui.ctx(), &name);
    }
    window(app, ui.ctx());
}

/// The Set Key… window: press any combination, then Assign. Backspace removes the shortcut and Esc
/// cancels, as in Edit › Keyboard Shortcuts.
pub(crate) fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut cap) = app.ui.actions.key_capture.take() else { return };
    cap.drawn = ctx.cumulative_pass_nr();
    let (mods, pressed) = ctx.input(|i| key_press(cap.mods, i));
    cap.mods = mods;
    // None: keep listening; Some(true): assign; Some(false): cancel.
    let mut done: Option<bool> = None;
    if let Some((key, m)) = pressed {
        let bare = !(m.shift || m.alt || m.ctrl || m.command || m.mac_cmd);
        match key {
            Key::Escape if bare => done = Some(false),
            Key::Enter if bare && cap.key.is_some() => done = Some(true),
            Key::Backspace | Key::Delete if bare => cap.key = Some(String::new()),
            _ => {
                if let Some(text) = shortcut_text(key, m) {
                    cap.key = Some(text);
                }
            }
        }
        cap.error = None;
        // The keys were for the capture, not for a focused button or the canvas.
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))));
    }
    let current = app.session.prefs().shortcuts.get(&shortcut_id(&cap.name)).cloned().unwrap_or_default();
    let reserved = cap.key.as_deref().is_some_and(|k| !k.is_empty() && reserved_shortcut(k));
    let taken_from = cap.key.as_deref().filter(|k| !k.is_empty() && *k != current).and_then(|k| owner(app, &cap.name, k));
    let t = Tokens::get(ctx);
    egui::Modal::new(egui::Id::unique("action-key-capture")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(tl!("Action Shortcut"));
        ui.label(egui::RichText::new(&cap.name).color(t.text_dim));
        ui.separator();
        ui.label(tl!("Press any key combination, for example Ctrl+Alt+1, Shift+Q or a single key."));
        ui.add_space(6.0);
        let shown = match cap.key.as_deref() {
            None if current.is_empty() => tl!("Waiting for keys…").to_string(),
            None => crate::shortcuts::pretty(&current),
            Some("") => tl!("None").to_string(),
            Some(k) => crate::shortcuts::pretty(k),
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| ui.label(egui::RichText::new(shown).size(18.0).strong()));
        });
        ui.add_space(4.0);
        if reserved {
            ui.colored_label(t.danger, tl!("The system uses this shortcut, so it can't be assigned."));
        } else if let Some(owner) = &taken_from {
            ui.colored_label(t.warning, crate::i18n::fmt(tl!("Now used by {owner}. The action will take it over."), &[("owner", owner)]));
        }
        if let Some(e) = &cap.error {
            ui.colored_label(t.danger, e);
        }
        ui.label(egui::RichText::new(tl!("Backspace removes the shortcut. Esc cancels.")).color(t.text_dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(cap.key.is_some() && !reserved, egui::Button::new(tl!("Assign"))).clicked() {
                done = Some(true);
            }
            if ui.add_enabled(!current.is_empty(), egui::Button::new(tl!("Remove Shortcut"))).clicked() {
                cap.key = Some(String::new());
                done = Some(true);
            }
            if ui.button(tl!("Cancel")).clicked() {
                done = Some(false);
            }
        });
    });
    match done {
        Some(true) if !reserved => {
            let key = cap.key.clone().unwrap_or(current);
            match assign_shortcut(app, &cap.name, &key) {
                Ok(_) => {
                    app.ui.status = if key.is_empty() {
                        tl!("Action shortcut removed").to_string()
                    } else {
                        crate::i18n::fmt(tl!("Action shortcut set to {key}"), &[("key", &crate::shortcuts::pretty(&key))])
                    };
                    app.ui.status_error = false;
                }
                Err(e) => {
                    cap.error = Some(e);
                    app.ui.actions.key_capture = Some(cap);
                }
            }
        }
        Some(_) => {}
        None => app.ui.actions.key_capture = Some(cap),
    }
}
