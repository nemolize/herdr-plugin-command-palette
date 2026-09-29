import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";
import { fileURLToPath } from "node:url";

import { releaseNotes } from "./release-notes.mjs";
import { planRelease } from "./release-plan.mjs";
import { checkVersions, readPackageVersion } from "./sync-version.mjs";

const repo = fileURLToPath(new URL("..", import.meta.url));
const FIXTURE_FILES = [
  "package.json",
  ".changeset/config.json",
  ".changeset/README.md",
  "CHANGELOG.md",
  "Cargo.toml",
  "Cargo.lock",
  "herdr-plugin.toml",
  "scripts/sync-version.mjs",
];

function git(cwd, ...args) {
  return execFileSync("git", ["-c", "user.name=fixture", "-c", "user.email=fixture@example.com", ...args], {
    cwd,
    encoding: "utf8",
  });
}

function versionPackages(cwd) {
  try {
    execFileSync("npm", ["run", "--silent", "version-packages"], { cwd, encoding: "utf8", stdio: "pipe" });
  } catch (error) {
    throw new Error(`version-packages failed:\n${error.stdout}${error.stderr}`);
  }
}

function occurrences(text, needle) {
  return text.split(needle).length - 1;
}

// The merge commit repeats the feature commit's subject in its body, the shape
// release-please collected twice (#41).
describe("changeset version on a fixture repository", () => {
  const SUMMARY = "Fixture: add a thing to the palette";
  let dir;
  let base;

  before(() => {
    dir = mkdtempSync(join(tmpdir(), "changesets-fixture-"));
    for (const file of FIXTURE_FILES) cpSync(join(repo, file), join(dir, file), { recursive: true });
    symlinkSync(join(repo, "node_modules"), join(dir, "node_modules"), "dir");
    writeFileSync(join(dir, ".gitignore"), "node_modules\n");
    git(dir, "init", "-q", "-b", "main");
    git(dir, "add", "-A");
    git(dir, "commit", "-qm", "chore: baseline");
    base = readPackageVersion(dir);

    git(dir, "switch", "-qc", "feat/fixture");
    writeFileSync(join(dir, ".changeset/fixture.md"), `---\n"herdr-command-palette": minor\n---\n\n${SUMMARY}\n`);
    git(dir, "add", "-A");
    git(dir, "commit", "-qm", `feat: ${SUMMARY}`);
    git(dir, "switch", "-q", "main");
    git(dir, "merge", "--no-ff", "-q", "feat/fixture", "-m", "Merge pull request #1 from feat/fixture", "-m", `feat: ${SUMMARY}`);

    versionPackages(dir);
  });

  after(() => rmSync(dir, { recursive: true, force: true }));

  test("bumps to the expected version", () => {
    const [major, minor] = base.split(".").map(Number);
    assert.equal(readPackageVersion(dir), `${major}.${minor + 1}.0`);
  });

  test("updates every version declaration", () => {
    assert.deepEqual(checkVersions(dir), []);
  });

  test("consumes the changeset", () => {
    assert.equal(existsSync(join(dir, ".changeset/fixture.md")), false);
  });

  test("writes one changelog entry despite the merge commit", () => {
    const changelog = readFileSync(join(dir, "CHANGELOG.md"), "utf8");
    assert.equal(occurrences(changelog, SUMMARY), 1);
    assert.equal(occurrences(releaseNotes(changelog, readPackageVersion(dir)), SUMMARY), 1);
  });

  test("a second run with nothing pending refuses and changes nothing", () => {
    git(dir, "add", "-A");
    git(dir, "commit", "-qm", "Version Packages");
    assert.throws(() => versionPackages(dir), /No unreleased changesets found/);
    assert.equal(git(dir, "status", "--porcelain"), "");
  });
});

describe("release notes", () => {
  const changelog = "# Changelog\n\n## 0.5.0\n\n### Minor Changes\n\n- abc1234: New thing\n\n## [0.4.0](x) (2026-09-28)\n\n* old\n";

  test("returns only the requested section", () => {
    assert.equal(releaseNotes(changelog, "0.5.0"), "### Minor Changes\n\n- abc1234: New thing\n");
  });

  test("refuses a version without a section", () => {
    assert.throws(() => releaseNotes(changelog, "0.6.0"), /no "## 0.6.0" section/);
  });

  test("refuses an empty section", () => {
    assert.throws(() => releaseNotes("# Changelog\n\n## 0.5.0\n\n## 0.4.0\n\n* old\n", "0.5.0"), /is empty/);
  });
});

describe("release plan", () => {
  const commit = "a".repeat(40);
  const draftSha = "b".repeat(40);
  const plan = (releases, tagExists = false) => planRelease({ version: "0.5.0", commit, tagExists, releases });

  test("a published release means nothing to do", () => {
    assert.deepEqual(plan([{ tag_name: "v0.5.0", draft: false, target_commitish: "main" }], true), {
      action: "skip",
      tag: "v0.5.0",
    });
  });

  // #43: while the assets build, the release is a draft and its tag does not
  // exist yet; a push in that window must neither draft again nor move on.
  test("a draft is rebuilt at its own commit", () => {
    assert.deepEqual(plan([{ tag_name: "v0.5.0", draft: true, target_commitish: draftSha }]), {
      action: "retry",
      tag: "v0.5.0",
      sha: draftSha,
    });
  });

  test("an unreleased version is drafted at the commit that set it", () => {
    assert.deepEqual(plan([{ tag_name: "v0.4.0", draft: false, target_commitish: "main" }]), {
      action: "create",
      tag: "v0.5.0",
      sha: commit,
    });
  });

  test("refuses a draft that targets a branch", () => {
    assert.throws(() => plan([{ tag_name: "v0.5.0", draft: true, target_commitish: "main" }]), /not a commit SHA/);
  });

  test("refuses two releases for one version", () => {
    const draft = { tag_name: "v0.5.0", draft: true, target_commitish: draftSha };
    assert.throws(() => plan([draft, draft]), /2 releases carry v0.5.0/);
  });

  test("refuses a draft whose tag already exists", () => {
    assert.throws(() => plan([{ tag_name: "v0.5.0", draft: true, target_commitish: draftSha }], true), /both exist/);
  });

  test("refuses a tag that has no release", () => {
    assert.throws(() => plan([], true), /exists without a release/);
  });
});
