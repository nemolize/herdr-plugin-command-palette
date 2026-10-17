"""The herdr fixture the real-herdr E2E scripts share: finding the binary, and
building and tearing down a session of panes, tabs and workspaces to act on.

Every E2E session is built from nothing under its own home and torn down after,
so one script's state never decides what another finds.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time
from pathlib import Path

from herdr_answer import answer, answer_field

REPO = Path(__file__).resolve().parent.parent

# Short because the api socket lands under it, and a unix socket path over
# ~104 bytes fails with `sun_path capacity` — which reads as a herdr fault.
FIXTURE_ROOT = Path(os.environ.get("HERDR_E2E_ROOT", "/tmp/herdr-e2e"))

SESSION = "catalog-e2e"
BOOT_TIMEOUT_SECS = 30
BOOT_POLL_SECS = 0.2


def herdr_binary() -> str | None:
    """Prefer the one `just fetch-herdr` puts in ./bin over whatever is on PATH,
    so a developer's own herdr is not what CI's verdict silently rests on."""
    fetched = REPO / "bin" / "herdr"
    if fetched.is_file() and os.access(fetched, os.X_OK):
        return str(fetched)
    return shutil.which("herdr")


HERDR = herdr_binary()


def fixture_env(home: Path) -> dict[str, str]:
    """HOME as well as the config dir: herdr creates worktrees under
    `~/.herdr/worktrees`, so a real HOME would collect one per run."""
    return dict(os.environ, XDG_CONFIG_HOME=str(home), HOME=str(home))


def herdr(*args: str, home: Path, check: bool = True) -> subprocess.CompletedProcess:
    env = fixture_env(home)
    proc = subprocess.run(
        [HERDR, "--session", SESSION, *args],
        env=env,
        capture_output=True,
        text=True,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(
            f"fixture setup failed: herdr {' '.join(args)}\n"
            f"exit={proc.returncode}\n{proc.stderr.strip()}"
        )
    return proc


def start_server_and_await_readiness(home: Path) -> subprocess.Popen:
    env = fixture_env(home)
    log = (home / "server.log").open("w")
    server = subprocess.Popen(
        [HERDR, "--session", SESSION, "server"],
        env=env,
        stdout=log,
        stderr=subprocess.STDOUT,
    )

    deadline = time.monotonic() + BOOT_TIMEOUT_SECS
    while time.monotonic() < deadline:
        if server.poll() is not None:
            raise RuntimeError(
                f"herdr server exited during boot (code {server.returncode})\n"
                f"{(home / 'server.log').read_text()[:2000]}"
            )
        if herdr("pane", "list", home=home, check=False).returncode == 0:
            return server
        time.sleep(BOOT_POLL_SECS)

    server.kill()
    raise RuntimeError(
        f"herdr server did not answer within {BOOT_TIMEOUT_SECS}s\n"
        f"{(home / 'server.log').read_text()[:2000]}"
    )


def init_repository(path: Path, home: Path) -> None:
    """A one-commit repository: `worktree create` needs a HEAD to branch from."""
    path.mkdir()
    for args in (
        ["init", "--quiet"],
        ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com",
         "commit", "--quiet", "--allow-empty", "--message", "e2e"],
    ):
        proc = subprocess.run(
            ["git", "-C", str(path), *args],
            env=fixture_env(home),
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"fixture setup failed: git {' '.join(args)}\n"
                f"exit={proc.returncode}\n{proc.stderr.strip()}"
            )


class Fixture:
    """A session holding two tabs, so an entry that needs a second target has one.

    `pane.move.tab` moves a pane into a *different* tab, and `tab.focus` is only
    meaningful with somewhere to switch to. The first tab is split into a cross
    whose centre pane is `{pane}`, so a resize or swap has a neighbour on every
    side to act against rather than being a no-op (#119). The workspace is
    rooted in a Git repository so the worktree entries have one to act on, and
    `worktree` adds a linked worktree — `remove` refuses the main checkout, so
    acting on that one would prove nothing. `closed` closes its workspace, so
    `open` opens it rather than focusing what is already there.
    """

    def __init__(self, home: Path, worktree: bool = False, closed: bool = False):
        self.home = home
        repo = home / "repo"
        init_repository(repo, home)
        self.server = start_server_and_await_readiness(home)
        # Past this point the server is running, so anything that raises has to
        # stop it — an exception here binds no Fixture for the caller to close.
        try:
            herdr("workspace", "create", "--cwd", str(repo), home=home)
            herdr("tab", "create", home=home)
            self._surround(self._read_ids()["{pane}"])
            self.ids = self._read_ids()
            if worktree:
                self.ids.update(self._create_worktree(closed))
        except BaseException:
            self.close()
            raise

    def _surround(self, pane: str) -> None:
        """left | (top / centre / bottom) | right, focusing the centre last so
        `_read_ids` picks it as `{pane}`."""

        def split(target: str, direction: str, focus: str = "--no-focus") -> str:
            proc = herdr("pane", "split", "--pane", target, "--direction", direction,
                         focus, home=self.home)
            return answer("pane split", proc.stdout, "pane.pane_id")

        split(pane, "right")
        middle = split(pane, "right")
        split(middle, "down")
        split(middle, "down", "--focus")

    def _create_worktree(self, closed: bool) -> dict[str, str]:
        proc = herdr(
            "worktree", "create", "--workspace", self.ids["{workspace}"],
            "--branch", "e2e-existing", "--no-focus", home=self.home,
        )
        row = answer("worktree create", proc.stdout, "worktree", dict)
        workspace, path = (
            answer_field("worktree create", row, key, within="result.worktree")
            for key in ("open_workspace_id", "path")
        )
        # The linked worktree's workspace is where `create` and `open` refuse to
        # start from, so the context is moved there while it stays open.
        listing = herdr(
            "worktree", "list", "--workspace", workspace, home=self.home
        )
        repo = answer("worktree list", listing.stdout, "source.repo_root")
        ids = {}
        if closed:
            herdr("workspace", "close", workspace, home=self.home)
        else:
            ids["{workspace}"] = workspace
        return {
            **ids,
            "{repo}": repo,
            "{worktree path}": path,
            "{worktree workspace}": workspace,
        }

    def _read_ids(self) -> dict[str, str]:
        rows = answer("pane list", herdr("pane", "list", home=self.home).stdout, "panes", list)
        if not rows:
            raise RuntimeError("herdr pane list answered an empty result.panes")
        for i, row in enumerate(rows):
            for key in ("pane_id", "tab_id", "workspace_id"):
                answer_field("pane list", row, key, within=f"result.panes[{i}]")
        focused = next((p for p in rows if p.get("focused")), rows[0])
        tabs = {p["tab_id"] for p in rows}
        another_tab = next((t for t in sorted(tabs) if t != focused["tab_id"]), None)
        if another_tab is None:
            # Falling back to the focused tab would let `pane.move.tab` move a
            # pane into the tab it already occupies and still report ok.
            raise RuntimeError(
                f"fixture has only one tab ({focused['tab_id']}); "
                "the entries needing a second target would not be exercised"
            )
        return {
            "{pane}": focused["pane_id"],
            "{tab}": focused["tab_id"],
            "{workspace}": focused["workspace_id"],
            "{another tab}": another_tab,
        }

    def close(self) -> None:
        herdr("server", "stop", home=self.home, check=False)
        try:
            self.server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.server.kill()


def running_herdr_version() -> str | None:
    proc = subprocess.run([HERDR, "--version"], capture_output=True, text=True)
    words = proc.stdout.split() if proc.returncode == 0 else []
    return words[-1] if words else None
