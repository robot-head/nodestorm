//! Start a local or SSH coding-agent session from Nodestorm.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::agent_launcher::{
    AgentKind, CommandSpec, GitMode, LaunchRequest, LaunchTarget, PreparedWorkspace, agent_command,
    compose_prompt, ensure_executable, inspect_local, open_terminal, prepare_local,
    read_ssh_aliases, remote_agent_command, suggest_branch, suggest_worktree, validate_request,
};
use crate::cli::Cli;
use crate::prefs::Preferences;
use crate::sessions::Sessions;

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
    integrated: bool,
}

impl Default for LaunchDraft {
    fn default() -> Self {
        Self {
            session_name: String::new(),
            task: String::new(),
            agent: AgentKind::Claude,
            remote: false,
            ssh_alias: String::new(),
            repository: String::new(),
            branch: String::new(),
            worktree: true,
            worktree_path: String::new(),
            branch_edited: false,
            worktree_edited: false,
            integrated: true,
        }
    }
}

impl LaunchDraft {
    fn set_session_name(&mut self, name: String) {
        self.session_name = name;
        if !self.branch_edited {
            self.branch = if self.session_name.trim().is_empty() {
                String::new()
            } else {
                suggest_branch(&self.session_name)
            };
        }
        self.refresh_worktree();
    }

    fn set_repository(&mut self, repository: String) {
        self.repository = repository;
        self.refresh_worktree();
    }

    fn set_remote(&mut self, remote: bool) {
        self.remote = remote;
        self.refresh_worktree();
    }

    fn refresh_worktree(&mut self) {
        if self.worktree_edited {
            return;
        }
        self.worktree_path = if self.repository.trim().is_empty() || self.branch.is_empty() {
            String::new()
        } else {
            suggest_worktree(&self.repository, &self.branch, self.remote).unwrap_or_default()
        };
    }

    fn request(&self, mcp_port: u16) -> LaunchRequest {
        LaunchRequest {
            session_name: self.session_name.clone(),
            task: self.task.clone(),
            agent: self.agent,
            target: if self.remote {
                LaunchTarget::Ssh {
                    alias: self.ssh_alias.clone(),
                }
            } else {
                LaunchTarget::Local
            },
            repository: self.repository.clone(),
            branch: self.branch.clone(),
            git_mode: if self.worktree {
                GitMode::NewWorktree {
                    path: self.worktree_path.clone(),
                }
            } else {
                GitMode::ExistingCheckout
            },
            mcp_port,
        }
    }
}

enum LaunchOutcome {
    Started {
        terminal: Option<String>,
    },
    NeedsDirtyConfirmation,
    Failed {
        message: String,
        retained: Option<String>,
    },
    TerminalFailed {
        message: String,
        retained: Option<String>,
        command: CommandSpec,
        terminal: Option<String>,
    },
}

fn failed(error: impl std::fmt::Display, retained: Option<String>) -> LaunchOutcome {
    LaunchOutcome::Failed {
        message: error.to_string(),
        retained,
    }
}

fn perform_launch(
    request: LaunchRequest,
    sessions: Arc<Sessions>,
    allow_dirty: bool,
    terminals: Option<Arc<crate::terminal::TerminalManager>>,
) -> LaunchOutcome {
    let prepared = match &request.target {
        LaunchTarget::Local => {
            if let Err(err) = ensure_executable("git") {
                return failed(err, None);
            }
            if let Err(err) = ensure_executable(request.agent.executable()) {
                return failed(err, None);
            }
            let inspection = match inspect_local(&request) {
                Ok(inspection) => inspection,
                Err(err) => return failed(err, None),
            };
            if needs_dirty_confirmation(&request.git_mode, inspection.dirty, allow_dirty) {
                return LaunchOutcome::NeedsDirtyConfirmation;
            }
            match prepare_local(&request, allow_dirty) {
                Ok(prepared) => Some(prepared),
                Err(err) => return failed(err, None),
            }
        }
        LaunchTarget::Ssh { .. } => {
            if let Err(err) = ensure_executable("ssh") {
                return failed(err, None);
            }
            if let Err(err) = validate_request(&request) {
                return failed(err, None);
            }
            None
        }
    };

    let retained = prepared
        .as_ref()
        .map(|workspace| workspace.retained_path.clone());
    let slug = match sessions.create(&request.session_name) {
        Ok(slug) => slug,
        Err(err) => return failed(err, retained),
    };
    if let Err(err) = sessions.switch(&slug) {
        return failed(err, retained);
    }

    let command = match &request.target {
        LaunchTarget::Local => {
            let PreparedWorkspace { directory, .. } = prepared.expect("local workspace prepared");
            let prompt = compose_prompt(&request, &slug);
            agent_command(request.agent, &slug, &prompt, &directory)
        }
        LaunchTarget::Ssh { .. } => match remote_agent_command(&request, &slug) {
            Ok(command) => command,
            Err(err) => return failed(err, retained),
        },
    };

    match terminals {
        Some(manager) => {
            let terminal_id = format!("{}-{slug}", request.agent.id());
            match manager.spawn(&terminal_id, &command) {
                Ok(()) => LaunchOutcome::Started {
                    terminal: Some(terminal_id),
                },
                Err(err) => LaunchOutcome::TerminalFailed {
                    message: err.to_string(),
                    retained,
                    command,
                    terminal: Some(terminal_id),
                },
            }
        }
        None => match open_terminal(&command) {
            Ok(()) => LaunchOutcome::Started { terminal: None },
            Err(err) => LaunchOutcome::TerminalFailed {
                message: err.to_string(),
                retained,
                command,
                terminal: None,
            },
        },
    }
}

fn needs_dirty_confirmation(mode: &GitMode, dirty: bool, allow_dirty: bool) -> bool {
    matches!(mode, GitMode::ExistingCheckout) && dirty && !allow_dirty
}

fn agent_from_value(value: &str) -> AgentKind {
    match value {
        "codex" => AgentKind::Codex,
        "opencode" => AgentKind::OpenCode,
        "pi" => AgentKind::Pi,
        _ => AgentKind::Claude,
    }
}

#[derive(Clone, Copy)]
struct LaunchSignals {
    running: Signal<bool>,
    dirty_warning: Signal<bool>,
    error: Signal<Option<String>>,
    retained: Signal<Option<String>>,
    retry: Signal<Option<(CommandSpec, String, Option<String>)>>,
    open: Signal<bool>,
}

fn remember_repository(prefs: &mut Preferences, cli: &Cli, repo: &str) {
    if prefs.record_repository(repo)
        && let Err(err) = cli
            .prefs_path()
            .and_then(|path| crate::prefs::save(&path, prefs))
    {
        tracing::warn!(%err, "saving recent repositories failed");
    }
}

// ponytail: 8 independent inputs (request/sessions config, run-target
// context, output signals) don't share a natural grouping beyond what
// LaunchSignals already bundles; allow rather than force an artificial struct.
#[allow(clippy::too_many_arguments)]
fn start_attempt(
    request: LaunchRequest,
    sessions: Arc<Sessions>,
    allow_dirty: bool,
    terminals: Option<Arc<crate::terminal::TerminalManager>>,
    panel: super::TerminalPanel,
    mut prefs: Signal<Preferences>,
    cli: Cli,
    mut signals: LaunchSignals,
) {
    let repo = request.repository.clone();
    signals.running.set(true);
    signals.error.set(None);
    signals.retained.set(None);
    signals.retry.set(None);
    spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || {
            perform_launch(request, sessions, allow_dirty, terminals)
        })
        .await
        .unwrap_or_else(|err| failed(format!("launcher worker failed: {err}"), None));
        signals.running.set(false);
        match outcome {
            LaunchOutcome::Started { terminal } => {
                if let Some(id) = terminal {
                    super::focus_terminal(&panel, &id);
                }
                remember_repository(&mut prefs.write(), &cli, &repo);
                signals.open.set(false);
            }
            LaunchOutcome::NeedsDirtyConfirmation => signals.dirty_warning.set(true),
            LaunchOutcome::Failed {
                message,
                retained: path,
            } => {
                signals.error.set(Some(message));
                signals.retained.set(path);
            }
            LaunchOutcome::TerminalFailed {
                message,
                retained: path,
                command,
                terminal,
            } => {
                signals.error.set(Some(message));
                signals.retained.set(path);
                signals.retry.set(Some((command, repo, terminal)));
            }
        }
    });
}

#[component]
pub fn AgentLauncher() -> Element {
    let sessions = use_context::<Arc<Sessions>>();
    let cli = use_context::<Cli>();
    let terminal_manager = use_context::<Arc<crate::terminal::TerminalManager>>();
    let panel = use_context::<super::TerminalPanel>();
    let mut open = use_context::<super::AgentLauncherOpen>().0;
    let mut prefs = use_context::<super::ThemePref>().0;
    let mut draft = use_signal(LaunchDraft::default);
    let aliases = use_signal(read_ssh_aliases);
    let mut running = use_signal(|| false);
    let dirty_warning = use_signal(|| false);
    let mut allow_dirty = use_signal(|| false);
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let retained: Signal<Option<String>> = use_signal(|| None);
    let retry: Signal<Option<(CommandSpec, String, Option<String>)>> = use_signal(|| None);

    rsx! {
        div { class: "agent-launch-overlay",
            div {
                class: "agent-launch-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: "agent-launch-title",
                tabindex: "0",
                onkeydown: move |event| {
                    if event.key() == Key::Escape && !running() {
                        open.set(false);
                    }
                },
                div { class: "agent-launch-heading",
                    div {
                        h2 { id: "agent-launch-title", "Start agentic session" }
                        p { "Create an isolated branch and open an interactive coding agent." }
                    }
                    button {
                        class: "panel-close",
                        aria_label: "Close agent launcher",
                        disabled: running(),
                        onclick: move |_| open.set(false),
                        "✕"
                    }
                }

                div { class: "agent-launch-grid",
                    label { class: "agent-launch-field",
                        span { "Session name" }
                        input {
                            id: "agent-session-name",
                            placeholder: "cache redesign",
                            value: "{draft.read().session_name}",
                            oninput: move |event| draft.write().set_session_name(event.value()),
                        }
                    }
                    label { class: "agent-launch-field",
                        span { "Agent" }
                        select {
                            id: "agent-kind",
                            value: "{draft.read().agent.id()}",
                            oninput: move |event| draft.write().agent = agent_from_value(&event.value()),
                            option { value: "claude", "Claude Code" }
                            option { value: "codex", "Codex" }
                            option { value: "opencode", "OpenCode" }
                            option { value: "pi", "Pi" }
                        }
                    }
                    fieldset { class: "agent-launch-field agent-launch-options",
                        legend { "Target" }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-target",
                                checked: !draft.read().remote,
                                oninput: move |_| draft.write().set_remote(false),
                            }
                            "Local"
                        }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-target",
                                checked: draft.read().remote,
                                oninput: move |_| draft.write().set_remote(true),
                            }
                            "SSH"
                        }
                    }
                    fieldset { class: "agent-launch-field agent-launch-options",
                        legend { "Run in" }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-run-in",
                                checked: draft.read().integrated,
                                oninput: move |_| draft.write().integrated = true,
                            }
                            "Integrated terminal"
                        }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-run-in",
                                checked: !draft.read().integrated,
                                oninput: move |_| draft.write().integrated = false,
                            }
                            "System terminal"
                        }
                    }
                    if draft.read().remote {
                        label { class: "agent-launch-field",
                            span { "SSH host alias" }
                            input {
                                id: "ssh-host-alias",
                                list: "ssh-hosts",
                                placeholder: "build-box",
                                value: "{draft.read().ssh_alias}",
                                oninput: move |event| draft.write().ssh_alias = event.value(),
                            }
                            datalist { id: "ssh-hosts",
                                for alias in aliases.read().iter() {
                                    option { key: "{alias}", value: "{alias}" }
                                }
                            }
                        }
                    }
                    label { class: "agent-launch-field agent-launch-wide",
                        span { if draft.read().remote { "Remote repository path" } else { "Repository path" } }
                        input {
                            id: "agent-repository",
                            list: "recent-repositories",
                            placeholder: if draft.read().remote { "/srv/projects/api" } else { "/home/me/projects/api" },
                            value: "{draft.read().repository}",
                            oninput: move |event| draft.write().set_repository(event.value()),
                        }
                        datalist { id: "recent-repositories",
                            for repo in prefs.read().recent_repositories.iter() {
                                option { key: "{repo}", value: "{repo}" }
                            }
                        }
                    }
                    label { class: "agent-launch-field agent-launch-wide",
                        span { "Initial task" }
                        textarea {
                            id: "agent-task",
                            placeholder: "Describe what the agent should design or build…",
                            value: "{draft.read().task}",
                            oninput: move |event| draft.write().task = event.value(),
                        }
                    }
                    label { class: "agent-launch-field agent-launch-wide",
                        span { "Branch name" }
                        input {
                            id: "agent-branch",
                            placeholder: "nodestorm/cache-redesign",
                            value: "{draft.read().branch}",
                            oninput: move |event| {
                                let mut value = draft.write();
                                value.branch = event.value();
                                value.branch_edited = true;
                                value.refresh_worktree();
                            },
                        }
                    }
                    fieldset { class: "agent-launch-field agent-launch-options agent-launch-wide",
                        legend { "Git workspace" }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-workspace",
                                checked: !draft.read().worktree,
                                oninput: move |_| draft.write().worktree = false,
                            }
                            "Branch in existing checkout"
                        }
                        label {
                            input {
                                r#type: "radio",
                                name: "agent-workspace",
                                checked: draft.read().worktree,
                                oninput: move |_| draft.write().worktree = true,
                            }
                            "New worktree (recommended)"
                        }
                    }
                    if draft.read().worktree {
                        label { class: "agent-launch-field agent-launch-wide",
                            span { if draft.read().remote { "Remote worktree path" } else { "Worktree path" } }
                            input {
                                id: "agent-worktree",
                                value: "{draft.read().worktree_path}",
                                oninput: move |event| {
                                    let mut value = draft.write();
                                    value.worktree_path = event.value();
                                    value.worktree_edited = true;
                                },
                            }
                        }
                    }
                }

                if dirty_warning() {
                    label { class: "agent-launch-warning",
                        input {
                            r#type: "checkbox",
                            checked: allow_dirty(),
                            oninput: move |event| allow_dirty.set(event.checked()),
                        }
                        "This checkout has uncommitted changes. Carry them onto the new branch."
                    }
                }
                if let Some(message) = error.read().as_ref() {
                    div { class: "agent-launch-error", role: "alert", "{message}" }
                }
                if let Some(path) = retained.read().as_ref() {
                    p { class: "agent-launch-retained", "Created workspace retained at {path}." }
                }

                div { class: "agent-launch-actions",
                    button {
                        class: "btn",
                        disabled: running(),
                        onclick: move |_| open.set(false),
                        "Cancel"
                    }
                    if retry.read().is_some() {
                        button {
                            class: "btn btn-primary",
                            disabled: running(),
                            onclick: move |_| {
                                let (command, repo, terminal) = retry.read().clone().expect("retry command");
                                let cli = cli.clone();
                                let manager = terminal_manager.clone();
                                running.set(true);
                                error.set(None);
                                spawn(async move {
                                    let attempt = tokio::task::spawn_blocking({
                                        let terminal = terminal.clone();
                                        move || match &terminal {
                                            Some(id) => manager.spawn(id, &command).map_err(|e| e.to_string()),
                                            None => open_terminal(&command).map_err(|e| e.to_string()),
                                        }
                                    })
                                    .await;
                                    running.set(false);
                                    match attempt {
                                        Ok(Ok(())) => {
                                            if let Some(id) = &terminal {
                                                super::focus_terminal(&panel, id);
                                            }
                                            remember_repository(&mut prefs.write(), &cli, &repo);
                                            open.set(false);
                                        }
                                        Ok(Err(message)) => error.set(Some(message)),
                                        Err(err) => error.set(Some(format!("launcher worker failed: {err}"))),
                                    }
                                });
                            },
                            if running() { "Opening…" } else { "Retry terminal" }
                        }
                    } else {
                        button {
                            class: "btn btn-primary",
                            disabled: running() || (dirty_warning() && !allow_dirty()),
                            onclick: move |_| {
                                start_attempt(
                                    draft.read().request(cli.port),
                                    sessions.clone(),
                                    allow_dirty(),
                                    draft.read().integrated.then(|| terminal_manager.clone()),
                                    panel,
                                    prefs,
                                    cli.clone(),
                                    LaunchSignals {
                                        running,
                                        dirty_warning,
                                        error,
                                        retained,
                                        retry,
                                        open,
                                    },
                                );
                            },
                            if running() { "Preparing…" } else { "Create branch & start agent" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use yare::parameterized;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nodestorm-agent-launcher-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn successful_terminal_retry_records_repository() {
        let path = tmp_path("retry-prefs.json");
        let cli = Cli::parse_from(["nodestorm", "--prefs", path.to_str().unwrap()]);
        let mut prefs = Preferences::default();

        remember_repository(&mut prefs, &cli, "  /work/api  ");

        assert2::assert!((prefs.recent_repositories) == (["/work/api"]));
        assert2::assert!((crate::prefs::load_or_default(&path)) == (prefs));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn draft_defaults_to_local_claude_new_worktree() {
        let draft = LaunchDraft::default();
        assert2::assert!((draft.agent) == (crate::agent_launcher::AgentKind::Claude));
        assert2::assert!(!draft.remote);
        assert2::assert!(draft.worktree);
    }

    #[test]
    fn derived_fields_follow_session_and_repository_until_edited() {
        let mut draft = LaunchDraft::default();
        draft.set_repository("/work/api".into());
        draft.set_session_name("Cache Redesign".into());
        assert2::assert!((draft.branch) == ("nodestorm/cache-redesign"));
        assert2::assert!((draft.worktree_path) == ("/work/api-worktrees/nodestorm/cache-redesign"));

        draft.branch = "feature/custom".into();
        draft.branch_edited = true;
        draft.worktree_path = "/custom/tree".into();
        draft.worktree_edited = true;
        draft.set_session_name("Other Session".into());
        assert2::assert!((draft.branch) == ("feature/custom"));
        assert2::assert!((draft.worktree_path) == ("/custom/tree"));
    }

    #[parameterized(
        missing_repository = { "", "feature/test" },
        missing_branch = { "/work/api", "" },
    )]
    fn worktree_derivation_requires_both_repository_and_branch(repository: &str, branch: &str) {
        let mut draft = LaunchDraft {
            repository: repository.into(),
            branch: branch.into(),
            ..LaunchDraft::default()
        };
        draft.refresh_worktree();
        assert2::assert!(draft.worktree_path.is_empty());
    }

    #[parameterized(
        dirty_unconfirmed = { GitMode::ExistingCheckout, true, false, true },
        clean = { GitMode::ExistingCheckout, false, false, false },
        dirty_confirmed = { GitMode::ExistingCheckout, true, true, false },
        new_worktree = { GitMode::NewWorktree { path: "/tmp/tree".into() }, true, false, false },
    )]
    fn dirty_confirmation_covers_every_branch(
        mode: GitMode,
        dirty: bool,
        allow_dirty: bool,
        expected: bool,
    ) {
        assert2::assert!(needs_dirty_confirmation(&mode, dirty, allow_dirty) == expected);
    }

    #[parameterized(
        codex = { "codex", AgentKind::Codex },
        opencode = { "opencode", AgentKind::OpenCode },
        pi = { "pi", AgentKind::Pi },
        unknown_defaults_to_claude = { "unknown", AgentKind::Claude },
    )]
    fn agent_values_cover_every_branch(value: &str, expected: AgentKind) {
        assert2::assert!(agent_from_value(value) == expected);
    }

    #[test]
    fn draft_defaults_to_integrated_terminal() {
        assert2::assert!(LaunchDraft::default().integrated);
    }

    #[test]
    fn integrated_launch_spawns_a_terminal_and_reports_its_id() {
        let sessions =
            crate::sessions::Sessions::open(tmp_path("integrated-sessions"), None).unwrap();
        let terminals = crate::terminal::TerminalManager::new();
        // A repo-free request exercises only the terminal branch: SSH targets
        // skip local git preparation, and `ssh` exists on dev machines and CI.
        let request = LaunchRequest {
            session_name: "Integrated Test".into(),
            task: "do things".into(),
            agent: AgentKind::Claude,
            target: LaunchTarget::Ssh {
                alias: "nodestorm-test-invalid-host".into(),
            },
            repository: "/srv/repo".into(),
            branch: "nodestorm/integrated-test".into(),
            git_mode: GitMode::NewWorktree {
                path: "/srv/repo-worktrees/x".into(),
            },
            mcp_port: 4747,
        };

        let outcome = perform_launch(request, sessions, false, Some(terminals.clone()));

        let LaunchOutcome::Started { terminal: Some(id) } = outcome else {
            panic!("expected an integrated start");
        };
        assert2::assert!((id) == ("claude-integrated-test"));
        assert2::assert!(terminals.status(&id).is_some());
        terminals.close(&id);
        std::fs::remove_dir_all(tmp_path("integrated-sessions")).ok();
    }

    #[test]
    fn request_uses_remote_target_and_default_worktree() {
        let mut draft = LaunchDraft::default();
        draft.set_repository("/srv/api".into());
        draft.set_session_name("Remote Build".into());
        draft.task = "Implement the API".into();
        draft.set_remote(true);
        draft.ssh_alias = "build-box".into();

        let request = draft.request(9000);
        assert2::assert!((request.session_name) == ("Remote Build"));
        assert2::assert!((request.mcp_port) == (9000));
        assert2::assert!(
            (request.target)
                == (crate::agent_launcher::LaunchTarget::Ssh {
                    alias: "build-box".into()
                })
        );
        assert2::assert!(
            (request.git_mode)
                == (crate::agent_launcher::GitMode::NewWorktree {
                    path: "/srv/api-worktrees/nodestorm/remote-build".into()
                })
        );
    }
}
