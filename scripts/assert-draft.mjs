// Without its draft, action-gh-release would create a public release tagged at
// the default branch's head rather than at the commit the assets were built from.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { parseReleases } from "./release-plan.mjs";

export function assertDraft({ tag, ref, releases }) {
  const drafts = releases.filter((release) => release.draft && release.tag_name === tag);
  if (drafts.length !== 1 || drafts[0].target_commitish !== ref) {
    const found = drafts.map((release) => release.target_commitish).join(", ") || "none";
    throw new Error(`expected one draft ${tag} targeting ${ref}, found: ${found}`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [flagTag, tag, flagRef, ref] = process.argv.slice(2);
  if (flagTag !== "--tag" || flagRef !== "--ref" || !tag || !ref || process.argv.length !== 6) {
    console.error("usage: node scripts/assert-draft.mjs --tag <tag> --ref <sha> < releases.ndjson");
    process.exit(2);
  }
  try {
    assertDraft({ tag, ref, releases: parseReleases(readFileSync(0, "utf8")) });
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}
