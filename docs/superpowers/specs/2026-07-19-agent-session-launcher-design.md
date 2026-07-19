# Agent Session Launcher Design

**Date:** 2026-07-19

**Status:** Approved

## Goal

Let a user create and start a Nodestorm-backed coding-agent session directly
from the desktop app. The user chooses Claude Code, Codex, OpenCode, or Pi,
runs it locally or on a Linux SSH host, and starts from either a new branch in
the selected checkout or, by default, a new branch in a sibling Git worktree.
The agent runs interactively in the system terminal.

Embedded provider APIs, an embedded terminal, custom agent command templates,
Windows SSH targets, and process transcript capture are outside this version.

## User Experience

The existing canvas-only **Create new session** action remains available. Two
new actions open the agent launcher:

- **Start agent** in the session menu.
- **Start an agentic session** in the empty state.

The launcher collects:

- Session name.
- Initial task.
- Agent: Claude Code, Codex, OpenCode, or Pi.
- Target: Local or SSH.
- Repository path. For SSH, this is a path on the remote Linux host.
- Branch name, initially `nodestorm/<session-slug>`.
- Git mode:
  - Create the branch in the existing checkout.
  - Create the branch in a new worktree; this is the default.
- Worktree path when that mode is selected, initially
  `<repository-parent>/<repository-name>-worktrees/<branch>` and editable.
- SSH host alias when SSH is selected.

The initial defaults are Local, Claude Code, and a new-worktree branch. Session
name changes update the generated branch name and worktree path until the user
edits those generated fields. Switching to SSH makes it explicit that the
repository and worktree paths belong to the remote host.

Literal, non-wildcard `Host` aliases from `~/.ssh/config` are offered as
suggestions. The alias field remains editable so aliases supplied by `Include`
files or patterns can still be entered. Nodestorm does not edit SSH
configuration.

## Architecture

A new launcher module owns the launch domain:

- Request and enum types for agent, target, and Git mode.
- Field and Git validation.
- Local Git preparation.
- Remote POSIX command construction and escaping.
- Agent-specific interactive command arguments.
- Initial prompt composition.
- Platform terminal command construction and launch.

A separate Dioxus component owns the dialog, draft state, progress, warnings,
and errors. It calls the launcher module but does not build shell or Git
commands. The existing `Sessions` manager remains the sole owner of session
creation, switching, and persistence.

No additional Rust dependency is required. Nodestorm uses the installed
`git`, `ssh`, agent CLI, and platform terminal programs.

## Launch Request

The domain request contains:

- `session_name`
- `task`
- `agent`: Claude, Codex, OpenCode, or Pi
- `target`: Local or an SSH alias
- `repository`
- `branch`
- `git_mode`: existing checkout or new worktree
- `worktree_path` when applicable
- the local Nodestorm MCP port

The request produces a prepared working directory, Nodestorm session slug,
agent identity, and terminal launch command. Agent identity is stable within
the launch and combines the selected agent with the created session slug.

## Local Launch Flow

Local submission runs in this order:

1. Validate all required fields.
2. Confirm `git` and the selected agent executable are available.
3. Confirm the repository with `git rev-parse`.
4. Validate the branch with `git check-ref-format --branch`.
5. Confirm that the branch does not exist.
6. For worktree mode, confirm that the destination does not exist.
7. Prepare the branch or worktree.
8. Create and switch to the Nodestorm session.
9. Compose the initial prompt.
10. Open the selected agent in the system terminal from the prepared working
    directory.

The Git operations are:

```text
git -C <repository> switch -c <branch>
git -C <repository> worktree add -b <branch> <worktree-path> HEAD
```

The first command is used only for existing-checkout mode; the second is used
for the default worktree mode.

If existing-checkout mode finds uncommitted changes, the dialog warns that the
changes will carry onto the new branch and requires explicit confirmation. The
default worktree mode starts from `HEAD`, does not include uncommitted changes,
and does not alter the original checkout.

## SSH Launch Flow

SSH targets are Linux hosts addressed through the user's system
`~/.ssh/config`. Nodestorm passes only the selected alias to `ssh`; usernames,
ports, keys, proxy jumps, host verification, and authentication remain SSH's
responsibility.

Remote validation and Git preparation run inside the interactive SSH
connection so host-key and authentication prompts work normally. The remote
script validates `git`, the selected agent executable, repository membership,
branch syntax and availability, and worktree destination availability before
making a Git change.

The interactive connection is conceptually:

```sh
ssh -t \
  -o ExitOnForwardFailure=yes \
  -R 4747:127.0.0.1:<local-nodestorm-port> \
  <alias> \
  sh -lc '<validate; prepare; cd; exec agent>'
```

The reverse tunnel binds the Nodestorm endpoint to loopback port 4747 on the
remote host. A remote-port conflict fails the SSH launch clearly. The remote
host must already have the selected CLI and the Nodestorm plugin installed.
On setup failure, the terminal remains at a diagnostic remote shell instead
of closing immediately.

Existing-checkout mode asks for confirmation in the remote terminal when the
checkout is dirty. Worktree mode uses the same sibling path convention as a
local launch, interpreted entirely on the remote host.

## Agent Commands and Prompt

Each adapter starts the agent's interactive interface with the initial prompt:

```text
claude <composed-prompt>
codex <composed-prompt>
opencode --prompt <composed-prompt>
pi --name <session-name> <composed-prompt>
```

The composed prompt begins with the user's task and appends generated
instructions to load the installed Nodestorm skill, use the created Nodestorm
session slug, and use the generated agent identity in MCP calls that support
agent identity. The user's task is not rewritten.

The command shapes follow the upstream interactive CLI references:

- Claude Code: <https://code.claude.com/docs/en/cli-usage>
- Codex: <https://learn.chatgpt.com/docs/developer-commands?surface=cli>
- OpenCode: <https://dev.opencode.ai/docs/cli/>
- Pi: <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md>

## Terminal Launching

Nodestorm opens the prepared command in the system terminal on Windows,
macOS, or Linux. The target agent continues to own its native interactive UI,
authentication, permissions, resumption, and exit behavior. Linux is the only
supported SSH target OS, regardless of the OS running Nodestorm.

Git validation and preparation use executable-plus-argument arrays and do not
invoke a shell. Linux and Windows terminal adapters preserve that argument
structure where the platform terminal supports it. macOS Terminal and any
platform fallback that accepts only a command line receive a command serialized
with the platform's escaping rules. SSH necessarily passes one remote POSIX
command; every dynamic value in that command is POSIX-escaped. There is no
custom command or custom executable-path field in this version.

## Errors and Partial Success

Local validation or Git preparation errors keep the dialog open, display the
captured error, and do not create a Nodestorm session.

After Git preparation succeeds, Nodestorm never automatically removes the new
branch or worktree. If session creation or terminal opening fails, the dialog
shows the retained path and offers a retry. This avoids deleting work the user
may already have inspected or changed.

Remote setup errors are displayed in the opened terminal. Because remote Git
preparation occurs in that connection, an empty Nodestorm session may already
exist when remote preparation fails; the user may retry it or manage it using
the existing session controls. Any remotely created branch or worktree is
retained after a later failure.

## Security and Privacy

- Nodestorm never reads, stores, copies, or modifies SSH private keys or agent
  credentials.
- System SSH retains host verification and authentication policy; Nodestorm
  never enables automatic host-key acceptance.
- Git validation and preparation use argument arrays; serialized terminal
  commands use platform-specific escaping covered by adversarial-input tests.
- Remote values are POSIX-escaped and covered by adversarial-input tests.
- Custom shells, agent commands, and executable paths are not accepted.
- No agent transcript, task history, or launcher history is persisted by
  Nodestorm.
- No branch or worktree is automatically deleted after a partial failure.

## Testing

Unit tests cover:

- Exact interactive argument lists for all four agents.
- Prompt composition, including session slug and agent identity.
- SSH alias extraction from literal `Host` declarations while ignoring
  wildcard declarations.
- POSIX escaping of spaces, apostrophes, newlines, dollar expressions, command
  substitutions, and shell metacharacters.
- Request validation and exact Git command plans.
- Terminal adapter command construction for Windows, macOS, and Linux.

Temporary-repository integration tests cover:

- Creating a branch in the existing checkout.
- Creating the default branch in a sibling worktree.
- Starting the worktree from `HEAD` while leaving the original checkout and
  its uncommitted changes untouched.
- Rejecting existing branches and existing worktree destinations without
  mutation.
- Reporting invalid repositories and invalid branch names.

UI verification covers opening the launcher from both entry points and the
Local, Claude Code, new-worktree defaults. The full verification run includes
the existing Rust tests, JavaScript tests, Clippy checks, and release gates.

## Completion Criteria

The feature is complete when a user can:

1. Open the launcher from Nodestorm.
2. Enter a session, repository, task, branch, and optional SSH alias.
3. Start any of the four supported CLIs locally in a new branch or default
   sibling worktree.
4. Start any of the four supported CLIs on a Linux SSH host with remote Git
   preparation and an automatic loopback reverse MCP tunnel.
5. See actionable errors without silent cleanup or credential handling by
   Nodestorm.
