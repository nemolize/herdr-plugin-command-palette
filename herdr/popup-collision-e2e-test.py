#!/usr/bin/env python3
"""Assert that popup-collision-e2e fails every pair of presses that is not a
clean open followed by the collision, and that it reads each press's own run.
Against a herdr that collides correctly neither failure branch fires, so a
broken verdict would look the same as a passing one (#133).
"""

from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest import mock

HARNESS = Path(__file__).resolve().parent / "popup-collision-e2e.py"
spec = importlib.util.spec_from_file_location("popup_collision_e2e", HARNESS)
popup_collision_e2e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(popup_collision_e2e)
failure = popup_collision_e2e.failure
COLLISION = "command palette: a popup is already open (press Esc in it to close)\n"

OPENED = {"exit_code": 0, "stderr": "", "stdout": ""}
COLLIDED = {"exit_code": 1, "stderr": COLLISION, "stdout": ""}


class Failure(unittest.TestCase):
    def test_passes_an_open_then_the_collision(self):
        self.assertIsNone(failure(OPENED, COLLIDED))

    def test_fails_a_first_press_that_exited_non_zero(self):
        first = {"exit_code": 1, "stderr": "command palette: plugin not found\n"}
        self.assertIn("could not open a popup on the first press", failure(first, COLLIDED))

    def test_fails_a_first_press_that_wrote_to_stderr(self):
        first = {"exit_code": 0, "stderr": "warning\n"}
        self.assertIn("could not open a popup on the first press", failure(first, COLLIDED))

    def test_fails_a_second_press_that_opened(self):
        self.assertIn("did not take the collision branch", failure(OPENED, OPENED))

    def test_fails_the_collision_message_under_exit_0(self):
        second = {"exit_code": 0, "stderr": COLLISION}
        self.assertIn("did not take the collision branch", failure(OPENED, second))

    def test_fails_a_second_press_on_the_generic_error_path(self):
        second = {"exit_code": 1, "stderr": "command palette: a popup pane is already open\n"}
        self.assertIn("did not take the collision branch", failure(OPENED, second))


def completed(result: dict) -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess([], 0, json.dumps({"result": result}), "")


class RunOpenAction(unittest.TestCase):
    """herdr lists logs oldest first today; nothing promises it, so the run is
    picked by the id `invoke` returns rather than by its position."""

    def test_waits_for_the_run_the_invoke_named_to_finish(self):
        running = {"log_id": "plugin-log-2", "status": "running"}
        finished = {"log_id": "plugin-log-2", "finished_unix_ms": 2, **COLLIDED}
        others = [
            {"log_id": "plugin-log-1", "finished_unix_ms": 1, **OPENED},
            {"log_id": "plugin-log-3", "finished_unix_ms": 3, **OPENED},
        ]
        lists = iter([[others[0], running, others[1]], [others[0], finished, others[1]]])

        def herdr(*args, home, check=True):
            if args[:3] == ("plugin", "action", "invoke"):
                return completed({"log": {"log_id": "plugin-log-2"}})
            return completed({"logs": next(lists)})

        with mock.patch.object(popup_collision_e2e.catalog_e2e, "herdr", herdr), \
                mock.patch.object(popup_collision_e2e.time, "sleep"):
            run = popup_collision_e2e.run_open_action(Path("/nonexistent"))
        self.assertEqual(run, finished)

    def test_names_the_call_that_failed_rather_than_fixture_setup(self):
        def herdr(*args, home, check=True):
            return subprocess.CompletedProcess([], 1, "", "plugin not found")

        with mock.patch.object(popup_collision_e2e.catalog_e2e, "herdr", herdr):
            with self.assertRaisesRegex(RuntimeError, r"^herdr plugin action invoke .* exited 1"):
                popup_collision_e2e.run_open_action(Path("/nonexistent"))


class MainVerdict(unittest.TestCase):
    """Runs `main()` with each run stubbed, so the call site of `failure()`,
    the run's exit status and the server's shutdown are covered."""

    def run_main(self, *runs: dict | Exception) -> tuple[int, str]:
        catalog_e2e = popup_collision_e2e.catalog_e2e
        stderr = io.StringIO()
        with tempfile.TemporaryDirectory() as root, \
                mock.patch.object(catalog_e2e, "HERDR", "herdr"), \
                mock.patch.object(catalog_e2e, "FIXTURE_ROOT", Path(root)), \
                mock.patch.object(catalog_e2e, "Fixture") as fixture, \
                mock.patch.object(catalog_e2e, "herdr"), \
                mock.patch.object(catalog_e2e, "running_herdr_version", return_value="0.9.3"), \
                mock.patch.object(popup_collision_e2e, "run_open_action", side_effect=runs), \
                redirect_stderr(stderr):
            code = popup_collision_e2e.main()
        fixture.return_value.close.assert_called_once_with()
        return code, stderr.getvalue()

    def test_passes_an_open_then_the_collision(self):
        code, stderr = self.run_main(OPENED, COLLIDED)
        self.assertEqual(code, 0, stderr)

    def test_fails_a_second_press_that_opened(self):
        code, stderr = self.run_main(OPENED, OPENED)
        self.assertEqual(code, 1)
        self.assertIn("did not take the collision branch", stderr)

    def test_fails_when_a_press_could_not_run(self):
        code, stderr = self.run_main(RuntimeError("the hop did not finish"))
        self.assertEqual(code, 1)
        self.assertIn("could not run: the hop did not finish", stderr)


if __name__ == "__main__":
    unittest.main()
