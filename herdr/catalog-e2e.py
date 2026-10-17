#!/usr/bin/env python3
"""Run every catalog entry against a real herdr and fail on any that it rejects.

The unit tests in `src/catalog.rs` check entries against a hand-copied
transcription of `herdr <sub> <cmd> --help`, so they go stale the moment herdr
adds a constraint the table does not carry — which is how `pane.move.tab`
shipped with `--tab` and no `--split` even though its flags were valid and its
positional count was right (docs/design.md §4, issue #24).

This asks herdr instead. herdr's clap layer accepts that combination and its
runtime rejects it, writing usage to stderr and exiting non-zero, so the exit
code separates a runnable entry from a broken one. An entry that exits 0 must
also answer with an envelope the palette accepts — a non-null `result` and no
`error` — because the palette reports anything else as a failed action (#146).
A resize or swap entry must also move its pane, since herdr answers both well
when they change nothing (#119).

Every entry gets its own fixture session, built from nothing and torn down
after. That is what makes running the destructive entries (`pane close`,
`workspace close`) safe, and it removes the ordering coupling that would
otherwise decide whether an entry finds the state it needs.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import tomllib
from pathlib import Path

from catalog_placeholders import flag_before, resolved_args
from herdr_answer import answer, parse_envelope
import herdr_fixture

CATALOG = herdr_fixture.REPO / "herdr" / "catalog.toml"


def unaccepted_response(stdout: str, stderr: str) -> str | None:
    """Why `read_response` in src/herdr.rs would call this answer a failure, or
    None when it would accept it. Mirrors its order: the envelope from stdout,
    else from stderr; then `error`; then a `result` that is absent or null."""
    body = parse_envelope(stdout)
    if body is None:
        body = parse_envelope(stderr)
    if body is None:
        return "no JSON envelope"
    if body.get("error") is not None:
        return f"an error: {json.dumps(body['error'])}"
    if "result" not in body:
        return "no result"
    if body["result"] is None:
        return "a null result"
    return None


DIRECTIONS = {"left": ("x", -1), "right": ("x", 1), "up": ("y", -1), "down": ("y", 1)}


def effect_target(entry_id: str, args: list[str]) -> tuple[str, str, str] | None:
    """(verb, direction, pane) for an entry whose effect is checked, else None.

    Only resize and swap: herdr exits 0 with `changed: false` when they have
    nothing to act on, so the exit code and envelope cannot tell a working
    entry from a no-op (#119). The direction comes from the id, not the argv,
    so an argv aimed the wrong way fails rather than checking itself."""
    group, verb, direction = (entry_id.split(".") + ["", "", ""])[:3]
    if group != "pane" or verb not in ("resize", "swap"):
        return None
    if direction not in DIRECTIONS:
        raise RuntimeError(
            f"{entry_id}: its id names no direction this harness can check"
        )
    at = args.index("--pane") + 1 if "--pane" in args else len(args)
    if at >= len(args):
        raise RuntimeError(
            f"{entry_id}: its argv has no --pane value to read the layout of"
        )
    return verb, direction, args[at]


def missing_effect(verb: str, direction: str, before: dict, after: dict) -> str | None:
    """Why the pane's rect shows no effect of the entry, or None when it does.

    A resize must push the pane's edge on the named side outward. A swap must
    move the pane's centre mostly along the named direction: the cross's side
    neighbours are full height, so a sideways swap also shifts `y`."""
    axis, sign = DIRECTIONS[direction]
    extent = "width" if axis == "x" else "height"
    if verb == "resize":
        def edge(rect: dict) -> int:
            return rect[axis] + (rect[extent] if sign > 0 else 0)

        if (edge(after) - edge(before)) * sign > 0:
            return None
        return f"its {direction} edge went {edge(before)} -> {edge(after)}, not outward"

    def shift(a: str) -> float:
        span = "width" if a == "x" else "height"
        return (after[a] + after[span] / 2) - (before[a] + before[span] / 2)

    other = "y" if axis == "x" else "x"
    if shift(axis) * sign > 0 and abs(shift(axis)) > abs(shift(other)):
        return None
    return f"its centre moved {shift('x'):+g}, {shift('y'):+g}, not {direction}"


def wrong_action(verb: str, stdout: str, stderr: str) -> str | None:
    """Why herdr's answer names another action than the id's verb, or None.

    The rect alone cannot tell them apart: a swap pushes the named edge out as
    a resize does, and a resize nudges the centre the named way as a swap does."""
    result = (parse_envelope(stdout) or parse_envelope(stderr) or {}).get("result")
    answered = result.get("type") if isinstance(result, dict) else None
    if answered == f"pane_{verb}":
        return None
    return f"herdr answered {answered!r}, not 'pane_{verb}'"


RECT_FIELDS = ("x", "y", "width", "height")


def pane_rect(entry_id: str, pane: str, home: Path) -> dict:
    proc = herdr_fixture.herdr("pane", "layout", "--pane", pane, home=home, check=False)
    if proc.returncode != 0:
        raise RuntimeError(
            f"{entry_id}: herdr pane layout --pane {pane} exited {proc.returncode}: "
            f"{proc.stderr.strip()}"
        )
    return layout_rect(entry_id, pane, proc.stdout)


def is_numeric_rect(rect: object) -> bool:
    return isinstance(rect, dict) and all(
        isinstance(rect.get(f), (int, float)) and not isinstance(rect.get(f), bool)
        for f in RECT_FIELDS
    )


def layout_rect(entry_id: str, pane: str, stdout: str) -> dict:
    try:
        rows = answer(f"pane layout --pane {pane}", stdout, "layout.panes", list)
    except RuntimeError as e:
        raise RuntimeError(f"{entry_id}: {e}") from None
    row = next((r for r in rows if isinstance(r, dict) and r.get("pane_id") == pane), None)
    if row is None:
        raise RuntimeError(f"{entry_id}: pane {pane} is missing from its own layout")
    rect = row.get("rect")
    if not is_numeric_rect(rect):
        raise RuntimeError(
            f"{entry_id}: pane {pane}'s layout rect is not numeric "
            f"{'/'.join(RECT_FIELDS)}: {json.dumps(rect)}"
        )
    return rect



def note_if_pin_disagrees_with_running_herdr(
    checked_against: str | None, running: str | None
) -> None:
    """Say so when the catalog's pin names a different herdr than the one that ran.

    Not an error — the entries either run here or they do not, and that verdict
    stands either way. It prints before the entries run, so a red run reports
    the mismatch too, where the success line never prints.
    """
    if checked_against and running and running != checked_against:
        print(
            f"note: catalog says checked_against = {checked_against}, "
            f"running herdr {running}",
            file=sys.stderr,
        )


def success_line(count: int, checked_against: str | None, running: str | None) -> str:
    line = f"all {count} catalog entries ran against herdr {running or '(version unknown)'}"
    if checked_against and running != checked_against:
        line += f" (catalog checked_against = {checked_against})"
    return line


def report_rejected(rejected: list[tuple[str, list[str], int, str]]) -> None:
    """Print the entry id and argv per rejection, because #24's whole point is
    that a drifted catalog must say which line to edit — herdr's own runtime
    already reports that something is wrong and that was not enough."""
    plural = "y" if len(rejected) == 1 else "ies"
    print(f"\n{len(rejected)} catalog entr{plural} herdr rejected:\n", file=sys.stderr)
    for entry_id, args, code, stderr in rejected:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    exit: {code}", file=sys.stderr)
        for line in stderr.splitlines()[:4]:
            print(f"    {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_unaccepted(unaccepted: list[tuple[str, list[str], str, str, str]]) -> None:
    """Report separately from rejections: herdr ran these and exited 0, so the
    argv is fine — it is the answer that changed, and the palette would show a
    failure for an action that worked."""
    plural = "y" if len(unaccepted) == 1 else "ies"
    print(
        f"\n{len(unaccepted)} catalog entr{plural} herdr ran but answered with "
        "a response the palette reports as a failure:\n",
        file=sys.stderr,
    )
    for entry_id, args, reason, stdout, stderr in unaccepted:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    answered with {reason}", file=sys.stderr)
        for name, text in (("stdout", stdout), ("stderr", stderr)):
            for line in text.splitlines()[:4]:
                print(f"    {name}: {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_ineffective(ineffective: list[tuple[str, list[str], str, dict, dict]]) -> None:
    """Report separately from rejections and unaccepted answers: herdr accepted
    these, but they did not do what their id names to the pane."""
    plural = "y" if len(ineffective) == 1 else "ies"
    print(
        f"\n{len(ineffective)} catalog entr{plural} herdr ran without the "
        "effect the entry names:\n",
        file=sys.stderr,
    )
    for entry_id, args, reason, before, after in ineffective:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    missing effect: {reason}", file=sys.stderr)
        print(f"    before: {json.dumps(before, sort_keys=True)}", file=sys.stderr)
        print(f"    after:  {json.dumps(after, sort_keys=True)}", file=sys.stderr)
        print(file=sys.stderr)


def report_broken(broken: list[tuple[str, str]]) -> None:
    """Report separately from rejections: this is the harness failing to build a
    session or to read an entry it does not know, not the catalog being wrong,
    and reading one as the other sends whoever is on the red build to edit a
    file that is fine."""
    plural = "y" if len(broken) == 1 else "ies"
    print(
        f"\n{len(broken)} entr{plural} could not be checked — the harness "
        "could not set it up, which is a harness fault rather than catalog drift:\n",
        file=sys.stderr,
    )
    for entry_id, message in broken:
        print(f"  {entry_id}", file=sys.stderr)
        for line in message.splitlines()[:6]:
            print(f"    {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_unknown_bindings(entries: list[dict], home: Path) -> list[tuple[str, str]]:
    """Every `binding` herdr does not recognise as a `[keys]` action.

    The oracle is `herdr config check` rather than `--default-config`, because
    the template under-reports: it omits `swap_pane_*`, which herdr binds by
    default, so a template-based check calls a working action unknown. Writing
    every binding into one config and reading the warnings asks herdr itself.

    A wrong `binding` is otherwise invisible — it shows a blank key column,
    which is also what a correct entry with no counterpart shows.
    """
    named = [(e["id"], e["binding"]) for e in entries if e.get("binding")]
    if not named:
        return []

    # The verdict is an ABSENCE of warnings, which a crashed or reworded herdr
    # produces too — so a name it cannot know must come back flagged.
    sentinel = "zzz_not_a_herdr_action"
    config = home / "herdr" / "config.toml"
    config.parent.mkdir(parents=True, exist_ok=True)
    lines = "\n".join(f'{action} = "prefix+z"' for _, action in named)
    config.write_text(f'onboarding = false\n[keys]\n{lines}\n{sentinel} = "prefix+z"\n')

    proc = herdr_fixture.herdr("config", "check", home=home, check=False)
    warned = proc.stdout + proc.stderr

    def flagged(action: str) -> bool:
        return f"keys.{action};" in warned or f"keys.{action} " in warned

    if not flagged(sentinel):
        raise RuntimeError(
            "`herdr config check` did not flag the sentinel "
            f"`{sentinel}`, so its silence proves nothing about the real "
            f"bindings. Output was:\n{warned.strip() or '(nothing)'}"
        )

    return [(entry_id, action) for entry_id, action in named if flagged(action)]


def main() -> int:
    if herdr_fixture.HERDR is None:
        print(
            "no herdr found — run `just fetch-herdr`, or put one on PATH",
            file=sys.stderr,
        )
        return 1

    catalog = tomllib.loads(CATALOG.read_text())
    entries = catalog.get("command", [])
    if not entries:
        print(f"no entries in {CATALOG}", file=sys.stderr)
        return 1

    checked_against = catalog.get("checked_against")
    running = herdr_fixture.running_herdr_version()
    note_if_pin_disagrees_with_running_herdr(checked_against, running)

    herdr_fixture.FIXTURE_ROOT.mkdir(parents=True, exist_ok=True)
    rejected: list[tuple[str, list[str], int, str]] = []
    unaccepted: list[tuple[str, list[str], str, str, str]] = []
    ineffective: list[tuple[str, list[str], str, dict, dict]] = []
    broken: list[tuple[str, str]] = []

    bindings_home = Path(tempfile.mkdtemp(dir=herdr_fixture.FIXTURE_ROOT))
    try:
        unknown_bindings = report_unknown_bindings(entries, bindings_home)
    except RuntimeError as e:
        print(f"binding check could not run: {e}", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(bindings_home, ignore_errors=True)
    for entry_id, action in unknown_bindings:
        print(f"FAIL {entry_id}: binding `{action}`", file=sys.stderr)

    for entry in entries:
        home = Path(tempfile.mkdtemp(dir=herdr_fixture.FIXTURE_ROOT))
        try:
            fixture = herdr_fixture.Fixture(
                home,
                worktree=entry["args"][0] == "worktree",
                closed=flag_before(entry["args"], "{}") == "--path",
            )
        except RuntimeError as e:
            # Keep going: one flaky boot must not swallow every later entry's
            # verdict, and a harness fault is not a drifted catalog.
            broken.append((entry["id"], str(e)))
            print(f"BROKE {entry['id']}", file=sys.stderr)
            shutil.rmtree(home, ignore_errors=True)
            continue

        try:
            args = resolved_args(entry, fixture.ids)
            target = effect_target(entry["id"], args)
            verb, direction, pane = target or (None, None, None)
            before = pane_rect(entry["id"], pane, home) if target else None
            proc = herdr_fixture.herdr(*args, home=home, check=False)
            if proc.returncode != 0:
                rejected.append((entry["id"], args, proc.returncode, proc.stderr.strip()))
                print(f"FAIL {entry['id']}", file=sys.stderr)
            # `herdr update` answers in plain text; the palette judges it by exit status (src/herdr.rs).
            elif args[0] != "update" and (
                reason := unaccepted_response(proc.stdout, proc.stderr)
            ) is not None:
                unaccepted.append(
                    (entry["id"], args, reason, proc.stdout.strip(), proc.stderr.strip())
                )
                print(f"FAIL {entry['id']}", file=sys.stderr)
            else:
                after, reason = None, None
                if target:
                    try:
                        after = pane_rect(entry["id"], pane, home)
                        reason = wrong_action(verb, proc.stdout, proc.stderr) or missing_effect(
                            verb, direction, before, after
                        )
                    except RuntimeError as e:
                        after, reason = {}, f"its layout could not be read after the entry ran: {e}"
                if reason:
                    ineffective.append((entry["id"], args, reason, before, after))
                    print(f"FAIL {entry['id']}", file=sys.stderr)
                else:
                    print(f"ok   {entry['id']}", file=sys.stderr)
        except RuntimeError as e:
            broken.append((entry["id"], str(e)))
            print(f"BROKE {entry['id']}", file=sys.stderr)
        finally:
            fixture.close()
            shutil.rmtree(home, ignore_errors=True)

    if rejected:
        report_rejected(rejected)
    if unaccepted:
        report_unaccepted(unaccepted)
    if ineffective:
        report_ineffective(ineffective)
    if broken:
        report_broken(broken)
    if unknown_bindings:
        print(
            f"\n{len(unknown_bindings)} catalog `binding` value(s) herdr does not "
            "know as a [keys] action — each shows a blank key column:\n",
            file=sys.stderr,
        )
        for entry_id, action in unknown_bindings:
            print(f"  {entry_id}: binding = \"{action}\"", file=sys.stderr)
    if rejected or unaccepted or ineffective or broken or unknown_bindings:
        return 1

    print(f"\n{success_line(len(entries), checked_against, running)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
