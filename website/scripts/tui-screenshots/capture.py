#!/usr/bin/env python3
"""Regenerate the terminal UI screenshots in website/src/assets/tui/.

macOS only. Run it from anywhere inside the repository:

    python3 website/scripts/tui-screenshots/capture.py

It builds `bacnet` (with sc-tls and serial) and the loopback lab in
lab/, starts the lab, and runs `bacnet tui` in a tmux pane. A NEW Terminal
window, opened just for this run, attaches to that pane; tmux sends the keys
and `screencapture -l` photographs that window only. The window is closed at
the end, also on failure or Ctrl-C. Your own Terminal windows and tabs are
not touched.

Safety:
  * Every socket is on 127.0.0.1: the lab binds loopback only and every
    profile the script writes binds 127.0.0.1.
  * MS/TP: the script never joins a trunk. Without --mstp-port the MS/TP
    shots are drawn from the TUI's insta snapshots (simulated stations).
    With --mstp-port it opens that port with `--mstp-monitor ... --passive`,
    which listens only and cannot join, and refuses to send `y` while a join
    confirmation is on screen.
  * BACNET_CONFIG_DIR is /tmp/bacnet-docs, a link to a fresh directory, so
    no shot shows your home directory or user name.

Options:
  --reuse              use the binaries already built, skip cargo
  --only A,B           take only these shots (names from SHOTS, no extension)
  --mstp-port PORT     live, receive-only MS/TP monitor shots on this port
  --mstp-baud BAUD     the trunk's baud rate (default 38400)
  --tmux-only DIR      no Terminal window: save each shot's text to DIR
  --terminal-profile N a Terminal settings set for the window's colours
  --font NAME          window font (default Menlo)
  --font-size PT       window font size (default 12)
  --width PX           scale shots down to at most this width (default 1440)
  --list               print the shot list and exit

Environment: CARGO_TARGET_DIR (default <repo>/target), TMUX_BIN.
Standard library only.
"""

import argparse
import json
import os
import plistlib
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
WEBSITE = os.path.join(REPO, "website")
ASSETS = os.path.join(WEBSITE, "src", "assets", "tui")
MANIFEST = os.path.join(ASSETS, "manifest.json")
SNAPSHOTS = os.path.join(REPO, "crates", "bacnet-cli", "src", "tui", "tests", "snapshots")
TARGET = os.environ.get("CARGO_TARGET_DIR", os.path.join(REPO, "target"))
BACNET = os.path.join(TARGET, "release", "bacnet")
LAB = os.path.join(TARGET, "release", "tui-lab")
TMUX = os.environ.get("TMUX_BIN") or shutil.which("tmux") or "/opt/homebrew/bin/tmux"
CWEBP = shutil.which("cwebp") or "/opt/homebrew/bin/cwebp"
# What every shot shows as the config root. A link, so a rerun shows the same text.
CONFIG_LINK = "/tmp/bacnet-docs"
SOCKET = "bacnet-docs-%d" % os.getpid()
MAIN = (120, 36)

# Every shot the pages use: name -> (size, alt text). The names are the
# files in src/assets/tui/ and stay stable across refreshes.
SHOTS = {
    "mstp-scan-stopped": ((80, 24), "Original-rate restoration failed; the scan evidence remains visible."),
    "mstp-scan-cancelled": ((80, 24), "Manual cancellation retains partial evidence and restores the original monitor."),
    "mstp-scan-partial": ((80, 24), "A failed candidate leaves partial evidence and no global best."),
    "mstp-scan-insufficient": ((80, 24), "No valid decoded frames: insufficient traffic for a baud suggestion."),
    "mstp-scan-ambiguous": ((80, 24), "Equal best ratios remain ambiguous across two rates."),
    "mstp-scan-result": ((120, 40), "A scripted 38400 baud suggestion, with the original 19200 baud monitor restored."),
    "mstp-scan-progress": ((80, 24), "A receive-only scan measuring the first of six rates."),
    "profiles-picker": (MAIN, "The profile picker listing four saved profiles, with keys to preview, create or reload them."),
    "profiles-new-bip": (MAIN, "The new B/IP profile form with its fields and their defaults."),
    "profiles-new-mstp": (MAIN, "The new MS/TP profile form: serial port, MAC, baud, Max_Master and Max_Info_Frames."),
    "profiles-preview-attest": (MAIN, "A B/IP profile preview in Foreign Device mode, asking for first-use authorization."),
    "profiles-passive": (MAIN, "A passive B/IP session: the status bar reads listening and PASSIVE, and the footer says foreign-device settings are ignored."),
    "discovery-whois-form": (MAIN, "The Who-Is form with its Scope, Target, Range and Listen fields."),
    "discovery-whois-warning": (MAIN, "The Who-Is form warning that a global Who-Is with no range reaches every device."),
    "discovery-devices": (MAIN, "The device table after a global Who-Is: two local devices and one routed device on network 2."),
    "discovery-filter": (MAIN, "The device table filtered to instances containing 100."),
    "discovery-who-has": (MAIN, "A Who-Has panel below the device table, listing the device that has an object named Damper."),
    "browser-device": (MAIN, "The device browser: the device card, the object list grouped by type, and an empty property pane."),
    "browser-status-flags": (MAIN, "An analog input in alarm, in fault and out of service, with its status badges."),
    "browser-priority-array": (MAIN, "A commandable analog output with its 16-slot priority array; priority 10 is in control."),
    "browser-trend-log": (MAIN, "A Trend Log object's properties in the browser."),
    "browser-schedule": (MAIN, "A Schedule object's properties in the browser."),
    "browser-object-filter": (MAIN, "The object list filtered to binary objects."),
    "writes-not-armed": (MAIN, "The footer refusing a write because no write session is armed."),
    "writes-arm-dialog": (MAIN, "The dialog that arms a write session, with a reason and a length in minutes."),
    "writes-form": (MAIN, "The write form for an analog output: property, value and priority."),
    "writes-preview-t1": (MAIN, "A T1 write preview at priority 12: the slot changes but Present_Value does not."),
    "writes-result": (MAIN, "The result of a write the device acknowledged; the status bar counts one ledger entry."),
    "writes-preview-t2": (MAIN, "A T2 write preview at priority 8, which needs y to send."),
    "writes-preview-t3": (MAIN, "A T3 write preview at priority 5, which needs the device instance typed to send."),
    "writes-blocked": (MAIN, "A write at priority 6, blocked because the profile does not allow dangerous writes."),
    "writes-relinquish": (MAIN, "The relinquish form, which writes NULL to one priority slot."),
    "writes-writem-preview": (MAIN, "A WritePropertyMultiple batch preview of two writes, confirmed at T2."),
    "writes-ledger": (MAIN, "The session ledger with one write and its undo value."),
    "writes-exit-guard": (MAIN, "The exit guard listing a change still in effect, with revert, keep and cancel."),
    "watch-list": (MAIN, "The watch list: COV and polled rows with values, units, status flags, age and sparklines."),
    "overview-log-pane": (MAIN, "The device browser with the log pane open below it."),
    "overview-help": (MAIN, "The help overlay: the global keys, then the current screen's keys, then the commands."),
    "network-fd-bdt": (MAIN, "The Network screen: the foreign-device panel and the BBMD's broadcast distribution table."),
    "network-fdt": (MAIN, "The Network screen's FDT tab with this client's own entry."),
    "network-matrix": (MAIN, "The BDT consistency matrix after a survey: both BBMDs agree."),
    "network-networks": (MAIN, "The Networks tab: one router reaches network 2."),
    "network-bbmd-unreachable": (MAIN, "The foreign-device panel when the BBMD never answers: not registered, attempts counting."),
    "sc-preview": (MAIN, "A BACnet/SC profile preview: hub URL, PEM file paths and first-use authorization."),
    "sc-profile-editor": (MAIN, "The BACnet/SC profile editor."),
    "sc-lint-warning": (MAIN, "An SC profile preview warning that the operational certificate expires within 30 days."),
    "sc-hub-certs": (MAIN, "The SC screen connected: hub connector, VMAC and UUID, and the hub's certificate chain."),
    "sc-doctor": (MAIN, "The connection doctor with every phase passed."),
    "sc-credentials": (MAIN, "My credentials: this node's certificate, the CA bundle, the key match and the issuer check."),
    "sc-devices": (MAIN, "The device table over BACnet/SC, with a VMAC as the address."),
    "sc-doctor-failure": (MAIN, "The connection doctor after a failed dial: TLS failed because the hub's CA is not trusted."),
    "mstp-monitor-stations": ((120, 40), "The MS/TP monitor's Stations view: an address map and a row per station heard."),
    "mstp-monitor-ring": ((120, 40), "The Token ring view: successors, rotation times and per-master token statistics."),
    "mstp-monitor-findings": ((80, 24), "The Findings view warning that one master's Max_Master leaves a station out."),
    "mstp-monitor-errors": ((80, 24), "The Errors view: CRC errors, noise, a lost token and the adapter's timing note."),
    "mstp-join-proposal": ((80, 24), "The safe-join proposal: a free MAC, the Max_Master to use and the free addresses."),
    "mstp-join-confirm": ((80, 24), "The join confirmation the safe join opens, naming the station and the port."),
    "mstp-participant-join": ((80, 24), "The confirmation shown before joining a trunk with --mstp."),
    "mstp-participant": ((120, 40), "The MS/TP screen as a participant: this station's token state and the transport counters."),
    "troubleshoot-narrow": ((80, 24), "The device browser at 80x24, where only the focused pane is shown."),
    "troubleshoot-too-small": ((70, 20), "The notice shown when the terminal is smaller than 80x24."),
}

# MS/TP shots drawn from the TUI's insta snapshots unless --mstp-port gives a
# live port. The last three always come from snapshots: they need stations to
# answer, or a join, which this script never makes.
SNAPSHOT_SHOTS = {
    "mstp-scan-stopped": "mstp_scan__mstp_scan_stopped_80x24",
    "mstp-scan-cancelled": "mstp_scan__mstp_scan_cancelled_80x24",
    "mstp-scan-partial": "mstp_scan__mstp_scan_partial_80x24",
    "mstp-scan-insufficient": "mstp_scan__mstp_scan_insufficient_80x24",
    "mstp-scan-ambiguous": "mstp_scan__mstp_scan_ambiguous_80x24",
    "mstp-scan-result": "mstp_scan__mstp_scan_result_120x40",
    "mstp-scan-progress": "mstp_scan__mstp_scan_progress_80x24",
    "mstp-monitor-stations": "mstp_monitor__monitor_stations_120x40",
    "mstp-monitor-ring": "mstp_monitor__monitor_ring_120x40",
    "mstp-monitor-findings": "mstp_monitor__monitor_findings_80x24",
    "mstp-monitor-errors": "mstp_monitor__monitor_errors_80x24",
    "mstp-join-proposal": "mstp_monitor__monitor_proposal_80x24",
    "mstp-join-confirm": "mstp_monitor__monitor_join_80x24",
    "mstp-participant-join": "mstp_screen__mstp_join_80x24",
    "mstp-participant": "mstp_screen__mstp_panel_120x40",
}
LIVE_MSTP = ["mstp-monitor-stations", "mstp-monitor-ring", "mstp-monitor-findings", "mstp-monitor-errors"]


class Failure(Exception):
    pass


def log(msg):
    print("[capture] " + msg, file=sys.stderr, flush=True)


def run(cmd, check=True, **kw):
    res = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if check and res.returncode != 0:
        raise Failure("%s: %s" % (" ".join(cmd), (res.stderr or res.stdout).strip()))
    return res.stdout


# ---------------------------------------------------------------- build


def build():
    env = dict(os.environ, CARGO_TARGET_DIR=TARGET)
    log("building bacnet (sc-tls, serial)")
    subprocess.run(["cargo", "build", "--release", "--locked", "--manifest-path",
                    os.path.join(REPO, "Cargo.toml"), "-p", "bacnet-cli", "--features", "sc-tls,serial"],
                   check=True, env=env)
    log("building tui-lab")
    # The lab is its own workspace; start it from the workspace's lock so it
    # resolves the same versions. Its Cargo.lock is not committed.
    shutil.copyfile(os.path.join(REPO, "Cargo.lock"), os.path.join(HERE, "lab", "Cargo.lock"))
    subprocess.run(["cargo", "build", "--release", "--manifest-path", os.path.join(HERE, "lab", "Cargo.toml")],
                   check=True, env=env)


# ---------------------------------------------------------------- lab


class Lab:
    """The tui-lab process, on 127.0.0.1 only. Closing its stdin stops it."""

    def __init__(self, workdir):
        self.dir = os.path.join(workdir, "lab")
        os.makedirs(self.dir, exist_ok=True)
        self.err = open(os.path.join(self.dir, "lab.log"), "w")
        self.proc = subprocess.Popen([LAB, self.dir], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=self.err, text=True, start_new_session=True,
                                     env=dict(os.environ, RUST_LOG="warn"))
        self.env = {}
        deadline = time.time() + 30
        for line in self.proc.stdout:
            line = line.strip()
            if line == "READY" or time.time() > deadline:
                break
            if "=" in line:
                key, value = line.split("=", 1)
                self.env[key] = value
        if "LAB_BBMD_PORT" not in self.env:
            self.stop()
            raise Failure("the lab did not start; see %s/lab.log" % self.dir)
        log("lab up: %s" % self.env)

    def stop(self):
        for step in ("stdin", signal.SIGTERM, signal.SIGKILL):
            if self.proc.poll() is not None:
                break
            try:
                if step == "stdin":
                    self.proc.stdin.close()
                else:
                    os.killpg(self.proc.pid, step)
                self.proc.wait(timeout=8)
            except Exception:
                pass
        self.err.close()


def write_profiles(cfg, lab):
    """B/IP through the lab's BBMD and three SC variants. None is authorized
    yet, so the first-use attestation can be shot."""
    prof = os.path.join(cfg, "profiles")
    os.makedirs(prof, exist_ok=True)
    with open(os.path.join(prof, "lab.toml"), "w") as f:
        # A foreign device must not name its own address as the broadcast
        # address: a datagram to it would count as a broadcast.
        f.write('version = 1\ntransport = "bip"\n\n[bip]\ninterface = "127.0.0.1"\nport = 0\n'
                'broadcast = "127.255.255.255"\ntimeout_ms = 3000\nmin_interval_ms = 0\n'
                'ttl = 300\nbbmd = "127.0.0.1:%s"\n\n[safety]\nwrites = "session"\ndangerous = false\n'
                % lab.env["LAB_BBMD_PORT"])
    for name, ca, leaf in [("sc-lab", "ca.pem", "tui"), ("sc-expiring", "ca.pem", "tui-expiring"),
                           ("sc-untrusted", "other-ca.pem", "tui")]:
        with open(os.path.join(prof, name + ".toml"), "w") as f:
            f.write('version = 1\ntransport = "sc"\n\n[sc]\nhub_url = "%s"\nca = "%s"\ncert = "%s"\n'
                    'key = "%s"\ntimeout_ms = 3000\nmin_interval_ms = 0\n\n[safety]\nwrites = "session"\n'
                    'dangerous = false\n'
                    % (lab.env["LAB_HUB_URL"], os.path.join(lab.dir, ca), os.path.join(lab.dir, leaf + ".pem"),
                       os.path.join(lab.dir, leaf + ".key")))


# ---------------------------------------------------------------- tmux + Terminal


def tmux(*args, check=True):
    return run([TMUX, "-L", SOCKET] + list(args), check=check)


def osascript(script):
    return run(["osascript", "-e", script]).strip()



def read_bmp(path):
    """(width, height, px) for an uncompressed 24- or 32-bit BMP, as sips
    writes it; px(x, y) is an (r, g, b) tuple with y counted from the top."""
    import struct
    data = open(path, "rb").read()
    offset = struct.unpack_from("<I", data, 10)[0]
    width, height = struct.unpack_from("<ii", data, 18)
    bpp = struct.unpack_from("<H", data, 28)[0] // 8
    stride = (width * bpp + 3) & ~3
    flipped = height > 0
    height = abs(height)

    def px(x, y):
        row = (height - 1 - y) if flipped else y
        i = offset + row * stride + x * bpp
        return (data[i + 2], data[i + 1], data[i])

    return width, height, px



def find_pane(width, height, px, cols, rows, window_cells):
    """(top, left, bottom, right) in pixels of a cols x rows tmux pane drawn
    at the top left of a larger window whose spare area is block-filled, or
    None. See Window.crop_to_pane."""
    wcols, wrows = window_cells
    cw, ch = width / wcols, height / wrows
    # The fill colour: the commonest colour in a column near the right edge,
    # lower half (all fill there).
    x_fill = int(width - 2 * cw)
    counts = {}
    for y in range(height // 2, int(height - 2 * ch)):
        c = tuple(v // 16 for v in px(x_fill, y))
        counts[c] = counts.get(c, 0) + 1
    common = max(counts, key=counts.get)
    fill = next(px(x_fill, y) for y in range(height // 2, height)
                if tuple(v // 16 for v in px(x_fill, y)) == common)

    def is_fill(x, y):
        # Colour management moves values by a unit or two.
        return all(abs(a - b) <= 8 for a, b in zip(px(x, y), fill))
    # Cell height: the spacing of the stripes' top edges in that column.
    edges = [y for y in range(height // 2, int(height - 2 * ch))
             if is_fill(x_fill, y) and not is_fill(x_fill, y - 1)]
    if len(edges) < 3:
        return None
    # The mean spacing: rows are a fraction of a pixel over a whole number,
    # which adds up over a pane's height.
    cell_h = (edges[-1] - edges[0]) / (len(edges) - 1)

    def block_at(x, y):
        # A block cell: fill colour across most of the cell's width.
        return all(is_fill(int(x + f * cw), y) for f in (0.15, 0.5, 0.85))

    # First block row under the pane, from the top. Terminal pads its left
    # edge, so look a quarter of the way across (still under the pane).
    x_under = width * cols / wcols / 2
    fill_top = next((y for y in range(height) if block_at(x_under, y)
                     and block_at(x_under, min(height - 1, int(y + cell_h / 2)))), None)
    if fill_top is None:
        return None
    # A block glyph is shorter than its cell: the line gap is split above
    # and below it, so the cell starts half a gap above the glyph.
    run_len = 0
    while is_fill(x_fill, edges[0] + run_len):
        run_len += 1
    fill_top -= (cell_h - run_len) / 2
    bottom = fill_top - cell_h          # the border row sits between
    top = bottom - rows * cell_h
    # First block column on a pane row, from the right: one row well inside
    # the pane, at a height that lands on a block, not a gap.
    y = int(fill_top - cell_h * (rows // 2) + cell_h * 0.3)
    x = int(width - 2 * cw)
    while x > 0 and is_fill(x, y):
        x -= 1
    fill_left = x + 1
    # The left edge: where the fill starts on a block row under the pane
    # (Terminal pads its left edge).
    y_under = int(fill_top + cell_h * 1.4)
    left = 0
    while left < fill_left and not is_fill(left, y_under):
        left += 1
    cell_w = (fill_left - left) / (cols + 1)
    right = int(round(fill_left - cell_w))  # the border column sits between
    if top < 0 or right - left < cols * 4:
        return None
    return (int(round(top)), left, int(round(bottom)), right)


class Window:
    """The Terminal window this run opens, attached to the tmux session."""

    def __init__(self, opts):
        self.opts = opts
        attach = "cd /tmp && exec %s -L %s attach -t docs" % (shlex.quote(TMUX), SOCKET)
        # Modern Terminal also inherits working-directory and active-process
        # title components that its tab AppleScript flags do not control. Copy
        # the chosen settings into a unique, temporary profile; never mutate a
        # user's profile or default. close() removes only this owned profile.
        self.profile = "bacnet-docs-%d" % os.getpid()
        prefs = plistlib.loads(run(["defaults", "export", "com.apple.Terminal", "-"]).encode())
        base = opts.terminal_profile or prefs.get("Default Window Settings", "Basic")
        settings = dict(prefs["Window Settings"][base])
        settings["name"] = self.profile
        settings["WindowTitle"] = "bacnet"
        for key in ("ShowComponentsWhenTabHasCustomTitle", "ShowActiveProcessInTitle",
                    "ShowActiveProcessArgumentsInTitle", "ShowRepresentedURLInTitle",
                    "ShowRepresentedURLPathInTitle", "ShowShellCommandInTitle",
                    "ShowWindowSettingsNameInTitle", "ShowTTYNameInTitle", "ShowDimensionsInTitle"):
            settings[key] = False
        # An imported profile must never inherit a user's startup command.
        settings.pop("CommandString", None)
        settings.pop("RunCommandAsShell", None)
        self.id = None
        try:
            with tempfile.TemporaryDirectory(prefix="bacnet-capture-") as directory:
                # Terminal names an imported profile from the filename.
                profile = os.path.join(directory, self.profile + ".terminal")
                with open(profile, "wb") as stream:
                    stream.write(plistlib.dumps(settings))
                run(["open", "-a", "Terminal", profile])
                deadline = time.time() + 5
                while True:
                    found = osascript(
                        'tell application "Terminal"\n'
                        '  repeat with w in (get id of every window)\n'
                        '    try\n'
                        '      if name of current settings of tab 1 of window id w is "%s" then return w as integer\n'
                        '    end try\n'
                        '  end repeat\n'
                        'end tell' % self.profile).strip()
                    if found:
                        self.id = int(found)
                        break
                    if time.time() >= deadline:
                        raise Failure("the capture Terminal profile did not open a window")
                    time.sleep(0.1)
            osascript(
                'tell application "Terminal"\n'
                '  set t to tab 1 of window id %d\n'
                '  set custom title of t to "bacnet"\n'
                '  set title displays custom title of t to true\n'
                '  set font name of t to "%s"\n'
                '  set font size of t to %d\n'
                '  do script "%s" in t\n'
                'end tell' % (self.id, opts.font, opts.font_size, attach.replace('"', '\\"')))
        except BaseException:
            # Ctrl-C/SIGTERM during initialization also retire the owned profile.
            self.close()
            raise
        log("Terminal window %d" % self.id)

    def tab(self):
        return "tab 1 of window id %d" % self.id

    def resize(self, cols, rows):
        """Size the window to the pane. A full-screen Terminal opens new
        windows as full-screen tabs that can't be resized; then tmux keeps
        the pane at exactly cols x rows in the top-left corner (window-size
        manual, the rest filled with solid blocks) and each shot is cropped
        to it."""
        self.size = (cols, rows)
        if getattr(self, "crop", False):
            return
        osascript('tell application "Terminal"\n  set number of columns of %s to %d\n'
                  '  set number of rows of %s to %d\nend tell' % (self.tab(), cols, self.tab(), rows))
        deadline = time.time() + 5
        got = ""
        while time.time() < deadline:
            got = self.cells()
            if got == (cols, rows):
                return
            time.sleep(0.2)
        if got and got[0] >= cols and got[1] >= rows:
            log("Terminal keeps its window at %dx%d (full screen?): cropping each shot to %dx%d"
                % (got[0], got[1], cols, rows))
            self.start_cropping()
            return
        raise Failure("the Terminal window did not take %dx%d (it is %s); is the screen big enough?"
                      % (cols, rows, got))

    def start_cropping(self):
        """Crop each shot from now on. tmux draws the block fill around the
        pane only when the window is resized after the client grew, so
        resize it to its own size once."""
        self.crop = True
        tmux("resize-window", "-t", "docs", "-x", str(self.size[0]), "-y", str(self.size[1]))
        time.sleep(0.5)

    def cells(self):
        got = osascript('tell application "Terminal" to get {number of columns, number of rows} of %s'
                        % self.tab())
        try:
            c, r = (int(x) for x in got.replace(" ", "").split(","))
            return (c, r)
        except ValueError:
            return ()

    def crop_to_pane(self, path):
        """Crop a full-window shot to the pane's cells.

        The pane sits at the top left. tmux draws a border one cell wide
        right of and below it, and fills the rest with solid blocks. Blocks
        join across a row but leave a gap under each row, so the fill reads
        as stripes whose period is the cell height. The pane ends one cell
        before the first row and column of blocks."""
        cols, rows = self.size
        bmp = path + ".bmp"
        run(["sips", "-s", "format", "bmp", path, "--out", bmp])
        width, height, px = read_bmp(bmp)
        os.unlink(bmp)
        window_cells = self.cells()
        box = find_pane(width, height, px, cols, rows, window_cells)
        if not box:
            raise Failure("could not find the %dx%d pane in %s" % (cols, rows, path))
        top, left, bottom, right = box
        # The pane takes cols of the window's columns; a box much wider means
        # the fill wasn't found (not drawn yet, or another colour).
        cell_w = width / window_cells[0]
        if abs((right - left) - cols * cell_w) > 2 * cell_w:
            raise Failure("found a %d px wide box for the %dx%d pane in %s, expected about %d px"
                          % (right - left, cols, rows, path, cols * cell_w))
        run(["sips", "-c", str(bottom - top), str(right - left), "--cropOffset", str(top), str(left), path])

    def shoot(self, path):
        """Shoot the window to `path` (.webp): capture as PNG, crop when the
        window is full screen, scale, then encode WebP (sharp text at about
        a twentieth of the PNG's size)."""
        png = path[:-len(".webp")] + ".capture.png"
        if not getattr(self, "crop", False):
            # Terminal can enter full screen after resize() checked the
            # size; then the window is bigger than the pane.
            got = self.cells()
            if got and got != self.size and got[0] >= self.size[0] and got[1] >= self.size[1]:
                log("Terminal window is now %dx%d (full screen?): cropping each shot to the pane"
                    % got)
                self.start_cropping()
        run(["screencapture", "-x", "-o", "-l", str(self.id), "-t", "png", png])
        try:
            if getattr(self, "crop", False):
                self.crop_to_pane(png)
            width = int(re.search(r"pixelWidth: (\d+)", run(["sips", "-g", "pixelWidth", png])).group(1))
            if width > self.opts.width:
                run(["sips", "--resampleWidth", str(self.opts.width), png])
            run([CWEBP, "-quiet", "-q", "92", "-m", "6", "-metadata", "none", png, "-o", path])
        finally:
            if os.path.exists(png):
                os.unlink(png)

    def close(self):
        try:
            # Match only the profile created by this process, including an
            # imported window if initialization failed before its ID was read.
            osascript('tell application "Terminal"\n'
                      '  repeat with w in (get id of every window)\n'
                      '    try\n'
                      '      if name of current settings of tab 1 of window id w is "%s" then close (window id w) saving no\n'
                      '    end try\n'
                      '  end repeat\n'
                      '  if exists settings set "%s" then delete settings set "%s"\n'
                      'end tell' % (self.profile, self.profile, self.profile))
        except Failure as e:
            log("could not close capture window/profile %s: %s" % (self.profile, e))


class Pane:
    """The one tmux pane every shot is taken from."""

    def __init__(self, shooter):
        self.shooter = shooter
        self.keys = []
        self.command = ""
        # The last shot taken in this scenario, for timeout messages
        # (Shooter.step prefixes the scenario's name).
        self.last_shot = ""

    def text(self):
        return tmux("capture-pane", "-p", "-J", "-t", "docs")

    def alive(self):
        return tmux("display-message", "-p", "-t", "docs", "#{pane_dead}").strip() == "0"

    def start(self, argv, size=MAIN):
        if "--mstp" in argv or "--mstp-mac" in argv:
            raise Failure("refusing to start an MS/TP participant: %s" % argv)
        self.shooter.size(*size)
        env = ["-e", "BACNET_CONFIG_DIR=" + CONFIG_LINK, "-e", "COLORTERM=truecolor", "-e", "RUST_LOG="]
        cmd = "unset NO_COLOR; exec %s %s" % (shlex.quote(BACNET), " ".join(shlex.quote(a) for a in argv))
        tmux("respawn-pane", "-k", "-t", "docs", "-c", "/tmp", *env, cmd)
        self.command = "bacnet " + " ".join(argv)
        self.keys = []

    def show_text(self, path, size):
        """Show a snapshot's text, without a cursor, until the next start."""
        self.shooter.size(*size)
        cmd = "printf '\\033[?25l\\033[2J\\033[H'; cat %s; exec sleep 86400" % shlex.quote(path)
        tmux("respawn-pane", "-k", "-t", "docs", cmd)
        self.command = "(insta snapshot)"
        self.keys = []

    def send(self, *keys, literal=False, gap=0.12):
        for key in keys:
            if not literal and key == "y" and "Join the MS/TP trunk" in self.text():
                raise Failure("refusing to send y to an MS/TP join confirmation")
            tmux("send-keys", "-t", "docs", *(["-l", key] if literal else [key]))
            self.keys.append('"%s"' % key if literal else key)
            time.sleep(gap)

    def type(self, text):
        self.send(text, literal=True)

    def wait(self, pattern, timeout=15.0, absent=False):
        rx = re.compile(pattern, re.M)
        deadline = time.time() + timeout
        while True:
            text = self.text()
            if bool(rx.search(text)) != absent:
                return text
            if time.time() > deadline or not self.alive():
                where = "after %s" % self.last_shot if self.last_shot else "before the first shot"
                raise Failure("%s %r%s (%s)\n--- pane ---\n%s"
                              % ("waited %.0f s for" % timeout if self.alive() else "the TUI exited waiting for",
                                 pattern, " to go" if absent else "", where, text))
            time.sleep(0.1)

    def settle(self, quiet=0.4, limit=2.5):
        """Until the pane stops changing (clocks tick each second, so give up
        after `limit`)."""
        end = time.time() + limit
        last, since = self.text(), time.time()
        while time.time() < end:
            time.sleep(0.1)
            now = self.text()
            if now != last:
                last, since = now, time.time()
            elif time.time() - since >= quiet:
                return

    def shot(self, name, wait=None, timeout=15.0, source="live"):
        if wait:
            self.wait(wait, timeout)
        self.settle()
        self.shooter.save(name, self, source)
        self.keys = []
        self.last_shot = name


class Shooter:
    def __init__(self, opts):
        self.opts = opts
        self.only = set(filter(None, opts.only.split(",")))
        self.window = None
        self.current = None
        self.taken = {}
        self.problems = []

    def want(self, name):
        return not self.only or name in self.only

    def size(self, cols, rows):
        if self.current == (cols, rows):
            return
        tmux("resize-window", "-t", "docs", "-x", str(cols), "-y", str(rows))
        if self.window:
            self.window.resize(cols, rows)
        self.current = (cols, rows)

    def save(self, name, pane, source):
        if name not in SHOTS:
            raise Failure("%s is not in SHOTS" % name)
        if not self.want(name):
            return
        got = tmux("display-message", "-p", "-t", "docs", "#{pane_width}x#{pane_height}").strip()
        if got != "%dx%d" % self.current:
            raise Failure("%s: the pane is %s, not %dx%d" % (name, got, *self.current))
        if self.window:
            time.sleep(0.5)  # let Terminal draw the last frame
            self.window.shoot(os.path.join(ASSETS, name + ".webp"))
        else:
            with open(os.path.join(self.opts.tmux_only, name + ".txt"), "w") as f:
                f.write(pane.text())
        self.taken[name] = dict(file=name + ".webp", alt=SHOTS[name][1], size="%dx%d" % self.current,
                                source=source, command=pane.command, keys=" ".join(pane.keys))
        log("shot %s (%s)" % (name, source))

    def step(self, label, fn, pane, *args):
        pane.last_shot = ""
        try:
            fn(self, pane, *args)
        except Failure as e:
            log("FAILED %s: %s" % (label, e))
            self.problems.append("%s: %s" % (label, str(e).splitlines()[0]))


# ---------------------------------------------------------------- scenarios


def focus_objects(p):
    """Put the browser's focus on the Objects pane (its border turns cyan)."""
    for _ in range(4):
        raw = tmux("capture-pane", "-e", "-p", "-t", "docs")
        if re.search(r"\x1b\[38;5;6m┌ Objects \(", raw):
            return
        p.send("BTab")
    raise Failure("could not focus the Objects pane")


def select_object(p, label, tries=60):
    target = re.compile(r">\s+%s(\s|│|$)" % re.escape(label), re.M)
    focus_objects(p)
    p.send("g")
    for _ in range(tries):
        if target.search(p.text()):
            return
        p.send("j", gap=0.05)
    raise Failure("could not select %s" % label)


def open_object(p, label, expect):
    select_object(p, label)
    p.send("Enter")
    p.wait(expect)


def preview_delay():
    # Enter and y are ignored for 300 ms after a write preview opens.
    time.sleep(0.5)


def attest(p, profile):
    """Open `profile`'s preview and start it, authorizing it if this is its
    first use. When an earlier scenario stopped before its exit guard
    reverted, the session opens on that run's ledger: close it, so the
    following keys reach the screen."""
    text = p.wait(r"%s — resolved" % re.escape(profile))
    p.send("a" if "a attest" in text else "Enter")
    p.wait(r"\| (up|listening) \|")
    time.sleep(0.5)
    if "earlier session are still recorded" in p.text():
        p.send("Escape")
        p.wait(r"┌ Session ledger", absent=True)


def whois_global(p):
    """d, then a global Who-Is with no range, which asks for a second Enter."""
    p.send("d")
    p.wait(r"┌ Who-Is ")
    p.send("Right", "Enter")
    p.wait(r"Press Enter again")


def bip(s, p, lab):
    p.start(["tui"])
    p.shot("profiles-picker", wait=r"Choose a profile")
    p.send("n")
    p.shot("profiles-new-bip", wait=r"Create a B/IP profile")
    p.send("Escape")
    p.wait(r"Choose a profile")
    p.send("m")
    p.shot("profiles-new-mstp", wait=r"Create a MS/TP profile")
    p.send("Escape")
    p.wait(r"Choose a profile")
    p.send("Enter")  # lab sorts first
    p.shot("profiles-preview-attest", wait=r"lab — resolved")
    p.send("a")
    p.wait(r"No devices yet")

    p.send("d")
    p.shot("discovery-whois-form", wait=r"Scope")
    p.send("Right", "Enter")
    p.shot("discovery-whois-warning", wait=r"Press Enter again")
    p.send("Enter")
    p.shot("discovery-devices", wait=r"done, 3 replies", timeout=20)
    p.send("/")
    p.type("100")
    p.send("Enter")
    p.shot("discovery-filter", wait=r"filter: \"100\"")
    p.send("/", "Escape")
    p.wait(r"2001")
    p.send(":")
    p.type("find Damper")
    p.send("Enter")
    p.shot("discovery-who-has", wait=r"Who-Has \"Damper\" \| 1 found")
    p.send(":")
    p.type("find")
    p.send("Enter")
    p.wait(r"Who-Has", absent=True)

    p.send("g", "Enter")
    p.shot("browser-device", wait=r"Objects \(\d+\)")
    open_object(p, "analog-input 1", r"Zone Temp")
    p.shot("browser-status-flags", wait=r"\[OOS\]")
    open_object(p, "analog-output 1", r"Priority array")
    p.shot("browser-priority-array")
    open_object(p, "trend-log 1", r"Outside Air Log")
    p.shot("browser-trend-log")
    open_object(p, "schedule 1", r"Occupancy Schedule")
    p.shot("browser-schedule")
    select_object(p, "analog-input 1")
    p.send("/")
    p.type("binary")
    p.shot("browser-object-filter", wait=r"filter: \"binary\"")
    p.send("Escape")

    open_object(p, "analog-output 1", r"Priority array")
    p.send("w")
    p.shot("writes-not-armed", wait=r"press W to arm")
    p.send("W")
    p.wait(r"Arm a write session")
    p.type("Docs lab")
    p.shot("writes-arm-dialog")
    p.send("Enter")
    p.wait(r"WRITE \d+:\d+")
    p.send("w")
    p.wait(r"Write: device 1001")
    p.type("65")
    p.send("Tab", "BSpace", "BSpace")
    p.type("12")
    p.shot("writes-form")
    p.send("Enter")
    p.shot("writes-preview-t1", wait=r"Confirm \(T1\)")
    preview_delay()
    p.send("Enter")
    p.shot("writes-result", wait=r"acknowledged")
    p.send("Enter")
    for priority, title, name in [("8", r"Confirm \(T2\)", "writes-preview-t2"),
                                  ("5", r"Confirm \(T3\)", "writes-preview-t3"),
                                  ("6", r"Confirm \(blocked\)", "writes-blocked")]:
        p.send("w")
        p.wait(r"Write: device 1001")
        p.type("70")
        p.send("Tab", "BSpace", "BSpace")
        p.type(priority)
        p.send("Enter")
        p.shot(name, wait=title)
        p.send("Escape")
        time.sleep(0.3)
        if re.search(r"┌ (Result|Confirm)", p.text()):
            p.send("Escape")
    p.send("n")
    p.shot("writes-relinquish", wait=r"Relinquish: device 1001")
    p.send("Escape")
    p.send(":")
    p.type("writem 1001 av:1 present-value 72@12 av:2 present-value 68@12")
    p.send("Enter")
    p.shot("writes-writem-preview", wait=r"One WritePropertyMultiple")
    p.send("Escape")
    time.sleep(0.3)
    if re.search(r"┌ (Result|Confirm)", p.text()):
        p.send("Escape")
    p.send("u")
    p.shot("writes-ledger", wait=r"Session ledger")
    p.send("Escape")

    p.send("a")  # watch the open object, AO:1
    # Every lab object here takes COV but the Schedule, which has no COV
    # criteria: its subscription is refused, so that row is polled.
    for cmd in ["watch 1001 ai:2", "watch 1001 ai:3", "watch 1001 ai:1", "watch 2:2001 ai:1",
                "watch 1002 av:7", "watch 1001 bo:1", "watch 1001 schedule:1"]:
        p.send(":")
        p.type(cmd)
        p.send("Enter")
    p.send("3")
    p.wait(r"Watch \(8\)")
    time.sleep(8)  # let the sparklines fill in
    # The Source column: a polled row, and no row still starting.
    p.wait(r"schedule 1 .* polled ")
    p.wait(r"^│.* start ", absent=True)
    p.shot("watch-list")
    p.send("2", "L")
    p.shot("overview-log-pane", wait=r"Log \(L to hide\)")
    p.send("L")

    p.send("4")
    p.shot("network-fd-bdt", wait=r"\[BDT\]")
    p.send("Tab")
    p.shot("network-fdt", wait=r"\[FDT\]")
    p.send("Tab", "m")
    p.shot("network-matrix", wait=r"BDTs agree|MISS")
    p.send("Tab", "r")
    p.shot("network-networks", wait=r"1 router")

    p.send("1", "?")
    p.shot("overview-help", wait=r"Help \(")
    p.send("Escape")
    p.send("q")
    p.shot("writes-exit-guard", wait=r"Changes still in effect")
    p.send("r")  # revert everything, then quit
    deadline = time.time() + 15
    while p.alive() and time.time() < deadline:
        time.sleep(0.2)


def bip_states(s, p, lab):
    p.start(["tui", "--profile", "lab", "--passive"])
    attest(p, "lab")
    p.shot("profiles-passive", wait=r"PASSIVE")

    # 127.0.0.1:9 is the discard port: a BBMD that never answers.
    p.start(["-i", "127.0.0.1", "-p", "0", "-b", "127.255.255.255", "--foreign-bbmd", "127.0.0.1:9",
             "tui", "--no-profile"])
    p.wait(r"bacnet \|")
    time.sleep(6)
    p.send("4")
    p.shot("network-bbmd-unreachable", wait=r"not registered")

    p.start(["tui", "--profile", "lab", "--passive"], size=(70, 20))
    p.shot("troubleshoot-too-small", wait=r"Terminal too small")

    p.start(["tui", "--profile", "lab"], size=(80, 24))
    attest(p, "lab")
    p.wait(r"No devices yet")
    whois_global(p)
    p.send("Enter")
    p.wait(r"done, 3 replies", timeout=20)
    p.send("g", "Enter")
    p.wait(r"Objects \(\d+\)")
    select_object_narrow(p, "analog-output 1")
    p.send("Enter")
    p.shot("troubleshoot-narrow", wait=r"Damper")
    p.send("q")


def select_object_narrow(p, label):
    """Below 120 columns only the focused pane shows; Tab to Objects."""
    for _ in range(4):
        if "┌ Objects (" in p.text():
            break
        p.send("Tab")
    target = re.compile(r">\s+%s(\s|│|$)" % re.escape(label), re.M)
    p.send("g")
    for _ in range(60):
        if target.search(p.text()):
            return
        p.send("j", gap=0.05)
    raise Failure("could not select %s" % label)


def sc(s, p, lab):
    p.start(["tui", "--profile", "sc-lab"])
    p.shot("sc-preview", wait=r"sc-lab — resolved")
    p.send("e")
    p.shot("sc-profile-editor", wait=r"Hub URL")
    p.send("Escape")
    p.wait(r"sc-lab — resolved")
    p.send("a")
    p.wait(r"\| up \|")
    p.send("5")
    p.shot("sc-hub-certs", wait=r"State\s+connected")
    p.send("Tab")
    p.shot("sc-doctor", wait=r"every phase passed")
    p.send("Tab")
    p.shot("sc-credentials", wait=r"\[My credentials\]")
    for attempt in range(2):
        # A local Who-Is with no range: the form asks for a second Enter.
        p.send("1", "d")
        p.wait(r"┌ Who-Is ")
        p.send("Enter")
        p.wait(r"Press Enter again")
        p.send("Enter")
        try:
            p.shot("sc-devices", wait=r"done, 1 repl", timeout=10)
            break
        except Failure:
            if attempt:
                raise
            s.problems.append("sc: the first Who-Is got no reply; asked again")
    p.send("x")
    p.wait(r"disconnected")

    p.start(["tui", "--profile", "sc-expiring"])
    p.shot("sc-lint-warning", wait=r"sc-expiring — resolved")

    p.start(["tui", "--profile", "sc-untrusted"])
    p.wait(r"sc-untrusted — resolved")
    p.send("y")
    time.sleep(0.4)
    p.send("a")
    p.wait(r"\| failed \|")
    p.send("5")
    p.shot("sc-doctor-failure", wait=r"\[FAIL\]")


def port_holders(port):
    return run(["lsof", "-t", port], check=False).split()


def mstp_live(s, p, port, baud):
    """Receive only: --mstp-monitor with --passive can't join, and Pane.send
    refuses y on a join confirmation."""
    if port_holders(port):
        raise Failure("%s is held by pid %s; not touching it" % (port, port_holders(port)))
    try:
        p.start(["tui", "--mstp-monitor", port, "--mstp-baud", baud, "--passive"], size=(120, 40))
        p.wait(r"listen only")
        time.sleep(6)
        p.shot("mstp-monitor-stations", wait=r"\[Stations\]")
        p.send("Tab")
        p.shot("mstp-monitor-ring", wait=r"\[Token ring\]")
        s.size(80, 24)
        p.send("Tab")
        p.shot("mstp-monitor-findings", wait=r"\[Findings")
        p.send("Tab")
        p.shot("mstp-monitor-errors", wait=r"\[Errors\]")
        p.send("q")
        time.sleep(1)
    finally:
        p.start(["--version"])
        time.sleep(0.5)
        if port_holders(port):
            s.problems.append("%s still held after the MS/TP monitor" % port)


def snapshot_text(stem):
    path = os.path.join(SNAPSHOTS, "bacnet__tui__tests__%s.snap" % stem)
    with open(path) as f:
        body = f.read().split("---\n", 2)[-1]
    rows = [json.loads(line) for line in (l.strip() for l in body.splitlines())
            if line.startswith('"') and line.endswith('"')]
    w, h = (int(x) for x in re.search(r"_(\d+)x(\d+)$", stem).groups())
    if len(rows) != h or any(len(r) != w for r in rows):
        raise Failure("snapshot %s is not %dx%d" % (stem, w, h))
    return rows, (w, h)


def mstp_snapshots(s, p, workdir, skip):
    for name, stem in SNAPSHOT_SHOTS.items():
        if name in skip or not s.want(name):
            continue
        rows, size = snapshot_text(stem)
        if tuple(SHOTS[name][0]) != size:
            raise Failure("%s: SHOTS says %s, the snapshot is %s" % (name, SHOTS[name][0], size))
        path = os.path.join(workdir, stem + ".txt")
        with open(path, "w") as f:
            f.write("\n".join(rows))  # no final newline: it would scroll a line
        p.show_text(path, size)
        p.wait(re.escape(rows[0].strip()[:20]))
        p.shot(name, source="snapshot")


# ---------------------------------------------------------------- main


def link_config():
    """/tmp/bacnet-docs -> a fresh directory under /tmp."""
    real = tempfile.mkdtemp(prefix="bacnet-docs-", dir="/tmp")
    if os.path.islink(CONFIG_LINK):
        os.unlink(CONFIG_LINK)
    elif os.path.exists(CONFIG_LINK):
        shutil.rmtree(real)
        raise Failure("%s exists and is not a link; move it away first" % CONFIG_LINK)
    os.symlink(real, CONFIG_LINK)
    return real


def write_manifest(s):
    try:
        with open(MANIFEST) as f:
            old = {shot["name"]: shot for shot in json.load(f)["shots"]}
    except (OSError, ValueError, KeyError):
        old = {}
    commit = run(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], check=False).strip()
    shots = []
    for name, (size, alt) in SHOTS.items():
        shot = old.get(name, dict(name=name, file=name + ".webp", alt=alt, size="%dx%d" % size,
                                  source="placeholder", command="", keys=""))
        if name in s.taken:
            shot = dict(name=name, **s.taken[name], commit=commit)
        shot["alt"] = alt
        shots.append(shot)
    with open(MANIFEST, "w") as f:
        json.dump(dict(generator="website/scripts/tui-screenshots/capture.py",
                       font="%s %d" % (s.opts.font, s.opts.font_size), shots=shots), f, indent=1)
        f.write("\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--reuse", action="store_true")
    ap.add_argument("--only", default="")
    ap.add_argument("--mstp-port", default="")
    ap.add_argument("--mstp-baud", default="38400")
    ap.add_argument("--tmux-only", default="")
    ap.add_argument("--terminal-profile", default="")
    ap.add_argument("--font", default="Menlo")
    ap.add_argument("--font-size", type=int, default=12)
    ap.add_argument("--width", type=int, default=1440)
    ap.add_argument("--list", action="store_true")
    opts = ap.parse_args()
    if opts.list:
        for name, (size, alt) in SHOTS.items():
            print("%-26s %3dx%-3d %s" % (name, size[0], size[1], alt))
        return 0
    unknown = set(filter(None, opts.only.split(","))) - set(SHOTS)
    if unknown:
        ap.error("unknown shot(s): %s" % ", ".join(sorted(unknown)))
    if sys.platform != "darwin" and not opts.tmux_only:
        ap.error("Terminal screenshots need macOS; --tmux-only runs elsewhere")
    if not os.path.exists(TMUX):
        ap.error("tmux not found; set TMUX_BIN")
    if opts.tmux_only:
        os.makedirs(opts.tmux_only, exist_ok=True)
        opts.tmux_only = os.path.abspath(opts.tmux_only)
    if not opts.reuse:
        build()
    for binary in (BACNET, LAB):
        if not os.access(binary, os.X_OK):
            ap.error("%s is missing; run without --reuse" % binary)

    # SIGTERM and SIGHUP unwind through the finally blocks below.
    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda signum, _frame: sys.exit(128 + signum))
    s = Shooter(opts)
    p = Pane(s)
    real = link_config()
    lab = None
    conf = os.path.join(real, "tmux.conf")
    with open(conf, "w") as f:
        f.write("set -g status off\nset -g default-terminal tmux-256color\nset -g escape-time 0\n"
                "set -g window-size manual\nset -g set-titles off\nset -g remain-on-exit on\n"
                "set -g fill-character \"\u2588\"\n")
    try:
        tmux("-f", conf, "new-session", "-d", "-s", "docs", "-x", str(MAIN[0]), "-y", str(MAIN[1]),
             "sleep 86400")
        s.current = MAIN
        # Through the link, so the SC profiles' PEM paths read /tmp/bacnet-docs/lab/.
        lab = Lab(CONFIG_LINK)
        write_profiles(CONFIG_LINK, lab)
        if not opts.tmux_only:
            if not os.access(CWEBP, os.X_OK):
                raise Failure("cwebp is missing; install it with `brew install webp`")
            s.window = Window(opts)
            s.window.resize(*MAIN)
            time.sleep(1.0)
        # SC first: the lab's SC device drops its hub connection after about
        # five idle minutes and does not redial, so its Who-Is reply in the
        # sc scenario depends on running soon after the lab starts.
        s.step("sc", sc, p, lab)
        s.step("bip", bip, p, lab)
        s.step("bip states", bip_states, p, lab)
        live = []
        if opts.mstp_port:
            s.step("mstp monitor", mstp_live, p, opts.mstp_port, opts.mstp_baud)
            live = [n for n in LIVE_MSTP if n in s.taken]
        s.step("mstp snapshots", mstp_snapshots, p, real, live)
    finally:
        tmux("kill-server", check=False)
        if s.window:
            time.sleep(0.5)
            s.window.close()
        if lab:
            lab.stop()
        if os.path.islink(CONFIG_LINK) and os.path.realpath(CONFIG_LINK) == os.path.realpath(real):
            os.unlink(CONFIG_LINK)
        shutil.rmtree(real, ignore_errors=True)
    if not opts.tmux_only:
        write_manifest(s)
    missing = [n for n in SHOTS if s.want(n) and n not in s.taken]
    log("%d shots, %d problems" % (len(s.taken), len(s.problems) + bool(missing)))
    for problem in s.problems:
        log("problem: " + problem)
    if missing:
        log("not taken: " + ", ".join(missing))
    return 1 if s.problems or missing else 0


if __name__ == "__main__":
    sys.exit(main())
