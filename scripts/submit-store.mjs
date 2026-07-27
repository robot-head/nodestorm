//! Submit a built MSIX bundle to the Microsoft Store via the Store submission
//! API, so a tagged release reaches certification without a manual Partner
//! Center upload.
//!
//! Usage: `node scripts/submit-store.mjs <package.zip>`
//!
//! The zip must contain the `.msixbundle`. Credentials come from the
//! environment: `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`,
//! `MSSTORE_CLIENT_SECRET` — an Azure AD application associated with the
//! Partner Center account and granted the **Manager** role.
//!
//! Two constraints from the API, both load-bearing:
//!
//! - A submission created through the API must only ever be *changed* through
//!   the API. Editing it in Partner Center leaves it uncommittable and it has
//!   to be deleted and recreated. So: once this runs, finish in the API or
//!   delete the pending submission.
//! - The app must already have one manually completed submission, including
//!   the age-ratings questionnaire. This script updates an existing listing;
//!   it cannot create one.
//!
//! Docs: https://learn.microsoft.com/en-us/windows/uwp/monetize/manage-app-submissions

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";

import { msixVersion, releaseVersion, root } from "./release-version.mjs";

const API = "https://manage.devcenter.microsoft.com/v1.0/my";
/** Certification takes hours; this only waits for the commit to be accepted. */
const COMMIT_POLL_ATTEMPTS = 40;
const COMMIT_POLL_INTERVAL_MS = 15_000;

const packageZip = process.argv[2];
assert.ok(packageZip, "usage: node scripts/submit-store.mjs <package.zip>");

const { MSSTORE_TENANT_ID, MSSTORE_CLIENT_ID, MSSTORE_CLIENT_SECRET } = process.env;
for (const [name, value] of Object.entries({
  MSSTORE_TENANT_ID,
  MSSTORE_CLIENT_ID,
  MSSTORE_CLIENT_SECRET,
})) {
  assert.ok(value, `${name} is not set; configure the Azure AD application secrets`);
}

const identity = JSON.parse(
  await readFile(path.join(root, "packaging/windows/store-identity.json"), "utf8"),
);
const version = await releaseVersion();
assert.equal(
  identity.msixVersion,
  msixVersion(version),
  "store-identity.json msixVersion is out of step with plugins/nodestorm/VERSION",
);

/** The Store ID from Partner Center, e.g. `9PL98XH1NQZB`. */
const applicationId = identity.productId;

async function token() {
  const body = new URLSearchParams({
    grant_type: "client_credentials",
    client_id: MSSTORE_CLIENT_ID,
    client_secret: MSSTORE_CLIENT_SECRET,
    resource: "https://manage.devcenter.microsoft.com",
  });
  const response = await fetch(
    `https://login.microsoftonline.com/${MSSTORE_TENANT_ID}/oauth2/token`,
    { method: "POST", body },
  );
  if (!response.ok) {
    throw new Error(`Azure AD token request failed: ${response.status} ${await response.text()}`);
  }
  return (await response.json()).access_token;
}

const accessToken = await token();

async function api(method, endpoint, body) {
  const response = await fetch(`${API}${endpoint}`, {
    method,
    headers: {
      Authorization: `Bearer ${accessToken}`,
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`${method} ${endpoint} failed: ${response.status} ${text}`);
  }
  return text ? JSON.parse(text) : {};
}

// A pending submission blocks a new one. Reclaim it rather than failing --
// a previous run that died between create and commit would otherwise wedge
// every later release until someone cleaned up by hand.
const app = await api("GET", `/applications/${applicationId}`);
if (app.pendingApplicationSubmission?.id) {
  const stale = app.pendingApplicationSubmission.id;
  console.log(`Deleting stale pending submission ${stale}.`);
  await api("DELETE", `/applications/${applicationId}/submissions/${stale}`);
}

// Create: clones the last published submission, so listing text, pricing, and
// age ratings carry over untouched and only the packages change.
const submission = await api("POST", `/applications/${applicationId}/submissions`);
console.log(`Created submission ${submission.id} for ${applicationId}.`);

const bundleName = path.basename(packageZip).replace(/\.zip$/, ".msixbundle");
submission.applicationPackages = [
  // Retire what the previous submission shipped...
  ...submission.applicationPackages.map((pkg) => ({ ...pkg, fileStatus: "PendingDelete" })),
  // ...and add this build, named by its path inside the uploaded zip.
  { fileName: bundleName, fileStatus: "PendingUpload" },
];

await api("PUT", `/applications/${applicationId}/submissions/${submission.id}`, submission);

// The SAS URI is a single block blob: one PUT of the whole zip.
const archive = await readFile(packageZip);
const upload = await fetch(submission.fileUploadUrl.replace("+", "%2B"), {
  method: "PUT",
  headers: { "x-ms-blob-type": "BlockBlob" },
  body: archive,
});
if (!upload.ok) {
  throw new Error(`Package upload failed: ${upload.status} ${await upload.text()}`);
}
console.log(`Uploaded ${bundleName} (${archive.length} bytes).`);

await api("POST", `/applications/${applicationId}/submissions/${submission.id}/commit`);
console.log("Commit requested; waiting for the Store to accept it.");

for (let attempt = 0; attempt < COMMIT_POLL_ATTEMPTS; attempt++) {
  const { status, statusDetails } = await api(
    "GET",
    `/applications/${applicationId}/submissions/${submission.id}/status`,
  );
  if (status === "CommitFailed") {
    throw new Error(`Store rejected the commit: ${JSON.stringify(statusDetails, null, 2)}`);
  }
  if (status !== "CommitStarted") {
    console.log(`Submission ${submission.id} accepted: ${status}. Certification continues in Partner Center.`);
    process.exit(0);
  }
  await new Promise((resolve) => setTimeout(resolve, COMMIT_POLL_INTERVAL_MS));
}

throw new Error(
  `Commit still pending after ${(COMMIT_POLL_ATTEMPTS * COMMIT_POLL_INTERVAL_MS) / 1000}s; check Partner Center.`,
);
