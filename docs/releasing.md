# Nodestorm release runbook

The release is deliberately blocked until all public identity and signing
prerequisites are real. No workflow substitutes unsigned downloads or example
identity values.

The version lives in exactly one place: **`plugins/nodestorm/VERSION`**. Every
script, test, and workflow derives from it, and
`node scripts/validate-release.mjs` fails if any of the files that must agree
(Cargo, npm, plugin manifests, `Info.plist`, Store identity) has drifted. A
release-gate test also fails if any workflow, script, or test starts naming the
version literally again.

## Cutting a release

1. Set the new version in `plugins/nodestorm/VERSION`.
2. Write the `CHANGELOG.md` section for it (`## [<version>] - YYYY-MM-DD`). That
   text is published verbatim as the Microsoft Store release notes and as the
   GitHub release body, so write it for the people reading the listing, not as a
   commit log. Validation rejects a tag whose version has no section, or whose
   section exceeds the Store's 1500-character release-notes field.
3. Update the files that carry their own copy — `Cargo.toml` (then `cargo check`
   to refresh `Cargo.lock`), `plugins/nodestorm/package.json` (and its lock),
   both plugin manifests, `packaging/macos/Info.plist`, `pi.js`, and
   `msixVersion` in `packaging/windows/store-identity.json` (which is
   `<version>.0`).
4. Run `node scripts/configure-store.mjs packaging/windows/store-identity.json`
   to regenerate the plugin's `store.json`.
5. Run `node scripts/validate-release.mjs --release --tag v<version>` as the
   preflight. It rejects missing/example Partner Center values or version drift.

## One-time external prerequisites

1. Reserve **Nodestorm** in Microsoft Partner Center. Copy its exact Identity
   Name, Publisher, Publisher display name, Product ID, application ID, and
   execution alias into `packaging/windows/store-identity.json`, following
   `store-identity.example.json`. Run
   `node scripts/configure-store.mjs packaging/windows/store-identity.json`,
   then commit both the public identity file and generated plugin `store.json`
   so Git-installed Claude Code and Codex plugins receive the real Product ID.
2. Complete **one submission by hand in Partner Center**, including the age
   ratings questionnaire. The submission API can only update an app that has
   already shipped once; it cannot create the first submission.
3. Configure repository secrets `APPLE_DEVELOPER_ID_P12_BASE64`,
   `APPLE_DEVELOPER_ID_P12_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
   `APPLE_TEAM_ID`, and `APPLE_APP_SPECIFIC_PASSWORD`.
4. *(Optional)* Associate a Microsoft Entra ID (Azure AD) application with the
   Partner Center account, grant it the **Manager** role, and configure
   repository secrets `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, and
   `MSSTORE_CLIENT_SECRET`. These authorize the automated Store submission in
   stage 1. Skip it and stage 1 still succeeds, printing manual upload steps
   instead — set up all three later to automate.

   **No company, Microsoft 365 subscription, or existing Entra tenant is
   required.** A personal Microsoft account with an individual Partner Center
   account is enough. If you have no directory, create a free one from inside
   Partner Center: *gear icon → Account settings → Tenants → Create Microsoft
   Entra ID*. It asks for a `<name>.onmicrosoft.com` domain, a contact email,
   and a global-administrator user to create — no business verification, no
   Azure subscription, no charge. Microsoft's prerequisites say so directly:
   "you can [create a new Azure AD in Partner Center] for no additional
   charge."

   The directory exists only to hold the service principal the workflow
   authenticates as. Note the global-admin account it creates is a *new*
   identity in that tenant (`you@<name>.onmicrosoft.com`), not your existing
   personal Microsoft account — keep its credentials, since only a Partner
   Center user with the **Manager** role can associate tenants later.

   The workflow uses the client-credentials (app-only) grant, so the MFA
   enforcement that applies to App+User Partner Center API calls from
   1 April 2026 does not affect it.
5. Configure npm trusted publishing for the `nodestorm` package and this
   repository's `release-publish.yml` workflow.

## Stage 1: build a draft and submit to the Store

Push tag `v<version>`. `release-build.yml` builds Linux x64/arm64 on native
Ubuntu runners, macOS x64/arm64 on native Apple runners, and Windows x64/arm64
on native Windows runners. It performs native version and MCP test gates,
notarizes and staples macOS, attests Linux/macOS artifacts, and creates a draft
GitHub release containing only the public Linux/macOS files and SHA-256 sums.

The unsigned x64/arm64 MSIX bundle is retained as a private workflow artifact
and **submitted automatically** to the Store by the `store-submit` job, which
uses the Store submission API to clone the last published submission, retire
its packages, upload the new bundle, and commit. Microsoft signs the package
during certification; the repository never holds a Store signing key.

The submission carries a fresh listing, not just a fresh package. Release notes
come from the `CHANGELOG.md` section for this version, and the `store-assets`
job regenerates the artwork from the build that is shipping: it widens the
runner desktop to 1920x1080, runs `verify-windows.ps1` at 1600x1000 for the
screenshots named in `packaging/windows/store-listing.json`, records the demo
with `record-demo.ps1`, and pads that video and its poster to the 1920x1080 the
Store demands for a trailer. Everything travels in the same zip as the bundle.
Hand-uploaded store logos are left alone — only the previous screenshots are
retired.

`store-assets` runs on manual dispatch too, and that is the way to rehearse it:
a dispatch proves the capture works and produces a downloadable `store-assets`
artifact to eyeball, while `store-submit` stays tag-gated and never fires. If
capture fails, no submission and no GitHub draft happen — a listing that cannot
be regenerated blocks the release rather than shipping last version's pictures.
Updating the listing copy (screenshot order, captions, trailer title) means
editing `packaging/windows/store-listing.json`, not Partner Center.

**Store credentials are optional.** With none of the three `MSSTORE_*` secrets
set, `store-submit` skips: it annotates the run with a warning, writes the
manual upload steps to the job summary, and exits 0, so an unconfigured
repository still gets a fully green release build. With **all three** set it
submits, and a genuine API error fails the job. A *partial* set fails
immediately — that means someone believes submission is wired up, and silently
skipping would quietly not ship the release.

The workflow also creates a disposable self-signed copy solely to test MSIX
installation and the execution alias in CI; that copy is never uploaded.

> **Once `store-submit` runs, finish the submission through the API.** The Store
> submission API refuses to commit a submission that was edited in Partner
> Center after being created by the API — such a submission has to be deleted
> and recreated. `submit-store.mjs` deletes a stale pending submission on its
> next run, so re-tagging recovers cleanly; editing in the web UI does not.

`store-submit` only runs for tag pushes. A manual `workflow_dispatch` rebuilds
artifacts and never touches the live listing.

> **If the app is on Pricing Version 2, `store-submit` may fail to commit.**
> Microsoft documents that the submission API returns an unknown tier for the
> pricing part of such products; other modules stay usable. Because
> `submit-store.mjs` clones the previous submission wholesale and PUTs it back,
> a degraded pricing part travels with it. Check whether *Pricing and
> availability* shows a **Review price per market** button — if it does, expect
> this. Recovery is the manual path below; the automation is an optimization,
> never the only route.

### Manual fallback

The Store submission API is a convenience, not a dependency. If `store-submit`
fails for any reason, download the `windows-store-msixbundle` artifact from the
build run and upload it to the product in Partner Center by hand. Everything
downstream — certification, stage 2, `winget` verification — is identical. Only
delete any pending API-created submission first, so the manual one starts clean.

## Stage 2: publish after Store certification

Certification takes hours to days. After Microsoft Store reports the version
live, dispatch `release-publish.yml` with the release version (e.g. `2.3.4`)
and confirmation `publish-v<version>`. The workflow independently checks the
live Store listing over `winget`, verifies the installed package's identity,
publisher, and `<version>.0` package version, injects the exact Store Product
ID into the npm setup package, publishes npm through trusted publishing with
provenance, and makes the GitHub draft public.

Run fresh-profile acceptance on Claude Code, Codex, OpenCode, and Pi before
announcing the release: install the plugin, explicitly request Nodestorm,
approve setup and launch, propose a graph, deliver a choice through
`await_decisions`, update the graph, and export Markdown.
