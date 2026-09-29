// Decides from releases, drafts included: a draft has no tag until every asset
// is attached and it is published, so a tag alone never means "released".

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SHA = /^[0-9a-f]{40}$/;

export function planRelease({ version, commit, tagExists, releases }) {
  const tag = `v${version}`;
  const matching = releases.filter((release) => release.tag_name === tag);
  if (matching.length > 1) {
    throw new Error(`${matching.length} releases carry ${tag}; delete the extra drafts`);
  }
  const [release] = matching;
  if (release && !release.draft) return { action: "skip", tag };
  if (release && tagExists) {
    throw new Error(`draft ${tag} and a tag ${tag} both exist; publish or delete one by hand`);
  }
  if (release) {
    if (!SHA.test(release.target_commitish)) {
      throw new Error(`draft ${tag} targets "${release.target_commitish}", not a commit SHA`);
    }
    return { action: "retry", tag, sha: release.target_commitish };
  }
  if (tagExists) {
    // Publishing onto it would attach assets built from `commit` to whatever
    // commit the tag names.
    throw new Error(`tag ${tag} exists without a release; publish it by hand or delete the tag`);
  }
  if (!SHA.test(commit)) throw new Error(`commit "${commit}" is not a commit SHA`);
  return { action: "create", tag, sha: commit };
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (!["--version", "--commit", "--tag-exists"].includes(flag) || value === undefined) {
      throw new Error("usage: node scripts/release-plan.mjs --version <v> --commit <sha> --tag-exists <true|false> < releases.ndjson");
    }
    args[flag.slice(2)] = value;
  }
  if (!["true", "false"].includes(args["tag-exists"])) throw new Error("--tag-exists must be true or false");
  return { version: args.version, commit: args.commit, tagExists: args["tag-exists"] === "true" };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const releases = readFileSync(0, "utf8")
      .split("\n")
      .filter((line) => line.trim() !== "")
      .map((line) => JSON.parse(line));
    const plan = planRelease({ ...parseArgs(process.argv.slice(2)), releases });
    for (const [key, value] of Object.entries(plan)) console.log(`${key}=${value}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
