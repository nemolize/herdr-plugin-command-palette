import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const CALLEE = "./.github/workflows/callee.yml";
const SELF_REPOSITORY = "$/.github/workflows/callee.yml";
const USES_LINE = 6;

// The workflow-call rule also reports an unreadable callee, which says
// nothing about `$/`, so a rejection must be about the value's format.
export function classify({ status, stdout }) {
  let findings;
  try {
    findings = JSON.parse(stdout);
  } catch {
    return "undecided";
  }
  if (!Array.isArray(findings)) return "undecided";
  if (status === 0 && findings.length === 0) return "accepted";
  const [finding] = findings;
  const rejected =
    status === 1 &&
    findings.length === 1 &&
    finding.kind === "workflow-call" &&
    finding.line === USES_LINE &&
    finding.message.includes("is not following the format");
  return rejected ? "rejected" : "undecided";
}

function probe(uses, actionlint) {
  const fixture = mkdtempSync(join(tmpdir(), "actionlint-probe-"));
  try {
    const workflows = join(fixture, ".github", "workflows");
    mkdirSync(workflows, { recursive: true });
    writeFileSync(
      join(workflows, "callee.yml"),
      [
        "name: callee",
        "on: workflow_call",
        "permissions: {}",
        "jobs:",
        "  run:",
        "    runs-on: ubuntu-24.04",
        "    timeout-minutes: 5",
        "    steps:",
        "      - run: echo",
        "",
      ].join("\n"),
    );
    const caller = join(workflows, "caller.yml");
    writeFileSync(
      caller,
      [
        "name: caller",
        "on: push",
        "permissions: {}",
        "jobs:",
        "  call:",
        `    uses: ${uses}`,
        "",
      ].join("\n"),
    );
    return spawnSync(actionlint, ["-format", "{{json .}}", caller], {
      cwd: fixture,
      encoding: "utf8",
      timeout: 60_000,
    });
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
}

function main() {
  const { values } = parseArgs({
    options: {
      uses: { type: "string", default: SELF_REPOSITORY },
      actionlint: { type: "string", default: "actionlint" },
    },
  });
  const result = probe(values.uses, values.actionlint);
  const verdict = result.error ? "undecided" : classify(result);

  if (verdict === "rejected") {
    console.log(
      "actionlint still rejects the `$/` self-repository form, so release-plan.yml keeps its zizmor ignore.",
    );
    return 0;
  }
  if (verdict === "accepted") {
    console.error(
      [
        "actionlint now accepts the `$/` self-repository form.",
        "In .github/workflows/release-plan.yml, change the `Assets` job's `uses: ./.github/workflows/release.yml` to `uses: $/.github/workflows/release.yml`,",
        "delete its `# zizmor: ignore[self-repository]` and the comment above it about actionlint,",
        "then delete this probe: the `actionlint-probe` recipes in the justfile, the `ci.yml` steps that run them, scripts/actionlint-probe*.mjs and the two docs/ci.md rows.",
      ].join("\n"),
    );
    return 1;
  }
  console.error(
    [
      "The probe could not tell whether actionlint accepts the `$/` form; this is neither verdict.",
      result.error ? String(result.error) : `exit ${result.status}`,
      result.stdout,
      result.stderr,
    ].join("\n"),
  );
  return 2;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(main());
}
