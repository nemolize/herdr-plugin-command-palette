#!/usr/bin/env python3
"""Assert that catalog-e2e's response check fails every answer the palette's
`read_response` (src/herdr.rs) reports as a failure, and passes the one it
accepts. Without this, a check that never fires looks the same as a herdr that
always answers well (#146).
"""

from __future__ import annotations

import importlib.util
import io
import os
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest import mock

HARNESS = Path(__file__).resolve().parent / "catalog-e2e.py"
spec = importlib.util.spec_from_file_location("catalog_e2e", HARNESS)
catalog_e2e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog_e2e)
unaccepted_response = catalog_e2e.unaccepted_response


class UnacceptedResponse(unittest.TestCase):
    def test_accepts_a_non_null_result(self):
        self.assertIsNone(
            unaccepted_response('{"id":"cli:pane","result":{"type":"ok"}}\n', "")
        )

    def test_rejects_a_null_result(self):
        self.assertEqual(
            unaccepted_response('{"id":"cli:pane","result":null}\n', ""),
            "a null result",
        )

    def test_rejects_a_missing_result(self):
        self.assertEqual(unaccepted_response('{"id":"cli:pane"}\n', ""), "no result")

    def test_rejects_an_error_envelope(self):
        reason = unaccepted_response(
            '{"id":"cli:pane","result":{"type":"ok"},'
            '"error":{"code":"x","message":"pane not found"}}\n',
            "",
        )
        self.assertEqual(
            reason, 'an error: {"code": "x", "message": "pane not found"}'
        )

    def test_rejects_unparseable_output(self):
        self.assertEqual(
            unaccepted_response("pane closed\n", ""), "no JSON envelope"
        )

    def test_rejects_empty_output(self):
        self.assertEqual(unaccepted_response("", ""), "no JSON envelope")

    def test_reads_the_envelope_from_stderr_when_stdout_has_none(self):
        self.assertIsNone(
            unaccepted_response("", '{"id":"cli:pane","result":{"type":"ok"}}\n')
        )

    def test_accepts_a_null_error_beside_a_result(self):
        self.assertIsNone(
            unaccepted_response('{"id":"cli:pane","result":{"type":"ok"},"error":null}\n', "")
        )


STUB_HERDR = """#!/bin/sh
[ "$1" = --version ] && { echo "herdr 0.9.3"; exit 0; }
shift 2
case "$1 $2" in
  "server ") echo $$ > "$HOME/stub-server.pid"; exec sleep 60 ;;
  "server stop") kill "$(cat "$HOME/stub-server.pid")"; exit 0 ;;
  "pane list") echo '{"result":{"panes":[
    {"pane_id":"p1","tab_id":"t1","workspace_id":"w1","focused":true},
    {"pane_id":"p2","tab_id":"t2","workspace_id":"w1"}]}}' ;;
  "workspace create"|"tab create") echo '{"result":{"type":"ok"}}' ;;
  *) echo "$STUB_ANSWER" >&"${STUB_FD:-1}"; exit "${STUB_EXIT:-0}" ;;
esac
"""


class MainVerdict(unittest.TestCase):
    """Runs `main()` against a stubbed herdr, so the check's call site and the
    run's exit status are covered, not just the response check itself."""

    def setUp(self):
        scratch = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, scratch, ignore_errors=True)
        stub = scratch / "herdr"
        stub.write_text(STUB_HERDR)
        stub.chmod(0o755)
        catalog = scratch / "catalog.toml"
        catalog.write_text(
            '[[command]]\nid = "pane.close"\ntitle = "Close pane"\n'
            'args = ["pane", "close", "{pane}"]\n'
        )
        fixtures = scratch / "fixtures"
        for name, value in (("HERDR", str(stub)), ("CATALOG", catalog), ("FIXTURE_ROOT", fixtures)):
            self.addCleanup(setattr, catalog_e2e, name, getattr(catalog_e2e, name))
            setattr(catalog_e2e, name, value)

    def run_main(self, answer: str, fd: int = 1, exit_code: int = 0) -> tuple[int, str]:
        stub_env = {"STUB_ANSWER": answer, "STUB_FD": str(fd), "STUB_EXIT": str(exit_code)}
        stderr = io.StringIO()
        with mock.patch.dict(os.environ, stub_env), redirect_stderr(stderr):
            code = catalog_e2e.main()
        return code, stderr.getvalue()

    def test_passes_when_every_entry_answers_with_a_result(self):
        code, output = self.run_main('{"id":"cli:pane","result":{"type":"ok"}}')
        self.assertEqual(code, 0, output)

    def test_fails_an_entry_that_answers_with_a_null_result(self):
        code, output = self.run_main('{"id":"cli:pane","result":null}')
        self.assertEqual(code, 1, output)
        self.assertIn("pane.close", output)
        self.assertIn("answered with a null result", output)

    def test_reports_a_non_zero_exit_as_rejected_not_unaccepted(self):
        code, output = self.run_main('{"error":{"message":"bad flags"}}', fd=2, exit_code=1)
        self.assertEqual(code, 1, output)
        self.assertIn("herdr rejected", output)
        self.assertNotIn("answered with", output)


if __name__ == "__main__":
    unittest.main()
