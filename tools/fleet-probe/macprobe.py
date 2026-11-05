#!/usr/bin/env python3
"""macOS driver for the probe: the counterpart of winprobe.py, playing the same scene files.

    macprobe.py run <scene.json> fleet|weep <workspace> <out dir>
    macprobe.py attach <scene.json> fleet <pid> <out dir>   a Fleet already open (single instance)
    macprobe.py check                                       is input allowed (Accessibility)?

Input goes through `macinput` (macinput.swift beside this file, compiled on first use into
~/Library/Caches/weep-probe): CGEvent keys and pointer, the Accessibility API for placing and
raising windows. That needs Accessibility permission for the terminal running this script
(System Settings > Privacy & Security > Accessibility). Shots are `screencapture -o -l <window>`:
the whole window without its shadow, which on macOS is the client area, since both Fleet and
WEEP draw up to the top edge. Coordinates are window-relative points; shots are device pixels
(2x on a Retina display), and compare.py reads the scale off the image width.

A scene's chords are written as on Windows (`ctrl+k`). Each is looked up in WEEP's keymap
(crates/weep/assets/keymap/default.json, Fleet's own key and mac pairs) and sent as the mac
chord (`cmd-k`); a chord the keymap lacks is sent as written. A scene may carry `steps` for
`fleet_mac` / `weep_mac` to override the translated ones.

Like winprobe, input is refused unless the launched editor is the frontmost app: run it in the
foreground and leave the Mac alone while a scene plays.
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
CACHE = os.path.expanduser("~/Library/Caches/weep-probe")
MACINPUT = os.path.join(CACHE, "macinput")
FLEET = os.environ.get("FLEET_EXE", "/Applications/Fleet.app/Contents/MacOS/Fleet")
# Fleet's user settings on macOS (its log names the file it loads); written for a run and put
# back after it.
FLEET_SETTINGS = [
    os.path.expanduser("~/Library/Application Support/JetBrains/Fleet/settings.json"),
]
# Written into the workspace's `.fleet/settings.json` for every Fleet run and removed after it.
# Fleet 1.48 bundles a rust-analyzer (2025-01-28) older than the stable toolchain's proc-macro
# server, which puts a "Failed spawning proc-macro server" toast over the editor's top right
# corner; the fixture uses only built-in derives, so expansion is not needed. Fleet reads
# `rust-analyzer.*` from the workspace's settings, not from the user's.
FLEET_WORKSPACE_SETTINGS = {"rust-analyzer.procMacro.enable": False}


def macinput(*args, check=True):
    src = os.path.join(HERE, "macinput.swift")
    if not os.path.exists(MACINPUT) or os.path.getmtime(MACINPUT) < os.path.getmtime(src):
        os.makedirs(CACHE, exist_ok=True)
        subprocess.run(["swiftc", "-O", "-o", MACINPUT, src], check=True)
    r = subprocess.run([MACINPUT, *map(str, args)], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise SystemExit(f"macinput {' '.join(map(str, args))}: {r.stderr.strip()}")
    return r.stdout.strip()


# --- chords ---------------------------------------------------------------------------------

def mac_chords():
    """Windows chord -> mac chord, from the keymap's `key`/`mac` pairs."""
    with open(os.path.join(REPO, "crates", "weep", "assets", "keymap", "default.json")) as fh:
        rows = json.load(fh)["keymap"]
    table = {}
    for row in rows:
        key, mac = row.get("key"), row.get("mac")
        if key and mac and key not in table:
            table[key] = mac
    return table


CHORDS = None


def to_mac(chord):
    global CHORDS
    if CHORDS is None:
        CHORDS = mac_chords()
    spelled = chord.lower().replace("+", "-")
    return CHORDS.get(spelled, spelled)


# --- windows --------------------------------------------------------------------------------

def descendants(pid):
    out, todo = {pid}, [pid]
    while todo:
        p = todo.pop()
        r = subprocess.run(["pgrep", "-P", str(p)], capture_output=True, text=True)
        for c in r.stdout.split():
            if int(c) not in out:
                out.add(int(c))
                todo.append(int(c))
    return out


def windows(pids):
    return [w for w in json.loads(macinput("windows", *pids) or "[]") if w["layer"] == 0]


def wait_for_window(pid, timeout, min_size=(400, 300)):
    deadline = time.time() + timeout
    while time.time() < deadline:
        best = None
        for w in windows(sorted(descendants(pid))):
            if w["w"] < min_size[0] or w["h"] < min_size[1]:
                continue
            if best is None or w["w"] * w["h"] > best["w"] * best["h"]:
                best = w
        if best:
            return best
        time.sleep(0.25)
    raise SystemExit(f"no window appeared for pid {pid} within {timeout}s")


def refresh(s):
    """The launched window's current id and bounds (they move when it is placed)."""
    for w in windows([s["pid"]]):
        if w["id"] == s["window"]:
            s["bounds"] = [w["x"], w["y"], w["w"], w["h"]]
            return s
    w = wait_for_window(s["pid"], 10)
    s["window"], s["bounds"] = w["id"], [w["x"], w["y"], w["w"], w["h"]]
    return s


def place(s, size, pos):
    macinput("place", s["pid"], pos[0], pos[1], size[0], size[1])
    time.sleep(0.4)
    return refresh(s)


def focus(s):
    macinput("focus", s["pid"])


def guard(s):
    """Refuse to send input unless the launched editor is the frontmost app."""
    focus(s)
    front = int(macinput("front") or 0)
    if front not in descendants(s["pid"]) and front != s["pid"]:
        raise SystemExit(f"the launched editor (pid {s['pid']}) is not in front (pid {front}); not sending input")


def at(s, x, y):
    """Window-relative points to global points."""
    return s["bounds"][0] + float(x), s["bounds"][1] + float(y)


def shot(s, out):
    focus(s)
    time.sleep(0.3)
    refresh(s)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    subprocess.run(["screencapture", "-x", "-o", f"-l{s['window']}", out], check=True)
    return out


# --- launching ------------------------------------------------------------------------------

def find_weep():
    for c in [os.environ.get("WEEP_EXE"),
              os.path.join(os.environ.get("CARGO_TARGET_DIR", ""), "release", "weep"),
              os.path.expanduser("~/Library/Caches/weep-target/release/weep"),
              os.path.join(REPO, "target", "release", "weep")]:
        if c and os.path.isfile(c):
            return c
    raise SystemExit("weep not found: build with `cargo build --release -p weep` or set WEEP_EXE")


def launch(editor, workspace, size, pos, extra):
    env = dict(os.environ)
    if editor == "fleet":
        if fleet_pids():
            raise SystemExit("Fleet is already running (a single instance): quit it, or use `attach`")
        cmd, timeout = [FLEET, workspace], 120
    else:
        cmd, timeout = [find_weep(), workspace, *extra], 30
        config = os.path.join(CACHE, "weep-config")
        shutil.rmtree(config, ignore_errors=True)
        os.makedirs(config, exist_ok=True)
        env["WEEP_CONFIG_DIR"] = config
        os.environ["WEEP_CONFIG_DIR"] = config
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
    w = wait_for_window(proc.pid, timeout)
    s = {"editor": editor, "pid": w["pid"], "launcher_pid": proc.pid, "window": w["id"],
         "bounds": [w["x"], w["y"], w["w"], w["h"]], "pointer": None}
    time.sleep(1.0)
    focus(s)
    place(s, size, pos)
    # Park the pointer on the empty middle of the status bar: over the text, Smart Mode's
    # hover card would open in either editor and leak into the shots.
    macinput("move", *at(s, size[0] / 2, size[1] - 8))
    return s


def fleet_pids():
    r = subprocess.run(["pgrep", "-f", FLEET], capture_output=True, text=True)
    return [int(p) for p in r.stdout.split()]


def close(s, timeout=15):
    if s["editor"] == "fleet":
        # Closing the window leaves Fleet running with an empty one, and the next launch would
        # hand its workspace to that instance: quit it.
        for pid in fleet_pids():
            subprocess.run(["kill", str(pid)], capture_output=True)
        deadline = time.time() + 30
        while time.time() < deadline and fleet_pids():
            time.sleep(0.3)
        return
    macinput("close", s["pid"], check=False)
    deadline = time.time() + timeout
    while time.time() < deadline and any(w["id"] == s["window"] for w in windows([s["pid"]])):
        time.sleep(0.2)
    if s["editor"] == "weep":
        subprocess.run(["kill", str(s["launcher_pid"])], capture_output=True)


def with_fleet_settings(values):
    for path in FLEET_SETTINGS:
        backup = path + ".weep-probe"
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if os.path.exists(path) and not os.path.exists(backup):
            os.replace(path, backup)
        elif not os.path.exists(path):
            open(backup + ".absent", "w").close()
        with open(path, "w") as fh:
            json.dump(values, fh, indent=2)


def restore_fleet_settings():
    for path in FLEET_SETTINGS:
        backup = path + ".weep-probe"
        if os.path.exists(backup + ".absent"):
            os.remove(backup + ".absent")
            if os.path.exists(path):
                os.remove(path)
        elif os.path.exists(backup):
            os.replace(backup, path)


def with_workspace_settings(workspace):
    """Write FLEET_WORKSPACE_SETTINGS unless the workspace has its own file; returns the path to
    remove after the run, or None."""
    path = os.path.join(workspace, ".fleet", "settings.json")
    if os.path.exists(path):
        return None
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        json.dump(FLEET_WORKSPACE_SETTINGS, fh, indent=2)
    return path


# --- scenes ---------------------------------------------------------------------------------

INPUT_STEPS = ("key", "type", "move", "click", "drag", "hover", "mousedown", "mouseup", "wheel")


def steps_for(scene, editor):
    steps = scene.get("steps", {})
    return steps.get(f"{editor}_mac", steps.get(editor, steps.get("both", [])))


def do(s, step, out_dir, prefix):
    op, *a = step
    if op in INPUT_STEPS:
        guard(s)
    pointer = s.get("pointer") or at(s, s["bounds"][2] / 2, s["bounds"][3] - 8)
    if op == "key":
        for chord in a:
            macinput("key", to_mac(chord))
            time.sleep(0.05)
    elif op == "type":
        macinput("type", a[0])
    elif op in ("move", "hover"):
        s["pointer"] = at(s, a[0], a[1])
        macinput("move", *s["pointer"])
        if op == "hover":
            time.sleep(float(a[2]) / 1000)
    elif op == "click":
        # ["click", kind, mods]: mods such as "alt" or "alt+shift" held for the click.
        kind = a[0] if a else "left"
        macinput("click", kind, *a[1:2], *pointer)
    elif op == "drag":
        x1, y1 = at(s, a[0], a[1])
        x2, y2 = at(s, a[2], a[3])
        macinput("move", x1, y1)
        macinput("down", "left", x1, y1)
        for k in range(1, 11):
            macinput("drag", x1, y1, x1 + (x2 - x1) * k / 10, y1 + (y2 - y1) * k / 10)
        macinput("up", "left", x2, y2)
        s["pointer"] = (x2, y2)
    elif op == "mousedown":
        macinput("down", "left", *pointer)
    elif op == "mouseup":
        macinput("up", "left", *pointer)
    elif op == "wheel":
        # As winprobe: one notch per unit, positive scrolls up; a notch is three lines.
        macinput("wheel", *pointer, int(a[0]) * 3)
    elif op == "weep-settings":
        config = os.environ.get("WEEP_CONFIG_DIR")
        if not config:
            raise SystemExit("weep-settings needs a WEEP launch (WEEP_CONFIG_DIR)")
        with open(os.path.join(config, "settings.json"), "w") as fh:
            json.dump(a[0], fh)
    elif op == "wait":
        time.sleep(float(a[0]) / 1000)
    elif op == "shot":
        out = os.path.join(out_dir, f"{prefix}-{a[0]}.png")
        print(shot(s, out))
    elif op in ("maximize", "restore", "place", "focus"):
        if op == "focus":
            focus(s)
        else:
            print(f"(skipped on macOS: {op})", file=sys.stderr)
    else:
        raise SystemExit(f"unknown step {step!r}")
    if op in ("key", "type", "click", "drag", "wheel", "mousedown", "mouseup"):
        time.sleep(0.05)


def load(scene_path):
    with open(scene_path) as fh:
        scene = json.load(fh)
    return scene, os.path.splitext(os.path.basename(scene_path))[0]


def run_scene(scene_path, editor, workspace, out_dir):
    scene, name = load(scene_path)
    size = tuple(scene.get("size", [1280, 800]))
    pos = tuple(scene.get("pos", [100, 100]))
    extra = [a.replace("{workspace}", workspace) for a in scene.get(f"{editor}_args", [])]
    if editor == "fleet" and "fleet_settings" in scene:
        with_fleet_settings(scene["fleet_settings"])
    written = with_workspace_settings(workspace) if editor == "fleet" else None
    try:
        s = launch(editor, workspace, size, pos, extra)
        time.sleep(scene.get("settle_ms", 4000) / 1000)
        focus(s)
        try:
            for step in steps_for(scene, editor):
                do(s, step, out_dir, f"{name}-{editor}")
        finally:
            if scene.get("close", True):
                close(s)
    finally:
        if editor == "fleet" and "fleet_settings" in scene:
            restore_fleet_settings()
        if written:
            os.remove(written)


def attach_scene(scene_path, editor, pid, out_dir):
    scene, name = load(scene_path)
    size = tuple(scene.get("size", [1280, 800]))
    pos = tuple(scene.get("pos", [100, 100]))
    w = wait_for_window(pid, 10)
    s = {"editor": editor, "pid": w["pid"], "launcher_pid": pid, "window": w["id"],
         "bounds": [w["x"], w["y"], w["w"], w["h"]], "pointer": None}
    if editor == "fleet" and "fleet_settings" in scene:
        with_fleet_settings(scene["fleet_settings"])
    try:
        focus(s)
        place(s, size, pos)
        macinput("move", *at(s, size[0] / 2, size[1] - 8))
        time.sleep(1.5)
        for step in steps_for(scene, editor):
            do(s, step, out_dir, f"{name}-{editor}")
    finally:
        if editor == "fleet" and "fleet_settings" in scene:
            restore_fleet_settings()


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    cmd = argv[0]
    if cmd == "check":
        ok = macinput("trusted") == "1"
        print("input allowed" if ok else "no Accessibility permission for this terminal")
        return 0 if ok else 1
    if macinput("trusted") != "1":
        raise SystemExit("no Accessibility permission for this terminal: System Settings > "
                         "Privacy & Security > Accessibility")
    if cmd == "run" and len(argv) == 5:
        run_scene(argv[1], argv[2], os.path.abspath(argv[3]), argv[4])
        return 0
    if cmd == "attach" and len(argv) == 5:
        attach_scene(argv[1], argv[2], int(argv[3]), argv[4])
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
