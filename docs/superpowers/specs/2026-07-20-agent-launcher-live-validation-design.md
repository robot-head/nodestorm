# Agent Launcher Live Git Validation Design

## Goal

Give users immediate, trustworthy feedback about the Git inputs in the **Start
agentic session** dialog before Nodestorm creates a local or SSH-backed coding
session. After a short editing cooldown, the repository, branch, and visible
worktree fields show whether they are usable. Launch remains protected by the
existing authoritative submit-time checks.

## Scope

This change applies to both Local and SSH targets in the existing agent
launcher. It adds live, read-only validation for:

- the selected repository path;
- the proposed branch name; and
- the proposed worktree destination when **New worktree** is selected.

It does not persist validation results, create branches or directories during
validation, or add dependencies. Session name, task, agent executable, and SSH
host alias validation remain outside the new per-field icon pattern.

## Interaction

Each validated field ends with a compact status icon:

- grey while the value is being edited or has not been checked;
- orange while Nodestorm is checking it;
- green when the value is usable; and
- red when the value is invalid or unavailable.

The status includes an accessible label and a concise tooltip. Color is not
the only way the state or failure reason is communicated.

Editing the repository, branch, worktree, SSH host, target, or workspace mode
immediately invalidates every affected result. After 500 ms without another
relevant edit, all visible Git fields enter the checking state together. Values
derived automatically from the session name, repository, or branch follow the
same lifecycle as directly edited values.

The **Create branch & start agent** button is enabled only when every visible
Git field is green. The final launch path repeats the authoritative validation
to protect against filesystem or repository changes between the live check and
submission.

## Validation Semantics

The repository is valid only when the path exists and Git recognizes it as a
working tree.

The branch is valid only when:

1. Git accepts its name through `check-ref-format --branch`; and
2. `refs/heads/<branch>` does not already exist in the selected repository.

The worktree destination is valid only when the destination does not already
exist. This field is rendered and checked only in new-worktree mode.

Empty required values become invalid after the cooldown. If the repository is
invalid, any dependent result that cannot be established is also invalid and
explains that a valid repository is required.

## Local Checks

`src/agent_launcher.rs` owns a read-only workspace probe that returns separate
repository, branch, and optional worktree results. It uses the same primitives
as launch preparation:

- `git -C <repository> rev-parse --show-toplevel`;
- `git -C <repository> check-ref-format --branch <branch>`;
- `git -C <repository> show-ref --verify --quiet refs/heads/<branch>`; and
- `Path::exists` for the worktree destination.

The existing submit-time local inspection consumes the same validation logic,
then performs its additional dirty-check behavior. This keeps live feedback
and launch-time validation aligned without moving mutations into the probe.

## SSH Checks and Password-Only Hosts

Remote validation uses one bounded SSH process with non-interactive
authentication and one read-only shell script. The script performs the same
Git checks and a POSIX path existence check, returning structured results so
each field receives its own status. One probe avoids repeated SSH handshakes
for mutually dependent fields.

If the SSH client reports an authentication rejection indicating that
interactive credentials are required, Nodestorm treats the host as
password-only for this attempt. The Git fields return to grey, unchecked
states; a small note explains that Nodestorm did not check the remote inputs,
launch may fail, and the user may proceed. Launch remains enabled in this
specific fallback state, and the existing interactive terminal launch performs
its normal remote checks after authentication.

Connection timeout, DNS failure, unknown host, host-key failure, malformed SSH
configuration, and other transport errors are not password-only fallbacks.
They mark the validation red and block launch, with the SSH failure available
to the user.

## UI State and Concurrency

`src/ui/agent_launcher.rs` owns the presentation state and debounce. A field
status has four display states: editing/unchecked, checking, valid, and invalid
with a reason. The remote password-only fallback is represented separately so
the UI can distinguish permitted unchecked values from ordinary editing.

Every relevant input change increments a validation generation. A task captures
the complete validation input and generation before waiting for the cooldown.
Before entering checking, and again before publishing a result, it verifies
that both still match. Superseded local processes or SSH calls may finish, but
their results cannot overwrite the state for newer input.

Switching from new-worktree to existing-checkout mode removes the worktree
result from launch gating. Switching targets, SSH hosts, repositories, or back
to new-worktree mode starts a fresh validation lifecycle.

## Styling and Accessibility

`assets/main.css` places the status icon inside a small field wrapper without
changing the dialog grid. Colors use existing theme status tokens where
available and retain sufficient contrast in every supported theme. The icon
has readable status text for assistive technology, and invalid status exposes
the concrete reason rather than a generic failure.

The orange checking icon is static; no new animation is needed. The input
remains editable in every state.

## Error Handling

Expected field failures stay attached to their icons and tooltips. A shared SSH
transport error also appears in the launcher's existing error area so its full
actionable explanation is visible. Validation performs no mutation, so
cancellation or failure has no cleanup requirement.

The submit-time validation remains the source of truth. If a value becomes
invalid after a green result, launch fails through the existing error area and
does not create a Nodestorm session.

## Testing and Verification

Focused tests cover:

- editing, checking, valid, and invalid state transitions;
- rejection of results from stale validation generations;
- a valid checkout and missing or non-Git repositories;
- valid unused, malformed, and existing branch names;
- available and existing worktree destinations;
- remote probe command construction and structured result handling;
- authentication rejection classified separately from timeout, host-key, DNS,
  and other SSH failures;
- accessible status text, all four visual states, the password-only note, and
  launch gating.

Repository behavior tests use temporary Git repositories. Verification runs
`cargo fmt --check`, focused launcher tests, the full test suite, and Clippy.
A manual local flow confirms the icon lifecycle and launch gating. Remote
behavior remains deterministic in automated tests unless an SSH fixture is
already available.
