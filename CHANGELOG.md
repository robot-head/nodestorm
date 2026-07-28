# Changelog

Notable changes per release, newest first. The section for the version in
`plugins/nodestorm/VERSION` is published verbatim as the Microsoft Store release
notes and as the GitHub release body, so write it for the people who read the
listing — not as a commit log. `node scripts/validate-release.mjs --release`
refuses to build a tag whose version has no section here, or whose section is
longer than the Store's 1500-character release-notes field.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.1] - 2026-07-26

### Added

- A session-wide Decisions panel. The open-decision count in the topbar is now
  a button: it lists every choice still waiting on you, and the prev/next
  arrows step through them without hunting for the node that carries each one.

### Changed

- Decisions can be made straight from the panel, so a long graph no longer
  means scrolling back to the card.

## [1.0.0] - 2026-07-22

### Added

- First public release. Nodestorm is a visual architecture canvas your coding
  agent draws on: it proposes components and edges over MCP, marks the choices
  it wants you to make, and waits for your answers before continuing.
- Human-in-the-loop decisions: the agent blocks on `await_decisions` while you
  pick options on the canvas, then receives them as structured data.
- Direct editing — add, rename, connect, and remove components yourself, with
  undo and redo. Agent-authored nodes are soft-removed so the agent sees the
  request.
- Named sessions: create, switch, rename, and compare two sessions side by
  side, with a timeline of everything that happened in each.
- Markdown export of the whole decision record, mermaid diagram included.
- An integrated terminal that starts agent sessions in a fresh worktree.
- Seven themes with light and dark modes, matched to the native title bar.
