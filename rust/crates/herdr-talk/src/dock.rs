//! Shared dock workspace allocation: the parts of herdr-dock-grid's dock_layout.py this plugin uses.
//! Python plugins share the same registry, markers and lock, so these rules must stay in step with that file.

use crate::herdr_api::cli;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DOCK_LABEL: &str = "herdr-dock";
const DOCK_WAIT: Duration = Duration::from_secs(5);
const DOCK_MARKER_STALE: Duration = Duration::from_secs(10);
const MAX_DOCKS: usize = 4; // the grid-2x2 policy, the only one there is

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    value.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

pub fn pane_id(pane: &Value) -> &str {
    text(pane, "pane_id")
}

pub fn plugins_folder() -> PathBuf {
    if let Some(parent) = env::var("HERDR_PLUGIN_STATE_DIR").ok().filter(|state| !state.is_empty())
        .and_then(|state| Path::new(&state).parent().map(Path::to_path_buf)) {
        return parent;
    }
    let base = env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| {
        Path::new(&env::var("HOME").unwrap_or_default()).join(".local").join("state")
    });
    base.join("herdr").join("plugins")
}

fn dock_marker(folder: &Path, key: &str) -> PathBuf {
    folder.join(format!("{key}.dock"))
}

fn write_dock_marker(folder: &Path, key: &str, pane: &str) -> Result<(), String> {
    fs::create_dir_all(folder).and_then(|_| fs::write(dock_marker(folder, key), pane)).map_err(|error| error.to_string())
}

fn remove_dock_marker(folder: &Path, key: &str) {
    let _ = fs::remove_file(dock_marker(folder, key));
}

pub fn read_dock_pane(folder: &Path, key: &str) -> String {
    fs::read_to_string(dock_marker(folder, key)).unwrap_or_default().trim().to_string()
}

fn dock_marker_keys(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries.flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().to_string_lossy().strip_suffix(".dock").map(str::to_string))
        .collect()
}

fn claimed_dock_panes(folder: &Path, own: &str) -> HashSet<String> {
    dock_marker_keys(folder).iter().filter(|key| *key != own)
        .map(|key| read_dock_pane(folder, key)).filter(|pane| !pane.is_empty()).collect()
}

fn load_registry(folder: &Path) -> Result<Value, String> {
    let path = folder.join("herdr-dock-grid.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(json!({"version": 1, "layout_policy": "grid-2x2", "layout_snapshots": {}, "docks": {}, "registrations": {}}));
        }
        Err(error) => return Err(format!("invalid dock registry {}: {error}", path.display())),
    };
    let registry: Value = serde_json::from_str(&raw).map_err(|error| format!("invalid dock registry {}: {error}", path.display()))?;
    if registry.get("version") != Some(&json!(1)) || !registry["docks"].is_object() || !registry["registrations"].is_object() {
        return Err(format!("unsupported dock registry {}", path.display()));
    }
    Ok(registry)
}

/// Write beside the registry, then rename, so a reader never sees half a file.
fn write_registry(folder: &Path, registry: &Value) -> Result<(), String> {
    let temporary = folder.join(format!(".herdr-dock-grid-{:016x}.json", fastrand::u64(..)));
    let written = fs::create_dir_all(folder)
        .and_then(|_| fs::File::create(&temporary))
        .and_then(|mut file| file.write_all(registry.to_string().as_bytes()).and_then(|_| file.sync_all()))
        .and_then(|_| fs::rename(&temporary, folder.join("herdr-dock-grid.json")));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(|error| error.to_string())
}

struct RegistryLock(PathBuf);

impl Drop for RegistryLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

fn registry_lock(folder: &Path) -> Result<RegistryLock, String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let path = folder.join("herdr-dock-grid.lock");
    let deadline = Instant::now() + DOCK_WAIT;
    loop {
        match fs::create_dir(&path) {
            Ok(()) => return Ok(RegistryLock(path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let age = fs::metadata(&path).and_then(|meta| meta.modified()).ok().and_then(|time| time.elapsed().ok());
                if age.is_some_and(|age| age >= DOCK_MARKER_STALE) && fs::remove_dir(&path).is_ok() {
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err("timed out waiting for the dock registry lock".to_string());
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub fn is_sidebar(pane: &Value) -> bool {
    pane.get("tokens").and_then(Value::as_object).is_some_and(|tokens| tokens.keys().any(|name| name.starts_with("herdr-sidebar")))
}

fn dock_number(label: &str) -> Option<u64> {
    let digits = label.strip_prefix(DOCK_LABEL)?;
    if digits.is_empty() {
        return Some(1);
    }
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|number| *number >= 2)
}

/// Dock workspaces in order: herdr-dock, herdr-dock2, herdr-dock3, ...
fn dock_workspaces() -> Result<Vec<Value>, String> {
    let mut found: Vec<(u64, Value)> = list(&cli(&["workspace", "list"])?, "workspaces").into_iter()
        .filter_map(|workspace| Some((dock_number(text(&workspace, "label"))?, workspace)))
        .collect();
    found.sort_by_key(|(number, _)| *number);
    Ok(found.into_iter().map(|(_, workspace)| workspace).collect())
}

pub fn dock_workspace_id() -> Result<Option<String>, String> {
    Ok(dock_workspaces()?.first().map(|workspace| text(workspace, "workspace_id").to_string()))
}

fn next_workspace_label(workspaces: &[Value]) -> String {
    (2..).map(|number| format!("{DOCK_LABEL}{number}"))
        .find(|label| !workspaces.iter().any(|workspace| text(workspace, "label") == label))
        .unwrap_or_default()
}

fn panes_in(workspace_ids: &[String]) -> Result<Vec<Value>, String> {
    let panes = list(&cli(&["pane", "list"])?, "panes");
    Ok(panes.into_iter().filter(|pane| workspace_ids.iter().any(|id| id == text(pane, "workspace_id"))).collect())
}

pub fn workspace_panes(workspace_id: &str) -> Result<Vec<Value>, String> {
    panes_in(&[workspace_id.to_string()])
}

fn docks(registry: &mut Value) -> &mut Map<String, Value> {
    registry["docks"].as_object_mut().expect("checked by load_registry")
}

fn registry_claimed_panes(registry: &Value, own: &str) -> HashSet<String> {
    registry["docks"].as_object().into_iter().flatten()
        .filter(|(key, _)| *key != own)
        .map(|(_, dock)| text(dock, "pane_id").to_string())
        .filter(|pane| !pane.is_empty())
        .collect()
}

fn reconcile_registry(folder: &Path, registry: &mut Value, panes: &[Value]) -> bool {
    let mut owners = HashSet::new();
    let mut changed = false;
    let keys: Vec<String> = docks(registry).keys().cloned().collect();
    for key in keys {
        let dock = &registry["docks"][&key];
        let id = text(dock, "pane_id").to_string();
        let live = panes.iter().find(|pane| pane_id(pane) == id);
        let valid = live.is_some_and(|pane| text(pane, "workspace_id") == text(dock, "workspace_id"))
            && !owners.contains(&id)
            && read_dock_pane(folder, &key) == id;
        if valid {
            owners.insert(id);
        } else {
            docks(registry).remove(&key);
            changed = true;
        }
    }
    changed
}

/// Remove markers whose pane is gone. Recent markers stay: a legacy plugin may have just split its pane.
fn remove_dead_markers(folder: &Path, panes: &[Value]) {
    if panes.is_empty() {
        return;
    }
    for key in dock_marker_keys(folder) {
        let id = read_dock_pane(folder, &key);
        if id.is_empty() || panes.iter().any(|pane| pane_id(pane) == id) {
            continue;
        }
        let age = fs::metadata(dock_marker(folder, &key)).and_then(|meta| meta.modified()).ok().and_then(|time| time.elapsed().ok());
        if age.is_some_and(|age| age >= DOCK_MARKER_STALE) {
            remove_dock_marker(folder, &key);
        }
    }
}

fn migrate_legacy_markers(folder: &Path, registry: &mut Value, panes: &[Value]) -> bool {
    let mut claimed = registry_claimed_panes(registry, "");
    let mut changed = false;
    for key in dock_marker_keys(folder) {
        let id = read_dock_pane(folder, &key);
        let Some(pane) = panes.iter().find(|pane| pane_id(pane) == id) else { continue };
        if id.is_empty() || claimed.contains(&id) {
            continue;
        }
        docks(registry).insert(key, json!({"workspace_id": text(pane, "workspace_id"), "pane_id": id, "updated_at": now()}));
        claimed.insert(id);
        changed = true;
    }
    changed
}

fn grid_split(anchor: &str, cwd: &str) -> Result<String, String> {
    let answer = cli(&["pane", "layout", "--pane", anchor])?;
    let layout = &answer["layout"];
    let area = &layout["area"];
    let corner = |right: bool, bottom: bool| -> Option<String> {
        list(layout, "panes").iter().rev().find(|pane| {
            (pane["rect"]["x"].as_f64() > area["x"].as_f64()) == right && (pane["rect"]["y"].as_f64() > area["y"].as_f64()) == bottom
        }).map(|pane| pane_id(pane).to_string())
    };
    let (top_left, top_right) = (corner(false, false), corner(true, false));
    let (bottom_left, bottom_right) = (corner(false, true), corner(true, true));
    let (target, direction) = match (&top_left, &top_right, &bottom_left, &bottom_right) {
        (Some(pane), None, _, _) => (pane.clone(), "right"),
        (Some(pane), _, None, _) => (pane.clone(), "down"),
        (_, Some(pane), _, None) => (pane.clone(), "down"),
        (_, _, Some(pane), None) => (pane.clone(), "right"),
        _ => (anchor.to_string(), "down"),
    };
    let split = cli(&["pane", "split", &target, "--direction", direction, "--cwd", cwd])?;
    Ok(text(&split["pane"], "pane_id").to_string())
}

fn pick_pane(panes: &[&Value], agent_panes: &HashSet<String>, claimed: &HashSet<String>) -> Option<String> {
    panes.iter().find(|pane| {
        text(pane, "label").is_empty()
            && pane.get("tokens").and_then(Value::as_object).is_none_or(Map::is_empty)
            && !agent_panes.contains(pane_id(pane))
            && !claimed.contains(pane_id(pane))
    }).map(|pane| pane_id(pane).to_string())
}

fn create_dock_workspace(label: &str, folder: &Path) -> Result<Value, String> {
    let folder = folder.to_string_lossy();
    let created = cli(&["workspace", "create", "--label", label, "--cwd", &folder, "--no-focus"])?;
    Ok(json!({"workspace_id": text(&created["workspace"], "workspace_id"), "label": label}))
}

/// Allocate or recover the pane assigned to a dock plugin. Returns the pane id.
pub fn ensure_dock_pane(key: &str, cwd: &Path) -> Result<String, String> {
    let folder = plugins_folder();
    let _lock = registry_lock(&folder)?;
    let mut registry = load_registry(&folder)?;
    let mut workspaces = dock_workspaces()?;
    if workspaces.is_empty() {
        workspaces.push(create_dock_workspace(DOCK_LABEL, &folder)?);
    }
    let ids: Vec<String> = workspaces.iter().map(|workspace| text(workspace, "workspace_id").to_string()).collect();
    let panes = panes_in(&ids)?;
    let mut changed = reconcile_registry(&folder, &mut registry, &panes);
    remove_dead_markers(&folder, &panes);
    changed = migrate_legacy_markers(&folder, &mut registry, &panes) || changed;
    if let Some(current) = registry["docks"].get(key) {
        let id = text(current, "pane_id").to_string();
        if changed {
            write_registry(&folder, &registry)?;
        }
        return Ok(id);
    }

    write_dock_marker(&folder, key, "")?;
    let allocated = allocate(key, cwd, &folder, &mut registry, &workspaces, &panes);
    if allocated.is_err() {
        remove_dock_marker(&folder, key);
    }
    allocated
}

fn allocate(key: &str, cwd: &Path, folder: &Path, registry: &mut Value, workspaces: &[Value], panes: &[Value]) -> Result<String, String> {
    let mut claimed = registry_claimed_panes(registry, key);
    claimed.extend(claimed_dock_panes(folder, key));
    let agents = cli(&["agent", "list"])?;
    let agent_panes: HashSet<String> = list(&agents, "agents").iter().map(|agent| pane_id(agent).to_string()).collect();
    let mut found = None;
    for workspace in workspaces {
        let candidate = text(workspace, "workspace_id");
        let inside: Vec<&Value> = panes.iter().filter(|pane| text(pane, "workspace_id") == candidate).collect();
        let mut pane = pick_pane(&inside, &agent_panes, &claimed);
        if pane.is_none() {
            let used = registry["docks"].as_object().into_iter().flatten()
                .filter(|(name, dock)| *name != key && text(dock, "workspace_id") == candidate)
                .count();
            let anchor = inside.iter().find(|pane| !is_sidebar(pane)).map(|pane| pane_id(pane));
            let Some(anchor) = anchor.filter(|_| used < MAX_DOCKS) else { continue };
            pane = Some(grid_split(anchor, &cwd.to_string_lossy())?);
        }
        found = pane.map(|pane| (candidate.to_string(), pane));
        break;
    }
    let (workspace_id, pane) = match found {
        Some(found) => found,
        None => {
            let created = create_dock_workspace(&next_workspace_label(workspaces), folder)?;
            let workspace_id = text(&created, "workspace_id").to_string();
            let inside = workspace_panes(&workspace_id)?;
            let pane = pick_pane(&inside.iter().collect::<Vec<_>>(), &agent_panes, &claimed)
                .ok_or_else(|| format!("workspace {workspace_id} has no pane"))?;
            (workspace_id, pane)
        }
    };
    docks(registry).insert(key.to_string(), json!({"workspace_id": workspace_id, "pane_id": pane, "updated_at": now()}));
    write_registry(folder, registry)?;
    write_dock_marker(folder, key, &pane)?;
    Ok(pane)
}

/// Release a plugin's allocation without changing the pane itself.
pub fn release_dock_pane(key: &str) -> Result<(), String> {
    let folder = plugins_folder();
    let _lock = registry_lock(&folder)?;
    let mut registry = load_registry(&folder)?;
    if docks(&mut registry).remove(key).is_some() {
        write_registry(&folder, &registry)?;
    }
    remove_dock_marker(&folder, key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_labels() {
        assert_eq!(dock_number("herdr-dock"), Some(1));
        assert_eq!(dock_number("herdr-dock3"), Some(3));
        assert_eq!(dock_number("herdr-dock1"), None);
        assert_eq!(dock_number("herdr-dockx"), None);
        assert_eq!(dock_number("other"), None);
    }

    #[test]
    fn next_label_skips_used_numbers() {
        let used = [json!({"label": "herdr-dock"}), json!({"label": "herdr-dock2"})];
        assert_eq!(next_workspace_label(&used), "herdr-dock3");
    }

    #[test]
    fn pick_pane_skips_used_panes() {
        let panes = [
            json!({"pane_id": "labeled", "label": "x"}),
            json!({"pane_id": "tokened", "tokens": {"a": 1}}),
            json!({"pane_id": "agent"}),
            json!({"pane_id": "claimed"}),
            json!({"pane_id": "free", "label": null, "tokens": {}}),
        ];
        let agents = HashSet::from(["agent".to_string()]);
        let claimed = HashSet::from(["claimed".to_string()]);
        assert_eq!(pick_pane(&panes.iter().collect::<Vec<_>>(), &agents, &claimed), Some("free".to_string()));
    }

    #[test]
    fn sidebar_is_found_by_token_prefix() {
        assert!(is_sidebar(&json!({"tokens": {"herdr-sidebar-x": 1}})));
        assert!(!is_sidebar(&json!({"tokens": null})));
    }

    #[test]
    fn dead_docks_leave_the_registry() {
        let folder = env::temp_dir().join(format!("herdr-talk-test-{:016x}", fastrand::u64(..)));
        write_dock_marker(&folder, "live", "p1").unwrap();
        let mut registry = json!({"docks": {
            "live": {"pane_id": "p1", "workspace_id": "w"},
            "gone": {"pane_id": "p2", "workspace_id": "w"},
        }});
        let panes = [json!({"pane_id": "p1", "workspace_id": "w"})];
        assert!(reconcile_registry(&folder, &mut registry, &panes));
        assert_eq!(registry["docks"].as_object().unwrap().keys().collect::<Vec<_>>(), ["live"]);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn lock_is_exclusive_and_released() {
        let folder = env::temp_dir().join(format!("herdr-talk-test-{:016x}", fastrand::u64(..)));
        let lock = registry_lock(&folder).unwrap();
        assert!(folder.join("herdr-dock-grid.lock").is_dir());
        drop(lock);
        assert!(!folder.join("herdr-dock-grid.lock").exists());
        fs::remove_dir_all(folder).unwrap();
    }
}
