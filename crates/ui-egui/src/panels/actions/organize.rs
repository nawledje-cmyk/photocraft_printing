//! Organizing the Actions panel: selecting several actions and folders (⌘/Ctrl-click to add or
//! remove one, ⇧-click for a range), moving them into folders (dragged onto a folder row, or onto
//! the list's empty end for the top level), Copy / Paste / Duplicate / Rename / Delete from the
//! right-click menu, and new folders named in place where they appear.

use std::collections::HashMap;

use photocraft_engine::actions_cmds::{Action, folders};
use serde_json::json;

use super::RenameAction;
use crate::PhotocraftApp;

/// One selectable row: an action (by name) or a folder (by path).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pick {
    Action(String),
    Folder(String),
}

/// What Copy keeps: folders and actions, their folders relative to the copied ones' parent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Copied {
    Folder(String),
    Action(Box<Action>, Option<String>),
}

/// A folder whose name is being typed where it shows (a new one, or Rename).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FolderRename {
    pub path: String,
    pub text: String,
    pub focused: bool,
}

/// Delete's confirmation, for folders or several items.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeletePrompt {
    picks: Vec<Pick>,
    actions: usize,
    folders: usize,
}

/// The drag payload of folder rows and of several selected items.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DragPicks(pub Vec<Pick>);

fn report(app: &mut PhotocraftApp, r: Result<(), String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn index_of(app: &PhotocraftApp, name: &str) -> Option<usize> {
    app.session.actions.list.iter().position(|a| a.name == name)
}

fn folder_of(app: &PhotocraftApp, name: &str) -> Option<String> {
    app.session.actions.list.iter().find(|a| a.name == name).and_then(|a| a.set.clone())
}

fn join(parent: Option<&str>, name: &str) -> String {
    parent.map_or_else(|| name.to_string(), |p| format!("{p}{}{name}", folders::SEP))
}

/// The selected items: the ⌘/⇧-clicked ones, else the selected action (not a step) or folder.
pub(crate) fn selection(app: &PhotocraftApp) -> Vec<Pick> {
    let ui = &app.ui.actions;
    let known = folders::all(&app.session.actions);
    if !ui.picked.is_empty() {
        return ui
            .picked
            .iter()
            .filter(|p| match p {
                Pick::Action(n) => index_of(app, n).is_some(),
                Pick::Folder(f) => known.contains(f),
            })
            .cloned()
            .collect();
    }
    if let Some(i) = ui.selected.filter(|_| ui.selected_step.is_none()) {
        return app.session.actions.list.get(i).map(|a| vec![Pick::Action(a.name.clone())]).unwrap_or_default();
    }
    ui.selected_folder.clone().filter(|f| known.contains(f)).map(Pick::Folder).into_iter().collect()
}

fn set_primary(app: &mut PhotocraftApp, pick: &Pick) {
    let index = match pick {
        Pick::Action(n) => index_of(app, n),
        Pick::Folder(_) => None,
    };
    let ui = &mut app.ui.actions;
    ui.selected_step = None;
    match pick {
        Pick::Action(_) => {
            ui.selected = index;
            ui.selected_folder = None;
        }
        Pick::Folder(f) => {
            ui.selected = None;
            ui.selected_folder = Some(f.clone());
        }
    }
}

/// Select `pick` alone.
pub(crate) fn select_only(app: &mut PhotocraftApp, pick: &Pick) {
    app.ui.actions.picked.clear();
    app.ui.actions.anchor = Some(pick.clone());
    set_primary(app, pick);
}

/// Nothing selected.
pub(crate) fn clear(app: &mut PhotocraftApp) {
    let ui = &mut app.ui.actions;
    ui.picked.clear();
    ui.anchor = None;
    ui.selected = None;
    ui.selected_step = None;
    ui.selected_folder = None;
}

/// A click on `pick`, with `order` the rows as shown: ⇧ selects the range from the last item
/// clicked, ⌘/Ctrl adds or removes it, and a plain click selects it alone. Returns whether the
/// click was plain (it then also opens a folder or plays a button).
pub(crate) fn click(app: &mut PhotocraftApp, pick: Pick, mods: egui::Modifiers, order: &[Pick]) -> bool {
    if mods.shift {
        let anchor = app.ui.actions.anchor.clone().or_else(|| selection(app).first().cloned());
        let at = |p: &Pick| order.iter().position(|o| o == p);
        if let (Some(a), Some(b)) = (anchor.as_ref().and_then(at), at(&pick)) {
            let range = order.get(a.min(b)..=a.max(b)).unwrap_or_default().to_vec();
            app.ui.actions.picked = range;
            set_primary(app, &pick);
            return false;
        }
    }
    if mods.command || mods.ctrl {
        let mut picked = if app.ui.actions.picked.is_empty() { selection(app) } else { app.ui.actions.picked.clone() };
        if let Some(i) = picked.iter().position(|p| *p == pick) {
            picked.remove(i);
        } else {
            picked.push(pick.clone());
        }
        app.ui.actions.anchor = Some(pick.clone());
        match picked.as_slice() {
            [] => clear(app),
            [only] => {
                let only = only.clone();
                select_only(app, &only);
            }
            _ => {
                app.ui.actions.picked = picked;
                set_primary(app, &pick);
            }
        }
        return false;
    }
    select_only(app, &pick);
    true
}

/// The items of `picks` that aren't inside a picked folder: what moves, copies or deletes.
fn roots(app: &PhotocraftApp, picks: &[Pick]) -> Vec<Pick> {
    let in_picked = |path: &str| picks.iter().any(|p| matches!(p, Pick::Folder(f) if folders::within(path, f) && f != path));
    picks
        .iter()
        .filter(|p| match p {
            Pick::Folder(f) => !in_picked(f),
            Pick::Action(n) => !folder_of(app, n).is_some_and(|f| picks.iter().any(|p| matches!(p, Pick::Folder(pf) if folders::within(&f, pf)))),
        })
        .cloned()
        .collect()
}

/// Copy `picks`: folders with everything inside them, and actions.
pub(crate) fn copy(app: &PhotocraftApp, picks: &[Pick]) -> Vec<Copied> {
    let mut out = Vec::new();
    let all = folders::all(&app.session.actions);
    for r in roots(app, picks) {
        match r {
            Pick::Folder(p) => {
                // Paths relative to the folder's parent, so the copy can go anywhere.
                let base = folders::parent(&p).map_or(0, |parent| parent.len() + 1);
                let rel = |f: &str| f.get(base..).unwrap_or(f).to_string();
                out.extend(all.iter().filter(|f| folders::within(f, &p)).map(|f| Copied::Folder(rel(f))));
                out.extend(
                    app.session.actions.list.iter().filter(|a| a.set.as_deref().is_some_and(|s| folders::within(s, &p))).map(|a| Copied::Action(Box::new(a.clone()), a.set.as_deref().map(rel))),
                );
            }
            Pick::Action(n) => {
                if let Some(a) = app.session.actions.list.iter().find(|a| a.name == n) {
                    out.push(Copied::Action(Box::new(a.clone()), None));
                }
            }
        }
    }
    out
}

/// Put copied items in `target` (`None`: the top level). Copied folders take a free name there
/// (`Retouch 2`) and actions a free one too (`Sharpen copy`).
pub(crate) fn paste(app: &mut PhotocraftApp, items: &[Copied], target: Option<String>) -> Result<(), String> {
    let mut renamed: HashMap<String, String> = HashMap::new();
    let mapped = |rel: &str, renamed: &HashMap<String, String>| -> String {
        let (root, rest) = rel.split_once(folders::SEP).map_or((rel, None), |(r, rest)| (r, Some(rest)));
        let root = renamed.get(root).map_or(root, String::as_str);
        rest.map_or_else(|| root.to_string(), |rest| format!("{root}{}{rest}", folders::SEP))
    };
    for item in items {
        if let Copied::Folder(rel) = item {
            if rel.contains(folders::SEP) {
                let path = mapped(rel, &renamed);
                let parent = folders::parent(&path).map(|p| join(target.as_deref(), p));
                app.run("actions.newFolder", json!({"name": folders::leaf(&path), "parent": parent}))?;
            } else {
                let name = folders::unique_child(&app.session.actions, target.as_deref(), rel);
                app.run("actions.newFolder", json!({"name": name, "parent": target}))?;
                renamed.insert(rel.clone(), name);
            }
        }
    }
    for item in items {
        if let Copied::Action(a, rel) = item {
            let folder = match rel {
                Some(r) => Some(join(target.as_deref(), &mapped(r, &renamed))),
                None => target.clone(),
            };
            let action = serde_json::to_value(a.as_ref()).map_err(|e| e.to_string())?;
            app.run("actions.add", json!({"action": action, "folder": folder}))?;
        }
    }
    if let Some(t) = &target {
        open(app, t);
    }
    Ok(())
}

/// Where Paste and New Folder put things for a right-click on `pick`: inside a folder, next to an
/// action (in its folder).
fn target_of(app: &PhotocraftApp, pick: &Pick) -> Option<String> {
    match pick {
        Pick::Folder(f) => Some(f.clone()),
        Pick::Action(n) => folder_of(app, n),
    }
}

/// Duplicate `picks`: each next to itself, in its own folder.
pub(crate) fn duplicate(app: &mut PhotocraftApp, picks: &[Pick]) -> Result<(), String> {
    for r in roots(app, picks) {
        let items = copy(app, std::slice::from_ref(&r));
        let target = match &r {
            Pick::Folder(f) => folders::parent(f).map(str::to_string),
            Pick::Action(n) => folder_of(app, n),
        };
        paste(app, &items, target)?;
    }
    Ok(())
}

/// Open `folder` and the folders holding it, so what goes in shows.
fn open(app: &mut PhotocraftApp, folder: &str) {
    app.ui.actions.collapsed_sets.retain(|c| !folders::within(folder, c));
}

/// Move `picks` into `target` (`None`: the top level). A folder never goes inside itself.
pub(crate) fn move_into(app: &mut PhotocraftApp, picks: &[Pick], target: Option<String>) -> Result<(), String> {
    for r in roots(app, picks) {
        match r {
            Pick::Folder(f) => {
                if target.as_deref().is_some_and(|t| folders::within(t, &f)) || folders::parent(&f) == target.as_deref() {
                    continue;
                }
                let moved = app.run("actions.moveFolder", json!({"folder": f, "into": target}))?;
                let new = moved["folder"].as_str().unwrap_or_default().to_string();
                let ui = &mut app.ui.actions;
                ui.collapsed_sets = ui.collapsed_sets.iter().map(|c| if folders::within(c, &f) { format!("{new}{}", &c[f.len()..]) } else { c.clone() }).collect();
            }
            Pick::Action(n) => {
                if folder_of(app, &n) != target {
                    app.run("actions.moveToFolder", json!({"action": n, "folder": target}))?;
                }
            }
        }
    }
    if let Some(t) = &target {
        open(app, t);
    }
    clear(app);
    Ok(())
}

/// Put `picks` among the folders in `parent` (`None`: the top level): folders go just before
/// `before` (a folder in `parent`), or after the others there; actions go into `parent`.
pub(crate) fn place(app: &mut PhotocraftApp, picks: &[Pick], parent: Option<String>, before: Option<String>) -> Result<(), String> {
    for r in roots(app, picks) {
        match r {
            Pick::Folder(f) => {
                if parent.as_deref().is_some_and(|p| folders::within(p, &f)) || before.as_deref() == Some(f.as_str()) {
                    continue;
                }
                let moved = app.run("actions.moveFolder", json!({"folder": f, "into": parent, "before": before}))?;
                let new = moved["folder"].as_str().unwrap_or_default().to_string();
                let ui = &mut app.ui.actions;
                ui.collapsed_sets = ui.collapsed_sets.iter().map(|c| if folders::within(c, &f) { format!("{new}{}", &c[f.len()..]) } else { c.clone() }).collect();
            }
            Pick::Action(n) => {
                if folder_of(app, &n) != parent {
                    app.run("actions.moveToFolder", json!({"action": n, "folder": parent}))?;
                }
            }
        }
    }
    if let Some(p) = &parent {
        open(app, p);
    }
    clear(app);
    Ok(())
}

/// New Folder: "New Folder" (or "New Folder 2"…) inside the selected folder, next to the
/// selected action, or at the top level, selected and named where it shows.
pub(crate) fn new_folder(app: &mut PhotocraftApp) {
    let target = match selection(app).as_slice() {
        [one] => target_of(app, one),
        _ => None,
    };
    new_folder_in(app, target);
}

/// New Folder inside `parent` (`None`: the top level).
pub(crate) fn new_folder_in(app: &mut PhotocraftApp, parent: Option<String>) {
    let name = folders::unique_child(&app.session.actions, parent.as_deref(), "New Folder");
    match app.run("actions.newFolder", json!({"name": name, "parent": parent})) {
        Ok(r) => {
            let path = r["folder"].as_str().unwrap_or_default().to_string();
            if let Some(p) = &parent {
                open(app, p);
            }
            select_only(app, &Pick::Folder(path.clone()));
            app.ui.actions.folder_rename = Some(FolderRename { path, text: name, focused: false });
        }
        Err(e) => report(app, Err(e)),
    }
}

/// Start naming folder `path` where it shows.
pub(crate) fn start_rename(app: &mut PhotocraftApp, path: &str) {
    app.ui.actions.folder_rename = Some(FolderRename { path: path.to_string(), text: folders::leaf(path).to_string(), focused: false });
}

/// The name field over folder `path`'s name (`rect`) while it's being named: ↩, Tab or a click
/// elsewhere keeps the name, Esc the old one.
pub(crate) fn rename_field(app: &mut PhotocraftApp, ui: &mut egui::Ui, path: &str, rect: egui::Rect) {
    let Some(mut r) = app.ui.actions.folder_rename.take().filter(|r| r.path == path) else { return };
    let id = egui::Id::unique(("action-folder-rename", path));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).id(id).font(egui::FontId::proportional(12.0))).accessible_name(tl!("Folder name"));
    if !r.focused {
        te.request_focus();
        // The whole name starts selected, so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        r.focused = true;
        app.ui.actions.folder_rename = Some(r);
        return;
    }
    let (enter, tab, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        return;
    }
    if !(enter || tab || te.lost_focus() || !te.has_focus()) {
        app.ui.actions.folder_rename = Some(r);
        return;
    }
    let name = r.text.trim().to_string();
    if name.is_empty() || name == folders::leaf(path) {
        return;
    }
    match app.run("actions.renameFolder", json!({"folder": path, "name": name})) {
        Ok(v) => {
            let new = v["folder"].as_str().unwrap_or_default().to_string();
            let moved = |p: &str| if folders::within(p, path) { format!("{new}{}", &p[path.len()..]) } else { p.to_string() };
            let ui_state = &mut app.ui.actions;
            ui_state.collapsed_sets = ui_state.collapsed_sets.iter().map(|c| moved(c)).collect();
            ui_state.selected_folder = ui_state.selected_folder.as_deref().map(moved);
            ui_state.anchor = ui_state.anchor.take().map(|a| match a {
                Pick::Folder(f) => Pick::Folder(moved(&f)),
                a => a,
            });
            ui_state.picked.clear();
        }
        Err(e) => report(app, Err(e)),
    }
}

/// Delete `picks`: one action at once, as before; folders or several items after asking.
pub(crate) fn request_delete(app: &mut PhotocraftApp, picks: Vec<Pick>) {
    let picks = roots(app, &picks);
    if let [Pick::Action(n)] = picks.as_slice() {
        if let Some(i) = index_of(app, n) {
            app.ui.actions.selected = Some(i);
            app.ui.actions.selected_step = None;
            app.ui.actions.picked.clear();
            super::delete_selected_action(app);
        }
        return;
    }
    if picks.is_empty() || app.session.actions.recording.is_some() {
        return;
    }
    let in_folders = |a: &Action| a.set.as_deref().is_some_and(|s| picks.iter().any(|p| matches!(p, Pick::Folder(f) if folders::within(s, f))));
    let actions = app.session.actions.list.iter().filter(|a| in_folders(a)).count() + picks.iter().filter(|p| matches!(p, Pick::Action(_))).count();
    let all = folders::all(&app.session.actions);
    let folders = all.iter().filter(|f| picks.iter().any(|p| matches!(p, Pick::Folder(pf) if folders::within(f, pf)))).count();
    app.ui.actions.delete_prompt = Some(DeletePrompt { picks, actions, folders });
}

fn delete_now(app: &mut PhotocraftApp, picks: &[Pick]) -> Result<(), String> {
    for p in picks {
        match p {
            Pick::Folder(f) => {
                app.run("actions.deleteFolder", json!({"folder": f}))?;
                app.ui.actions.collapsed_sets.retain(|c| !folders::within(c, f));
            }
            Pick::Action(n) => {
                app.run("actions.delete", json!({"action": n}))?;
            }
        }
    }
    Ok(())
}

/// Delete's confirmation window.
pub(crate) fn delete_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(prompt) = app.ui.actions.delete_prompt.take() else { return };
    let mut ok = false;
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Modal::new(egui::Id::unique("actions-delete-prompt")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(tl!("Delete from Actions"));
        ui.separator();
        let template = "Delete {folders} folders and {actions} actions? This can't be undone.";
        ui.label(crate::i18n::fmt(tl!(template), &[("folders", &prompt.folders.to_string()), ("actions", &prompt.actions.to_string())]));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Yes, Delete")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ok = true;
            }
            if ui.button(tl!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if cancel {
        return;
    }
    if !ok {
        app.ui.actions.delete_prompt = Some(prompt);
        return;
    }
    let r = delete_now(app, &prompt.picks);
    clear(app);
    app.ui.actions.expanded.clear();
    report(app, r);
}

/// The right-click menu of an action or folder row: it acts on the selection when the row is in
/// it, else on that row alone.
pub(crate) fn item_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, pick: &Pick) {
    let chosen = selection(app);
    let picks = if chosen.contains(pick) { chosen } else { vec![pick.clone()] };
    if ui.button(tl!("Copy")).clicked() {
        app.ui.actions.clipboard = copy(app, &picks);
        ui.close();
    }
    if ui.add_enabled(!app.ui.actions.clipboard.is_empty(), egui::Button::new(tl!("Paste"))).clicked() {
        let items = app.ui.actions.clipboard.clone();
        let target = target_of(app, pick);
        let r = paste(app, &items, target);
        report(app, r);
        ui.close();
    }
    if ui.button(tl!("Duplicate")).clicked() {
        let r = duplicate(app, &picks);
        report(app, r);
        ui.close();
    }
    if ui.add_enabled(picks.len() == 1, egui::Button::new(tl!("Rename"))).clicked() {
        match pick {
            Pick::Folder(f) => start_rename(app, f),
            Pick::Action(n) => {
                if let Some(index) = index_of(app, n) {
                    app.ui.actions.renaming = Some(RenameAction { index, text: n.clone(), focused: false });
                }
            }
        }
        ui.close();
    }
    if ui.add_enabled(app.session.actions.recording.is_none(), egui::Button::new(tl!("Delete"))).clicked() {
        request_delete(app, picks.clone());
        ui.close();
    }
    ui.separator();
    if ui.button(tl!("New Folder")).clicked() {
        let target = target_of(app, pick);
        new_folder_in(app, target);
        ui.close();
    }
    let nested = roots(app, &picks).iter().any(|p| match p {
        Pick::Folder(f) => folders::parent(f).is_some(),
        Pick::Action(n) => folder_of(app, n).is_some(),
    });
    if ui.add_enabled(nested, egui::Button::new(tl!("Move to Top Level"))).clicked() {
        let r = move_into(app, &picks, None);
        report(app, r);
        ui.close();
    }
}

/// The right-click menu of the list's empty end: New Folder and Paste at the top level.
pub(crate) fn empty_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.button(tl!("New Folder")).clicked() {
        new_folder_in(app, None);
        ui.close();
    }
    if ui.add_enabled(!app.ui.actions.clipboard.is_empty(), egui::Button::new(tl!("Paste"))).clicked() {
        let items = app.ui.actions.clipboard.clone();
        let r = paste(app, &items, None);
        report(app, r);
        ui.close();
    }
}
