"""Resolve a catalog entry's placeholders into the argv `catalog-e2e.py` runs.

Pure: a catalog entry and the fixture's ids in, argv out, so it is checked
without a herdr (see `catalog-e2e-test.py`).
"""

from __future__ import annotations


def flag_before(args: list[str], placeholder: str) -> str | None:
    at = args.index(placeholder) if placeholder in args else 0
    return args[at - 1] if at > 0 else None


def resolved_args(entry: dict, ids: dict[str, str]) -> list[str]:
    """Substitute the placeholders the palette would have filled at open time.

    A tab-resolving entry takes the tab the pane is NOT in, so the move it
    performs is a real one. `{text}` carries a space because that is the shape
    that breaks if the palette ever splits the typed name across argv elements —
    except as a branch name, which git refuses with one.

    Raises on a `resolve` this file does not know: defaulting it to a tab id
    would substitute a plausible argument into an entry never taught here and
    report ok, which is the silent staleness this check exists to remove.
    """
    args = entry["args"]
    table = dict(ids)
    table["{text}"] = "e2e-created" if flag_before(args, "{text}") == "--branch" else "e2e renamed"
    resolve = entry.get("resolve")
    if resolve is not None:
        picked = {
            "tab list": "{another tab}",
            "pane list": "{pane}",
            "workspace list": "{workspace}",
            "worktree list": {
                "--path": "{worktree path}",
                "--workspace": "{worktree workspace}",
            }.get(flag_before(args, "{}")),
        }.get(resolve)
        if picked is None:
            raise RuntimeError(
                f"{entry['id']}: resolve = {resolve!r} is not a list this "
                "harness knows how to pick a target from"
            )
        table["{}"] = ids[picked]

    return [table.get(a, a) for a in args]
