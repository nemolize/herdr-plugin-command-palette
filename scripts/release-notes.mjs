import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function releaseNotes(changelog, version) {
  const lines = changelog.split("\n");
  const start = lines.findIndex((line) => line.trim() === `## ${version}`);
  if (start === -1) throw new Error(`CHANGELOG.md has no "## ${version}" section`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => line.startsWith("## "));
  const body = (end === -1 ? rest : rest.slice(0, end)).join("\n").trim();
  if (body === "") throw new Error(`CHANGELOG.md's "## ${version}" section is empty`);
  return `${body}\n`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const version = process.argv[2];
  if (!version || process.argv.length !== 3) {
    console.error("usage: node scripts/release-notes.mjs <version>");
    process.exit(2);
  }
  process.stdout.write(releaseNotes(readFileSync("CHANGELOG.md", "utf8"), version));
}
