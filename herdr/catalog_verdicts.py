"""Judge what herdr answered for one catalog entry, for `catalog-e2e.py`.

Pure: an entry's id and herdr's answer in, a pass or a named failure out, so
each check is exercised without a herdr (see `catalog-e2e-test.py`).
"""

from __future__ import annotations

import json

from herdr_answer import answer, parse_envelope


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
