"""Read what herdr's JSON answer to one call holds, for the E2E scripts.

Pure: stdout text in, a value or a named RuntimeError out, so it is checked
without a herdr (see `catalog-e2e-test.py`). Every E2E script reads a herdr
answer through here, so a missing or reshaped field reads the same whichever
script hit it.
"""

from __future__ import annotations

import json


def parse_envelope(text: str) -> dict | None:
    try:
        body = json.loads(text)
    except ValueError:
        return None
    return body if isinstance(body, dict) else None


def answer_field(
    call: str, node: object, path: str, kind: type = str, within: str = ""
) -> object:
    """The `kind` value at dotted `path` under `node`, the part of herdr's answer
    to `call` found at `within`. A reshaped answer raises a RuntimeError naming the
    call and the key, which `catalog-e2e.py`'s `main` reports as that entry's BROKE
    line where a KeyError or TypeError would abort the whole run (#173)."""
    walked = within
    for key in path.split("."):
        walked = f"{walked}.{key}" if walked else key
        if not isinstance(node, dict) or key not in node:
            raise RuntimeError(f"herdr {call} answered without {walked}")
        node = node[key]
    if not isinstance(node, kind):
        raise RuntimeError(
            f"herdr {call} answered {walked} as a "
            f"{type(node).__name__}, not a {kind.__name__}"
        )
    return node


def answer(call: str, stdout: str, path: str, kind: type = str) -> object:
    """`answer_field` on the `result` of herdr's answer to `call`; an empty
    `path` is the `result` itself."""
    body = parse_envelope(stdout)
    if body is None:
        raise RuntimeError(
            f"herdr {call} answered without a JSON envelope: {stdout.strip()[:200]}"
        )
    return answer_field(call, body, f"result.{path}" if path else "result", kind)
