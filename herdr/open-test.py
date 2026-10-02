#!/usr/bin/env python3
"""Run the action hop (`herdr/open.sh`) against a stubbed herdr and assert on
what it prints and how it exits, for each answer `plugin pane open` can give.

The hop tells a popup collision from any other failure by reading the error
envelope, whose shape changed between herdr releases (docs/design.md §6), and a
shape it no longer matches only shows as a different message.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
HOP = REPO / "herdr" / "open.sh"

COLLISION = "command palette: a popup is already open (press Esc in it to close)\n"

CASES = [
    (
        "0.9 collision",
        """echo '{"error":{"code":"ui_busy","message":"a popup pane is already open"},"id":"cli:plugin"}' >&2
exit 1""",
        1,
        COLLISION,
    ),
    (
        "0.8.2 collision",
        """echo '{"error":{"code":"plugin_pane_open_failed","message":"popup already open"},"id":"cli:plugin"}' >&2
exit 1""",
        1,
        COLLISION,
    ),
    (
        "other failure under the 0.8.2 generic code",
        """echo '{"error":{"code":"plugin_pane_open_failed","message":"plugin popup disappeared"},"id":"cli:plugin"}' >&2
exit 1""",
        1,
        "command palette: plugin popup disappeared\n",
    ),
    (
        "other failure under another code",
        """echo '{"error":{"code":"invalid_plugin_entrypoint","message":"invalid entrypoint id"},"id":"cli:plugin"}' >&2
exit 1""",
        1,
        "command palette: invalid entrypoint id\n",
    ),
    (
        "opened",
        """echo '{"id":"cli:plugin","result":{"type":"ok"}}'
exit 0""",
        0,
        "",
    ),
    (
        "exit 0 without a result",
        """echo '{}'
exit 0""",
        1,
        "command palette: could not open the palette (exit 0): {}\n",
    ),
    (
        "output that is not JSON",
        """echo "error: unrecognized subcommand 'pane'" >&2
exit 2""",
        1,
        "command palette: could not open the palette (exit 2): "
        "error: unrecognized subcommand 'pane'\n",
    ),
]


def run(stub_body: str, scratch: Path) -> subprocess.CompletedProcess[str]:
    stub = scratch / "herdr"
    stub.write_text("#!/bin/sh\n" + stub_body + "\n")
    stub.chmod(0o755)
    env = dict(
        os.environ,
        HERDR_BIN_PATH=str(stub),
        HERDR_PLUGIN_ID="command-palette",
    )
    return subprocess.run(
        ["sh", str(HOP)], env=env, capture_output=True, text=True, timeout=10
    )


def main() -> int:
    failures = 0
    with tempfile.TemporaryDirectory() as tmp:
        scratch = Path(tmp)
        for name, body, want_status, want_stderr in CASES:
            got = run(body, scratch)
            if got.returncode == want_status and got.stderr == want_stderr:
                print(f"ok   {name}")
                continue
            failures += 1
            print(f"FAIL {name}")
            print(f"  exit:   want {want_status}, got {got.returncode}")
            print(f"  stderr: want {want_stderr!r}")
            print(f"          got  {got.stderr!r}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
