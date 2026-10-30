//! Action folders: an action's `set` is a folder path, its segments joined by [`SEP`]
//! (`Retouch/Skin`), so folders nest. Empty folders live in [`super::ActionState::folders`];
//! a folder that holds actions exists through them as well. Save Actions writes a nested folder as
//! a set named by its path (Photoshop's sets don't nest).

use serde_json::{Value, json};

use super::{ActionState, PLAY_SHORTCUT_PREFIX, always, bad, not_playing, resolve};
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

pub fn specs() -> Vec<CommandSpec> {
    vec![
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
