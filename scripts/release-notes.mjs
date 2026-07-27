//! Print the `CHANGELOG.md` section for the release version to stdout, so the
//! GitHub release body and the Store release notes are the same text.
//!
//! Usage: `node scripts/release-notes.mjs > notes.md`

import { changelogSection, releaseVersion } from "./release-version.mjs";

process.stdout.write(`${await changelogSection(await releaseVersion())}\n`);
