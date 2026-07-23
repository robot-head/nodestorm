# Agent Session Launcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users create a Nodestorm session, isolated Git branch, and interactive Claude Code, Codex, OpenCode, or Pi process locally or on a Linux SSH host from the desktop UI.

**Architecture:** A new `agent_launcher` domain module validates launch requests, prepares local Git state, constructs agent/SSH commands, and opens platform terminals. A focused Dioxus modal owns drafts and orchestration while `Sessions` remains the only session-persistence owner. System `git`, `ssh`, agent CLIs, and terminals are reused; no dependency or credential store is added.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7, `std::process::Command`, system Git/OpenSSH/terminal programs, existing CSS and PowerShell UIA verification.

## Global Constraints

- Supported agents are exactly Claude Code, Codex, OpenCode, and Pi.
- Agent interaction happens in an external system terminal.
- Nodestorm runs on Windows, macOS, or Linux; SSH targets are Linux only.
- SSH targets are `~/.ssh/config` aliases and receive an automatic loopback-only reverse tunnel from remote port 4747 to the app's configured local MCP port.
- Git mode defaults to a new branch in a sibling worktree at `<repository-parent>/<repository-name>-worktrees/<branch>`.
- The alternate Git mode creates and checks out the new branch in the selected existing checkout.
- Remote hosts already have the selected CLI and Nodestorm plugin installed.
- Do not persist agent credentials, tasks, transcripts, launcher history, branches, or worktree metadata.
- Do not add dependencies or accept custom command templates.
- Never automatically delete a branch or worktree after partial success.
- Follow red-green-refactor for every production-code task.

---

## File Map

- Create `src/agent_launcher.rs`: launch types, defaults, validation, prompt/command planning, SSH parsing/escaping, local Git preparation, and terminal adapters.
- Create `src/ui/agent_launcher.rs`: modal draft state, launch orchestration, progress, dirty-checkout confirmation, and errors.
- Modify `src/lib.rs`: export the launcher module.
- Modify `src/ui/mod.rs`: register the UI module and launcher-open context.
- Modify `src/ui/app.rs`: provide launcher state, mount the modal, and add the empty-state entry point.
- Modify `src/ui/topbar.rs`: add the session-menu entry point without changing canvas-only session creation.
- Modify `assets/main.css`: modal, form, progress, warning, and responsive styles.
- Modify `README.md`: document local/SSH launches, Git modes, prerequisites, and failure retention.
- Modify `scripts/verify-windows.ps1`: verify launcher entry points and defaults without starting an external agent.

---

### Task 1: Pure Launch Model and Agent Adapters

**Files:**
- Create: `src/agent_launcher.rs`
- Modify: `src/lib.rs:9-21`

**Interfaces:**
- Produces: `AgentKind`, `LaunchTarget`, `GitMode`, `LaunchRequest`, `CommandSpec`, `suggest_branch`, `suggest_worktree`, `parse_ssh_aliases`, `compose_prompt`, `agent_command`, `posix_quote`, and `validate_request`.
- Consumes: `crate::store::slugify(&str) -> String` and the configured MCP port supplied by the UI.

- [ ] **Step 1: Export the empty module and write failing pure-behavior tests**

Add `pub mod agent_launcher;` to `src/lib.rs`. Create `src/agent_launcher.rs` with only the test module below so compilation fails on the missing public API:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn request(agent: AgentKind) -> LaunchRequest {
        LaunchRequest {
            session_name: "cache redesign".into(),
            task: "Design the cache; don't run $(touch /tmp/nope).".into(),
            agent,
            target: LaunchTarget::Local,
            repository: "/work/api".into(),
            branch: "nodestorm/cache-redesign".into(),
            git_mode: GitMode::NewWorktree {
                path: "/work/api-worktrees/nodestorm/cache-redesign".into(),
            },
            mcp_port: 8123,
        }
    }

    #[test]
    fn defaults_derive_branch_and_sibling_worktree() {
        assert_eq!(suggest_branch("Cache Redesign"), "nodestorm/cache-redesign");
        assert_eq!(
            suggest_worktree("/work/api", "nodestorm/cache-redesign", false).unwrap(),
            "/work/api-worktrees/nodestorm/cache-redesign"
        );
        assert_eq!(
            suggest_worktree("/srv/api", "nodestorm/cache-redesign", true).unwrap(),
            "/srv/api-worktrees/nodestorm/cache-redesign"
        );
    }

    #[test]
    fn every_agent_gets_its_interactive_arguments() {
        let cases = [
            (AgentKind::Claude, "claude", vec!["TASK"]),
            (AgentKind::Codex, "codex", vec!["TASK"]),
            (AgentKind::OpenCode, "opencode", vec!["--prompt", "TASK"]),
            (AgentKind::Pi, "pi", vec!["--name", "cache-redesign", "TASK"]),
        ];
        for (agent, program, args) in cases {
            let spec = agent_command(agent, "cache-redesign", "TASK", "/work/tree");
            assert_eq!(spec.program, program);
            assert_eq!(spec.args, args);
            assert_eq!(spec.current_dir.as_deref(), Some("/work/tree"));
        }
    }

    #[test]
    fn prompt_preserves_task_and_addresses_the_nodestorm_session() {
        let prompt = compose_prompt(&request(AgentKind::Codex), "cache-redesign");
        assert!(prompt.starts_with("Design the cache; don't run $(touch /tmp/nope)."));
        assert!(prompt.contains("Nodestorm session `cache-redesign`"));
        assert!(prompt.contains("agent identity `codex-cache-redesign`"));
        assert!(prompt.contains("installed Nodestorm skill"));
    }

    #[test]
    fn ssh_aliases_include_literals_only() {
        let config = "Host prod bastion\n  HostName prod.test\nHost *.corp !blocked\nHost dev\n";
        assert_eq!(parse_ssh_aliases(config), vec!["bastion", "dev", "prod"]);
    }

    #[test]
    fn posix_quote_neutralizes_shell_syntax() {
        assert_eq!(posix_quote("plain"), "'plain'");
        assert_eq!(posix_quote("a'b"), "'a'\\''b'");
        assert_eq!(posix_quote("$(touch /tmp/nope)\nnext"), "'$(touch /tmp/nope)\nnext'");
    }

    #[test]
    fn validation_names_the_missing_field() {
        let mut req = request(AgentKind::Claude);
        req.task.clear();
        assert_eq!(validate_request(&req).unwrap_err().to_string(), "initial task is required");
        req.task = "task".into();
        req.target = LaunchTarget::Ssh { alias: "".into() };
        assert_eq!(validate_request(&req).unwrap_err().to_string(), "SSH host alias is required");
    }
}
```

- [ ] **Step 2: Run the targeted test and verify RED**

Run: `cargo test agent_launcher::tests --lib`

Expected: compilation fails because `AgentKind`, `LaunchRequest`, and the helper functions do not exist.

- [ ] **Step 3: Implement the minimal pure launch model**

Add these concrete types and functions above the tests:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentKind { #[default] Claude, Codex, OpenCode, Pi }

impl AgentKind {
    pub fn executable(self) -> &'static str {
        match self { Self::Claude => "claude", Self::Codex => "codex", Self::OpenCode => "opencode", Self::Pi => "pi" }
    }
    pub fn id(self) -> &'static str {
        match self { Self::Claude => "claude", Self::Codex => "codex", Self::OpenCode => "opencode", Self::Pi => "pi" }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LaunchTarget { #[default] Local, Ssh { alias: String } }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GitMode { ExistingCheckout, #[default] NewWorktree { path: String } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub session_name: String,
    pub task: String,
    pub agent: AgentKind,
    pub target: LaunchTarget,
    pub repository: String,
    pub branch: String,
    pub git_mode: GitMode,
    pub mcp_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: Option<String>,
}

pub fn suggest_branch(session: &str) -> String {
    format!("nodestorm/{}", crate::store::slugify(session))
}

pub fn suggest_worktree(repository: &str, branch: &str, remote: bool) -> anyhow::Result<String> {
    if remote {
        let repo = repository.trim_end_matches('/');
        let (parent, name) = repo.rsplit_once('/').ok_or_else(|| anyhow::anyhow!("remote repository must be an absolute path"))?;
        return Ok(format!("{parent}/{name}-worktrees/{branch}"));
    }
    let repo = Path::new(repository);
    let parent = repo.parent().ok_or_else(|| anyhow::anyhow!("repository has no parent directory"))?;
    let name = repo.file_name().ok_or_else(|| anyhow::anyhow!("repository has no directory name"))?.to_string_lossy();
    Ok(parent.join(format!("{name}-worktrees")).join(branch).to_string_lossy().into_owned())
}

pub fn parse_ssh_aliases(text: &str) -> Vec<String> {
    let mut aliases = text.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        fields.next().filter(|key| key.eq_ignore_ascii_case("host"))?;
        Some(fields.filter(|host| !host.contains(['*', '?', '!'])).map(str::to_owned).collect::<Vec<_>>())
    }).flatten().collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

pub fn posix_quote(value: &str) -> String { format!("'{}'", value.replace('\'', "'\\''")) }

pub fn validate_request(req: &LaunchRequest) -> anyhow::Result<()> {
    for (value, name) in [
        (&req.session_name, "session name"), (&req.task, "initial task"),
        (&req.repository, "repository"), (&req.branch, "branch name"),
    ] { anyhow::ensure!(!value.trim().is_empty(), "{name} is required"); }
    if let LaunchTarget::Ssh { alias } = &req.target { anyhow::ensure!(!alias.trim().is_empty(), "SSH host alias is required"); }
    if let GitMode::NewWorktree { path } = &req.git_mode { anyhow::ensure!(!path.trim().is_empty(), "worktree path is required"); }
    Ok(())
}

pub fn compose_prompt(req: &LaunchRequest, slug: &str) -> String {
    format!("{}\n\nUse the installed Nodestorm skill for design choices and implementation progress. Use Nodestorm session `{slug}` and agent identity `{}-{slug}` in MCP calls that support agent identity.", req.task, req.agent.id())
}

pub fn agent_command(agent: AgentKind, slug: &str, prompt: &str, cwd: &str) -> CommandSpec {
    let args = match agent {
        AgentKind::Claude | AgentKind::Codex => vec![prompt.into()],
        AgentKind::OpenCode => vec!["--prompt".into(), prompt.into()],
        AgentKind::Pi => vec!["--name".into(), slug.into(), prompt.into()],
    };
    CommandSpec { program: agent.executable().into(), args, current_dir: Some(cwd.into()) }
}
```

Remove the unused `PathBuf` import if Clippy reports it.

- [ ] **Step 4: Run tests and formatting to verify GREEN**

Run: `cargo test agent_launcher::tests --lib && cargo fmt --check`

Expected: all Task 1 tests pass and formatting is clean.

- [ ] **Step 5: Commit Task 1**

```bash
git add src/lib.rs src/agent_launcher.rs
git commit -m "feat: model agent session launches"
```

---

### Task 2: Local Git Inspection and Preparation

**Files:**
- Modify: `src/agent_launcher.rs`

**Interfaces:**
- Consumes: `LaunchRequest` with `LaunchTarget::Local`.
- Produces: `LocalInspection { dirty: bool }`, `PreparedWorkspace { directory, retained_path }`, `ensure_executable(&str)`, `inspect_local(&LaunchRequest)`, and `prepare_local(&LaunchRequest, allow_dirty)`.

- [ ] **Step 1: Write failing real-Git tests**

Add a `TempRepo` test helper that creates a uniquely named directory under `std::env::temp_dir()`, initializes Git, configures a test identity, writes and commits `README.md`, and removes only that exact directory tree in `Drop`. Add these tests:

```rust
struct TempRepo {
    root: PathBuf,
    path: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nodestorm-launcher-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        let path = root.join("api");
        std::fs::create_dir_all(&path).unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Nodestorm Test"],
            vec!["config", "user.email", "nodestorm@example.invalid"],
        ] {
            assert!(std::process::Command::new("git").arg("-C").arg(&path).args(args).status().unwrap().success());
        }
        std::fs::write(path.join("README.md"), "fixture\n").unwrap();
        assert!(std::process::Command::new("git").arg("-C").arg(&path).args(["add", "README.md"]).status().unwrap().success());
        assert!(std::process::Command::new("git").arg("-C").arg(&path).args(["commit", "-m", "fixture"]).status().unwrap().success());
        Self { root, path }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn worktree_mode_creates_branch_without_switching_or_carrying_dirty_files() {
    let repo = TempRepo::new("worktree");
    std::fs::write(repo.path.join("dirty.txt"), "not committed").unwrap();
    let original = git_output(&repo.path, &["branch", "--show-current"]);
    let worktree = repo.root.join("api-worktrees/nodestorm/cache-redesign");
    let mut req = request(AgentKind::Claude);
    req.repository = repo.path.to_string_lossy().into_owned();
    req.git_mode = GitMode::NewWorktree { path: worktree.to_string_lossy().into_owned() };

    let prepared = prepare_local(&req, false).unwrap();

    assert_eq!(prepared.directory, worktree.to_string_lossy().into_owned());
    assert_eq!(git_output(&repo.path, &["branch", "--show-current"]), original);
    assert!(!worktree.join("dirty.txt").exists());
    assert_eq!(git_output(&worktree, &["branch", "--show-current"]), "nodestorm/cache-redesign");
}

#[test]
fn existing_checkout_requires_dirty_confirmation_then_switches() {
    let repo = TempRepo::new("existing");
    std::fs::write(repo.path.join("dirty.txt"), "keep me").unwrap();
    let mut req = request(AgentKind::Claude);
    req.repository = repo.path.to_string_lossy().into_owned();
    req.git_mode = GitMode::ExistingCheckout;

    assert!(inspect_local(&req).unwrap().dirty);
    assert!(prepare_local(&req, false).unwrap_err().to_string().contains("uncommitted changes"));
    let prepared = prepare_local(&req, true).unwrap();
    assert_eq!(prepared.directory, repo.path.to_string_lossy().into_owned());
    assert_eq!(git_output(&repo.path, &["branch", "--show-current"]), "nodestorm/cache-redesign");
    assert!(repo.path.join("dirty.txt").exists());
}

#[test]
fn collisions_and_invalid_branches_do_not_mutate_repository() {
    let repo = TempRepo::new("invalid");
    let original = git_output(&repo.path, &["branch", "--show-current"]);
    let mut req = request(AgentKind::Claude);
    req.repository = repo.path.to_string_lossy().into_owned();
    req.branch = "bad branch".into();
    assert!(prepare_local(&req, false).is_err());
    assert_eq!(git_output(&repo.path, &["branch", "--show-current"]), original);

    assert!(std::process::Command::new("git").arg("-C").arg(&repo.path).args(["branch", "already-there"]).status().unwrap().success());
    req.branch = "already-there".into();
    assert!(prepare_local(&req, false).unwrap_err().to_string().contains("already exists"));

    let occupied = repo.root.join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    req.branch = "free-branch".into();
    req.git_mode = GitMode::NewWorktree { path: occupied.to_string_lossy().into_owned() };
    assert!(prepare_local(&req, false).unwrap_err().to_string().contains("destination"));
    assert_eq!(git_output(&repo.path, &["branch", "--show-current"]), original);
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test agent_launcher::tests --lib -- --nocapture`

Expected: compilation fails because the inspection/preparation API is absent.

- [ ] **Step 3: Implement direct-command Git preparation**

Implement:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInspection { pub dirty: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorkspace { pub directory: String, pub retained_path: String }

fn checked_output(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new(program).args(args).output()?;
    anyhow::ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn ensure_executable(program: &str) -> anyhow::Result<()> {
    checked_output(program, &["--version"])
        .map(|_| ())
        .map_err(|err| anyhow::anyhow!("`{program}` is unavailable: {err}"))
}

fn git(repository: &str, args: &[&str]) -> anyhow::Result<String> {
    let mut all = vec!["-C", repository];
    all.extend_from_slice(args);
    checked_output("git", &all)
}

pub fn inspect_local(req: &LaunchRequest) -> anyhow::Result<LocalInspection> {
    validate_request(req)?;
    anyhow::ensure!(matches!(req.target, LaunchTarget::Local), "local inspection requires a local target");
    git(&req.repository, &["rev-parse", "--show-toplevel"])?;
    git(&req.repository, &["check-ref-format", "--branch", &req.branch])?;
    let exists = std::process::Command::new("git").args(["-C", &req.repository, "show-ref", "--verify", "--quiet", &format!("refs/heads/{}", req.branch)]).status()?;
    anyhow::ensure!(!exists.success(), "branch `{}` already exists", req.branch);
    if let GitMode::NewWorktree { path } = &req.git_mode { anyhow::ensure!(!Path::new(path).exists(), "worktree destination `{path}` already exists"); }
    Ok(LocalInspection { dirty: !git(&req.repository, &["status", "--porcelain"])?.is_empty() })
}

pub fn prepare_local(req: &LaunchRequest, allow_dirty: bool) -> anyhow::Result<PreparedWorkspace> {
    let inspection = inspect_local(req)?;
    if matches!(req.git_mode, GitMode::ExistingCheckout) && inspection.dirty && !allow_dirty {
        anyhow::bail!("the existing checkout has uncommitted changes; confirm before creating the branch");
    }
    match &req.git_mode {
        GitMode::ExistingCheckout => {
            git(&req.repository, &["switch", "-c", &req.branch])?;
            Ok(PreparedWorkspace { directory: req.repository.clone(), retained_path: req.repository.clone() })
        }
        GitMode::NewWorktree { path } => {
            if let Some(parent) = Path::new(path).parent() { std::fs::create_dir_all(parent)?; }
            git(&req.repository, &["worktree", "add", "-b", &req.branch, path, "HEAD"])?;
            Ok(PreparedWorkspace { directory: path.clone(), retained_path: path.clone() })
        }
    }
}
```

Use owned `String` argument vectors where temporary `format!` values make borrowed slices invalid. Add `PathBuf` to the imports for the test helper. Keep every Git mutation after all validation checks. Add a small `ensure_executable("git")` test; the UI will pass the selected agent's fixed executable, while Git preparation tests remain independent of which agent CLIs happen to be installed on CI.

- [ ] **Step 4: Verify local Git behavior**

Run: `cargo test agent_launcher::tests --lib -- --nocapture`

Expected: all pure and temporary-repository tests pass.

- [ ] **Step 5: Commit Task 2**

```bash
git add src/agent_launcher.rs
git commit -m "feat: prepare isolated agent workspaces"
```

---

### Task 3: SSH Script and Platform Terminal Adapters

**Files:**
- Modify: `src/agent_launcher.rs`

**Interfaces:**
- Consumes: validated `LaunchRequest`, canonical session slug, and `CommandSpec` from Task 1.
- Produces: `remote_agent_command`, `TerminalFlavor`, `terminal_command`, `open_terminal`, and `read_ssh_aliases`.

- [ ] **Step 1: Write failing command-construction tests**

Add tests asserting:

```rust
#[test]
fn ssh_command_has_tty_tunnel_validation_and_quoted_values() {
    let mut req = request(AgentKind::Claude);
    req.target = LaunchTarget::Ssh { alias: "build-box".into() };
    req.repository = "/srv/repo with spaces".into();
    req.git_mode = GitMode::NewWorktree { path: "/srv/repo-worktrees/nodestorm/cache-redesign".into() };
    let spec = remote_agent_command(&req, "cache-redesign").unwrap();
    assert_eq!(spec.program, "ssh");
    assert_eq!(&spec.args[..5], ["-t", "-o", "ExitOnForwardFailure=yes", "-R", "4747:127.0.0.1:8123"]);
    assert!(spec.args.contains(&"build-box".into()));
    let script = spec.args.last().unwrap();
    assert!(script.contains("command -v 'claude'"));
    assert!(script.contains("'/srv/repo with spaces'"));
    assert!(script.contains("git -C"));
    assert!(script.contains("worktree add -b"));
    assert!(script.contains("exec 'claude'"));
    assert!(script.contains(&posix_quote(&compose_prompt(&req, "cache-redesign"))));
}

#[test]
fn terminal_wrappers_preserve_program_and_arguments() {
    let child = CommandSpec { program: "codex".into(), args: vec!["task with spaces".into()], current_dir: Some("/work/tree".into()) };
    let windows = terminal_command(TerminalFlavor::WindowsTerminal, &child);
    assert_eq!(windows.program, "wt.exe");
    assert_eq!(&windows.args[..5], ["-w", "new", "new-tab", "-d", "/work/tree"]);
    assert!(windows.args.ends_with(&["codex".into(), "task with spaces".into()]));

    let linux = terminal_command(TerminalFlavor::LinuxXTerminal, &child);
    assert_eq!(linux.program, "x-terminal-emulator");
    assert_eq!(&linux.args[..2], ["-e", "codex"]);

    let mac = terminal_command(TerminalFlavor::MacTerminal, &child);
    assert_eq!(mac.program, "osascript");
    assert!(mac.args.last().unwrap().contains("cd -- '/work/tree' && exec 'codex' 'task with spaces'"));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test agent_launcher::tests --lib`

Expected: compilation fails because remote and terminal construction functions are missing.

- [ ] **Step 3: Implement remote setup as one escaped POSIX script**

Build the remote script only from `posix_quote` values. It must execute this sequence:

```sh
set -u
command -v '<agent>' >/dev/null 2>&1 || { printf '%s\n' '<agent> is not installed'; exec "${SHELL:-/bin/sh}" -l; }
command -v 'git' >/dev/null 2>&1 || { printf '%s\n' 'git is not installed'; exec "${SHELL:-/bin/sh}" -l; }
git -C '<repository>' rev-parse --show-toplevel >/dev/null || exec "${SHELL:-/bin/sh}" -l
git -C '<repository>' check-ref-format --branch '<branch>' >/dev/null || exec "${SHELL:-/bin/sh}" -l
git -C '<repository>' show-ref --verify --quiet 'refs/heads/<branch>' && { printf '%s\n' 'branch already exists'; exec "${SHELL:-/bin/sh}" -l; }
mkdir -p '<worktree-parent>'
git -C '<repository>' worktree add -b '<branch>' '<worktree>' HEAD || exec "${SHELL:-/bin/sh}" -l
cd '<worktree>' || exec "${SHELL:-/bin/sh}" -l
exec '<agent>' '<prompt>'
```

For existing-checkout mode, replace the `mkdir`/`worktree add` steps with a dirty check that reads `y` or `yes`, followed by `git switch -c`. Construct the `ssh` arguments as `-t`, `-o`, `ExitOnForwardFailure=yes`, `-R`, `4747:127.0.0.1:<port>`, `--`, alias, `sh`, `-lc`, script. Placing `--` before the editable alias prevents an alias beginning with `-` from becoming an SSH option.

- [ ] **Step 4: Implement terminal wrappers and runtime selection**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFlavor { WindowsTerminal, MacTerminal, LinuxXTerminal, LinuxGnome, LinuxKonsole }
```

`terminal_command` must produce:

- Windows: `wt.exe -w new new-tab -d <cwd> <program> <args...>`.
- macOS: `osascript` with an `on run argv` script that asks Terminal to `do script (item 1 of argv)`; pass one POSIX-serialized `cd -- <cwd> && exec <program> <args...>` as the final `osascript` argument.
- `x-terminal-emulator`: `x-terminal-emulator -e <program> <args...>` with `current_dir` set on the wrapper process.
- GNOME Terminal: `gnome-terminal -- <program> <args...>` with `current_dir` set.
- Konsole: `konsole -e <program> <args...>` with `current_dir` set.

`open_terminal` selects the compile-target native adapter. On Linux, try the three listed terminals in order and continue only on `ErrorKind::NotFound`; return any other spawn error immediately. On Windows and macOS, return an actionable error naming Windows Terminal or Terminal.app if spawning fails. Successful spawn returns immediately without monitoring or capturing the terminal.

Read SSH alias suggestions from `directories::BaseDirs::new().home_dir().join(".ssh/config")`; missing or unreadable configuration returns an empty list because the alias field remains editable.

- [ ] **Step 5: Verify adapters and lint**

Run: `cargo test agent_launcher::tests --lib && cargo clippy --lib -- -D warnings`

Expected: all launcher tests pass and Clippy emits no warnings.

- [ ] **Step 6: Commit Task 3**

```bash
git add src/agent_launcher.rs
git commit -m "feat: launch agents locally or over ssh"
```

---

### Task 4: Agent Launcher Modal and Entry Points

**Files:**
- Create: `src/ui/agent_launcher.rs`
- Modify: `src/ui/mod.rs:3-20,56-93`
- Modify: `src/ui/app.rs:18-25,35-52,130-191`
- Modify: `src/ui/topbar.rs:21-31,165-243`
- Modify: `assets/main.css:380-465,1130-1307`

**Interfaces:**
- Consumes: Tasks 1-3 launcher API, `Arc<Sessions>`, and `Cli::port` from Dioxus context.
- Produces: `AgentLauncherOpen(Signal<bool>)` context and `AgentLauncher` component.

- [ ] **Step 1: Write the failing draft-default test**

Create `src/ui/agent_launcher.rs`, register it in `src/ui/mod.rs`, and add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchDraft {
    session_name: String,
    task: String,
    agent: AgentKind,
    remote: bool,
    ssh_alias: String,
    repository: String,
    branch: String,
    worktree: bool,
    worktree_path: String,
    branch_edited: bool,
    worktree_edited: bool,
}

impl Default for LaunchDraft {
    fn default() -> Self {
        Self {
            session_name: String::new(), task: String::new(), agent: AgentKind::Claude,
            remote: false, ssh_alias: String::new(), repository: String::new(),
            branch: suggest_branch(""), worktree: true, worktree_path: String::new(),
            branch_edited: false, worktree_edited: false,
        }
    }
}

#[test]
fn draft_defaults_to_local_claude_new_worktree() {
    let draft = LaunchDraft::default();
    assert_eq!(draft.agent, AgentKind::Claude);
    assert!(!draft.remote);
    assert!(draft.worktree);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test ui::agent_launcher::tests --lib`

Expected: compilation fails until the module imports and draft implementation are complete.

- [ ] **Step 3: Add shared modal-open context and both entry points**

In `src/ui/mod.rs`, add:

```rust
mod agent_launcher;

#[derive(Clone, Copy)]
pub(crate) struct AgentLauncherOpen(pub Signal<bool>);
```

In `App`, provide `AgentLauncherOpen(Signal::new(false))`, mount `AgentLauncher {}` when true, and replace the single empty-state command with an action row containing:

```rust
button {
    class: "btn btn-primary",
    onclick: move |_| launcher_open.set(true),
    "Start an agentic session"
}
```

Keep the existing copyable Claude MCP command as the secondary action. In the session menu, insert a full-width **Start agent** button immediately above **Manage session**; close the session dropdown before opening the modal. Do not alter the existing canvas-only create form.

- [ ] **Step 4: Implement the complete accessible form**

Render a fixed overlay with `role="dialog"`, `aria_modal="true"`, heading **Start agentic session**, and buttons **Cancel** and **Create branch & start agent**. Include labeled controls for every approved field. Use radio buttons for Local/SSH and existing-checkout/new-worktree; use a `<select>` for agent; use `<input list="ssh-hosts">` plus `<datalist>` for parsed aliases.

Field behavior:

- Updating session name refreshes branch and worktree suggestions only while their edited flags are false.
- Updating repository refreshes worktree suggestion only while its edited flag is false.
- Toggling SSH recalculates worktree paths using POSIX semantics and reveals the alias control.
- New worktree is checked initially and displays the editable worktree path.
- Existing checkout hides worktree path and displays a warning after `inspect_local` reports dirty state.
- Escape and Cancel close the modal only while no launch is running.
- The submit button is disabled while running and reads **Preparing…**.
- Errors render in an element with `role="alert"`.

- [ ] **Step 5: Implement launch orchestration without blocking the UI thread**

On submit, build `LaunchRequest` from the draft and run the local preparation path through `tokio::task::spawn_blocking`. For Local:

1. Call `ensure_executable(req.agent.executable())`.
2. Call `inspect_local`.
3. If existing-checkout mode is dirty and not confirmed, return to the form with the confirmation warning.
4. Call `prepare_local`.
5. Call `sessions.create`, then `sessions.switch`.
6. Compose the prompt, build `agent_command`, and call `open_terminal`.

For SSH:

1. Call `validate_request` locally.
2. Call `sessions.create`, then `sessions.switch`.
3. Build `remote_agent_command` and call `open_terminal`.

Close the modal only after the terminal process spawns. If terminal spawning fails after a local workspace was created, retain `PreparedWorkspace` in component state, show its `retained_path`, and make **Retry terminal** rerun only prompt/terminal construction without repeating Git. Remote errors after terminal spawn stay in that terminal as designed.

- [ ] **Step 6: Add focused responsive styling**

Add `.agent-launch-overlay`, `.agent-launch-dialog`, `.agent-launch-grid`, `.agent-launch-field`, `.agent-launch-options`, `.agent-launch-actions`, `.agent-launch-warning`, `.agent-launch-error`, and `.empty-actions`. Reuse existing colors, borders, inputs, `.btn`, and `.btn-primary`. Set dialog width to `min(680px, calc(100vw - 32px))`, max height to `calc(100vh - 32px)`, and `overflow-y: auto`. At `max-width: 680px`, collapse the two-column grid to one column and stack action buttons.

- [ ] **Step 7: Verify UI compilation and targeted tests**

Run: `cargo test ui::agent_launcher::tests --lib && cargo test --lib && cargo fmt --check`

Expected: launcher defaults pass, the full library suite remains green, and formatting is clean.

- [ ] **Step 8: Commit Task 4**

```bash
git add src/ui/agent_launcher.rs src/ui/mod.rs src/ui/app.rs src/ui/topbar.rs assets/main.css
git commit -m "feat: start agent sessions from nodestorm"
```

---

### Task 5: Documentation and Windows UI Contract

**Files:**
- Modify: `README.md:117-123,161-181,226-254`
- Modify: `scripts/verify-windows.ps1:195-217`

**Interfaces:**
- Consumes: final visible labels and behavior from Task 4.
- Produces: user documentation and a non-launching Windows UIA assertion of the new workflow.

- [ ] **Step 1: Add a failing UIA expectation before the existing session round-trip**

Extend `scripts/verify-windows.ps1` to open the session menu, click **Start agent**, and assert these accessible names exist:

```powershell
if (-not (Wait-Element 'Start agent' 5)) { Fail 'agent launcher entry missing' }
Click-Element $hwnd 'Start agent'
if (-not (Wait-Element 'Start agentic session' 5)) { Fail 'agent launcher did not open' }
if (-not (Wait-Element 'Claude Code' 5)) { Fail 'Claude Code is not the default agent' }
if (-not (Wait-Element 'Local' 5)) { Fail 'Local target control missing' }
if (-not (Wait-Element 'New worktree (recommended)' 5)) { Fail 'new-worktree default missing' }
Click-Element $hwnd 'Cancel'
```

Do not click the submit button in automated UIA; release CI must not depend on an installed agent CLI or create Git branches.

- [ ] **Step 2: Run the Windows script when a Windows runner is available**

Run: `powershell -ExecutionPolicy Bypass -File scripts/verify-windows.ps1`

Expected: the launcher opens with the approved defaults and closes without starting an agent. On non-Windows development hosts, verify the script parses by inspection and rely on the Windows CI job for execution.

- [ ] **Step 3: Document launch behavior and prerequisites**

Add a **Start an agent from Nodestorm** section after **Try it without an agent**. Document the exact four CLIs, local/SSH selector, required fields, default sibling-worktree mode, existing-checkout dirty-change behavior, `~/.ssh/config` aliases, Linux-only SSH targets, reverse tunnel, remote plugin prerequisite, and retained branches/worktrees after partial failure. Add one sentence to **Sessions** clarifying that canvas-only creation remains available.

- [ ] **Step 4: Run documentation/package tests**

Run: `node --test tests/*.mjs`

Expected: all JavaScript contract, installer, adapter, and release-gate tests pass.

- [ ] **Step 5: Commit Task 5**

```bash
git add README.md scripts/verify-windows.ps1
git commit -m "docs: explain agent session launching"
```

---

### Task 6: Full Verification and Review

**Files:**
- Review only: all files changed in Tasks 1-5

**Interfaces:**
- Consumes: completed feature.
- Produces: evidence that the feature and existing contracts pass together.

- [ ] **Step 1: Format and inspect the final diff**

Run:

```bash
cargo fmt
git diff --check
git status --short
git diff --stat origin/main...HEAD
```

Expected: no whitespace errors and only the approved launcher, UI, docs, test, spec, and plan files are changed.

- [ ] **Step 2: Run the full Rust suite**

Run: `cargo test`

Expected: all unit, integration, MCP round-trip, Git-workspace, and launcher tests pass.

- [ ] **Step 3: Run strict linting**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: no warnings or errors.

- [ ] **Step 4: Run host/package suites**

Run: `node --test tests/*.mjs`

Expected: all Node tests pass.

- [ ] **Step 5: Perform safe manual smoke checks**

With throwaway local and Linux SSH repositories, verify:

1. Local default mode creates the sibling worktree and opens each installed agent with the initial task.
2. Existing-checkout mode warns on dirty changes before switching.
3. SSH mode prompts through system SSH, creates the remote worktree, and exposes Nodestorm at remote loopback port 4747.
4. A deliberately occupied remote port produces the `ExitOnForwardFailure` error without silently changing SSH security policy.
5. Canceling the modal creates no session, branch, or worktree.

Do not use a repository containing uncommitted work for these checks.

- [ ] **Step 6: Request code review**

Invoke `superpowers:requesting-code-review` and review the complete diff against `docs/superpowers/specs/2026-07-19-agent-session-launcher-design.md`. Resolve correctness, security, and spec-coverage findings, then rerun Steps 1-4.
