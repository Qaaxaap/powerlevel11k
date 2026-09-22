#!/usr/bin/env python3
"""Record terminal sessions for the README images.

Two subcommands:

    record.py pty SCENE -o OUT.cast
        Run a command in a pty of a fixed size and record it as an asciicast v2
        file.  The scene is a JSON file; `$VAR` in its strings comes from the
        environment, so the scenes stay free of absolute paths:

            {"cols": 94, "rows": 22,
             "command": ["$P11K", "--shell", "zsh", "--config", "$DEMO_THEME"],
             "cwd": "$DEMO_HOME",
             "env": {"HOME": "$DEMO_HOME", "P11K_USER_ZSHRC": "$DEMO_HOME/.zshrc"},
             "tail": 1.5,
             "steps": [{"sleep": 1.4},
                       {"type": "cd dev/p11k", "delay": 0.055},
                       {"send": "\\r"},
                       {"mark": "a comment"},
                       {"sleep": 1.1}]}

        `type` writes one byte at a time with `delay` seconds in between, `send`
        writes its string at once, `sleep` waits, `mark` does nothing.

    record.py screen SCREEN -o OUT.cast --cols N --rows M
        Wrap a screen dump (`tmux capture-pane -e -p`, escape sequences and all)
        into a one-frame asciicast, for rendering a still image with agg.

Both are only meaningful because the engine draws the whole header itself: the
recorder never has to understand what it records.
"""

import argparse
import fcntl
import json
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time


def expand(value):
    if isinstance(value, str):
        return os.path.expandvars(value)
    if isinstance(value, list):
        return [expand(v) for v in value]
    if isinstance(value, dict):
        return {k: expand(v) for k, v in value.items()}
    return value


def write_cast(path, cols, rows, events, shell="unknown"):
    with open(path, "w") as fh:
        fh.write(
            json.dumps(
                {
                    "version": 2,
                    "width": cols,
                    "height": rows,
                    "timestamp": int(time.time()),
                    "env": {"SHELL": shell, "TERM": "xterm-256color"},
                }
            )
            + "\n"
        )
        for at, data in events:
            fh.write(json.dumps([round(at, 6), "o", data]) + "\n")


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def record_pty(scene, out_path):
    cols = scene.get("cols", 94)
    rows = scene.get("rows", 22)
    master, slave = pty.openpty()
    set_winsize(slave, rows, cols)
    set_winsize(master, rows, cols)

    env = {"PATH": "/usr/bin:/bin", "TERM": "xterm-256color", "LANG": "C.utf8"}
    env.update(expand(scene.get("env", {})))

    def preexec():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen(
        expand(scene["command"]),
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        cwd=expand(scene.get("cwd", os.getcwd())),
        preexec_fn=preexec,
        close_fds=True,
    )
    os.close(slave)

    start = time.monotonic()
    events = []

    def pump(timeout):
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return
            try:
                ready, _, _ = select.select([master], [], [], left)
            except InterruptedError:
                continue
            if not ready:
                return
            try:
                data = os.read(master, 65536)
            except OSError:
                return
            if not data:
                return
            events.append((time.monotonic() - start, data.decode("utf-8", "replace")))

    try:
        for step in scene["steps"]:
            if "mark" in step:
                continue
            if "sleep" in step:
                pump(step["sleep"])
            elif "type" in step:
                delay = step.get("delay", 0.045)
                for ch in expand(step["type"]):
                    pump(delay)
                    os.write(master, ch.encode())
            elif "send" in step:
                pump(step.get("sleep", 0.0))
                os.write(master, expand(step["send"]).encode())
            else:
                raise SystemExit("unknown step: %r" % (step,))
        pump(scene.get("tail", 2.0))
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        os.close(master)

    write_cast(out_path, cols, rows, events, scene["command"][-1])
    return events[-1][0] if events else 0.0


def record_screen(args):
    with open(args.screen) as fh:
        text = fh.read().rstrip("\n")
    # Home, clear, then the screen: the pane dump is a full repaint by definition, and one
    # frame is all a still image needs.
    events = [(0.0, "\x1b[2J\x1b[H" + text.replace("\n", "\r\n"))]
    write_cast(args.out, args.cols, args.rows, events)
    return 0.0


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="mode", required=True)

    pty_ap = sub.add_parser("pty")
    pty_ap.add_argument("scene")
    pty_ap.add_argument("-o", "--out", required=True)

    screen_ap = sub.add_parser("screen")
    screen_ap.add_argument("screen")
    screen_ap.add_argument("-o", "--out", required=True)
    screen_ap.add_argument("--cols", type=int, required=True)
    screen_ap.add_argument("--rows", type=int, required=True)

    args = ap.parse_args()
    if args.mode == "pty":
        with open(args.scene) as fh:
            scene = json.load(fh)
        duration = record_pty(scene, args.out)
    else:
        duration = record_screen(args)
    print("%s: %.2fs" % (args.out, duration))


if __name__ == "__main__":
    main()
