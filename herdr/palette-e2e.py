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
import tomllib
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

# The shipped catalog's own pin, so the "older than the catalog" footer note —
# which takes the place of READY_MARKER — stays out of every test but its own.
STUB_VERSION = tomllib.loads((REPO / "herdr" / "catalog.toml").read_text())[
    "checked_against"
]

STUB = """#!/bin/sh
if [ "$1" = "--version" ]; then echo "herdr %s"; exit 0; fi""" % STUB_VERSION + """
if [ "$1" = "--default-config" ]; then exit 0; fi
if [ "$1" = "plugin" ]; then echo '{"result":{"actions":[]}}'; exit 0; fi
echo "$@" >> "$HERDR_STUB_LOG"
%s
"""

# One stub per verdict. Each answers in JSON because the palette reads the body
# rather than the exit status, which alone cannot name a failure. A failure's
# envelope goes to stderr, where herdr 0.9 writes it.
REJECTS = STUB % ("""echo '{"error":{"message":"%s"}}' >&2\nexit 1""" % FAILURE)
ACCEPTS = STUB % """echo '{"result":{"type":"ok"}}'\nexit 0"""

# Everything but the listing succeeds, so the palette still opens and the entry
# is still offered — the emptiness is met only after it is picked.
NO_TABS = STUB % """if [ "$1" = "tab" ] && [ "$2" = "list" ]; then
  echo '{"result":{"tabs":[]}}'
  exit 0
fi
echo '{"result":{"type":"ok"}}'
exit 0"""

LISTING_REFUSED = "tab list is not available on this session"

REJECTS_LISTING = STUB % (
    """if [ "$1" = "tab" ] && [ "$2" = "list" ]; then
  echo '{"error":{"message":"%s"}}' >&2
  exit 1
fi
echo '{"result":{"type":"ok"}}'
exit 0"""
    % LISTING_REFUSED
)

# `manual` is a `git worktree add` checkout opened as a plain workspace (w3):
# its row matches a herdr-created one, and only `workspace list` tells them apart.
WORKTREES = STUB % """if [ "$1" = "worktree" ] && [ "$2" = "list" ]; then
  echo '{"result":{"source":{"repo_root":"/src/repo"},"worktrees":[{"branch":"main","is_linked_worktree":false,"label":"repo","open_workspace_id":"w1","path":"/src/repo"},{"branch":"feat-x","is_linked_worktree":true,"label":"repo","path":"/wt/feat-x"},{"branch":"feat-y","is_linked_worktree":true,"label":"repo","open_workspace_id":"w2","path":"/wt/feat-y"},{"branch":"manual","is_linked_worktree":true,"label":"repo","open_workspace_id":"w3","path":"/src/manual"}]}}'
  exit 0
fi
if [ "$1" = "workspace" ] && [ "$2" = "list" ]; then
  echo '{"result":{"workspaces":[{"workspace_id":"w1","label":"repo","worktree":{"is_linked_worktree":false}},{"workspace_id":"w2","label":"feat-y","worktree":{"is_linked_worktree":true}},{"workspace_id":"w3","label":"manual"}]}}'
  exit 0
fi
echo '{"result":{"type":"ok"}}'
exit 0"""

NOT_GIT = "Herdr worktree actions require a workspace inside a Git work tree"

REJECTS_WORKTREE_LIST = STUB % (
    """if [ "$1" = "worktree" ] && [ "$2" = "list" ]; then
  echo '{"error":{"code":"not_git_worktree","message":"%s"}}' >&2
  exit 1
fi
echo '{"result":{"type":"ok"}}'
exit 0"""
    % NOT_GIT
)


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
    def __init__(
        self, stub: Path, log: Path, errlog: Path, config: Path | None = None
    ):
        env = dict(
            os.environ,
            HERDR_BIN_PATH=str(stub),
            HERDR_PLUGIN_ROOT=str(REPO),
            # Pinned into the scratch dir so a developer's own catalog and
            # ranking cannot change which entry this picks, or get written to.
            HERDR_PLUGIN_CONFIG_DIR=str(config or stub.parent / "config"),
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

    def wait_until_squeezed(self, needle: str, timeout: float) -> bool:
        """Matches with all whitespace removed, because ratatui writes a row
        cell by cell and a wrap can fall mid-message: the words of a status line
        are never reliably contiguous in what the pty carries. `needle` must
        therefore be given already squeezed."""
        deadline = time.monotonic() + timeout
        while True:
            if needle in "".join(visible(self.painted).split()):
                return True
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return False
            if not self._pump(min(0.1, remaining)):
                return needle in "".join(visible(self.painted).split())

    def wait_for_exit(self, timeout: float) -> int | None:
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
        # Read before the parent dies: once it is reaped its pid can be recycled,
        # and `getpgid` would then name some unrelated process's group.
        group = self._group()

        self._signal(group, signal.SIGTERM)
        self._reap(2.0)
        # Sent whether or not the parent went, because a descendant that ignored
        # SIGTERM is precisely the process that outlives it.
        self._signal(group, signal.SIGKILL)
        self._reap(2.0)

    def _group(self) -> int | None:
        try:
            return os.getpgid(self.pid)
        except (ProcessLookupError, PermissionError):
            return None

    def _signal(self, group: int | None, sig: int) -> None:
        if group is None:
            return
        try:
            os.killpg(group, sig)
        except (ProcessLookupError, PermissionError):
            pass

    def _reap(self, timeout: float) -> None:
        if self.reaped:
            return
        deadline = time.monotonic() + timeout
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
            "the failure shows herdr's message, not its envelope",
            '"error"' not in squeezed,
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


def an_empty_listing_is_reported(scratch: Path) -> bool:
    """Picking a `resolve` entry with nothing to pick from. The palette has no
    targets to show, so without a message it just sits there — the same
    nothing-happened the dispatch path was fixed for."""
    stub = write_stub(scratch / "herdr-no-tabs", NO_TABS)
    log = scratch / "no-tabs.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "no-tabs.stderr")
    name = "an empty target list is reported, not silent"
    try:
        if not started(palette, name):
            return False
        palette.send(b"Focus tab\r")
        seen = palette.wait_until_squeezed("nothingtopickfrom", OUTCOME_TIMEOUT)
        text = visible(palette.painted).replace("\n", " ")
        passed = check(name, seen, f"drew: {text[-400:]!r}")
        passed &= check(
            "the palette stays up after an empty listing",
            palette.wait_for_exit(1.0) is None,
            "it exited instead of letting the user pick something else",
        )
        return passed
    finally:
        palette.close()


def a_refused_listing_is_reported(scratch: Path) -> bool:
    """The other way a `resolve` entry fails before reaching a target: herdr
    refuses the listing. The message it gives is the only account of why."""
    stub = write_stub(scratch / "herdr-no-listing", REJECTS_LISTING)
    log = scratch / "no-listing.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "no-listing.stderr")
    name = "a refused target listing surfaces herdr's reason"
    try:
        if not started(palette, name):
            return False
        palette.send(b"Focus tab\r")
        seen = palette.wait_until_squeezed(
            "".join(LISTING_REFUSED.split()), OUTCOME_TIMEOUT
        )
        text = visible(palette.painted).replace("\n", " ")
        passed = check(name, seen, f"drew: {text[-400:]!r}")
        passed &= check(
            "the palette stays up after a refused listing",
            palette.wait_for_exit(1.0) is None,
            "it exited instead of letting the user pick something else",
        )
        return passed
    finally:
        palette.close()


def a_worktree_pick_reaches_the_context_repository(scratch: Path) -> bool:
    """`main` scopes the listing and fills `{repo}` from the context workspace,
    which neither the unit tests nor the catalog E2E reach."""
    stub = write_stub(scratch / "herdr-worktrees", WORKTREES)
    log = scratch / "worktrees.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "worktrees.stderr")
    name = "the worktree listing is scoped to the context workspace"
    try:
        if not started(palette, name):
            return False
        palette.send(b"Open worktree\r")
        seen = palette.wait_for("feat-x", OUTCOME_TIMEOUT)
        palette.send(b"feat-x\r")
        code = palette.wait_for_exit(EXIT_TIMEOUT)
        calls = log.read_text()
        passed = check(name, "worktree list --workspace w1" in calls, f"stub log: {calls!r}")
        passed &= check(
            "worktree candidates are labelled by branch",
            seen,
            f"drew: {visible(palette.painted)[-400:]!r}",
        )
        passed &= check(
            "the picked worktree opens from the repository root",
            "worktree open --cwd /src/repo --path /wt/feat-x --focus" in calls and code == 0,
            f"exit {code}, stub log: {calls!r}",
        )
        return passed
    finally:
        palette.close()


def removing_offers_only_worktrees_herdr_created(scratch: Path) -> bool:
    stub = write_stub(scratch / "herdr-remove", WORKTREES)
    log = scratch / "remove.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "remove.stderr")
    name = "remove lists the worktree herdr created"
    try:
        if not started(palette, name):
            return False
        palette.send(b"Remove worktree\r")
        seen = palette.wait_for("feat-y", OUTCOME_TIMEOUT)
        drawn = visible(palette.painted)
        palette.send(b"\r")
        code = palette.wait_for_exit(EXIT_TIMEOUT)
        calls = log.read_text()
        passed = check(name, seen, f"drew: {drawn[-400:]!r}")
        passed &= check(
            "remove leaves out a checkout herdr did not create",
            "manual" not in drawn.split("Remove worktree")[-1],
            f"drew: {drawn[-400:]!r}",
        )
        passed &= check(
            "the picked worktree is removed through its workspace",
            "worktree remove --workspace w2" in calls and code == 0,
            f"exit {code}, stub log: {calls!r}",
        )
        return passed
    finally:
        palette.close()


def a_typed_repo_placeholder_is_kept_as_typed(scratch: Path) -> bool:
    """`{repo}` is filled before the typed name goes in, so a name that spells
    it reaches herdr as typed rather than as the repository's path."""
    stub = write_stub(scratch / "herdr-typed-repo", WORKTREES)
    log = scratch / "typed-repo.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "typed-repo.stderr")
    name = "a branch typed as {repo} reaches herdr as typed"
    try:
        if not started(palette, name):
            return False
        palette.send(b"New worktree\r")
        palette.wait_for("New branch name", OUTCOME_TIMEOUT)
        palette.send(b"{repo}\r")
        code = palette.wait_for_exit(EXIT_TIMEOUT)
        calls = log.read_text()
        return check(
            name,
            "worktree create --cwd /src/repo --branch {repo} --focus" in calls and code == 0,
            f"exit {code}, stub log: {calls!r}",
        )
    finally:
        palette.close()


def an_unfillable_repo_is_reported_before_dispatch(scratch: Path) -> bool:
    """`{repo}` is looked up when the entry is picked, so a workspace outside
    any Git repository fails there, before the branch name is asked for."""
    stub = write_stub(scratch / "herdr-not-git", REJECTS_WORKTREE_LIST)
    log = scratch / "not-git.log"
    log.write_text("")
    palette = Palette(stub, log, scratch / "not-git.stderr")
    name = "an unresolvable {repo} reports herdr's reason"
    try:
        if not started(palette, name):
            return False
        palette.send(b"New worktree\r")
        # The head only: the wrapped remainder is redrawn cell by cell.
        seen = palette.wait_until_squeezed(
            "`worktree.create`failed:Herdrworktreeactionsrequire", OUTCOME_TIMEOUT
        )
        text = visible(palette.painted).replace("\n", " ")
        passed = check(name, seen, f"drew: {text[-400:]!r}")
        passed &= check(
            "the branch name is not asked for without a repository",
            "New branch name" not in visible(palette.painted),
            f"drew: {text[-400:]!r}",
        )
        passed &= check(
            "the palette stays up after an unresolvable {repo}",
            palette.wait_for_exit(1.0) is None,
            "it exited instead of showing why",
        )
        passed &= check(
            "nothing was dispatched without a repository",
            "worktree create" not in log.read_text(),
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


def opened_with_settings(
    scratch: Path,
    name: str,
    settings: str | None,
    ready: str = READY_MARKER,
    stub_body: str = ACCEPTS,
    catalog: str | None = None,
) -> str:
    """What the palette draws on opening with `settings` as its settings.toml,
    or with none at all. `ready` is matched with whitespace removed, because a
    status line replaces the footer that carries READY_MARKER and may wrap."""
    stub = write_stub(scratch / f"herdr-{name}", stub_body)
    log = scratch / f"{name}.log"
    log.write_text("")
    config = scratch / f"config-{name}"
    config.mkdir()
    if settings is not None:
        (config / "settings.toml").write_text(settings)
    if catalog is not None:
        (config / "catalog.toml").write_text(catalog)
    palette = Palette(stub, log, scratch / f"{name}.stderr", config)
    try:
        if not palette.wait_until_squeezed("".join(ready.split()), READY_TIMEOUT):
            return ""
        return visible(palette.painted)
    finally:
        palette.close()


def icons_follow_settings(scratch: Path) -> bool:
    """`settings.toml` is read by `main`, which no unit test reaches: these
    prove the switch arrives, and that a missing file is not an error."""
    absent = opened_with_settings(scratch, "settings-absent", None)
    passed = check(
        "with no settings.toml the rows carry icons",
        "◫" in absent,
        f"drew: {absent[-400:]!r}",
    )
    passed &= check(
        "a missing settings.toml says nothing",
        "settings.toml" not in absent,
        f"drew: {absent[-400:]!r}",
    )

    off = opened_with_settings(scratch, "settings-off", "icons = false\n")
    passed &= check(
        "icons = false draws no icons",
        bool(off) and "◫" not in off and "Splitpane" in "".join(off.split()),
        f"drew: {off[-400:]!r}",
    )

    broken = opened_with_settings(
        scratch, "settings-broken", "icon = false\n", ready="using defaults"
    )
    squeezed = "".join(broken.split())
    passed &= check(
        "an unusable settings.toml is reported and the defaults kept",
        "settings.toml" in squeezed and "usingdefaults" in squeezed and "◫" in broken,
        f"drew: {broken[-400:]!r}",
    )
    return passed


# One well-formed entry so the palette opens, and one whose icon is two glyphs,
# so the catalog has something to skip.
ONE_SKIPPED = """checked_against = "0.8.2"

[[command]]
id = "pane.split.right"
title = "Split pane: right"
args = ["pane", "split", "--pane", "{pane}", "--direction", "right"]
contexts = ["pane"]

[[command]]
id = "pane.bad.icon"
title = "Bad icon"
args = ["pane", "zoom", "--pane", "{pane}", "--toggle"]
contexts = ["pane"]
icon = "◫◫"
"""


def every_footer_note_is_kept(scratch: Path) -> bool:
    """Each of `main`'s three startup notes is added on its own path, so all
    three are raised at once: any one assigned over the others drops a note."""
    drew = opened_with_settings(
        scratch,
        "all-notes",
        "icon = false\n",
        ready="using defaults",
        stub_body=ACCEPTS.replace(f"herdr {STUB_VERSION}", "herdr 0.1.0"),
        catalog=ONE_SKIPPED,
    )
    squeezed = "".join(drew.split())
    passed = True
    for note in ("isolderthanthecatalog's", "1skipped", "usingdefaults"):
        passed &= check(
            f"the footer keeps the `{note}` note beside the others",
            note in squeezed,
            f"drew: {drew[-500:]!r}",
        )
    return passed


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
    passed &= an_empty_listing_is_reported(scratch)
    passed &= a_refused_listing_is_reported(scratch)
    passed &= a_worktree_pick_reaches_the_context_repository(scratch)
    passed &= an_unfillable_repo_is_reported_before_dispatch(scratch)
    passed &= a_typed_repo_placeholder_is_kept_as_typed(scratch)
    passed &= removing_offers_only_worktrees_herdr_created(scratch)
    passed &= esc_closes_the_palette(scratch)
    passed &= icons_follow_settings(scratch)
    passed &= every_footer_note_is_kept(scratch)

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
