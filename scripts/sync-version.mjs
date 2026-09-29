// package.json holds the version Changesets bumps; every other declaration copies it.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const CRATE = "herdr-command-palette";

export const DECLARATIONS = [
  {
    file: "Cargo.toml",
    // The [package] table's own version, never a dependency's.
    pattern: /(^\[package\]\n(?:(?!\[)[^\n]*\n)*?version\s*=\s*")([^"]+)"/m,
  },
  {
    file: "Cargo.lock",
    pattern: new RegExp(`(^\\[\\[package\\]\\]\\nname = "${CRATE}"\\nversion = ")([^"]+)"`, "m"),
  },
  {
    file: "herdr-plugin.toml",
    // Top level only: the first table header ends the search.
    pattern: /(^(?:(?!\[)[^\n]*\n)*?version\s*=\s*")([^"]+)"/,
  },
];

export function readPackageVersion(root) {
  const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  if (typeof version !== "string" || version === "") {
    throw new Error("package.json has no version");
  }
  return version;
}

function locate(root, { file, pattern }) {
  const text = readFileSync(join(root, file), "utf8");
  const match = pattern.exec(text);
  if (!match) throw new Error(`${file}: version declaration not found`);
  return { text, match };
}

export function checkVersions(root) {
  const expected = readPackageVersion(root);
  return DECLARATIONS.flatMap((declaration) => {
    const found = locate(root, declaration).match[2];
    return found === expected ? [] : [`${declaration.file} has ${found}, package.json has ${expected}`];
  });
}

export function syncVersions(root) {
  const version = readPackageVersion(root);
  for (const declaration of DECLARATIONS) {
    const { text, match } = locate(root, declaration);
    const next = text.slice(0, match.index) + `${match[1]}${version}"` + text.slice(match.index + match[0].length);
    if (next !== text) writeFileSync(join(root, declaration.file), next);
  }
  return version;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = process.cwd();
  if (process.argv[2] === "--check") {
    const mismatches = checkVersions(root);
    for (const line of mismatches) console.error(line);
    process.exit(mismatches.length === 0 ? 0 : 1);
  } else if (process.argv.length === 2) {
    console.log(syncVersions(root));
  } else {
    console.error("usage: node scripts/sync-version.mjs [--check]");
    process.exit(2);
  }
}
