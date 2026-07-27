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
2. Update the files that carry their own copy — `Cargo.toml` (then `cargo check`
   to refresh `Cargo.lock`), `plugins/nodestorm/package.json` (and its lock),
   both plugin manifests, `packaging/macos/Info.plist`, `pi.js`, and
   `msixVersion` in `packaging/windows/store-identity.json` (which is
   `<version>.0`).
3. Run `node scripts/configure-store.mjs packaging/windows/store-identity.json`
   to regenerate the plugin's `store.json`.
4. Run `node scripts/validate-release.mjs --release --tag v<version>` as the
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
4. Associate an Azure AD application with the Partner Center account, grant it
   the **Manager** role, and configure repository secrets `MSSTORE_TENANT_ID`,
   `MSSTORE_CLIENT_ID`, and `MSSTORE_CLIENT_SECRET`. These authorize the
   automated Store submission in stage 1.
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

The workflow also creates a disposable self-signed copy solely to test MSIX
installation and the execution alias in CI; that copy is never uploaded.

> **Once `store-submit` runs, finish the submission through the API.** The Store
> submission API refuses to commit a submission that was edited in Partner
> Center after being created by the API — such a submission has to be deleted
> and recreated. `submit-store.mjs` deletes a stale pending submission on its
> next run, so re-tagging recovers cleanly; editing in the web UI does not.

`store-submit` only runs for tag pushes. A manual `workflow_dispatch` rebuilds
artifacts and never touches the live listing.

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
