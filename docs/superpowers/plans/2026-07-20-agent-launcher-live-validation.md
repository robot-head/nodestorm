# Agent Launcher Live Git Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add debounced, accessible repository, branch, and worktree validation to local and SSH agent launches, while allowing an explained unchecked fallback when SSH requires interactive authentication.

**Architecture:** `src/agent_launcher.rs` owns one read-only workspace probe and the shared definitions of valid Git input. `src/ui/agent_launcher.rs` owns the 500 ms cooldown, stale-result guard, field presentation state, and launch gating. Local and remote probes return the same per-field result type; submission still re-runs authoritative checks.

**Tech Stack:** Rust 2024, Dioxus 0.7, Tokio, `std::process::Command`, Git, OpenSSH, existing CSS theme tokens, `assert2`, and `yare`.

## Global Constraints

- Add no dependency and no persisted state or schema.
- Validation is read-only: it must not create directories, branches, worktrees, sessions, or terminals.
- Use a 500 ms cooldown after each relevant edit.
- Repository is valid only when Git recognizes the path as a working tree.
- Branch is valid only when its format is accepted and its local ref does not exist.
- A visible worktree destination is valid only when it does not exist.
- Editing, checking, invalid, or transport-error states block launch; all visible fields valid allows launch.
- SSH authentication rejection that offers password or keyboard-interactive authentication permits an unchecked launch with explanatory UI copy.
- Timeout, DNS, host-key, SSH configuration, and other transport failures remain blocking errors.
- Submit-time checks remain authoritative against races.
- Every status has accessible text; color is never the only signal.

## File Map

- `src/agent_launcher.rs`: result types, local checks, remote command/script/parser, password-only classification, and submit-time reuse.
- `src/ui/agent_launcher.rs`: validation state, debounce, generation matching, icons, note, and launch gating.
- `assets/main.css`: input/icon wrapper, status colors, and note styling.
- `src/theme.rs`: source/CSS assertions for accessibility and theme tokens.

---

### Task 1: Shared Local Workspace Validation

**Files:**
- Modify: `src/agent_launcher.rs:184-257`
- Test: `src/agent_launcher.rs:565-906`

**Interfaces:**
- Consumes: `LaunchRequest`, `GitMode`, `git`, and `checked_output`.
- Produces: `FieldCheck`, `WorkspaceCheck`, `check_local_workspace(&LaunchRequest) -> WorkspaceCheck`, `WorkspaceCheck::require_valid() -> anyhow::Result<()>`, and shared submit-time validation in `inspect_local`.

- [ ] **Step 1: Write failing field-result tests**

Add beside the `TempRepo` tests:

```rust
#[test]
fn local_workspace_check_reports_each_usable_field() {
    let repo = TempRepo::new("live-valid");
    let destination = repo.root.join("available-worktree");
    let mut req = request(AgentKind::Claude);
    req.repository = repo.path.to_string_lossy().into_owned();
    req.branch = "feature/available".into();
    req.git_mode = GitMode::NewWorktree {
        path: destination.to_string_lossy().into_owned(),
    };
    assert2::assert!(
        (check_local_workspace(&req))
            == (WorkspaceCheck {
                repository: FieldCheck::Valid,
                branch: FieldCheck::Valid,
                worktree: Some(FieldCheck::Valid),
            })
    );
}

#[test]
fn local_workspace_check_reports_collisions_and_dependencies() {
    let repo = TempRepo::new("live-invalid");
    let occupied = repo.root.join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::process::Command::new("git")
        .arg("-C").arg(&repo.path).args(["branch", "already-there"])
        .status().unwrap();
    let mut req = request(AgentKind::Claude);
    req.repository = repo.path.to_string_lossy().into_owned();
    req.branch = "already-there".into();
    req.git_mode = GitMode::NewWorktree {
        path: occupied.to_string_lossy().into_owned(),
    };
    let checked = check_local_workspace(&req);
    assert2::assert!(matches!(checked.branch, FieldCheck::Invalid(message) if message.contains("already exists")));
    assert2::assert!(matches!(checked.worktree, Some(FieldCheck::Invalid(message)) if message.contains("already exists")));

    req.repository = repo.root.join("missing").to_string_lossy().into_owned();
    let checked = check_local_workspace(&req);
    assert2::assert!(matches!(checked.repository, FieldCheck::Invalid(_)));
    assert2::assert!(checked.branch == FieldCheck::Invalid("select a valid Git repository first".into()));
}

#[test]
fn local_workspace_check_rejects_empty_and_malformed_values() {
    let mut req = request(AgentKind::Claude);
    req.repository.clear();
    req.branch = "bad branch".into();
    req.git_mode = GitMode::NewWorktree { path: String::new() };
    let checked = check_local_workspace(&req);
    assert2::assert!(checked.repository == FieldCheck::Invalid("repository is required".into()));
    assert2::assert!(checked.branch == FieldCheck::Invalid("branch name is invalid".into()));
    assert2::assert!(checked.worktree == Some(FieldCheck::Invalid("worktree path is required".into())));
}
```

- [ ] **Step 2: Run RED**

Run `cargo test agent_launcher::tests::local_workspace_check -- --nocapture`.

Expected: compilation fails because the three new interfaces do not exist.

- [ ] **Step 3: Implement the shared result types**

Add above `LocalInspection`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldCheck { Valid, Invalid(String) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCheck {
    pub repository: FieldCheck,
    pub branch: FieldCheck,
    pub worktree: Option<FieldCheck>,
}

impl WorkspaceCheck {
    pub fn require_valid(&self) -> anyhow::Result<()> {
        for check in [Some(&self.repository), Some(&self.branch), self.worktree.as_ref()]
            .into_iter().flatten()
        {
            if let FieldCheck::Invalid(message) = check {
                anyhow::bail!(message.clone());
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Implement local checks and reuse them at submit time**

Add after `git`:

```rust
pub fn check_local_workspace(req: &LaunchRequest) -> WorkspaceCheck {
    let repository = if req.repository.trim().is_empty() {
        FieldCheck::Invalid("repository is required".into())
    } else if git(&req.repository, &["rev-parse", "--is-inside-work-tree"])
        .is_ok_and(|value| value == "true")
    {
        FieldCheck::Valid
    } else {
        FieldCheck::Invalid("repository is not a Git checkout".into())
    };
    let branch = if req.branch.trim().is_empty()
        || checked_output("git", &["check-ref-format", "--branch", &req.branch]).is_err()
    {
        FieldCheck::Invalid("branch name is invalid".into())
    } else if repository != FieldCheck::Valid {
        FieldCheck::Invalid("select a valid Git repository first".into())
    } else {
        let reference = format!("refs/heads/{}", req.branch);
        match std::process::Command::new("git")
            .args(["-C", &req.repository, "show-ref", "--verify", "--quiet", &reference])
            .status().ok().and_then(|status| status.code())
        {
            Some(0) => FieldCheck::Invalid(format!("branch `{}` already exists", req.branch)),
            Some(1) => FieldCheck::Valid,
            _ => FieldCheck::Invalid(format!("git could not inspect branch `{}`", req.branch)),
        }
    };
    let worktree = match &req.git_mode {
        GitMode::ExistingCheckout => None,
        GitMode::NewWorktree { path } if path.trim().is_empty() =>
            Some(FieldCheck::Invalid("worktree path is required".into())),
        GitMode::NewWorktree { path } if Path::new(path).exists() =>
            Some(FieldCheck::Invalid(format!("worktree destination `{path}` already exists"))),
        GitMode::NewWorktree { .. } => Some(FieldCheck::Valid),
    };
    WorkspaceCheck { repository, branch, worktree }
}
```

Replace the duplicate checks in `inspect_local` with:

```rust
pub fn inspect_local(req: &LaunchRequest) -> anyhow::Result<LocalInspection> {
    validate_request(req)?;
    anyhow::ensure!(matches!(req.target, LaunchTarget::Local), "local inspection requires a local target");
    check_local_workspace(req).require_valid()?;
    Ok(LocalInspection {
        dirty: !git(&req.repository, &["status", "--porcelain"])?.is_empty(),
    })
}
```

- [ ] **Step 5: Run GREEN and regression tests**

Run:

```bash
cargo test agent_launcher::tests::local_workspace_check -- --nocapture
cargo test agent_launcher::tests::collisions_and_invalid_branches_do_not_mutate_repository -- --nocapture
cargo test agent_launcher::tests::worktree_mode_creates_branch_without_switching_or_carrying_dirty_files -- --nocapture
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/agent_launcher.rs
git commit -m "feat: validate launcher Git fields"
```

### Task 2: Single Remote SSH Workspace Probe

**Files:**
- Modify: `src/agent_launcher.rs:291-420`
- Test: `src/agent_launcher.rs:909-958`

**Interfaces:**
- Consumes: `FieldCheck`, `WorkspaceCheck`, `LaunchRequest`, `LaunchTarget`, `GitMode`, and `posix_quote`.
- Produces: `WorkspaceProbe`, `probe_workspace(&LaunchRequest) -> WorkspaceProbe`, `remote_probe_command(&LaunchRequest) -> anyhow::Result<CommandSpec>`, `parse_remote_probe(&str, bool) -> anyhow::Result<WorkspaceCheck>`, and `password_only_rejection(&str) -> bool`.

- [ ] **Step 1: Write failing remote command and parser tests**

```rust
#[test]
fn remote_probe_is_one_bounded_noninteractive_ssh_command() {
    let mut req = request(AgentKind::Claude);
    req.target = LaunchTarget::Ssh { alias: "build-box".into() };
    req.repository = "/srv/api".into();
    req.git_mode = GitMode::NewWorktree { path: "/srv/api-worktrees/feature/check".into() };
    let command = remote_probe_command(&req).unwrap();
    assert2::assert!(command.program == "ssh");
    for option in ["BatchMode=yes", "ConnectTimeout=5", "NumberOfPasswordPrompts=0", "StrictHostKeyChecking=yes"] {
        assert2::assert!(command.args.iter().any(|arg| arg == option));
    }
    let script = command.args.last().unwrap();
    assert2::assert!(script.contains("NODESTORM_REPOSITORY"));
    assert2::assert!(script.contains("NODESTORM_BRANCH"));
    assert2::assert!(script.contains("NODESTORM_WORKTREE"));
    assert2::assert!(!script.contains("mkdir"));
    assert2::assert!(!script.contains("worktree add"));
}

#[test]
fn remote_probe_parser_preserves_independent_results() {
    let output = "banner\nNODESTORM_REPOSITORY\tvalid\t\nNODESTORM_BRANCH\tinvalid\tbranch already exists\nNODESTORM_WORKTREE\tvalid\t\n";
    assert2::assert!((parse_remote_probe(output, true).unwrap()) == (WorkspaceCheck {
        repository: FieldCheck::Valid,
        branch: FieldCheck::Invalid("branch already exists".into()),
        worktree: Some(FieldCheck::Valid),
    }));
}

#[yare::parameterized(
    password = { "Permission denied (publickey,password).", true },
    keyboard = { "Permission denied (publickey,keyboard-interactive).", true },
    key_only = { "Permission denied (publickey).", false },
    host_key = { "Host key verification failed.", false },
    timeout = { "ssh: connect to host build-box port 22: Connection timed out", false },
)]
fn only_interactive_auth_rejection_enables_fallback(stderr: &str, expected: bool) {
    assert2::assert!(password_only_rejection(stderr) == expected);
}
```

- [ ] **Step 2: Run RED**

Run:

```bash
cargo test agent_launcher::tests::remote_probe -- --nocapture
cargo test agent_launcher::tests::only_interactive_auth_rejection -- --nocapture
```

Expected: compilation fails because the remote probe interfaces do not exist.

- [ ] **Step 3: Add the remote outcome and read-only script**

Add near `WorkspaceCheck`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceProbe {
    Checked(WorkspaceCheck),
    PasswordOnly,
    TransportError(String),
}
```

```rust
fn probe_line(name: &str, valid: bool, message: &str) -> String {
    format!("printf 'NODESTORM_{name}\\t{}\\t{}\\n'", if valid { "valid" } else { "invalid" }, message)
}

fn remote_probe_script(req: &LaunchRequest) -> anyhow::Result<String> {
    anyhow::ensure!(req.repository.starts_with('/'), "remote repository must be absolute");
    let repo = posix_quote(&req.repository);
    let branch = posix_quote(&req.branch);
    let reference = posix_quote(&format!("refs/heads/{}", req.branch));
    let repository_valid = probe_line("REPOSITORY", true, "");
    let repository_invalid = probe_line("REPOSITORY", false, "repository is not a Git checkout");
    let branch_invalid = probe_line("BRANCH", false, "branch name is invalid");
    let branch_needs_repo = probe_line("BRANCH", false, "select a valid Git repository first");
    let branch_exists = probe_line("BRANCH", false, "branch already exists");
    let branch_error = probe_line("BRANCH", false, "could not inspect remote branches");
    let branch_valid = probe_line("BRANCH", true, "");
    let mut lines = vec![
        "repo_ok=0".to_owned(),
        format!("if git -C {repo} rev-parse --is-inside-work-tree 2>/dev/null | grep -qx true; then repo_ok=1; {repository_valid}; else {repository_invalid}; fi"),
        format!("if ! git check-ref-format --branch {branch} >/dev/null 2>&1; then {branch_invalid}; elif [ \"$repo_ok\" -ne 1 ]; then {branch_needs_repo}; else branch_status=0; git -C {repo} show-ref --verify --quiet {reference} || branch_status=$?; if [ \"$branch_status\" -eq 0 ]; then {branch_exists}; elif [ \"$branch_status\" -eq 1 ]; then {branch_valid}; else {branch_error}; fi; fi"),
    ];
    if let GitMode::NewWorktree { path } = &req.git_mode {
        anyhow::ensure!(path.starts_with('/'), "remote worktree path must be absolute");
        let worktree = posix_quote(path);
        let exists = probe_line("WORKTREE", false, "worktree destination already exists");
        let available = probe_line("WORKTREE", true, "");
        lines.push(format!("if [ -e {worktree} ]; then {exists}; else {available}; fi"));
    }
    Ok(lines.join("\n"))
}
```

The fixed messages keep paths and Git output out of the wire format. This
script contains no `mkdir`, `switch`, or `worktree add` mutation.

- [ ] **Step 4: Build, parse, and run the one SSH command**

Construct the command exactly as:

```rust
pub fn remote_probe_command(req: &LaunchRequest) -> anyhow::Result<CommandSpec> {
    let LaunchTarget::Ssh { alias } = &req.target else {
        anyhow::bail!("remote workspace probe requires an SSH target");
    };
    anyhow::ensure!(!alias.trim().is_empty(), "SSH host alias is required");
    let script = remote_probe_script(req)?;
    Ok(CommandSpec {
        program: "ssh".into(),
        args: vec![
            "-o".into(), "BatchMode=yes".into(),
            "-o".into(), "ConnectTimeout=5".into(),
            "-o".into(), "NumberOfPasswordPrompts=0".into(),
            "-o".into(), "StrictHostKeyChecking=yes".into(),
            "--".into(), alias.clone(), "sh".into(), "-lc".into(), posix_quote(&script),
        ],
        current_dir: None,
    })
}
```

Parse the prefixed lines with these complete helpers:

```rust
fn parsed_field(value: &str, message: &str) -> anyhow::Result<FieldCheck> {
    match value {
        "valid" => Ok(FieldCheck::Valid),
        "invalid" if !message.is_empty() => Ok(FieldCheck::Invalid(message.into())),
        _ => anyhow::bail!("remote validation returned an invalid field result"),
    }
}

pub fn parse_remote_probe(output: &str, wants_worktree: bool) -> anyhow::Result<WorkspaceCheck> {
    let mut repository = None;
    let mut branch = None;
    let mut worktree = None;
    for line in output.lines() {
        let parts = line.splitn(3, '\t').collect::<Vec<_>>();
        if parts.len() != 3 { continue; }
        match parts[0] {
            "NODESTORM_REPOSITORY" => {
                anyhow::ensure!(repository.is_none(), "duplicate remote repository status");
                repository = Some(parsed_field(parts[1], parts[2])?);
            }
            "NODESTORM_BRANCH" => {
                anyhow::ensure!(branch.is_none(), "duplicate remote branch status");
                branch = Some(parsed_field(parts[1], parts[2])?);
            }
            "NODESTORM_WORKTREE" if wants_worktree => {
                anyhow::ensure!(worktree.is_none(), "duplicate remote worktree status");
                worktree = Some(parsed_field(parts[1], parts[2])?);
            }
            _ => {}
        }
    }
    Ok(WorkspaceCheck {
        repository: repository.ok_or_else(|| anyhow::anyhow!("remote validation omitted repository status"))?,
        branch: branch.ok_or_else(|| anyhow::anyhow!("remote validation omitted branch status"))?,
        worktree: if wants_worktree {
            Some(worktree.ok_or_else(|| anyhow::anyhow!("remote validation omitted worktree status"))?)
        } else {
            None
        },
    })
}
```

Classify fallback with:

```rust
pub fn password_only_rejection(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("permission denied")
        && (stderr.contains("password") || stderr.contains("keyboard-interactive"))
}
```

Run exactly one process:

```rust
pub fn probe_workspace(req: &LaunchRequest) -> WorkspaceProbe {
    if matches!(req.target, LaunchTarget::Local) {
        return WorkspaceProbe::Checked(check_local_workspace(req));
    }
    let command = match remote_probe_command(req) {
        Ok(command) => command,
        Err(error) => return WorkspaceProbe::TransportError(error.to_string()),
    };
    let output = match std::process::Command::new(&command.program).args(&command.args).output() {
        Ok(output) => output,
        Err(error) => return WorkspaceProbe::TransportError(error.to_string()),
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return if password_only_rejection(&stderr) {
            WorkspaceProbe::PasswordOnly
        } else {
            WorkspaceProbe::TransportError(stderr.trim().to_owned())
        };
    }
    match parse_remote_probe(
        &String::from_utf8_lossy(&output.stdout),
        matches!(req.git_mode, GitMode::NewWorktree { .. }),
    ) {
        Ok(checked) => WorkspaceProbe::Checked(checked),
        Err(error) => WorkspaceProbe::TransportError(error.to_string()),
    }
}
```

- [ ] **Step 5: Run GREEN and SSH launch regressions**

```bash
cargo test agent_launcher::tests::remote_probe -- --nocapture
cargo test agent_launcher::tests::only_interactive_auth_rejection -- --nocapture
cargo test agent_launcher::tests::ssh_command_has_tty_tunnel_validation_and_quoted_values -- --nocapture
```

Expected: all selected tests pass; the interactive launch command is unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/agent_launcher.rs
git commit -m "feat: probe remote launcher workspace"
```

### Task 3: Debounced UI State, Icons, and Launch Gating

**Files:**
- Modify: `src/ui/agent_launcher.rs:1-530`
- Test: `src/ui/agent_launcher.rs:532-651`

**Interfaces:**
- Consumes: `FieldCheck`, `WorkspaceCheck`, `WorkspaceProbe`, and `probe_workspace` from Tasks 1-2.
- Produces: `FieldStatus`, `ValidationUi`, `ValidationUi::allows_launch(bool)`, `queue_validation`, accessible status markup, the password-only note, and submit gating.

- [ ] **Step 1: Write failing mapping, gating, and staleness tests**

Add to the UI test module:

```rust
#[test]
fn checked_workspace_maps_to_status_and_gates_visible_worktree() {
    let valid = ValidationUi::from_probe(WorkspaceProbe::Checked(WorkspaceCheck {
        repository: FieldCheck::Valid,
        branch: FieldCheck::Valid,
        worktree: Some(FieldCheck::Valid),
    }), true);
    assert2::assert!(valid.allows_launch(true));
    assert2::assert!(valid.repository == FieldStatus::Valid);

    let invalid = ValidationUi::from_probe(WorkspaceProbe::Checked(WorkspaceCheck {
        repository: FieldCheck::Valid,
        branch: FieldCheck::Invalid("branch already exists".into()),
        worktree: Some(FieldCheck::Valid),
    }), true);
    assert2::assert!(!invalid.allows_launch(true));
    assert2::assert!(invalid.branch == FieldStatus::Invalid("branch already exists".into()));
}

#[test]
fn password_only_fallback_allows_unchecked_remote_launch() {
    let state = ValidationUi::from_probe(WorkspaceProbe::PasswordOnly, true);
    assert2::assert!(state.password_only);
    assert2::assert!(state.repository == FieldStatus::Editing);
    assert2::assert!(state.allows_launch(true));
}

#[yare::parameterized(
    same = { 4, 4, true },
    stale = { 5, 4, false },
)]
fn only_current_validation_generation_can_publish(current: u64, completed: u64, expected: bool) {
    assert2::assert!(is_current_generation(current, completed) == expected);
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test ui::agent_launcher::tests::checked_workspace_maps -- --nocapture
cargo test ui::agent_launcher::tests::password_only_fallback -- --nocapture
```

Expected: compilation fails because `FieldStatus` and `ValidationUi` do not exist.

- [ ] **Step 3: Implement presentation state and pure gating**

Import `std::time::Duration` plus the Task 1-2 types and function. Add near
`LaunchDraft`:

```rust
const VALIDATION_COOLDOWN: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldStatus { Editing, Checking, Valid, Invalid(String) }

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidationUi {
    repository: FieldStatus,
    branch: FieldStatus,
    worktree: Option<FieldStatus>,
    password_only: bool,
    transport_error: Option<String>,
}

impl ValidationUi {
    fn editing(worktree: bool) -> Self {
        Self {
            repository: FieldStatus::Editing,
            branch: FieldStatus::Editing,
            worktree: worktree.then_some(FieldStatus::Editing),
            password_only: false,
            transport_error: None,
        }
    }

    fn checking(worktree: bool) -> Self {
        Self {
            repository: FieldStatus::Checking,
            branch: FieldStatus::Checking,
            worktree: worktree.then_some(FieldStatus::Checking),
            password_only: false,
            transport_error: None,
        }
    }

    fn from_probe(probe: WorkspaceProbe, worktree: bool) -> Self {
        let map = |check| match check {
            FieldCheck::Valid => FieldStatus::Valid,
            FieldCheck::Invalid(message) => FieldStatus::Invalid(message),
        };
        match probe {
            WorkspaceProbe::Checked(checked) => Self {
                repository: map(checked.repository),
                branch: map(checked.branch),
                worktree: checked.worktree.map(map),
                password_only: false,
                transport_error: None,
            },
            WorkspaceProbe::PasswordOnly => Self {
                password_only: true,
                ..Self::editing(worktree)
            },
            WorkspaceProbe::TransportError(message) => Self {
                repository: FieldStatus::Invalid(message.clone()),
                branch: FieldStatus::Invalid(message.clone()),
                worktree: worktree.then(|| FieldStatus::Invalid(message.clone())),
                password_only: false,
                transport_error: Some(message),
            },
        }
    }

    fn allows_launch(&self, worktree: bool) -> bool {
        self.password_only || (self.repository == FieldStatus::Valid
            && self.branch == FieldStatus::Valid
            && (!worktree || self.worktree == Some(FieldStatus::Valid)))
    }
}
```

- [ ] **Step 4: Add one scheduler with generation checks**

Add before `AgentLauncher`:

```rust
fn is_current_generation(current: u64, completed: u64) -> bool {
    current == completed
}

fn queue_validation(
    request: LaunchRequest,
    mut generation: Signal<u64>,
    mut validation: Signal<ValidationUi>,
) {
    let worktree = matches!(request.git_mode, GitMode::NewWorktree { .. });
    let queued = generation() + 1;
    generation.set(queued);
    validation.set(ValidationUi::editing(worktree));
    spawn(async move {
        tokio::time::sleep(VALIDATION_COOLDOWN).await;
        if !is_current_generation(generation(), queued) { return; }
        validation.set(ValidationUi::checking(worktree));
        let probe = tokio::task::spawn_blocking(move || probe_workspace(&request)).await;
        if !is_current_generation(generation(), queued) { return; }
        validation.set(match probe {
            Ok(probe) => ValidationUi::from_probe(probe, worktree),
            Err(error) => ValidationUi::from_probe(
                WorkspaceProbe::TransportError(format!("validation worker failed: {error}")),
                worktree,
            ),
        });
    });
}
```

Initialize in `AgentLauncher`:

```rust
let validation_generation = use_signal(|| 0_u64);
let validation = use_signal(|| ValidationUi::editing(true));
```

Replace the relevant handlers with these exact forms:

```rust
oninput: move |event| {
    draft.write().set_session_name(event.value());
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |_| {
    draft.write().set_remote(false);
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |_| {
    draft.write().set_remote(true);
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |event| {
    draft.write().ssh_alias = event.value();
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |event| {
    draft.write().set_repository(event.value());
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |event| {
    let mut value = draft.write();
    value.branch = event.value();
    value.branch_edited = true;
    value.refresh_worktree();
    drop(value);
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |_| {
    draft.write().worktree = false;
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |_| {
    draft.write().worktree = true;
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},

oninput: move |event| {
    let mut value = draft.write();
    value.worktree_path = event.value();
    value.worktree_edited = true;
    drop(value);
    queue_validation(draft.read().request(cli.port), validation_generation, validation);
},
```

Apply the snippets respectively to session name, Local, SSH, SSH alias,
repository, branch, existing checkout, new worktree, and worktree path. Do not
schedule from task or agent-kind edits.

- [ ] **Step 5: Render accessible icons and password-only copy**

Add:

```rust
fn field_status(field: &str, status: &FieldStatus) -> Element {
    let (class, glyph, detail) = match status {
        FieldStatus::Editing => ("editing", "●", "editing; not checked"),
        FieldStatus::Checking => ("checking", "●", "checking"),
        FieldStatus::Valid => ("valid", "✓", "valid"),
        FieldStatus::Invalid(message) => ("invalid", "!", message.as_str()),
    };
    let label = format!("{field}: {detail}");
    rsx! {
        span {
            class: "agent-field-status {class}",
            role: "status",
            aria_label: "{label}",
            title: "{label}",
            "{glyph}"
        }
    }
}
```

Render the repository control as:

```rust
div { class: "agent-field-control",
    input {
        id: "agent-repository",
        list: "recent-repositories",
        placeholder: if draft.read().remote { "/srv/projects/api" } else { "/home/me/projects/api" },
        value: "{draft.read().repository}",
        oninput: move |event| {
            draft.write().set_repository(event.value());
            queue_validation(draft.read().request(cli.port), validation_generation, validation);
        },
    }
    {field_status("Repository path", &validation.read().repository)}
}
datalist { id: "recent-repositories",
    for repo in prefs.read().recent_repositories.iter() {
        option { key: "{repo}", value: "{repo}" }
    }
}
```

Render the branch control as:

```rust
div { class: "agent-field-control",
    input {
        id: "agent-branch",
        placeholder: "nodestorm/cache-redesign",
        value: "{draft.read().branch}",
        oninput: move |event| {
            let mut value = draft.write();
            value.branch = event.value();
            value.branch_edited = true;
            value.refresh_worktree();
            drop(value);
            queue_validation(draft.read().request(cli.port), validation_generation, validation);
        },
    }
    {field_status("Branch name", &validation.read().branch)}
}
```

Render the conditional worktree control as:

```rust
div { class: "agent-field-control",
    input {
        id: "agent-worktree",
        value: "{draft.read().worktree_path}",
        oninput: move |event| {
            let mut value = draft.write();
            value.worktree_path = event.value();
            value.worktree_edited = true;
            drop(value);
            queue_validation(draft.read().request(cli.port), validation_generation, validation);
        },
    }
    {field_status(
        "Worktree path",
        validation.read().worktree.as_ref().expect("visible worktree has status"),
    )}
}
```

After the grid, render when `validation.read().password_only`:

```rust
p {
    class: "agent-validation-note",
    role: "note",
    "This SSH host requires interactive authentication, so Nodestorm did not check the remote repository, branch, or worktree. Launch may fail."
}
```

Render `transport_error` through a separate existing `.agent-launch-error`
alert so validation edits never erase a terminal-launch error.

- [ ] **Step 6: Gate launch**

Replace only the ordinary create button expression with:

```rust
disabled: running()
    || !validation.read().allows_launch(draft.read().worktree)
    || (dirty_warning() && !allow_dirty()),
```

Leave Cancel, Retry terminal, and submit-time `perform_launch` behavior intact.

- [ ] **Step 7: Run GREEN and regressions**

```bash
cargo test ui::agent_launcher::tests -- --nocapture
cargo test agent_launcher::tests -- --nocapture
```

Expected: all launcher tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/ui/agent_launcher.rs
git commit -m "feat: debounce launcher validation"
```

### Task 4: Accessible Styling and Final Verification

**Files:**
- Modify: `assets/main.css:573-680`
- Modify: `src/theme.rs:132-134,287-306`
- Test: `src/theme.rs` colocated tests

**Interfaces:**
- Consumes: `.agent-field-control`, `.agent-field-status.{editing,checking,valid,invalid}`, and `.agent-validation-note` from Task 3.
- Produces: inline icon placement, grey/orange/green/red colors, note styling, and source assertions for accessibility and gating.

- [ ] **Step 1: Write failing source/CSS contract test**

Add with the existing source constants:

```rust
const AGENT_LAUNCHER_SOURCE: &str = include_str!("ui/agent_launcher.rs");
```

Add beside the delivery accessibility test:

```rust
#[test]
fn launcher_validation_states_are_accessible_and_theme_aware() {
    assert2::assert!(AGENT_LAUNCHER_SOURCE.contains(r#"role: "status""#));
    assert2::assert!(AGENT_LAUNCHER_SOURCE.contains(r#"aria_label: "{label}""#));
    assert2::assert!(AGENT_LAUNCHER_SOURCE.contains("requires interactive authentication"));
    assert2::assert!(AGENT_LAUNCHER_SOURCE.contains("allows_launch(draft.read().worktree)"));
    assert_block_contains(".agent-field-control", "position: relative");
    assert_block_contains(".agent-field-status.editing", "color: var(--text-dim)");
    assert_block_contains(".agent-field-status.checking", "color: var(--status-modified)");
    assert_block_contains(".agent-field-status.valid", "color: var(--badge-decided)");
    assert_block_contains(".agent-field-status.invalid", "color: var(--status-removed)");
}
```

- [ ] **Step 2: Run RED**

Run `cargo test theme::tests::launcher_validation_states_are_accessible_and_theme_aware -- --nocapture`.

Expected: failure because the status selectors are not styled.

- [ ] **Step 3: Add minimal static styles**

Add after the existing launcher input rules:

```css
.agent-field-control { position: relative; }
.agent-field-control input { padding-right: 34px; }

.agent-field-status {
  position: absolute;
  right: 11px;
  top: 50%;
  transform: translateY(-50%);
  font-size: 13px;
  font-weight: 700;
  line-height: 1;
  pointer-events: none;
}

.agent-field-status.editing { color: var(--text-dim); }
.agent-field-status.checking { color: var(--status-modified); }
.agent-field-status.valid { color: var(--badge-decided); }
.agent-field-status.invalid { color: var(--status-removed); }

.agent-validation-note {
  margin: 14px 0 0;
  padding: 9px 11px;
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text-dim);
  font-size: 12.5px;
  overflow-wrap: anywhere;
}
```

Do not add animation; color plus accessible status text communicates checking.

- [ ] **Step 4: Run all automated verification**

```bash
cargo fmt --check
cargo test agent_launcher::tests -- --nocapture
cargo test ui::agent_launcher::tests -- --nocapture
cargo test theme::tests::launcher_validation_states_are_accessible_and_theme_aware -- --nocapture
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: every command exits 0 without warnings.

- [ ] **Step 5: Manually verify the local lifecycle**

Run `cargo run`, open **Start agentic session**, select Local, and verify:

1. Editing shows grey immediately, orange after 500 ms, then green or red.
2. Missing/non-Git repository, malformed/existing branch, and existing worktree become red.
3. Valid checkout, unused branch, and absent worktree become green.
4. Repository, branch, target, and workspace edits reset affected icons; stale results never return.
5. Create is disabled unless every visible check is green.
6. Existing-checkout mode removes worktree status and gating.

Expected: closing without launching creates no branch, worktree, session, or terminal.

- [ ] **Step 6: Inspect and commit**

```bash
git diff --check
git status --short
git diff --stat
```

Expected: only the four implementation files differ from the implementation
starting point, and whitespace checks pass.

```bash
git add assets/main.css src/theme.rs
git commit -m "style: show launcher validation status"
```
