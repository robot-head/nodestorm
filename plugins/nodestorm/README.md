# Nodestorm host package

This is the canonical Nodestorm package for Claude Code, Codex, OpenCode, and
Pi. It connects each host to the native app's local MCP endpoint at
`http://127.0.0.1:4747/mcp` and ships one shared skill for the graph, decision,
session, and export workflow.

The package never removes the native app or user sessions when uninstalled.
On an explicit Nodestorm request, the skill can offer the platform setup
script; installation and launch always require confirmation.
