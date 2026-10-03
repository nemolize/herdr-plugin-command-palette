#!/usr/bin/env python3
"""Press the palette key twice against a real herdr and assert the second press
is reported as a popup collision.

`herdr/open.sh` recognises the collision by herdr's error envelope, and #99
shipped broken because herdr 0.9 changed that envelope while `open-test`, which
stubs herdr, kept passing. This asks a real herdr instead.

The real palette binary is not built here, so a fixture plugin stands in for
it: its popup entrypoint is a stub that stays open, and its action runs this
repository's `open.sh`. Each press goes through `herdr plugin action invoke`, so
herdr itself sets the environment the hop reads, and the hop's exit code and
stderr come back from `herdr plugin log list`.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
import time
from pathlib import Path

HARNESS = Path(__file__).resolve().parent / "catalog-e2e.py"
spec = importlib.util.spec_from_file_location("catalog_e2e", HARNESS)
catalog_e2e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog_e2e)

HOP = catalog_e2e.REPO / "herdr" / "open.sh"
PLUGIN_ID = "popup-collision-e2e"
COLLISION = "command palette: a popup is already open (press Esc in it to close)\n"
RUN_TIMEOUT_SECS = 15
RUN_POLL_SECS = 0.2

# `open.sh` opens the entrypoint named `palette`, so the stub carries that id.
MANIFEST = f"""\
id = "{PLUGIN_ID}"
name = "Popup collision E2E"
version = "0.0.0"
min_herdr_version = "0.8.2"
platforms = ["linux", "macos"]

[[panes]]
id = "palette"
title = "Popup collision E2E"
placement = "popup"
width = 60
height = "45%"
command = ["sh", "-c", "exec sleep 600"]

[[actions]]
id = "open"
title = "Open"
contexts = ["global"]
command = ["sh", {json.dumps(str(HOP))}]
"""


def herdr_result(*args: str, home: Path) -> dict:
    """The `result` of a call made after setup, failing with its own argv —
    `catalog_e2e.herdr` would label any failure as fixture setup."""
    proc = catalog_e2e.herdr(*args, home=home, check=False)
    try:
        result = json.loads(proc.stdout)["result"]
    except (ValueError, KeyError, TypeError):
        result = None
    if proc.returncode != 0 or not isinstance(result, dict):
        raise RuntimeError(
            f"herdr {' '.join(args)} exited {proc.returncode}\n"
            f"{(proc.stdout + proc.stderr).strip()}"
        )
    return result


def press(home: Path) -> dict:
    """Invoke the action once and return its log entry once the hop has exited."""
    invoked = herdr_result("plugin", "action", "invoke", "open", "--plugin", PLUGIN_ID, home=home)
    log_id = (invoked.get("log") or {}).get("log_id")
    if not log_id:
        raise RuntimeError(f"`plugin action invoke` named no log_id: {json.dumps(invoked)}")
    deadline = time.monotonic() + RUN_TIMEOUT_SECS
    while time.monotonic() < deadline:
        logs = herdr_result("plugin", "log", "list", home=home).get("logs")
        if not isinstance(logs, list):
            raise RuntimeError(f"`plugin log list` answered without a log list: {logs!r}")
        run = next((log for log in logs if log.get("log_id") == log_id), None)
        if run is not None and run.get("finished_unix_ms") is not None:
            return run
        time.sleep(RUN_POLL_SECS)
    raise RuntimeError(f"the hop ({log_id}) did not finish within {RUN_TIMEOUT_SECS}s")


def describe(run: dict) -> str:
    return (
        f"exit {run.get('exit_code')}, stderr {run.get('stderr')!r}, "
        f"stdout {run.get('stdout')!r}"
    )


def failure(first: dict, second: dict) -> str | None:
    """Why these two presses fail the check, or None when they pass.

    A first press that did not open the popup leaves the second press nothing
    to collide with, so its result says nothing about the collision branch.
    """
    if first.get("exit_code") != 0 or first.get("stderr"):
        return f"could not open a popup on the first press: {describe(first)}"
    if second.get("exit_code") != 1 or second.get("stderr") != COLLISION:
        return (
            "the second press did not take the collision branch: "
            f"{describe(second)}; expected exit 1, stderr {COLLISION!r}"
        )
    return None


def main() -> int:
    if catalog_e2e.HERDR is None:
        print(
            "no herdr found — run `just fetch-herdr`, or put one on PATH",
            file=sys.stderr,
        )
        return 1

    catalog_e2e.FIXTURE_ROOT.mkdir(parents=True, exist_ok=True)
    home = Path(tempfile.mkdtemp(dir=catalog_e2e.FIXTURE_ROOT))
    try:
        fixture = catalog_e2e.Fixture(home)
        try:
            plugin = home / "plugin"
            plugin.mkdir()
            (plugin / "herdr-plugin.toml").write_text(MANIFEST)
            catalog_e2e.herdr("plugin", "link", str(plugin), home=home)
            first = press(home)
            second = press(home)
        finally:
            fixture.close()
    except RuntimeError as e:
        print(f"popup collision check could not run: {e}", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(home, ignore_errors=True)

    running = catalog_e2e.running_herdr_version() or "(version unknown)"
    reason = failure(first, second)
    if reason is not None:
        print(f"FAIL against herdr {running}: {reason}", file=sys.stderr)
        return 1

    print(f"a second press reported the popup collision on herdr {running}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
