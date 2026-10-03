#!/usr/bin/env python3
"""Assert that catalog-e2e's response check fails every answer the palette's
`read_response` (src/herdr.rs) reports as a failure, and passes the one it
accepts. Without this, a check that never fires looks the same as a herdr that
always answers well (#146).
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
