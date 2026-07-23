# Nodestorm v1.0 release runbook

The release is deliberately blocked until all public identity and signing
prerequisites are real. No workflow substitutes unsigned downloads or example
identity values.

## One-time external prerequisites

1. Reserve **Nodestorm** in Microsoft Partner Center. Copy its exact Identity
   Name, Publisher, Publisher display name, Product ID, application ID, and
   execution alias into `packaging/windows/store-identity.json`, following
   `store-identity.example.json`. Run
   `node scripts/configure-store.mjs packaging/windows/store-identity.json`,
   then commit both the public identity file and generated plugin `store.json`
   so Git-installed Claude Code and Codex plugins receive the real Product ID.
2. Configure repository secrets `APPLE_DEVELOPER_ID_P12_BASE64`,
   `APPLE_DEVELOPER_ID_P12_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
   `APPLE_TEAM_ID`, and `APPLE_APP_SPECIFIC_PASSWORD`.
3. Configure npm trusted publishing for the `nodestorm` package and this
   repository's `release-publish.yml` workflow.

`node scripts/validate-release.mjs --release --tag v1.0.0` is the preflight.
It rejects missing/example Partner Center values or version drift.

## Stage 1: build a draft

Push tag `v1.0.0`. `release-build.yml` builds Linux x64/arm64 on native Ubuntu
runners, macOS x64/arm64 on native Apple runners, and Windows x64/arm64 on
native Windows runners. It performs native version and MCP test gates,
notarizes and staples macOS, attests Linux/macOS artifacts, and creates a draft
GitHub release containing only the public Linux/macOS files and SHA-256 sums.

The unsigned x64/arm64 MSIX bundle is retained only as a private workflow
artifact. Download it from the workflow run and submit it to Partner Center.
The workflow also creates a disposable self-signed copy solely to test MSIX
installation and the execution alias in CI; that copy is never uploaded.

## Stage 2: publish after Store certification

After Microsoft Store reports version `1.0.0.0` live, dispatch
`release-publish.yml` with Store version `1.0.0.0` and confirmation
`publish-v1.0.0`. The workflow independently checks the live Store listing,
injects the exact Store Product ID into the npm setup package, publishes npm
through trusted publishing with provenance, and makes the GitHub draft public.

Run fresh-profile acceptance on Claude Code, Codex, OpenCode, and Pi before
announcing the release: install the plugin, explicitly request Nodestorm,
approve setup and launch, propose a graph, deliver a choice through
`await_decisions`, update the graph, and export Markdown.
