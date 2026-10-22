"""Shared dock workspace allocation and state management."""

import json
import os
import re
import tempfile
import time
from contextlib import contextmanager

import herdr_api

DOCK_LABEL = "herdr-dock"
DOCK_WAIT_SECONDS = 5
DOCK_MARKER_STALE_SECONDS = 10
REGISTRY_FILENAME = "herdr-dock-grid.json"
LOCK_DIRECTORY_NAME = "herdr-dock-grid.lock"
REGISTRY_VERSION = 1
LAYOUT_SNAPSHOT_VERSION = 1
DEFAULT_LAYOUT_POLICY = "grid-2x2"
LAYOUT_POLICIES = {DEFAULT_LAYOUT_POLICY: {"max_docks": 4}}


def plugins_folder():
    state = os.environ.get("HERDR_PLUGIN_STATE_DIR")
    if state:
        return os.path.dirname(os.path.abspath(state))
    base = os.environ.get("LOCALAPPDATA") or os.path.join(os.path.expanduser("~"), ".local", "state")
    return os.path.join(base, "herdr", "plugins")


def dock_marker(folder, key):
    return os.path.join(folder, f"{key}.dock")


def registry_path(folder):
    return os.path.join(folder, REGISTRY_FILENAME)


def lock_path(folder):
    return os.path.join(folder, LOCK_DIRECTORY_NAME)


def write_dock_marker(folder, key, pane_id):
    os.makedirs(folder, exist_ok=True)
    with open(dock_marker(folder, key), "w", encoding="utf-8") as file:
        file.write(pane_id or "")


def remove_dock_marker(folder, key):
    try:
        os.remove(dock_marker(folder, key))
    except FileNotFoundError:
        pass


def read_dock_pane(folder, key):
    try:
        with open(dock_marker(folder, key), encoding="utf-8") as file:
            return file.read().strip()
    except FileNotFoundError:
        return ""


def dock_starting(folder, key):
    path = dock_marker(folder, key)
    try:
        if read_dock_pane(folder, key):
            return False
        return time.time() - os.path.getmtime(path) < DOCK_MARKER_STALE_SECONDS
    except FileNotFoundError:
        return False


def dock_marker_keys(folder):
    try:
        entries = os.scandir(folder)
    except FileNotFoundError:
        return []
    with entries:
        return [
            entry.name[:-5]
            for entry in entries
            if entry.is_file() and entry.name.endswith(".dock")
        ]


def wait_for_earlier(folder, own):
    """Legacy entry point kept for plugins that call it. Waits for alphabetically earlier docks that are still claiming."""
    earlier = [key for key in dock_marker_keys(folder) if key < own]
    deadline = time.monotonic() + DOCK_WAIT_SECONDS
    while any(dock_starting(folder, key) for key in earlier) and time.monotonic() < deadline:
        time.sleep(0.2)


def claimed_dock_panes(folder, own):
    claimed = set()
    for key in dock_marker_keys(folder):
        pane_id = read_dock_pane(folder, key) if key != own else ""
        if pane_id:
            claimed.add(pane_id)
    return claimed


def _empty_registry():
    return {
        "version": REGISTRY_VERSION,
        "layout_policy": DEFAULT_LAYOUT_POLICY,
        "layout_snapshots": {},
        "docks": {},
        "registrations": {},
    }


def load_registry(folder):
    path = registry_path(folder)
    try:
        with open(path, encoding="utf-8") as file:
            registry = json.load(file)
    except FileNotFoundError:
        return _empty_registry()
    except json.JSONDecodeError as error:
        raise herdr_api.HerdrError(f"invalid dock registry {path}: {error}") from error

    if not isinstance(registry, dict) or registry.get("version") != REGISTRY_VERSION:
        raise herdr_api.HerdrError(f"unsupported dock registry {path}")
    if (
        not isinstance(registry.get("docks"), dict)
        or not isinstance(registry.get("registrations"), dict)
        or ("layout_snapshots" in registry and not isinstance(registry["layout_snapshots"], dict))
    ):
        raise herdr_api.HerdrError(f"invalid dock registry {path}")
    return registry


def write_registry(folder, registry):
    os.makedirs(folder, exist_ok=True)
    handle, temporary_path = tempfile.mkstemp(prefix=".herdr-dock-grid-", suffix=".json", dir=folder, text=True)
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as file:
            json.dump(registry, file, ensure_ascii=False, sort_keys=True)
            file.flush()
            os.fsync(file.fileno())
        os.replace(temporary_path, registry_path(folder))
    except OSError:
        try:
            os.remove(temporary_path)
        except FileNotFoundError:
            pass
        raise


@contextmanager
def registry_lock(folder):
    os.makedirs(folder, exist_ok=True)
    path = lock_path(folder)
    deadline = time.monotonic() + DOCK_WAIT_SECONDS
    while True:
        try:
            os.mkdir(path)
            break
        except FileExistsError:
            try:
                stale = time.time() - os.path.getmtime(path) >= DOCK_MARKER_STALE_SECONDS
                if stale:
                    os.rmdir(path)
                    continue
            except FileNotFoundError:
                continue
            except OSError:
                pass
            if time.monotonic() >= deadline:
                raise herdr_api.HerdrError("timed out waiting for the dock registry lock")
            time.sleep(0.1)
    try:
        yield
    finally:
        try:
            os.rmdir(path)
        except FileNotFoundError:
            pass


def _validate_key(key):
    if not isinstance(key, str) or not key:
        raise ValueError("dock key must be a non-empty string")


def _validate_layout_policy(policy):
    if policy not in LAYOUT_POLICIES:
        supported = ", ".join(sorted(LAYOUT_POLICIES))
        raise ValueError(f"unsupported layout policy {policy!r}; supported policies: {supported}")


def layout_policy(registry):
    policy = registry.get("layout_policy", DEFAULT_LAYOUT_POLICY)
    _validate_layout_policy(policy)
    return policy


def get_layout_policy():
    """Return the active dock layout policy."""
    folder = plugins_folder()
    with registry_lock(folder):
        return layout_policy(load_registry(folder))


def set_layout_policy(policy):
    """Persist the policy used for newly allocated dock panes."""
    _validate_layout_policy(policy)
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        registry["layout_policy"] = policy
        write_registry(folder, registry)
    return policy


def _layout_snapshot(workspace_id, layout):
    return {
        "version": LAYOUT_SNAPSHOT_VERSION,
        "captured_at": time.time(),
        "workspace_id": workspace_id,
        "tab_id": layout["tab_id"],
        "area": layout["area"],
        "panes": [
            {"pane_id": pane["pane_id"], "rect": pane["rect"]}
            for pane in layout["panes"]
        ],
        "splits": [
            {"id": split["id"], "direction": split["direction"], "ratio": split["ratio"]}
            for split in layout["splits"]
        ],
    }


def save_layout_snapshot():
    """Persist the current dock pane topology for later migration or restoration."""
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        workspace_id = dock_workspace_id()
        if workspace_id is None:
            raise herdr_api.HerdrError("dock workspace does not exist")
        panes = _panes_in([workspace_id])
        anchor = next((pane["pane_id"] for pane in panes if not is_sidebar(pane)), None)
        if anchor is None:
            raise herdr_api.HerdrError(f"workspace {workspace_id} has no dock pane")
        layout = herdr_api.cli("pane", "layout", "--pane", anchor)["layout"]
        if layout["workspace_id"] != workspace_id:
            raise herdr_api.HerdrError("dock pane layout belongs to a different workspace")
        snapshot = _layout_snapshot(workspace_id, layout)
        registry.setdefault("layout_snapshots", {})[workspace_id] = snapshot
        write_registry(folder, registry)
        return snapshot


def get_layout_snapshot(workspace_id=None):
    """Return the most recently saved topology for a dock workspace."""
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        target_workspace = workspace_id or dock_workspace_id()
        if target_workspace is None:
            return None
        snapshot = registry.get("layout_snapshots", {}).get(target_workspace)
        if snapshot is None:
            return None
        if snapshot.get("version") != LAYOUT_SNAPSHOT_VERSION:
            raise herdr_api.HerdrError(f"unsupported layout snapshot for workspace {target_workspace}")
        return json.loads(json.dumps(snapshot))


def register_dock(key, *, title=None, default_position=None, min_size=None, entrypoint=None):
    """Register a dock definition without allocating a pane."""
    _validate_key(key)
    definition = {
        name: value
        for name, value in {
            "title": title,
            "default_position": default_position,
            "min_size": min_size,
            "entrypoint": entrypoint,
        }.items()
        if value is not None
    }
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        registry["registrations"][key] = definition
        write_registry(folder, registry)
    return definition.copy()


def workspace_panes(workspace_id):
    panes = herdr_api.cli("pane", "list").get("panes", [])
    return [pane for pane in panes if pane["workspace_id"] == workspace_id]


def is_sidebar(pane):
    return any(name.startswith("herdr-sidebar") for name in (pane.get("tokens") or {}))


def dock_workspaces():
    """Return dock workspaces in order: herdr-dock, herdr-dock2, herdr-dock3, ..."""
    found = []
    for workspace in herdr_api.cli("workspace", "list").get("workspaces", []):
        match = re.fullmatch(re.escape(DOCK_LABEL) + r"(\d*)", workspace.get("label") or "")
        if not match:
            continue
        number = int(match.group(1)) if match.group(1) else 1
        if number >= 2 or not match.group(1):
            found.append((number, workspace))
    return [workspace for _, workspace in sorted(found, key=lambda item: item[0])]


def dock_workspace_ids():
    return [workspace["workspace_id"] for workspace in dock_workspaces()]


def dock_workspace_id():
    ids = dock_workspace_ids()
    return ids[0] if ids else None


def _next_workspace_label(workspaces):
    used = {workspace.get("label") for workspace in workspaces}
    number = 2
    while f"{DOCK_LABEL}{number}" in used:
        number += 1
    return f"{DOCK_LABEL}{number}"


def _panes_in(workspace_ids):
    wanted = set(workspace_ids)
    return [pane for pane in herdr_api.cli("pane", "list").get("panes", []) if pane["workspace_id"] in wanted]


def _live_pane_ids():
    return {pane["pane_id"] for pane in _panes_in(dock_workspace_ids())}


def _registry_claimed_panes(registry, own):
    return {
        dock["pane_id"]
        for key, dock in registry["docks"].items()
        if key != own and isinstance(dock, dict) and dock.get("pane_id")
    }


def _reconcile_registry(folder, registry, panes):
    pane_workspaces = {pane["pane_id"]: pane["workspace_id"] for pane in panes}
    owners = set()
    changed = False
    for key in sorted(registry["docks"]):
        dock = registry["docks"][key]
        dock = dock if isinstance(dock, dict) else {}
        pane_id = dock.get("pane_id")
        if (
            pane_id not in pane_workspaces
            or dock.get("workspace_id") != pane_workspaces[pane_id]
            or pane_id in owners
            or read_dock_pane(folder, key) != pane_id
        ):
            del registry["docks"][key]
            changed = True
            continue
        owners.add(pane_id)
    return changed


def _remove_dead_markers(folder, panes):
    """Remove markers whose pane is gone. Recent markers are kept: a legacy plugin may have just split its pane."""
    if not panes:
        return False
    live = {pane["pane_id"] for pane in panes}
    removed = False
    for key in dock_marker_keys(folder):
        pane_id = read_dock_pane(folder, key)
        if not pane_id or pane_id in live:
            continue
        try:
            age = time.time() - os.path.getmtime(dock_marker(folder, key))
        except FileNotFoundError:
            continue
        if age >= DOCK_MARKER_STALE_SECONDS:
            remove_dock_marker(folder, key)
            removed = True
    return removed


def _migrate_legacy_markers(folder, registry, panes):
    pane_workspaces = {pane["pane_id"]: pane["workspace_id"] for pane in panes}
    claimed = _registry_claimed_panes(registry, None)
    changed = False
    for key in dock_marker_keys(folder):
        pane_id = read_dock_pane(folder, key)
        if not pane_id or pane_id not in pane_workspaces or pane_id in claimed:
            continue
        registry["docks"][key] = {
            "workspace_id": pane_workspaces[pane_id],
            "pane_id": pane_id,
            "updated_at": time.time(),
        }
        claimed.add(pane_id)
        changed = True
    return changed


def reconcile_dock_state():
    """Rebuild the registry from valid live dock markers across all dock workspaces."""
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        panes = _panes_in(dock_workspace_ids())
        changed = _reconcile_registry(folder, registry, panes)
        _remove_dead_markers(folder, panes)
        changed = _migrate_legacy_markers(folder, registry, panes) or changed
        if changed:
            write_registry(folder, registry)
        return registry["docks"].copy()


def grid_split(anchor, cwd):
    layout = herdr_api.cli("pane", "layout", "--pane", anchor)["layout"]
    area = layout["area"]
    corners = {}
    for pane in layout["panes"]:
        rect = pane["rect"]
        corners[(0 if rect["x"] <= area["x"] else 1, 0 if rect["y"] <= area["y"] else 1)] = pane["pane_id"]
    top_left, top_right = corners.get((0, 0)), corners.get((1, 0))
    bottom_left, bottom_right = corners.get((0, 1)), corners.get((1, 1))
    if top_left and not top_right:
        target, direction = top_left, "right"
    elif top_left and not bottom_left:
        target, direction = top_left, "down"
    elif top_right and not bottom_right:
        target, direction = top_right, "down"
    elif bottom_left and not bottom_right:
        target, direction = bottom_left, "right"
    else:
        target, direction = anchor, "down"
    split = herdr_api.cli("pane", "split", target, "--direction", direction, "--cwd", cwd)
    return split["pane"]["pane_id"]


def pick_pane(panes, agent_panes, claimed):
    free = [
        pane["pane_id"]
        for pane in panes
        if not pane.get("label")
        and not pane.get("tokens")
        and pane["pane_id"] not in agent_panes
        and pane["pane_id"] not in claimed
    ]
    return free[0] if free else None


def split_for_layout_policy(policy, anchor, cwd):
    if policy == DEFAULT_LAYOUT_POLICY:
        return grid_split(anchor, cwd)
    raise herdr_api.HerdrError(f"unsupported layout policy {policy!r}")


def _create_dock_workspace(label, folder):
    created = herdr_api.cli("workspace", "create", "--label", label, "--cwd", folder, "--no-focus")
    return {"workspace_id": created["workspace"]["workspace_id"], "label": label}


def ensure_dock_pane(key, cwd):
    """Allocate or recover the pane assigned to a dock plugin."""
    _validate_key(key)
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        workspaces = dock_workspaces()
        if not workspaces:
            workspaces = [_create_dock_workspace(DOCK_LABEL, folder)]

        panes = _panes_in([workspace["workspace_id"] for workspace in workspaces])
        changed = _reconcile_registry(folder, registry, panes)
        _remove_dead_markers(folder, panes)
        changed = _migrate_legacy_markers(folder, registry, panes) or changed
        current = registry["docks"].get(key)
        if current:
            if changed:
                write_registry(folder, registry)
            return current["workspace_id"], current["pane_id"]

        write_dock_marker(folder, key, "")
        try:
            policy = layout_policy(registry)
            max_docks = LAYOUT_POLICIES[policy]["max_docks"]
            claimed = _registry_claimed_panes(registry, key) | claimed_dock_panes(folder, key)
            agent_panes = {agent["pane_id"] for agent in herdr_api.cli("agent", "list").get("agents", [])}
            workspace_id = pane_id = None
            for workspace in workspaces:
                candidate = workspace["workspace_id"]
                workspace_panes_ = [pane for pane in panes if pane["workspace_id"] == candidate]
                pane_id = pick_pane(workspace_panes_, agent_panes, claimed)
                if pane_id is None:
                    used = sum(
                        1 for name, dock in registry["docks"].items()
                        if name != key and dock.get("workspace_id") == candidate
                    )
                    anchor = next((pane["pane_id"] for pane in workspace_panes_ if not is_sidebar(pane)), None)
                    if used >= max_docks or anchor is None:
                        continue
                    pane_id = split_for_layout_policy(policy, anchor, cwd)
                workspace_id = candidate
                break
            if workspace_id is None:
                created = _create_dock_workspace(_next_workspace_label(workspaces), folder)
                workspace_id = created["workspace_id"]
                pane_id = pick_pane(_panes_in([workspace_id]), agent_panes, claimed)
                if pane_id is None:
                    raise herdr_api.HerdrError(f"workspace {workspace_id} has no pane")
            registry["docks"][key] = {
                "workspace_id": workspace_id,
                "pane_id": pane_id,
                "updated_at": time.time(),
            }
            write_registry(folder, registry)
            write_dock_marker(folder, key, pane_id)
            return workspace_id, pane_id
        except Exception:
            remove_dock_marker(folder, key)
            raise


def release_dock_pane(key):
    """Release a plugin's allocation without modifying the pane itself."""
    _validate_key(key)
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        released = registry["docks"].pop(key, None)
        if released is not None:
            write_registry(folder, registry)
        remove_dock_marker(folder, key)
    return released


def list_docks():
    """Return every known dock with its pane liveness. Does not modify state."""
    folder = plugins_folder()
    with registry_lock(folder):
        registry = load_registry(folder)
        workspaces = dock_workspaces()
        labels = {workspace["workspace_id"]: workspace.get("label") for workspace in workspaces}
        pane_workspaces = {
            pane["pane_id"]: pane["workspace_id"] for pane in _panes_in(list(labels))
        }
        keys = set(dock_marker_keys(folder)) | set(registry["docks"]) | set(registry["registrations"])
        docks = []
        for key in sorted(keys):
            pane_id = read_dock_pane(folder, key)
            workspace_id = pane_workspaces.get(pane_id)
            docks.append({
                "key": key,
                "pane_id": pane_id or None,
                "workspace_id": workspace_id,
                "workspace": labels.get(workspace_id),
                "live": workspace_id is not None,
                "registered": key in registry["registrations"],
                "definition": registry["registrations"].get(key, {}),
            })
        return docks


def _close_empty_extra_workspaces(folder, registry):
    """Close herdr-dock2 and later workspaces that hold no claimed, labeled, tokened or agent panes."""
    workspaces = dock_workspaces()[1:]
    if not workspaces:
        return []
    claimed = _registry_claimed_panes(registry, None) | claimed_dock_panes(folder, None)
    agent_panes = {agent["pane_id"] for agent in herdr_api.cli("agent", "list").get("agents", [])}
    closed = []
    for workspace in workspaces:
        workspace_id = workspace["workspace_id"]
        panes = [pane for pane in _panes_in([workspace_id]) if not is_sidebar(pane)]
        if all(pick_pane([pane], agent_panes, claimed) for pane in panes):
            herdr_api.cli("workspace", "close", workspace_id)
            closed.append(workspace_id)
    return closed


def reset_dock_state():
    """Drop allocations and markers whose pane is gone, and close empty herdr-dockN (N>=2) workspaces.

    Registrations and live panes are kept. Returns the keys that were removed."""
    folder = plugins_folder()
    removed = []
    with registry_lock(folder):
        registry = load_registry(folder)
        live = _live_pane_ids()
        for key in dock_marker_keys(folder):
            pane_id = read_dock_pane(folder, key)
            if pane_id and pane_id not in live:
                remove_dock_marker(folder, key)
                removed.append(key)
        for key in list(registry["docks"]):
            if registry["docks"][key].get("pane_id") not in live:
                del registry["docks"][key]
        write_registry(folder, registry)
        _close_empty_extra_workspaces(folder, registry)
    return sorted(removed)
