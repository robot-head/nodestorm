# Nodestorm privacy

Nodestorm stores architecture sessions and preferences locally in the current
user's platform data directory. The native app exposes its MCP server only on
the loopback address `127.0.0.1` and does not include telemetry or a hosted
sync service.

The Claude Code, Codex, OpenCode, and Pi adapters connect to that loopback MCP
endpoint. They do not send session contents to Nodestorm infrastructure.
Your selected AI host and model provider may process tool inputs and outputs
under their own terms.

First-use setup contacts Microsoft Store on Windows, GitHub Releases and
GitHub artifact attestation services on Linux, or GitHub Releases on macOS to
obtain and verify the native app. It does not request administrator access or
modify PATH.

Removing a Nodestorm plugin does not remove the native app, preferences, or
sessions. Delete those local files separately if you want to erase them.

Security and privacy reports can be filed at
<https://github.com/robot-head/nodestorm/issues>.
