#!/usr/bin/env python3
"""Assert that catalog-e2e's response check fails every answer the palette's
`read_response` (src/herdr.rs) reports as a failure, and passes the one it
accepts. Without this, a check that never fires looks the same as a herdr that
always answers well (#146).
"""

from __future__ import annotations

import importlib.util
import io
import json
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
effect_target = catalog_e2e.effect_target
missing_effect = catalog_e2e.missing_effect


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


def rect(x: int, y: int, width: int, height: int) -> dict:
    return {"x": x, "y": y, "width": width, "height": height}


CENTRE = rect(30, 10, 30, 10)


class EffectTarget(unittest.TestCase):
    def test_skips_entries_other_than_pane_resize_and_swap(self):
        self.assertIsNone(effect_target("pane.focus.left", ["pane", "focus", "--pane", "p1"]))
        self.assertIsNone(effect_target("tab.close", ["tab", "close", "t2"]))

    def test_takes_the_direction_from_the_id_not_the_argv(self):
        args = ["pane", "swap", "--pane", "p1", "--direction", "right"]
        self.assertEqual(effect_target("pane.swap.left", args), ("swap", "left", "p1"))

    def test_raises_on_an_id_naming_no_direction(self):
        with self.assertRaises(RuntimeError):
            effect_target("pane.resize", ["pane", "resize", "--pane", "p1"])


class MissingEffect(unittest.TestCase):
    def test_accepts_a_resize_that_changes_the_named_extent(self):
        self.assertIsNone(missing_effect("resize", "left", CENTRE, rect(27, 10, 33, 10)))
        self.assertIsNone(missing_effect("resize", "down", CENTRE, rect(30, 11, 30, 11)))

    def test_rejects_a_resize_that_changes_only_the_other_extent(self):
        self.assertEqual(
            missing_effect("resize", "right", CENTRE, rect(30, 9, 30, 11)),
            "its width stayed 30",
        )
        self.assertEqual(
            missing_effect("resize", "up", CENTRE, rect(27, 10, 33, 10)),
            "its height stayed 10",
        )

    def test_accepts_a_swap_that_moves_the_pane_the_named_way(self):
        for direction, after in (
            ("left", rect(0, 0, 30, 40)),
            ("right", rect(60, 0, 60, 40)),
            ("up", rect(30, 0, 30, 10)),
            ("down", rect(30, 20, 30, 20)),
        ):
            with self.subTest(direction):
                self.assertIsNone(missing_effect("swap", direction, CENTRE, after))

    def test_rejects_a_swap_that_moves_the_pane_the_other_way(self):
        self.assertEqual(
            missing_effect("swap", "left", CENTRE, rect(60, 0, 60, 40)),
            "its x went 30 -> 60, not left",
        )

    def test_rejects_a_swap_that_leaves_the_pane_in_place(self):
        self.assertEqual(
            missing_effect("swap", "up", CENTRE, CENTRE),
            "its y went 10 -> 10, not up",
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
  "pane split") echo '{"result":{"pane":{"pane_id":"p9"}}}' ;;
  "pane layout")
    rect=$STUB_BEFORE; [ -f "$HOME/stub-acted" ] && rect=$STUB_AFTER
    printf '{"result":{"layout":{"panes":[{"pane_id":"p1","rect":%s}]}}}\n' "$rect" ;;
  *) : > "$HOME/stub-acted"; echo "$STUB_ANSWER" >&"${STUB_FD:-1}"; exit "${STUB_EXIT:-0}" ;;
esac
"""


OK_ANSWER = '{"id":"cli:pane","result":{"type":"ok"}}'
RESIZE_LEFT = '["pane", "resize", "--pane", "{pane}", "--direction", "left"]'
SWAP_LEFT = '["pane", "swap", "--pane", "{pane}", "--direction", "left"]'


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
        self.catalog = catalog
        self.use_entry("pane.close", '["pane", "close", "{pane}"]')
        fixtures = scratch / "fixtures"
        for name, value in (("HERDR", str(stub)), ("CATALOG", catalog), ("FIXTURE_ROOT", fixtures)):
            self.addCleanup(setattr, catalog_e2e, name, getattr(catalog_e2e, name))
            setattr(catalog_e2e, name, value)

    def use_entry(self, entry_id: str, args: str) -> None:
        self.catalog.write_text(
            f'[[command]]\nid = "{entry_id}"\ntitle = "{entry_id}"\nargs = {args}\n'
        )

    def run_main(
        self, answer: str, fd: int = 1, exit_code: int = 0,
        before: dict = CENTRE, after: dict = CENTRE,
    ) -> tuple[int, str]:
        stub_env = {
            "STUB_ANSWER": answer, "STUB_FD": str(fd), "STUB_EXIT": str(exit_code),
            "STUB_BEFORE": json.dumps(before), "STUB_AFTER": json.dumps(after),
        }
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

    def test_fails_a_resize_that_leaves_the_pane_unchanged(self):
        self.use_entry("pane.resize.left", RESIZE_LEFT)
        code, output = self.run_main(OK_ANSWER)
        self.assertEqual(code, 1, output)
        self.assertIn("without the effect the entry names", output)
        self.assertIn("pane.resize.left", output)
        self.assertIn("its width stayed 30", output)

    def test_passes_a_resize_that_changes_the_pane(self):
        self.use_entry("pane.resize.left", RESIZE_LEFT)
        code, output = self.run_main(OK_ANSWER, after=rect(27, 10, 33, 10))
        self.assertEqual(code, 0, output)

    def test_fails_a_swap_that_moves_the_pane_the_other_way(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(OK_ANSWER, after=rect(60, 0, 60, 40))
        self.assertEqual(code, 1, output)
        self.assertIn("pane.swap.left", output)
        self.assertIn("its x went 30 -> 60, not left", output)

    def test_passes_a_swap_that_moves_the_pane_the_named_way(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(OK_ANSWER, after=rect(0, 0, 30, 40))
        self.assertEqual(code, 0, output)


if __name__ == "__main__":
    unittest.main()
