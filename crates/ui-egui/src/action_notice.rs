//! The pop-up at the top of the canvas that shows an action ran: pressing an action's shortcut
//! (or playing it from the panel) gives visible feedback even when the action changes nothing on
//! screen, such as one that only prints. A print it starts shows "Printing…" until the job ends.

use serde_json::Value;

use crate::PhotocraftApp;

/// How long a finished notice stays up, and how long it takes to fade at the end.
const SHOW_MS: f64 = 2500.0;
const FADE_MS: f64 = 400.0;
/// A "Printing…" notice whose job never reports ends anyway.
const BUSY_MAX_MS: f64 = 120_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Waiting on a print the action started.
    Busy,
    Done,
    Error,
}

#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub(crate) text: String,
    pub(crate) kind: Kind,
    /// The action's name, for the follow-up text.
    pub(crate) action: String,
    since: f64,
}

pub(crate) fn show(app: &mut PhotocraftApp, action: &str, text: String, kind: Kind) {
    app.action_notice = Some(Notice { text, kind, action: action.to_string(), since: crate::gpu_canvas::now_ms() });
}

/// After an action played (`v` is `actions.play`'s result).
pub(crate) fn played(app: &mut PhotocraftApp, v: &Value) {
    let name = v.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    if v.get("paused").is_some_and(Value::is_object) {
        show(app, &name, crate::i18n::fmt(tl!("Action \"{name}\" is waiting for its dialog"), &[("name", &name)]), Kind::Done);
    } else if let Some(f) = v.get("failed").filter(|f| f.is_object()) {
        let error = f.get("error").and_then(Value::as_str).unwrap_or_default();
        show(app, &name, crate::i18n::fmt(tl!("Action \"{name}\" failed: {error}"), &[("name", &name), ("error", error)]), Kind::Error);
    } else if app.printing() {
        show(app, &name, crate::i18n::fmt(tl!("Action \"{name}\": printing..."), &[("name", &name)]), Kind::Busy);
    } else {
        show(app, &name, crate::i18n::fmt(tl!("Played action \"{name}\""), &[("name", &name)]), Kind::Done);
    }
}

/// A print job ended: a "Printing…" notice says how.
pub(crate) fn print_ended(app: &mut PhotocraftApp, error: Option<&str>) {
    let Some(n) = app.action_notice.as_ref().filter(|n| n.kind == Kind::Busy) else { return };
    let name = n.action.clone();
    match error {
        None => show(app, &name, crate::i18n::fmt(tl!("Action \"{name}\": sent to the printer"), &[("name", &name)]), Kind::Done),
        Some(e) => show(app, &name, crate::i18n::fmt(tl!("Action \"{name}\": printing failed: {error}"), &[("name", &name), ("error", e)]), Kind::Error),
    }
}

/// Draws the notice over the window while it lasts.
pub(crate) fn draw(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(n) = app.action_notice.as_ref() else { return };
    let age = crate::gpu_canvas::now_ms() - n.since;
    let life = if n.kind == Kind::Busy { BUSY_MAX_MS } else { SHOW_MS };
    if !(0.0..life).contains(&age) {
        app.action_notice = None;
        return;
    }
    let alpha = if n.kind == Kind::Busy { 1.0 } else { ((life - age) / FADE_MS).clamp(0.0, 1.0) as f32 };
    let t = crate::theme::Tokens::get(ctx);
    let edge = match n.kind {
        Kind::Busy => t.accent,
        Kind::Done => t.accent_border,
        Kind::Error => t.danger,
    };
    let text = n.text.clone();
    let busy = n.kind == Kind::Busy;
    egui::Area::new(egui::Id::unique("action-notice"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(alpha);
            egui::Frame::new()
                .fill(t.card)
                .stroke(egui::Stroke::new(1.5, edge))
                .corner_radius(t.radius_lg)
                .inner_margin(egui::Margin::symmetric(16, 10))
                .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: t.shadow })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if busy {
                            ui.add(egui::Spinner::new().size(14.0).color(edge));
                        }
                        ui.label(egui::RichText::new(text).color(t.text).size(14.0));
                    });
                });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(if busy { 100 } else { 30 }));
}

#[cfg(test)]
mod tests;
