#!/usr/bin/env python3
"""Drive the built palette through a PTY and assert on what the pane shows.

The unit tests reach every layer except the one where a pick becomes a running
command: `Screen` needs a real terminal, so nothing in-process can observe the
palette closing, the dispatch failing, and the palette coming back to report it.
That path is exactly where "I picked it and nothing happened" lives — the
message used to go to a stderr nobody reads, because the popup was already
gone — so it is checked here instead.

herdr is not needed. A stub stands in for it, which is the point: it can be made
to fail on demand, and a real herdr obliging by rejecting a valid entry is not
something a test can arrange.
"""

from __future__ import annotations

import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BINARY = REPO / "target" / "debug" / "herdr-command-palette"

COLUMNS = 60
ROWS = 20

CONTEXT = '{"focused_pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1"}'

FAILURE = "pane cannot be moved into the tab it already occupies"

STUB = """#!/bin/sh
if [ "$1" = "--version" ]; then echo "herdr 0.8.2"; exit 0; fi
if [ "$1" = "plugin" ]; then echo '{"result":{"actions":[]}}'; exit 0; fi
%s
"""

# Two stubs, one per verdict. Each answers in JSON because the palette reads the
# body rather than the exit status, which alone cannot name a failure.
REJECTS = STUB % ("""echo '{"error":{"message":"%s"}}'\nexit 1""" % FAILURE)
ACCEPTS = STUB % """echo '{"result":{"type":"ok"}}'\nexit 0"""


def write_stub(path: Path, body: str) -> Path:
    path.write_text(body)
    path.chmod(0o755)
    return path


def run_palette(stub: Path, keys: bytes, settle: float = 2.0) -> tuple[str, int]:
    """Presses `keys` and returns what the pane drew afterwards, plus the exit
    status. Reading stops on EOF, so a palette that exits is not waited out."""
    env = dict(
        os.environ,
        HERDR_BIN_PATH=str(stub),
        HERDR_PLUGIN_ROOT=str(REPO),
        HERDR_PLUGIN_ID="command-palette",
        HERDR_PLUGIN_CONTEXT_JSON=CONTEXT,
        TERM="xterm-256color",
    )

    # Off the pty because the distinction under test is drawn-in-the-pane vs
    # merely-printed, and a shared pty makes the second read as the first.
    errlog = stub.with_suffix(".stderr")
    pid, fd = pty.fork()
    if pid == 0:
        fileno = os.open(str(errlog), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        os.dup2(fileno, 2)
        os.execve(str(BINARY), [str(BINARY)], env)

    # Sized because a pty.fork() terminal starts at 0x0 and ratatui draws
    # nothing into an empty area — the palette would appear to render nothing.
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0))
    os.set_blocking(fd, False)

    def drain(seconds: float) -> str:
        out = b""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        return out.decode(errors="replace")

    drain(1.0)
    try:
        os.write(fd, keys)
    except OSError:
        # Swallowed because the palette having already exited is itself a
        # result: what it drew is still read below, and asserted on there.
        pass
    painted = drain(settle)

    try:
        os.write(fd, b"\x1b")
    except OSError:
        pass
    try:
        os.close(fd)
    except OSError:
        pass
    _, status = os.waitpid(pid, 0)
    return painted, os.waitstatus_to_exitcode(status)


def visible(painted: str) -> str:
    """The drawn text with the escape sequences positioning it removed, so an
    assertion can be made on what the user reads rather than on the cursor
    moves that put it there."""
    out = []
    i = 0
    while i < len(painted):
        if painted[i] == "\x1b":
            j = i + 1
            if j < len(painted) and painted[j] == "[":
                j += 1
                while j < len(painted) and not painted[j].isalpha():
                    j += 1
            i = j + 1
            continue
        out.append(painted[i])
        i += 1
    return "".join(out)


def check(name: str, ok: bool, detail: str) -> bool:
    print(f"{'ok  ' if ok else 'FAIL'} {name}", file=sys.stderr)
    if not ok:
        print(f"     {detail}", file=sys.stderr)
    return ok


def main() -> int:
    if not BINARY.is_file():
        print(f"no binary at {BINARY} — run `cargo build` first", file=sys.stderr)
        return 1

    scratch = REPO / "target" / "palette-e2e"
    scratch.mkdir(parents=True, exist_ok=True)
    rejects = write_stub(scratch / "herdr-rejects", REJECTS)
    accepts = write_stub(scratch / "herdr-accepts", ACCEPTS)

    passed = True

    painted, _ = run_palette(rejects, b"\r")
    text = visible(painted).replace("\n", " ")
    passed &= check(
        "a rejected command reports itself in the palette",
        FAILURE.split(" into ")[0] in text,
        f"drew: {text[-400:]!r}",
    )
    passed &= check(
        "the failure names the entry that produced it",
        "pane.split.right" in text,
        f"drew: {text[-400:]!r}",
    )

    # Paired with the two above so that keeping the palette up on a failure
    # cannot have quietly kept it up on success too.
    _, code = run_palette(accepts, b"\r", settle=1.0)
    passed &= check(
        "a command that runs closes the palette",
        code == 0,
        f"exit status {code}",
    )

    _, code = run_palette(accepts, b"\x1b", settle=1.0)
    passed &= check("esc closes the palette", code == 0, f"exit status {code}")

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
