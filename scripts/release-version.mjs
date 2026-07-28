//! The single source of truth for the release version.
//!
//! Every script, test, and workflow reads the version from
//! `plugins/nodestorm/VERSION` rather than repeating the literal. Before this
//! existed a bump meant editing ~25 files, and one of them — the deliberately
//! wrong tag in `tests/release_gates.mjs` — silently turned *correct* at the
//! next version and disabled its own assertion.

import { readFile } from "node:fs/promises";
import path from "node:path";

export const root = path.resolve(import.meta.dirname, "..");

/** Semver release version, e.g. `1.0.1`. Used by Cargo, npm, and plugins. */
export async function releaseVersion() {
  const version = (await readFile(path.join(root, "plugins/nodestorm/VERSION"), "utf8")).trim();
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`plugins/nodestorm/VERSION is not a three-part version: ${version}`);
  }
  return version;
}

/**
 * The four-part form the Microsoft Store requires for an MSIX. The Store
 * reserves the revision field for its own rebuilds, so a submitted package
 * always ends in `.0`.
 */
export function msixVersion(version) {
  return `${version}.0`;
}

/**
 * `version` escaped for embedding in a RegExp. Escapes every metacharacter,
 * not just the dots — a partial escape is the classic way this helper rots
 * once someone passes it something other than a bare version.
 */
export function versionPattern(version) {
  return version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Partner Center's limit on the release-notes field of a Store listing. */
export const RELEASE_NOTES_LIMIT = 1500;

/**
 * The `CHANGELOG.md` body for `version`: everything between its `## [version]`
 * heading and the next release heading. This is what customers read in the
 * Store listing and on the GitHub release, so it is derived from one file
 * rather than retyped into each.
 */
export async function changelogSection(version) {
  const changelog = await readFile(path.join(root, "CHANGELOG.md"), "utf8");
  const heading = changelog.search(new RegExp(`^## \\[${versionPattern(version)}\\]`, "m"));
  if (heading === -1) {
    throw new Error(`CHANGELOG.md has no section for ${version}; add one before releasing.`);
  }
  const body = changelog.slice(changelog.indexOf("\n", heading) + 1);
  const next = body.search(/^## \[/m);
  return (next === -1 ? body : body.slice(0, next)).trim();
}

/**
 * A tag that can never match `version`, for tests that prove the tag gate
 * rejects a mismatch. Derived so it stays wrong across bumps.
 */
export function mismatchedTag(version) {
  return `v${version}-mismatched`;
}
