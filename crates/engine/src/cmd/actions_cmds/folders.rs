//! Action folders: an action's `set` is a folder path, its segments joined by [`SEP`]
//! (`Retouch/Skin`), so folders nest. Empty folders live in [`super::ActionState::folders`];
//! a folder that holds actions exists through them as well. Save Actions writes a nested folder as
//! a set named by its path (Photoshop's sets don't nest).

use serde_json::{Value, json};

use super::{Action, ActionState, PLAY_SHORTCUT_PREFIX, always, bad, not_playing, resolve};
use crate::commands::CommandSpec;
use crate::{Result, Session};

/// Separates the folders of a path.
pub const SEP: char = '/';
/// Longest folder name, in characters.
const MAX_NAME: usize = 120;
/// Deepest nesting.
pub const MAX_DEPTH: usize = 16;

/// Every folder: the kept ones (empty folders included), then any only an action names, each
/// with every folder above it, parents first. No duplicates.
pub fn all(state: &ActionState) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let named = state.folders.iter().map(String::as_str).chain(state.list.iter().filter_map(|a| a.set.as_deref()));
    for path in named {
        let mut at = String::new();
        for seg in path.split(SEP) {
            if !at.is_empty() {
                at.push(SEP);
            }
            at.push_str(seg);
            if !out.contains(&at) {
                out.push(at.clone());
            }
        }
    }
    out
}

/// The folder's own name: its path's last segment.
pub fn leaf(path: &str) -> &str {
    path.rsplit(SEP).next().unwrap_or(path)
}

/// The folder holding `path`, `None` at the top level.
pub fn parent(path: &str) -> Option<&str> {
    path.rsplit_once(SEP).map(|(p, _)| p)
}

/// How many folders deep `path` is (0 at the top level).
pub fn depth(path: &str) -> usize {
    path.matches(SEP).count()
}

/// Whether `path` is `folder` or inside it.
pub fn within(path: &str, folder: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with(SEP))
}

fn name_param(cmd: &str, p: &Value) -> Result<String> {
    let name = match p.get("name") {
        Some(Value::String(n)) => n.trim().to_string(),
        Some(_) => return Err(bad(cmd, "\"name\" must be a string")),
        None => return Err(bad(cmd, "needs \"name\"")),
    };
    if name.is_empty() {
        return Err(bad(cmd, "the folder name is empty"));
    }
    if name.contains(SEP) {
        return Err(bad(cmd, format!("a folder name can't contain \"{SEP}\"")));
    }
    if name.chars().count() > MAX_NAME {
        return Err(bad(cmd, format!("the folder name is longer than {MAX_NAME} characters")));
    }
    Ok(name)
}

/// An existing folder named by `key` (`null` or missing is the top level when `optional`).
fn folder_param(s: &Session, cmd: &str, p: &Value, key: &str, optional: bool) -> Result<Option<String>> {
    match p.get(key) {
        None | Some(Value::Null) if optional => Ok(None),
        None | Some(Value::Null) => Err(bad(cmd, format!("needs \"{key}\""))),
        Some(Value::String(f)) => {
            let f = f.trim().trim_matches(SEP).to_string();
            if f.is_empty() && optional {
                return Ok(None);
            }
            if all(&s.actions).contains(&f) { Ok(Some(f)) } else { Err(bad(cmd, format!("there is no folder \"{f}\""))) }
        }
        Some(_) => Err(bad(cmd, format!("\"{key}\" must be a folder path"))),
    }
}

fn join(parent: Option<&str>, name: &str) -> String {
    parent.map_or_else(|| name.to_string(), |p| format!("{p}{SEP}{name}"))
}

/// A name for a new folder in `parent` that no folder there has: `base`, else `base 2`, `base 3`…
pub fn unique_child(state: &ActionState, parent: Option<&str>, base: &str) -> String {
    let taken = all(state);
    let free = |name: &str| !taken.contains(&join(parent, name));
    if free(base) {
        return base.to_string();
    }
    (2..10_000).map(|n| format!("{base} {n}")).find(|n| free(n)).unwrap_or_else(|| format!("{base} {}", taken.len() + 1))
}

/// An action name no action has: `base`, else `base copy`, `base copy 2`…
pub fn unique_action_name(state: &ActionState, base: &str) -> String {
    let free = |name: &str| !state.list.iter().any(|a| a.name == name);
    if free(base) {
        return base.to_string();
    }
    std::iter::once(format!("{base} copy"))
        .chain((2..10_000).map(|n| format!("{base} copy {n}")))
        .find(|n| free(n))
        .unwrap_or_else(|| format!("{base} copy {}", state.list.len() + 1))
}

/// `actions.newFolder`.
fn new_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.newFolder";
    not_playing(s, cmd)?;
    let name = name_param(cmd, p)?;
    let parent = folder_param(s, cmd, p, "parent", true)?;
    if parent.as_deref().is_some_and(|p| depth(p) + 1 >= MAX_DEPTH) {
        return Err(bad(cmd, format!("folders nest at most {MAX_DEPTH} deep")));
    }
    let path = join(parent.as_deref(), &name);
    if all(&s.actions).contains(&path) {
        return Err(bad(cmd, format!("a folder \"{name}\" already exists there")));
    }
    s.actions.folders.push(path.clone());
    s.actions.touch();
    Ok(json!({"folder": path}))
}

/// `actions.renameFolder`: the folder, its subfolders and their actions follow.
fn rename_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.renameFolder";
    not_playing(s, cmd)?;
    let Some(old) = folder_param(s, cmd, p, "folder", false)? else { return Err(bad(cmd, "needs \"folder\"")) };
    let name = name_param(cmd, p)?;
    let new = join(parent(&old), &name);
    if new == old {
        return Ok(json!({"folder": new}));
    }
    if all(&s.actions).contains(&new) {
        return Err(bad(cmd, format!("a folder \"{name}\" already exists there")));
    }
    let moved = |path: &str| -> Option<String> { within(path, &old).then(|| format!("{new}{}", &path[old.len()..])) };
    for f in s.actions.folders.iter_mut() {
        if let Some(m) = moved(f) {
            *f = m;
        }
    }
    for a in s.actions.list.iter_mut() {
        if let Some(m) = a.set.as_deref().and_then(moved) {
            a.set = Some(m);
        }
    }
    s.actions.touch();
    Ok(json!({"folder": new, "was": old}))
}

/// `actions.deleteFolder`: the folder, its subfolders and every action in them, as deleting an
/// action set does in Photoshop. Their shortcuts go with them.
fn delete_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.deleteFolder";
    not_playing(s, cmd)?;
    if s.actions.recording.is_some() {
        return Err(bad(cmd, "stop recording before deleting a folder"));
    }
    let Some(folder) = folder_param(s, cmd, p, "folder", false)? else { return Err(bad(cmd, "needs \"folder\"")) };
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut s.actions.list).into_iter().partition(|a| a.set.as_deref().is_some_and(|f| within(f, &folder)));
    s.actions.list = kept;
    s.actions.folders.retain(|f| !within(f, &folder));
    s.actions.touch();
    let names: Vec<String> = gone.into_iter().map(|a| a.name).collect();
    let keys: Vec<String> = names.iter().map(|n| format!("{PLAY_SHORTCUT_PREFIX}{n}")).filter(|k| s.prefs().shortcuts.contains_key(k)).collect();
    if !keys.is_empty() {
        s.edit_prefs(|prefs| {
            for k in &keys {
                prefs.shortcuts.remove(k);
            }
        });
    }
    Ok(json!({"deleted": folder, "actions": names}))
}

/// `actions.moveToFolder`: an action into a folder, or out to the top level with `null`.
fn move_to_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.moveToFolder";
    not_playing(s, cmd)?;
    let idx = resolve(&s.actions.list, p, cmd)?;
    if p.get("folder").is_none() {
        return Err(bad(cmd, "needs \"folder\" (a folder path, or null for the top level)"));
    }
    let folder = folder_param(s, cmd, p, "folder", true)?;
    // The folder it leaves stays, even if it's now empty.
    let Some(action) = s.actions.list.get_mut(idx) else { return Err(bad(cmd, "no such action")) };
    let left = std::mem::replace(&mut action.set, folder.clone());
    let name = action.name.clone();
    if let Some(left) = left.filter(|l| !s.actions.folders.contains(l)) {
        s.actions.folders.push(left);
    }
    s.actions.touch();
    Ok(json!({"action": name, "folder": folder}))
}

/// `actions.add`: a copy of an action (Copy and Paste, Duplicate), named apart from the others.
fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.add";
    not_playing(s, cmd)?;
    let mut action: Action = match p.get("action") {
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("\"action\" is not an action: {e}")))?,
        _ => return Err(bad(cmd, "needs \"action\" ({name, steps, …})")),
    };
    let base = action.name.trim().to_string();
    if base.is_empty() {
        return Err(bad(cmd, "the action's name is empty"));
    }
    if p.get("folder").is_none() && action.set.is_some() {
        // Keep the folder the action names, when it exists.
        let known = all(&s.actions);
        action.set = action.set.filter(|f| known.contains(f));
    } else {
        action.set = folder_param(s, cmd, p, "folder", true)?;
    }
    let steps = action.steps.len();
    action.disabled.retain(|i| *i < steps);
    action.dialogs.retain(|i| *i < steps);
    if !super::BUTTON_COLORS.contains(&action.color.as_str()) {
        action.color.clear();
    }
    action.name = unique_action_name(&s.actions, &base);
    let name = action.name.clone();
    s.actions.list.push(action);
    s.actions.touch();
    Ok(json!({"action": name, "index": s.actions.list.len() - 1}))
}

/// `actions.moveFolder`: a folder, with its subfolders and actions, into another folder or out to
/// the top level.
fn move_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.moveFolder";
    not_playing(s, cmd)?;
    let Some(old) = folder_param(s, cmd, p, "folder", false)? else { return Err(bad(cmd, "needs \"folder\"")) };
    if p.get("into").is_none() {
        return Err(bad(cmd, "needs \"into\" (a folder path, or null for the top level)"));
    }
    let into = folder_param(s, cmd, p, "into", true)?;
    if into.as_deref().is_some_and(|i| within(i, &old)) {
        return Err(bad(cmd, "a folder can't go inside itself"));
    }
    let new = join(into.as_deref(), leaf(&old));
    if new == old {
        return Ok(json!({"folder": new, "was": old}));
    }
    if all(&s.actions).contains(&new) {
        return Err(bad(cmd, format!("a folder \"{}\" is already there", leaf(&old))));
    }
    let deepest = all(&s.actions).iter().filter(|f| within(f, &old)).map(|f| depth(f) - depth(&old)).max().unwrap_or(0);
    if depth(&new) + deepest >= MAX_DEPTH {
        return Err(bad(cmd, format!("folders nest at most {MAX_DEPTH} deep")));
    }
    let moved = |path: &str| -> Option<String> { within(path, &old).then(|| format!("{new}{}", &path[old.len()..])) };
    // Folders an action names exist through it: keep every moved folder, so empty ones survive.
    let mut kept: Vec<String> = all(&s.actions).iter().filter_map(|f| moved(f)).collect();
    s.actions.folders.retain(|f| !within(f, &old));
    s.actions.folders.append(&mut kept);
    for a in s.actions.list.iter_mut() {
        if let Some(m) = a.set.as_deref().and_then(moved) {
            a.set = Some(m);
        }
    }
    s.actions.touch();
    Ok(json!({"folder": new, "was": old}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.add",
            label: "Add Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":{name, steps:[[id, params]…], disabled?, dialogs?, color?, set?}, "folder":path|null?} → {action, index}. Adds a copy of an action (Paste, Duplicate); a taken name gets " copy" (then " copy 2"…). Without "folder" it keeps the action's own set when that folder exists."##,
            enabled: always,
            run: add,
            journal: false,
        },
        CommandSpec {
            id: "actions.moveFolder",
            label: "Move Action Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"folder":path, "into":path|null} → {folder, was}. Moves the folder, its subfolders and actions inside "into" (null: the top level). Not into itself or a folder of the same name."##,
            enabled: always,
            run: move_folder,
            journal: false,
        },
        CommandSpec {
            id: "actions.newFolder",
            label: "New Action Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str, "parent":folder?} → {folder}. Creates an (empty) action folder at the top level, or inside "parent" (a folder path, segments joined by "/"). Names can't contain "/"."##,
            enabled: always,
            run: new_folder,
            journal: false,
        },
        CommandSpec {
            id: "actions.renameFolder",
            label: "Rename Action Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"folder":path, "name":str} → {folder, was}. Its subfolders and actions follow."##,
            enabled: always,
            run: rename_folder,
            journal: false,
        },
        CommandSpec {
            id: "actions.deleteFolder",
            label: "Delete Action Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"folder":path} → {deleted, actions:[name…]}. Deletes the folder, its subfolders and every action in them, with their shortcuts."##,
            enabled: always,
            run: delete_folder,
            journal: false,
        },
        CommandSpec {
            id: "actions.moveToFolder",
            label: "Move Action to Folder",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "folder":path|null} → {action, folder}. null moves it to the top level."##,
            enabled: always,
            run: move_to_folder,
            journal: false,
        },
    ]
}
