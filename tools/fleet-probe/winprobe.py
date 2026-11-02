#!/usr/bin/env python3
"""Windows driver for the probe: launch an editor, place its window, drive it, shoot it.

The Windows counterpart of the KWin/ydotool route in docs/probe-plan.md (harnesses B, C's
wrapper and H). One script, one state file, so each step is a command:

    winprobe.py launch fleet <workspace> [--size 1280x800] [--pos 100,100]
    winprobe.py launch oak <workspace> [--size 1280x800] [--pos 100,100] [-- <extra oak args>]
    winprobe.py launch exe <path> [args...]        any other program; the window is found by pid
    winprobe.py windows                            list top-level windows: pid, class, title, rects
    winprobe.py place [--size WxH] [--pos X,Y]     move the launched window (logical px)
    winprobe.py focus                              bring the launched window to the front
    winprobe.py key <chord> [<chord> ...]          e.g. ctrl+shift+f  escape  down  alt+1
    winprobe.py type <text>                        type unicode text
    winprobe.py move <x> <y>                       pointer to window-relative logical coordinates
    winprobe.py click [left|right|double]
    winprobe.py drag <x1> <y1> <x2> <y2>
    winprobe.py hover <x> <y> <ms>                 move, then wait (tooltips)
    winprobe.py wait <ms>
    winprobe.py shot <out.png> [--frame]           client area (or whole frame) of the launched window
    winprobe.py pixel <x> <y>                      colour at a window-relative logical coordinate
    winprobe.py close                              WM_CLOSE, then wait for exit
    winprobe.py run <scene.json> <editor> <workspace> <out dir>
                                                   launch, run the scene's steps for that editor, close

The launched window is remembered in %LOCALAPPDATA%/oak-probe/current.json (pid, hwnd, scale),
so the commands above can run one at a time from a shell. Sizes and coordinates are logical
pixels at the window's monitor scale (150% here); screenshots are device pixels.

Fleet is found through FLEET_HOME, then %LOCALAPPDATA%/Programs/Fleet, then the newest
%USERPROFILE%/Downloads/Fleet-*/. Oak is OAK_EXE, else <repo>/target/release/oak.exe or
$CARGO_TARGET_DIR/release/oak.exe.

Needs Pillow for `shot` and `pixel`; everything else is ctypes against user32.
"""
import ctypes
import ctypes.wintypes as wt
import glob
import json
import os
import subprocess
import sys
import time

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi", use_last_error=True)
shcore = ctypes.WinDLL("shcore", use_last_error=True)

# Per-monitor DPI awareness: every rect user32 hands back is in device pixels.
DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = ctypes.c_void_p(-4)
user32.SetProcessDpiAwarenessContext.argtypes = [ctypes.c_void_p]
user32.SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)

STATE = os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), "oak-probe", "current.json")
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

WNDENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [WNDENUMPROC, wt.LPARAM]
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.GetClientRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.ClientToScreen.argtypes = [wt.HWND, ctypes.POINTER(wt.POINT)]
user32.GetDpiForWindow.argtypes = [wt.HWND]
user32.GetDpiForWindow.restype = wt.UINT
user32.SetWindowPos.argtypes = [wt.HWND, wt.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wt.UINT]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsWindow.argtypes = [wt.HWND]
user32.GetWindowLongW.argtypes = [wt.HWND, ctypes.c_int]
user32.GetWindowLongW.restype = ctypes.c_long
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.GetForegroundWindow.restype = wt.HWND
user32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
user32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
kernel32.GetCurrentThreadId.restype = wt.DWORD
dwmapi.DwmGetWindowAttribute.argtypes = [wt.HWND, wt.DWORD, ctypes.c_void_p, wt.DWORD]

GWL_STYLE, GWL_EXSTYLE = -16, -20
WS_CAPTION, WS_EX_TOOLWINDOW = 0x00C00000, 0x00000080
SWP_NOZORDER, SWP_NOACTIVATE = 0x0004, 0x0010
SW_RESTORE = 9
WM_CLOSE = 0x0010
DWMWA_EXTENDED_FRAME_BOUNDS = 9


def rect_tuple(r):
    return (r.left, r.top, r.right, r.bottom)


def window_rect(hwnd):
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return rect_tuple(r)


def frame_rect(hwnd):
    """The visible frame (what DWM draws), without the invisible resize border."""
    r = wt.RECT()
    if dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, ctypes.byref(r), ctypes.sizeof(r)) == 0:
        return rect_tuple(r)
    return window_rect(hwnd)


def client_rect(hwnd):
    r = wt.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(r))
    p = wt.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(p))
    return (p.x, p.y, p.x + r.right, p.y + r.bottom)


def text_of(fn, hwnd):
    buf = ctypes.create_unicode_buffer(512)
    fn(hwnd, buf, 512)
    return buf.value


def top_windows():
    out = []

    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        pid = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        style = user32.GetWindowLongW(hwnd, GWL_EXSTYLE)
        out.append({
            "hwnd": hwnd, "pid": pid.value,
            "class": text_of(user32.GetClassNameW, hwnd), "title": text_of(user32.GetWindowTextW, hwnd),
            "rect": window_rect(hwnd), "tool": bool(style & WS_EX_TOOLWINDOW),
        })
        return True

    user32.EnumWindows(WNDENUMPROC(cb), 0)
    return out


def descendants(pid):
    """pid plus every process below it (Fleet's launcher spawns the real frontend)."""
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Json -Compress"],
            capture_output=True, text=True, check=True).stdout
        rows = json.loads(out)
    except Exception:
        return {pid}
    children = {}
    for row in rows:
        children.setdefault(row["ParentProcessId"], []).append(row["ProcessId"])
    seen, todo = set(), [pid]
    while todo:
        p = todo.pop()
        if p in seen:
            continue
        seen.add(p)
        todo.extend(children.get(p, []))
    return seen


def wait_for_window(pid, timeout, min_size=(400, 300)):
    deadline = time.time() + timeout
    while time.time() < deadline:
        pids = descendants(pid)
        best = None
        for w in top_windows():
            if w["pid"] not in pids or w["tool"]:
                continue
            l, t, r, b = w["rect"]
            if r - l < min_size[0] or b - t < min_size[1]:
                continue
            if best is None or (r - l) * (b - t) > best[0]:
                best = ((r - l) * (b - t), w)
        if best:
            return best[1]
        time.sleep(0.25)
    raise SystemExit(f"no window appeared for pid {pid} within {timeout}s")


def load_state():
    try:
        with open(STATE) as fh:
            s = json.load(fh)
    except FileNotFoundError:
        raise SystemExit("nothing launched yet (winprobe.py launch ...)")
    if not user32.IsWindow(s["hwnd"]):
        raise SystemExit(f"the launched window {s['hwnd']:#x} is gone")
    return s


def save_state(s):
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    with open(STATE, "w") as fh:
        json.dump(s, fh, indent=1)


def scale_of(hwnd):
    return user32.GetDpiForWindow(hwnd) / 96.0


# --- placing -------------------------------------------------------------------------------

def place(hwnd, size=None, pos=None):
    """Set the *client* area to `size` logical px and the frame's top-left to `pos`."""
    scale = scale_of(hwnd)
    user32.ShowWindow(hwnd, SW_RESTORE)
    fl, ft, fr, fb = frame_rect(hwnd)
    wl, wtp, wr, wb = window_rect(hwnd)
    cl, ct, cr, cb = client_rect(hwnd)
    # Non-client extents in device px, measured from the current window.
    left_nc, top_nc = cl - wl, ct - wtp
    right_nc, bottom_nc = wr - cr, wb - cb
    shadow_l, shadow_t = fl - wl, ft - wtp
    x = wl if pos is None else round(pos[0] * scale) - shadow_l
    y = wtp if pos is None else round(pos[1] * scale) - shadow_t
    if size is None:
        w, h = wr - wl, wb - wtp
    else:
        w = round(size[0] * scale) + left_nc + right_nc
        h = round(size[1] * scale) + top_nc + bottom_nc
    user32.SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE)
    time.sleep(0.2)
    return client_rect(hwnd)


def focus(hwnd):
    fg = user32.GetForegroundWindow()
    if fg == hwnd:
        return
    tid = user32.GetWindowThreadProcessId(fg, None) if fg else 0
    me = kernel32.GetCurrentThreadId()
    if tid:
        user32.AttachThreadInput(me, tid, True)
    # An Alt tap lets a background process take the foreground.
    send_key("alt")
    user32.SetForegroundWindow(hwnd)
    if tid:
        user32.AttachThreadInput(me, tid, False)
    time.sleep(0.15)


# --- input ---------------------------------------------------------------------------------

class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD),
                ("time", wt.DWORD), ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class _IU(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("u", _IU)]


INPUT_MOUSE, INPUT_KEYBOARD = 0, 1
KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE = 0x0001, 0x0002, 0x0004
MOUSEEVENTF_MOVE, MOUSEEVENTF_ABSOLUTE = 0x0001, 0x8000
MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP = 0x0002, 0x0004
MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP = 0x0008, 0x0010
MOUSEEVENTF_WHEEL = 0x0800
SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN = 78, 79, 76, 77

VK = {
    "ctrl": 0x11, "control": 0x11, "shift": 0x10, "alt": 0x12, "win": 0x5B, "meta": 0x5B,
    "enter": 0x0D, "return": 0x0D, "escape": 0x1B, "esc": 0x1B, "tab": 0x09, "space": 0x20,
    "backspace": 0x08, "delete": 0x2E, "del": 0x2E, "insert": 0x2D,
    "home": 0x24, "end": 0x23, "pageup": 0x21, "pagedown": 0x22,
    "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28,
    "minus": 0xBD, "equals": 0xBB, "plus": 0xBB, "comma": 0xBC, "period": 0xBE, "slash": 0xBF,
    "backquote": 0xC0, "grave": 0xC0, "semicolon": 0xBA, "quote": 0xDE,
    "openbracket": 0xDB, "closebracket": 0xDD, "backslash": 0xDC,
}
for _i in range(1, 25):
    VK[f"f{_i}"] = 0x6F + _i
EXTENDED = {0x2D, 0x2E, 0x24, 0x23, 0x21, 0x22, 0x25, 0x26, 0x27, 0x28}


def vk_of(name):
    n = name.lower()
    if n in VK:
        return VK[n]
    if len(n) == 1 and (n.isalnum()):
        return ord(n.upper())
    raise SystemExit(f"unknown key {name!r}")


def send_inputs(items):
    arr = (INPUT * len(items))(*items)
    sent = user32.SendInput(len(items), arr, ctypes.sizeof(INPUT))
    if sent != len(items):
        raise SystemExit(f"SendInput sent {sent}/{len(items)}: error {ctypes.get_last_error()}")


def key_input(vk, up=False):
    flags = (KEYEVENTF_EXTENDEDKEY if vk in EXTENDED else 0) | (KEYEVENTF_KEYUP if up else 0)
    scan = user32.MapVirtualKeyW(vk, 0)
    i = INPUT(type=INPUT_KEYBOARD)
    i.u.ki = KEYBDINPUT(vk, scan, flags, 0, None)
    return i


def send_key(chord):
    parts = [p for p in chord.replace("-", "+").split("+") if p] if chord not in ("+", "-") else [chord]
    vks = [vk_of(p) for p in parts]
    send_inputs([key_input(v) for v in vks])
    time.sleep(0.03)
    send_inputs([key_input(v, up=True) for v in reversed(vks)])
    time.sleep(0.05)


def send_text(text):
    items = []
    for ch in text:
        units = ch.encode("utf-16-le")
        for code in [int.from_bytes(units[i:i + 2], "little") for i in range(0, len(units), 2)]:
            down = INPUT(type=INPUT_KEYBOARD)
            down.u.ki = KEYBDINPUT(0, code, KEYEVENTF_UNICODE, 0, None)
            up = INPUT(type=INPUT_KEYBOARD)
            up.u.ki = KEYBDINPUT(0, code, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, 0, None)
            items += [down, up]
    for n in range(0, len(items), 32):
        send_inputs(items[n:n + 32])
        time.sleep(0.02)


def to_device(s, x, y):
    cl, ct, _, _ = client_rect(s["hwnd"])
    return cl + round(x * s["scale"]), ct + round(y * s["scale"])


def mouse_move_abs(dx, dy):
    vx, vy = user32.GetSystemMetrics(SM_XVIRTUALSCREEN), user32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    vw, vh = user32.GetSystemMetrics(SM_CXVIRTUALSCREEN), user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    ax = round((dx - vx) * 65535 / (vw - 1))
    ay = round((dy - vy) * 65535 / (vh - 1))
    i = INPUT(type=INPUT_MOUSE)
    i.u.mi = MOUSEINPUT(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | 0x4000, 0, None)  # VIRTUALDESK
    send_inputs([i])
    time.sleep(0.03)


def mouse_button(down_flag, up_flag):
    for flag in (down_flag, up_flag):
        i = INPUT(type=INPUT_MOUSE)
        i.u.mi = MOUSEINPUT(0, 0, 0, flag, 0, None)
        send_inputs([i])
        time.sleep(0.03)


# --- shooting ------------------------------------------------------------------------------

def shot(s, out, frame=False):
    """Screen grab of the window's client area (or visible frame). The grab reads the screen,
    so the window is raised first; a shot never includes the pointer."""
    from PIL import ImageGrab
    focus(s["hwnd"])
    time.sleep(0.3)
    box = frame_rect(s["hwnd"]) if frame else client_rect(s["hwnd"])
    img = ImageGrab.grab(bbox=box, all_screens=True)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    img.save(out)
    return img.size


# --- launching -----------------------------------------------------------------------------

def find_fleet():
    for cand in [os.environ.get("FLEET_HOME"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Programs", "Fleet")]:
        if cand and os.path.isfile(os.path.join(cand, "Fleet.exe")):
            return os.path.join(cand, "Fleet.exe")
    downloads = sorted(glob.glob(os.path.join(os.path.expanduser("~"), "Downloads", "Fleet-*", "Fleet.exe")))
    if downloads:
        return downloads[-1]
    raise SystemExit("Fleet.exe not found: set FLEET_HOME")


def find_oak():
    cands = [os.environ.get("OAK_EXE"),
             os.path.join(os.environ.get("CARGO_TARGET_DIR", ""), "release", "oak.exe"),
             os.path.join(REPO, "target", "release", "oak.exe")]
    for c in cands:
        if c and os.path.isfile(c):
            return c
    raise SystemExit("oak.exe not found: build with `cargo build --release -p oak` or set OAK_EXE")


def launch(editor, args, size, pos, extra):
    if editor == "fleet":
        cmd = [find_fleet(), *args]
        timeout = 90
    elif editor == "oak":
        cmd = [find_oak(), *args, *extra]
        timeout = 30
    elif editor == "exe":
        cmd = [*args, *extra]
        timeout = 60
    else:
        raise SystemExit(f"unknown editor {editor!r}")
    proc = subprocess.Popen(cmd, cwd=os.path.dirname(cmd[0]) or None,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    w = wait_for_window(proc.pid, timeout)
    s = {"editor": editor, "pid": w["pid"], "launcher_pid": proc.pid, "hwnd": w["hwnd"],
         "class": w["class"], "title": w["title"], "scale": scale_of(w["hwnd"])}
    time.sleep(1.0)
    s["client"] = place(w["hwnd"], size, pos)
    save_state(s)
    print(json.dumps(s))
    return s


def close(s, timeout=15):
    user32.PostMessageW(s["hwnd"], WM_CLOSE, 0, 0)
    deadline = time.time() + timeout
    while time.time() < deadline and user32.IsWindow(s["hwnd"]):
        time.sleep(0.2)
    if user32.IsWindow(s["hwnd"]):
        subprocess.run(["taskkill", "/PID", str(s["pid"]), "/T", "/F"], capture_output=True)
    try:
        os.remove(STATE)
    except FileNotFoundError:
        pass


# --- scenes --------------------------------------------------------------------------------

def run_scene(scene_path, editor, workspace, out_dir):
    """A scene file (scenes/<name>.json) lists `steps` per editor in the driver's vocabulary:
    ["key", "ctrl+shift+f"], ["type", "text"], ["move", x, y], ["click"], ["drag", x1, y1, x2, y2],
    ["hover", x, y, ms], ["wait", ms], ["shot", "name"]. Shots go to <out dir>/<scene>-<editor>-<name>.png.
    """
    with open(scene_path) as fh:
        scene = json.load(fh)
    name = os.path.splitext(os.path.basename(scene_path))[0]
    size = tuple(scene.get("size", [1280, 800]))
    pos = tuple(scene.get("pos", [100, 100]))
    # `{workspace}` in an argument stands for the fixture's path, so a scene can name a file to open.
    extra = [a.replace("{workspace}", workspace) for a in scene.get(f"{editor}_args", [])]
    s = launch(editor, [workspace], size, pos, extra)
    time.sleep(scene.get("settle_ms", 4000) / 1000)
    focus(s["hwnd"])
    steps = scene.get("steps", {}).get(editor, scene.get("steps", {}).get("both", []))
    try:
        for step in steps:
            do(s, step, out_dir, f"{name}-{editor}")
    finally:
        if scene.get("close", True):
            close(s)


def do(s, step, out_dir=".", prefix="shot"):
    op, *a = step
    if op == "key":
        for chord in a:
            send_key(chord)
    elif op == "type":
        send_text(a[0])
    elif op == "move":
        mouse_move_abs(*to_device(s, float(a[0]), float(a[1])))
    elif op == "click":
        kind = a[0] if a else "left"
        if kind == "right":
            mouse_button(MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
        else:
            mouse_button(MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
            if kind == "double":
                time.sleep(0.05)
                mouse_button(MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    elif op == "drag":
        x1, y1, x2, y2 = map(float, a[:4])
        mouse_move_abs(*to_device(s, x1, y1))
        i = INPUT(type=INPUT_MOUSE)
        i.u.mi = MOUSEINPUT(0, 0, 0, MOUSEEVENTF_LEFTDOWN, 0, None)
        send_inputs([i])
        for k in range(1, 11):
            mouse_move_abs(*to_device(s, x1 + (x2 - x1) * k / 10, y1 + (y2 - y1) * k / 10))
        i.u.mi = MOUSEINPUT(0, 0, 0, MOUSEEVENTF_LEFTUP, 0, None)
        send_inputs([i])
    elif op == "hover":
        mouse_move_abs(*to_device(s, float(a[0]), float(a[1])))
        time.sleep(float(a[2]) / 1000)
    elif op == "wait":
        time.sleep(float(a[0]) / 1000)
    elif op == "shot":
        out = os.path.join(out_dir, f"{prefix}-{a[0]}.png")
        print(out, shot(s, out, frame="--frame" in a))
    elif op == "pixel":
        from PIL import ImageGrab
        dx, dy = to_device(s, float(a[0]), float(a[1]))
        print("#%02x%02x%02x" % ImageGrab.grab(bbox=(dx, dy, dx + 1, dy + 1), all_screens=True).getpixel((0, 0))[:3])
    elif op == "place":
        print(place(s["hwnd"], a[0] if a else None, a[1] if len(a) > 1 else None))
    elif op == "focus":
        focus(s["hwnd"])
    else:
        raise SystemExit(f"unknown step {step!r}")


# --- cli -----------------------------------------------------------------------------------

def parse_size(v):
    w, h = v.lower().split("x")
    return (float(w), float(h))


def parse_pos(v):
    x, y = v.split(",")
    return (float(x), float(y))


def main(argv):
    if not argv:
        sys.exit(__doc__)
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    cmd, args = argv[0], argv[1:]
    if cmd == "windows":
        for w in top_windows():
            if w["tool"]:
                continue
            print(f"{w['pid']:6} {w['hwnd']:#010x} {w['class']:<32} {w['rect']}  {w['title']}")
        return
    if cmd == "launch":
        size, pos, extra, rest = None, None, [], []
        it = iter(args)
        for a in it:
            if a == "--size":
                size = parse_size(next(it))
            elif a == "--pos":
                pos = parse_pos(next(it))
            elif a == "--":
                extra = list(it)
            else:
                rest.append(a)
        if not rest:
            sys.exit("launch fleet|oak|exe <workspace|path> ...")
        launch(rest[0], rest[1:], size or (1280., 800.), pos or (100., 100.), extra)
        return
    if cmd == "run":
        run_scene(args[0], args[1], args[2], args[3] if len(args) > 3 else "out")
        return
    s = load_state()
    if cmd == "place":
        size, pos = None, None
        it = iter(args)
        for a in it:
            if a == "--size":
                size = parse_size(next(it))
            elif a == "--pos":
                pos = parse_pos(next(it))
        s["client"] = place(s["hwnd"], size, pos)
        save_state(s)
        print(s["client"])
    elif cmd == "focus":
        focus(s["hwnd"])
    elif cmd == "close":
        close(s)
    elif cmd == "shot":
        out = args[0] if args else "shot.png"
        print(out, shot(s, out, frame="--frame" in args))
    elif cmd in ("key", "type", "move", "click", "drag", "hover", "wait", "pixel"):
        if cmd in ("key", "type", "move", "click", "drag", "hover"):
            focus(s["hwnd"])
        do(s, [cmd, *args])
    else:
        sys.exit(f"unknown command {cmd!r}\n{__doc__}")


if __name__ == "__main__":
    main(sys.argv[1:])
