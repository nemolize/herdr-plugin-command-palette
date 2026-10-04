import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, test } from "node:test";
import { fileURLToPath } from "node:url";

const justfile = fileURLToPath(new URL("../justfile", import.meta.url));
const CHECKOUT_SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1";
const CHECKOUT_TAG = "v7.0.1";
let fixture;

beforeEach(() => {
  fixture = mkdtempSync(join(tmpdir(), "zizmor-fixture-"));
  mkdirSync(join(fixture, ".github", "workflows"), { recursive: true });
});

afterEach(() => {
  rmSync(fixture, { recursive: true, force: true });
});

function audit(usesLine) {
  const workflow = join(fixture, ".github", "workflows", "fixture.yml");
  const lines = [
    "name: fixture",
    "on: push",
    "permissions: {}",
    "jobs:",
    "  fixture:",
    "    runs-on: ubuntu-24.04",
    "    timeout-minutes: 5",
    "    steps:",
    `      - ${usesLine}`,
    "        with:",
    "          persist-credentials: false",
  ];
  writeFileSync(workflow, `${lines.join("\n")}\n`);
  const result = spawnSync("just", ["--justfile", justfile, "zizmor", fixture], {
    encoding: "utf8",
    timeout: 60_000,
  });
  return { result, location: `${workflow}:${lines.indexOf(`      - ${usesLine}`) + 1}` };
}

test("rejects a SHA pin with no version comment", () => {
  const { result, location } = audit(`uses: actions/checkout@${CHECKOUT_SHA}`);

  assert.equal(result.error, undefined);
  assert.ok(
    result.stderr.includes(`${location}: the version comment must be exactly the pinned tag`),
    result.stderr,
  );
  assert.notEqual(result.status, 0);
});

test("accepts a SHA pin whose comment is its tag", () => {
  const { result } = audit(`uses: actions/checkout@${CHECKOUT_SHA} # ${CHECKOUT_TAG}`);

  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stderr);
});
