import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { classify } from "./actionlint-probe.mjs";

const probe = fileURLToPath(new URL("./actionlint-probe.mjs", import.meta.url));

function run(...args) {
  return spawnSync("node", [probe, ...args], { encoding: "utf8", timeout: 60_000 });
}

const FORMAT_FINDING = {
  kind: "workflow-call",
  line: 6,
  message: 'reusable workflow call "$/x.yml" at "uses" is not following the format "owner/repo/path/to/workflow.yml@ref"',
};

test("passes while actionlint rejects the `$/` form", () => {
  const result = run();

  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stderr);
});

test("fails with the removal steps once actionlint accepts the form", () => {
  const result = run("--uses", "./.github/workflows/callee.yml");

  assert.equal(result.status, 1, result.stderr);
  assert.ok(result.stderr.includes("release-plan.yml"), result.stderr);
  assert.ok(result.stderr.includes("# zizmor: ignore[self-repository]"), result.stderr);
});

test("fails as undecided when actionlint cannot run", () => {
  const result = run("--actionlint", "actionlint-that-does-not-exist");

  assert.equal(result.status, 2, result.stderr);
  assert.ok(result.stderr.includes("could not tell"), result.stderr);
});

test("reads a lone format finding on the `uses` line as a rejection", () => {
  assert.equal(classify({ status: 1, stdout: JSON.stringify([FORMAT_FINDING]) }), "rejected");
});

test("reads a clean run as an acceptance", () => {
  assert.equal(classify({ status: 0, stdout: "[]" }), "accepted");
});

for (const [name, result] of [
  ["a finding of another rule", { status: 1, stdout: JSON.stringify([{ ...FORMAT_FINDING, kind: "syntax-check" }]) }],
  [
    "an unreadable callee, which the same rule reports",
    { status: 1, stdout: JSON.stringify([{ ...FORMAT_FINDING, message: "could not read reusable workflow file" }]) },
  ],
  ["a finding on another line", { status: 1, stdout: JSON.stringify([{ ...FORMAT_FINDING, line: 2 }]) }],
  ["an extra finding", { status: 1, stdout: JSON.stringify([FORMAT_FINDING, FORMAT_FINDING]) }],
  ["exit 3", { status: 3, stdout: "could not read" }],
  ["a failed start", { status: null, stdout: "" }],
  ["a clean exit that printed findings", { status: 0, stdout: JSON.stringify([FORMAT_FINDING]) }],
]) {
  test(`reads ${name} as undecided`, () => {
    assert.equal(classify(result), "undecided");
  });
}
