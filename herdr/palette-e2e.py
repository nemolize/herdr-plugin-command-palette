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
import signal
import struct
import sys
import tempfile
import termios
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BINARY = REPO / "target" / "debug" / "herdr-command-palette"

COLUMNS = 60
ROWS = 20

CONTEXT = '{"focused_pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1"}'

FAILURE = "pane cannot be moved into the tab it already occupies"

# Generous because they bound a hang, not a wait: every wait below returns as
# soon as its condition holds, so a fast machine never spends them.
READY_TIMEOUT = 30.0
OUTCOME_TIMEOUT = 30.0
EXIT_TIMEOUT = 10.0

# Drawn on the palette's first paint, so waiting for it rather than for a fixed
# interval is what keeps a loaded CI runner from reading as a failure.
READY_MARKER = "esc to close"

STUB = """#!/bin/sh
if [ "$1" = "--version" ]; then echo "herdr 0.8.2"; exit 0; fi
if [ "$1" = "plugin" ]; then echo '{"result":{"actions":[]}}'; exit 0; fi
echo "$@" >> "$HERDR_STUB_LOG"
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


class Palette:
    def __init__(self, stub: Path, log: Path, errlog: Path):
        env = dict(
            os.environ,
            HERDR_BIN_PATH=str(stub),
            HERDR_PLUGIN_ROOT=str(REPO),
            # Pinned into the scratch dir so a developer's own catalog and
            # ranking cannot change which entry this picks, or get written to.
            HERDR_PLUGIN_CONFIG_DIR=str(stub.parent / "config"),
            HERDR_PLUGIN_STATE_DIR=str(stub.parent / "state"),
            HERDR_PLUGIN_ID="command-palette",
            HERDR_PLUGIN_CONTEXT_JSON=CONTEXT,
            HERDR_STUB_LOG=str(log),
            TERM="xterm-256color",
        )

        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            # Off the pty because the distinction under test is drawn-in-the-pane
            # vs merely-printed, and a shared pty makes the second read as first.
            fileno = os.open(str(errlog), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            os.dup2(fileno, 2)
            os.execve(str(BINARY), [str(BINARY)], env)

        # Sized because a pty.fork() terminal starts at 0x0 and ratatui draws
        # nothing into an empty area — the palette would appear to render nothing.
        fcntl.ioctl(
            self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0)
        )
        os.set_blocking(self.fd, False)
        self.painted = ""
        self.reaped = False

    def _pump(self, budget: float) -> bool:
        ready, _, _ = select.select([self.fd], [], [], budget)
        if not ready:
            return True
        try:
            chunk = os.read(self.fd, 65536)
        except OSError:
            return False
        if not chunk:
            return False
        self.painted += chunk.decode(errors="replace")
        return True

    def wait_for(self, needle: str, timeout: float) -> bool:
        """Searches everything drawn so far, not the current screen — so every
        `needle` used here must be one that cannot appear before the state it
        is taken to signal."""
        deadline = time.monotonic() + timeout
        while True:
            if needle in visible(self.painted):
                return True
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return False
            if not self._pump(min(0.1, remaining)):
                return needle in visible(self.painted)

    def wait_for_exit(self, timeout: float) -> int | None:
        """The palette's exit status, or None if it was still up. A crash after
        the pick exits too, so the status is what separates the two."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.reaped = True
                return os.waitstatus_to_exitcode(status)
            self._pump(0.1)
        return None

    def send(self, keys: bytes) -> None:
        try:
            os.write(self.fd, keys)
        except OSError:
            # Swallowed because the palette having already exited is itself a
            # result: what it drew is still asserted on by the caller.
            pass

    def close(self) -> None:
        """Escalates instead of waiting, because a child ignoring SIGHUP would
        otherwise hold this until the CI job's own timeout killed the run."""
        try:
            os.close(self.fd)
        except OSError:
            pass
        if self.reaped:
            return
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                # The group, not the pid: `pty.fork` makes the child a session
                # leader, so a stub it spawned would outlive a bare kill.
                os.killpg(os.getpgid(self.pid), sig)
            except (ProcessLookupError, PermissionError):
                break
            deadline = time.monotonic() + 2.0
            while time.monotonic() < deadline:
                try:
                    pid, _ = os.waitpid(self.pid, os.WNOHANG)
                except ChildProcessError:
                    self.reaped = True
                    return
                if pid:
                    self.reaped = True
                    return
                time.sleep(0.05)


def check(name: str, ok: bool, detail: str) -> bool:
    print(f"{'ok  ' if ok else 'FAIL'} {name}", file=sys.stderr)
    if not ok:
        print(f"     {detail}", file=sys.stderr)
    return ok


def started(palette: Palette, name: str) -> bool:
    if palette.wait_for(READY_MARKER, READY_TIMEOUT):
        return True
    check(name, False, f"the palette never drew: {visible(palette.painted)[-300:]!r}")
    return False


def rejected_command_is_reported(scratch: Path) -> bool:
    stub = write_stub(scratch / "herdr-rejects", REJECTS)
    log = scratch / "rejects.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "rejects.stderr")
    name = "a rejected command reports itself in the palette"
    try:
        if not started(palette, name):
            return False
        palette.send(b"\r")
        # The tail, not the whole message: it lands on the row after the wrap,
        # so only the tail proves the wrapped remainder survived.
        seen = palette.wait_for("it already occupies", OUTCOME_TIMEOUT)
        text = visible(palette.painted).replace("\n", " ")
        # Compared without spaces because the wrap falls mid-message and the
        # row boundary is not part of what the assertion is about.
        squeezed = "".join(text.split())
        passed = check(
            name,
            seen and "".join(FAILURE.split()) in squeezed,
            f"drew: {text[-400:]!r}",
        )
        passed &= check(
            "the failure names the entry that produced it",
            "pane.split.right" in text,
            f"drew: {text[-400:]!r}",
        )
        passed &= check(
            "the rejected entry really reached the stub",
            "pane split" in log.read_text(),
            f"stub log: {log.read_text()!r}",
        )
        # Checked because printing the message and then exiting is exactly what
        # the old code did, and the accumulated output cannot tell the two apart.
        passed &= check(
            "the palette stays up holding the failure",
            palette.wait_for_exit(1.0) is None,
            "it exited after reporting, so the message was not left readable",
        )
        return passed
    finally:
        palette.close()


def accepted_command_closes_the_palette(scratch: Path) -> bool:
    stub = write_stub(scratch / "herdr-accepts", ACCEPTS)
    log = scratch / "accepts.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "accepts.stderr")
    name = "a command that runs closes the palette"
    try:
        if not started(palette, name):
            return False
        palette.send(b"\r")
        # No Esc follows: Esc closes the palette too, so a check that sent one
        # could not tell a successful pick from its own cleanup.
        code = palette.wait_for_exit(EXIT_TIMEOUT)
        passed = check(
            name,
            code == 0,
            "still up after the pick" if code is None else f"exited {code}",
        )
        passed &= check(
            "the accepted entry really reached the stub",
            "pane split" in log.read_text(),
            f"stub log: {log.read_text()!r}",
        )
        return passed
    finally:
        palette.close()


def esc_closes_the_palette(scratch: Path) -> bool:
    stub = write_stub(scratch / "herdr-esc", ACCEPTS)
    log = scratch / "esc.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "esc.stderr")
    name = "esc closes the palette"
    try:
        if not started(palette, name):
            return False
        palette.send(b"\x1b")
        code = palette.wait_for_exit(EXIT_TIMEOUT)
        passed = check(
            name,
            code == 0,
            "still up after esc" if code is None else f"exited {code}",
        )
        passed &= check(
            "esc dispatched nothing",
            log.read_text() == "",
            f"stub log: {log.read_text()!r}",
        )
        return passed
    finally:
        palette.close()


def main() -> int:
    if not BINARY.is_file():
        print(f"no binary at {BINARY} — run `cargo build` first", file=sys.stderr)
        return 1

    root = REPO / "target" / "palette-e2e"
    root.mkdir(parents=True, exist_ok=True)
    # Fresh per run, so two builds of the same checkout cannot overwrite each
    # other's stubs and logs while both are executing.
    scratch = Path(tempfile.mkdtemp(dir=root))

    passed = rejected_command_is_reported(scratch)
    passed &= accepted_command_closes_the_palette(scratch)
    passed &= esc_closes_the_palette(scratch)

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
