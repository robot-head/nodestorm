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

/** `version` escaped for embedding in a RegExp. */
export function versionPattern(version) {
  return version.replace(/\./g, "\\.");
}

/**
 * A tag that can never match `version`, for tests that prove the tag gate
 * rejects a mismatch. Derived so it stays wrong across bumps.
 */
export function mismatchedTag(version) {
  return `v${version}-mismatched`;
}
