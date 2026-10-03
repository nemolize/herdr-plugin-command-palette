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
import re
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
layout_rect = catalog_e2e.layout_rect
answer = catalog_e2e.answer
answer_field = catalog_e2e.answer_field


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

    def test_raises_naming_the_entry_when_the_argv_has_no_pane(self):
        for args in (
            ["pane", "resize", "--direction", "left"],
            ["pane", "resize", "--direction", "left", "--pane"],
        ):
            with self.subTest(args=args), self.assertRaisesRegex(
                RuntimeError, r"^pane\.resize\.left: .*no --pane value"
            ):
                effect_target("pane.resize.left", args)


def layout(*rows: object) -> str:
    return json.dumps({"result": {"layout": {"panes": list(rows)}}})


class LayoutRect(unittest.TestCase):
    def test_returns_the_named_pane_rect(self):
        self.assertEqual(
            layout_rect("e", "p1", layout({"pane_id": "p1", "rect": CENTRE})), CENTRE
        )

    def test_raises_naming_the_entry_on_each_reshaped_answer(self):
        for stdout, part in (
            ("layout unavailable", "without a result.layout.panes list"),
            ('{"result":{"layout":{"tabs":[]}}}', "without a result.layout.panes list"),
            (layout({"pane_id": "p9", "rect": CENTRE}), "pane p1 is missing from its own layout"),
            (layout("p1"), "pane p1 is missing from its own layout"),
            (layout({"pane_id": "p1"}), "rect is not numeric"),
            (layout({"pane_id": "p1", "rect": None}), "rect is not numeric"),
            (layout({"pane_id": "p1", "rect": {"x": 30, "y": 10}}), "rect is not numeric"),
            (layout({"pane_id": "p1", "rect": {**CENTRE, "width": "30"}}), "rect is not numeric"),
            (layout({"pane_id": "p1", "rect": {**CENTRE, "width": True}}), "rect is not numeric"),
        ):
            with self.subTest(stdout=stdout), self.assertRaisesRegex(
                RuntimeError, rf"^pane\.resize\.left: .*{re.escape(part)}"
            ):
                layout_rect("pane.resize.left", "p1", stdout)


class FixtureAnswer(unittest.TestCase):
    def test_returns_the_value_at_the_path(self):
        self.assertEqual(
            answer("pane split", '{"result":{"pane":{"pane_id":"p9"}}}', "pane.pane_id"), "p9"
        )

    def test_raises_naming_the_call_and_the_missing_key(self):
        for call, stdout, path, missing in (
            ("pane split", '{"result":{"pane":{"id":"p9"}}}', "pane.pane_id", "result.pane.pane_id"),
            ("pane split", '{"result":{"pane":"p9"}}', "pane.pane_id", "result.pane.pane_id"),
            ("pane list", '{"result":{"tabs":[]}}', "panes", "result.panes"),
            ("worktree create", '{"id":"cli:worktree"}', "worktree", "result"),
            ("worktree list", '{"result":{"repo_root":"/r"}}', "source.repo_root", "result.source"),
        ):
            with self.subTest(call=call, stdout=stdout), self.assertRaisesRegex(
                RuntimeError, rf"^fixture setup: herdr {call} answered without {re.escape(missing)}$"
            ):
                answer(call, stdout, path)

    def test_raises_naming_the_call_and_the_key_on_a_value_of_another_type(self):
        for stdout, kind, message in (
            ('{"result":{"pane":{"pane_id":null}}}', str, "result.pane.pane_id as a NoneType, not a str"),
            ('{"result":{"pane":{"pane_id":7}}}', str, "result.pane.pane_id as a int, not a str"),
            ('{"result":{"pane":"p9"}}', dict, "result.pane as a str, not a dict"),
        ):
            path = "pane.pane_id" if kind is str else "pane"
            with self.subTest(stdout=stdout), self.assertRaisesRegex(
                RuntimeError, rf"^fixture setup: herdr pane split answered {re.escape(message)}$"
            ):
                answer("pane split", stdout, path, kind)

    def test_raises_naming_the_call_on_an_answer_that_is_not_json(self):
        with self.assertRaisesRegex(
            RuntimeError, r"^fixture setup: herdr pane list answered without a JSON envelope: oops$"
        ):
            answer("pane list", "oops\n", "panes")

    def test_names_where_a_nested_row_sits_in_the_answer(self):
        with self.assertRaisesRegex(
            RuntimeError, r"herdr pane list answered without result\.panes\[1\]\.tab_id$"
        ):
            answer_field("pane list", {"pane_id": "p2"}, "tab_id", within="result.panes[1]")


# What herdr 0.9.3 left `{pane}` at after each action, from CENTRE in the cross.
OBSERVED = {
    ("resize", "left"): rect(27, 10, 33, 10),
    ("resize", "right"): rect(33, 10, 33, 10),
    ("resize", "up"): rect(30, 9, 30, 11),
    ("resize", "down"): rect(30, 11, 30, 11),
    ("swap", "left"): rect(0, 0, 30, 40),
    ("swap", "right"): rect(60, 0, 60, 40),
    ("swap", "up"): rect(30, 0, 30, 10),
    ("swap", "down"): rect(30, 20, 30, 20),
}


class MissingEffect(unittest.TestCase):
    def test_accepts_what_herdr_does_for_the_named_direction(self):
        for (verb, direction), after in OBSERVED.items():
            with self.subTest(verb=verb, direction=direction):
                self.assertIsNone(missing_effect(verb, direction, CENTRE, after))

    def test_rejects_what_herdr_does_for_any_other_direction(self):
        for (verb, done), after in OBSERVED.items():
            for named in ("left", "right", "up", "down"):
                if named != done:
                    with self.subTest(verb=verb, named=named, done=done):
                        self.assertIsNotNone(missing_effect(verb, named, CENTRE, after))

    def test_rejects_a_pane_left_in_place(self):
        self.assertEqual(
            missing_effect("resize", "right", CENTRE, CENTRE),
            "its right edge went 60 -> 60, not outward",
        )
        self.assertEqual(
            missing_effect("swap", "up", CENTRE, CENTRE),
            "its centre moved +0, +0, not up",
        )


STUB_HERDR = """#!/bin/sh
[ "$1" = --version ] && { echo "herdr 0.9.3"; exit 0; }
shift 2
case "$1 $2" in
  "server ") echo $$ > "$HOME/stub-server.pid"; exec sleep 60 ;;
  "server stop") kill "$(cat "$HOME/stub-server.pid")"; exit 0 ;;
  "pane list") [ -n "$STUB_PANES" ] && { echo "$STUB_PANES"; exit 0; }
    echo '{"result":{"panes":[
    {"pane_id":"p1","tab_id":"t1","workspace_id":"w1","focused":true},
    {"pane_id":"p2","tab_id":"t2","workspace_id":"w1"}]}}' ;;
  "workspace create"|"tab create") echo '{"result":{"type":"ok"}}' ;;
  "worktree create") [ -n "$STUB_WORKTREE" ] && { echo "$STUB_WORKTREE"; exit 0; }
    echo '{"result":{"worktree":{"open_workspace_id":"w2","path":"/wt"}}}' ;;
  "worktree list") echo '{"result":{"source":{"repo_root":"/repo"}}}' ;;
  "pane split") [ -n "$STUB_SPLIT" ] && { echo "$STUB_SPLIT"; exit 0; }
    echo '{"result":{"pane":{"pane_id":"p9"}}}' ;;
  "pane layout")
    rect=$STUB_BEFORE; [ -f "$HOME/stub-acted" ] && rect=$STUB_AFTER
    [ "$rect" = '"gone"' ] && { echo "no such pane" >&2; exit 1; }
    [ "$rect" = '"no panes"' ] && { echo '{"result":{"layout":{"tabs":[]}}}'; exit 0; }
    printf '{"result":{"layout":{"panes":[{"pane_id":"p1","rect":%s}]}}}\n' "$rect" ;;
  *) : > "$HOME/stub-acted"; [ -n "$STUB_ARGV" ] && echo "$*" >> "$STUB_ARGV"; echo "$STUB_ANSWER" >&"${STUB_FD:-1}"; exit "${STUB_EXIT:-0}" ;;
esac
"""


RESIZE_LEFT = '["pane", "resize", "--pane", "{pane}", "--direction", "left"]'
SWAP_LEFT = '["pane", "swap", "--pane", "{pane}", "--direction", "left"]'
RESIZED = '{"id":"cli:pane:resize","result":{"type":"pane_resize"}}'
SWAPPED = '{"id":"cli:pane:swap","result":{"type":"pane_swap"}}'


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

    def use_entry(self, entry_id: str, args: str, resolve: str | None = None) -> None:
        line = f'resolve = "{resolve}"\n' if resolve else ""
        self.catalog.write_text(
            f'[[command]]\nid = "{entry_id}"\ntitle = "{entry_id}"\nargs = {args}\n{line}'
        )

    def run_main(
        self, answer: str, fd: int = 1, exit_code: int = 0,
        before: dict | str = CENTRE, after: dict | str = CENTRE,
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
        code, output = self.run_main(RESIZED)
        self.assertEqual(code, 1, output)
        self.assertIn("without the effect the entry names", output)
        self.assertIn("pane.resize.left", output)
        self.assertIn("its left edge went 30 -> 30, not outward", output)

    def test_passes_a_resize_that_changes_the_pane(self):
        self.use_entry("pane.resize.left", RESIZE_LEFT)
        code, output = self.run_main(RESIZED, after=rect(27, 10, 33, 10))
        self.assertEqual(code, 0, output)

    def test_fails_a_swap_that_moves_the_pane_the_other_way(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(SWAPPED, after=rect(60, 0, 60, 40))
        self.assertEqual(code, 1, output)
        self.assertIn("pane.swap.left", output)
        self.assertIn("its centre moved +45, +5, not left", output)

    def test_reports_a_layout_unreadable_after_the_entry_as_its_missing_effect(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(SWAPPED, after="gone")
        self.assertEqual(code, 1, output)
        self.assertIn("without the effect the entry names", output)
        self.assertIn(
            "could not be read after the entry ran: "
            "pane.swap.left: herdr pane layout --pane p1 exited 1", output
        )
        self.assertNotIn("could not be checked", output)

    def test_reports_an_entry_without_a_pane_as_unchecked_naming_it(self):
        self.use_entry("pane.resize.left", '["pane", "resize", "--direction", "left"]')
        code, output = self.run_main(RESIZED)
        self.assertEqual(code, 1, output)
        self.assertIn("could not be checked", output)
        self.assertIn("pane.resize.left: its argv has no --pane value", output)
        self.assertNotIn("Traceback", output)

    def test_reports_a_reshaped_layout_before_the_entry_as_unchecked_naming_it(self):
        self.use_entry("pane.resize.left", RESIZE_LEFT)
        code, output = self.run_main(RESIZED, before="no panes")
        self.assertEqual(code, 1, output)
        self.assertIn("could not be checked", output)
        self.assertIn("pane.resize.left: herdr pane layout --pane p1 answered without", output)

    def test_reports_a_reshaped_layout_after_the_entry_as_its_missing_effect(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(SWAPPED, after="no panes")
        self.assertEqual(code, 1, output)
        self.assertIn("without the effect the entry names", output)
        self.assertIn("could not be read after the entry ran: pane.swap.left: ", output)
        self.assertNotIn("could not be checked", output)

    def test_fails_a_swap_entry_that_herdr_ran_as_a_resize(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(RESIZED, after=rect(0, 0, 30, 40))
        self.assertEqual(code, 1, output)
        self.assertIn("pane.swap.left", output)
        self.assertIn("herdr answered 'pane_resize', not 'pane_swap'", output)

    def test_reports_a_reshaped_fixture_answer_as_unchecked_naming_the_call(self):
        remove = ("worktree.remove", '["worktree", "remove", "--workspace", "{}"]')
        for entry, name, value, message in (
            (None, "STUB_SPLIT", '{"result":{"pane":{}}}',
             "herdr pane split answered without result.pane.pane_id"),
            (None, "STUB_SPLIT", '{"result":{"pane":{"pane_id":null}}}',
             "herdr pane split answered result.pane.pane_id as a NoneType, not a str"),
            (None, "STUB_PANES", '{"result":{"panes":[{"pane_id":"p1","workspace_id":"w1"}]}}',
             "herdr pane list answered without result.panes[0].tab_id"),
            (None, "STUB_PANES", '{"result":{"panes":[]}}',
             "herdr pane list answered an empty result.panes"),
            (remove, "STUB_WORKTREE", '{"result":{"worktree":{"path":"/wt"}}}',
             "herdr worktree create answered without result.worktree.open_workspace_id"),
        ):
            with self.subTest(message=message), mock.patch.dict(os.environ, {name: value}):
                entry_id, args = entry or ("pane.close", '["pane", "close", "{pane}"]')
                self.use_entry(entry_id, args)
                code, output = self.run_main('{"id":"cli:pane","result":{"type":"ok"}}')
                self.assertEqual(code, 1, output)
                self.assertIn(f"BROKE {entry_id}", output)
                self.assertIn("could not be checked", output)
                self.assertIn(message, output)
                self.assertNotIn("Traceback", output)

    def test_runs_worktree_entries_with_the_ids_worktree_create_answered(self):
        for entry_id, args, ran in (
            ("worktree.remove", '["worktree", "remove", "--workspace", "{}"]',
             ["worktree remove --workspace w2"]),
            ("worktree.open", '["worktree", "open", "--cwd", "{repo}", "--path", "{}", "--focus"]',
             ["workspace close w2", "worktree open --cwd /repo --path /wt --focus"]),
        ):
            with self.subTest(entry_id=entry_id):
                self.use_entry(entry_id, args, resolve="worktree list")
                argv = self.catalog.parent / f"{entry_id}.argv"
                with mock.patch.dict(os.environ, {"STUB_ARGV": str(argv)}):
                    code, output = self.run_main('{"id":"cli:worktree","result":{"type":"ok"}}')
                self.assertEqual(code, 0, output)
                self.assertEqual(argv.read_text().splitlines(), ran)

    def test_passes_a_swap_that_moves_the_pane_the_named_way(self):
        self.use_entry("pane.swap.left", SWAP_LEFT)
        code, output = self.run_main(SWAPPED, after=rect(0, 0, 30, 40))
        self.assertEqual(code, 0, output)


if __name__ == "__main__":
    unittest.main()
