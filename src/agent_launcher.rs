//! Create an isolated workspace and open an interactive coding agent.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
    OpenCode,
    Pi,
}

impl AgentKind {
    pub fn executable(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LaunchTarget {
    #[default]
    Local,
    Ssh {
        alias: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitMode {
    ExistingCheckout,
    NewWorktree { path: String },
}

impl Default for GitMode {
    fn default() -> Self {
        Self::NewWorktree {
            path: String::new(),
        }
    }
}

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
        let (parent, name) = repo
            .rsplit_once('/')
            .ok_or_else(|| anyhow::anyhow!("remote repository must be an absolute path"))?;
        anyhow::ensure!(!name.is_empty(), "remote repository has no directory name");
        let parent = if parent.is_empty() { "/" } else { parent };
        return Ok(format!(
            "{}/{name}-worktrees/{branch}",
            parent.trim_end_matches('/')
        ));
    }

    let repo = Path::new(repository);
    let parent = repo
        .parent()
        .ok_or_else(|| anyhow::anyhow!("repository has no parent directory"))?;
    let name = repo
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("repository has no directory name"))?
        .to_string_lossy();
    // Forward slashes everywhere: git accepts them on Windows, and the
    // suggestion stays identical across platforms (branch names already use /).
    Ok(parent
        .join(format!("{name}-worktrees"))
        .join(branch)
        .to_string_lossy()
        .replace('\\', "/"))
}

pub fn parse_ssh_aliases(text: &str) -> Vec<String> {
    let mut aliases = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            fields
                .next()
                .filter(|key| key.eq_ignore_ascii_case("host"))?;
            Some(
                fields
                    .filter(|host| !host.contains(['*', '?', '!']))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

pub fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn validate_request(req: &LaunchRequest) -> anyhow::Result<()> {
    for (value, name) in [
        (&req.session_name, "session name"),
        (&req.task, "initial task"),
        (&req.repository, "repository"),
        (&req.branch, "branch name"),
    ] {
        anyhow::ensure!(!value.trim().is_empty(), "{name} is required");
    }
    if let LaunchTarget::Ssh { alias } = &req.target {
        anyhow::ensure!(!alias.trim().is_empty(), "SSH host alias is required");
        anyhow::ensure!(
            req.repository.starts_with('/'),
            "remote repository must be an absolute path"
        );
        if let GitMode::NewWorktree { path } = &req.git_mode {
            anyhow::ensure!(
                path.starts_with('/'),
                "remote worktree path must be absolute"
            );
        }
    }
    if let GitMode::NewWorktree { path } = &req.git_mode {
        anyhow::ensure!(!path.trim().is_empty(), "worktree path is required");
    }
    Ok(())
}

pub fn compose_prompt(req: &LaunchRequest, slug: &str) -> String {
    format!(
        "{}\n\nUse the installed Nodestorm skill for design choices and implementation progress. Use Nodestorm session `{slug}` and agent identity `{}-{slug}` in MCP calls that support agent identity.",
        req.task,
        req.agent.id()
    )
}

pub fn agent_command(agent: AgentKind, slug: &str, prompt: &str, cwd: &str) -> CommandSpec {
    let args = match agent {
        AgentKind::Claude | AgentKind::Codex => vec![prompt.into()],
        AgentKind::OpenCode => vec!["--prompt".into(), prompt.into()],
        AgentKind::Pi => vec!["--name".into(), slug.into(), prompt.into()],
    };
    CommandSpec {
        program: agent.executable().into(),
        args,
        current_dir: Some(cwd.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldCheck {
    Unchecked,
    Valid,
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCheck {
    pub repository: FieldCheck,
    pub branch: FieldCheck,
    pub worktree: Option<FieldCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceProbe {
    Checked(WorkspaceCheck),
    PasswordOnly,
    TransportError(String),
}

impl WorkspaceCheck {
    pub fn require_valid(&self) -> anyhow::Result<()> {
        for check in [
            Some(&self.repository),
            Some(&self.branch),
            self.worktree.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            match check {
                FieldCheck::Unchecked => anyhow::bail!("workspace validation is incomplete"),
                FieldCheck::Invalid(message) => anyhow::bail!(message.clone()),
                FieldCheck::Valid => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInspection {
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorkspace {
    pub directory: String,
    pub retained_path: String,
}

fn checked_output(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new(program).args(args).output()?;
    anyhow::ensure!(
        output.status.success(),
        "`{program}` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn ensure_executable(program: &str) -> anyhow::Result<()> {
    let version_flag = if program == "ssh" { "-V" } else { "--version" };
    checked_output(program, &[version_flag])
        .map(|_| ())
        .map_err(|err| anyhow::anyhow!("`{program}` is unavailable: {err}"))
}

fn git(repository: &str, args: &[&str]) -> anyhow::Result<String> {
    let mut all = vec!["-C", repository];
    all.extend_from_slice(args);
    checked_output("git", &all)
}

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
            .args([
                "-C",
                &req.repository,
                "show-ref",
                "--verify",
                "--quiet",
                &reference,
            ])
            .status()
            .ok()
            .and_then(|status| status.code())
        {
            Some(0) => FieldCheck::Invalid(format!("branch `{}` already exists", req.branch)),
            Some(1) => FieldCheck::Valid,
            _ => FieldCheck::Invalid(format!("git could not inspect branch `{}`", req.branch)),
        }
    };
    let worktree = match &req.git_mode {
        GitMode::ExistingCheckout => None,
        GitMode::NewWorktree { path } if path.trim().is_empty() => {
            Some(FieldCheck::Invalid("worktree path is required".into()))
        }
        GitMode::NewWorktree { path } if Path::new(path).exists() => Some(FieldCheck::Invalid(
            format!("worktree destination `{path}` already exists"),
        )),
        GitMode::NewWorktree { .. } => Some(FieldCheck::Valid),
    };
    WorkspaceCheck {
        repository,
        branch,
        worktree,
    }
}

fn probe_line(name: &str, valid: bool, message: &str) -> String {
    format!(
        "printf 'NODESTORM_{name}\\t{}\\t{}\\n'",
        if valid { "valid" } else { "invalid" },
        message
    )
}

fn remote_probe_script(req: &LaunchRequest) -> anyhow::Result<String> {
    anyhow::ensure!(
        req.repository.starts_with('/'),
        "remote repository must be absolute"
    );
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
        format!(
            "if git -C {repo} rev-parse --is-inside-work-tree 2>/dev/null | grep -qx true; then repo_ok=1; {repository_valid}; else {repository_invalid}; fi"
        ),
        format!(
            "if ! git check-ref-format --branch {branch} >/dev/null 2>&1; then {branch_invalid}; elif [ \"$repo_ok\" -ne 1 ]; then {branch_needs_repo}; else branch_status=0; git -C {repo} show-ref --verify --quiet {reference} || branch_status=$?; if [ \"$branch_status\" -eq 0 ]; then {branch_exists}; elif [ \"$branch_status\" -eq 1 ]; then {branch_valid}; else {branch_error}; fi; fi"
        ),
    ];
    if let GitMode::NewWorktree { path } = &req.git_mode {
        anyhow::ensure!(
            path.starts_with('/'),
            "remote worktree path must be absolute"
        );
        let worktree = posix_quote(path);
        let exists = probe_line("WORKTREE", false, "worktree destination already exists");
        let available = probe_line("WORKTREE", true, "");
        lines.push(format!(
            "if [ -e {worktree} ]; then {exists}; else {available}; fi"
        ));
    }
    Ok(lines.join("\n"))
}

fn check_remote_input_with(
    req: &LaunchRequest,
    check_branch: impl FnOnce(&str) -> Option<bool>,
) -> Option<WorkspaceCheck> {
    let repository = if req.repository.trim().is_empty() {
        FieldCheck::Invalid("repository is required".into())
    } else if !req.repository.starts_with('/') {
        FieldCheck::Invalid("remote repository must be absolute".into())
    } else {
        FieldCheck::Unchecked
    };
    let branch = if matches!(repository, FieldCheck::Invalid(_)) {
        FieldCheck::Invalid("select a valid Git repository first".into())
    } else if req.branch.trim().is_empty() {
        FieldCheck::Invalid("branch name is invalid".into())
    } else {
        match check_branch(&req.branch) {
            Some(false) => FieldCheck::Invalid("branch name is invalid".into()),
            Some(true) | None => FieldCheck::Unchecked,
        }
    };
    let worktree = match &req.git_mode {
        GitMode::ExistingCheckout => None,
        GitMode::NewWorktree { path } if path.trim().is_empty() => {
            Some(FieldCheck::Invalid("worktree path is required".into()))
        }
        GitMode::NewWorktree { path } if !path.starts_with('/') => Some(FieldCheck::Invalid(
            "remote worktree path must be absolute".into(),
        )),
        GitMode::NewWorktree { .. } => Some(FieldCheck::Unchecked),
    };
    let checked = WorkspaceCheck {
        repository,
        branch,
        worktree,
    };
    let invalid = [
        Some(&checked.repository),
        Some(&checked.branch),
        checked.worktree.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|check| matches!(check, FieldCheck::Invalid(_)));
    invalid.then_some(checked)
}

fn check_remote_input(req: &LaunchRequest) -> Option<WorkspaceCheck> {
    check_remote_input_with(req, |branch| {
        std::process::Command::new("git")
            .args(["check-ref-format", "--branch", branch])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|status| status.success())
    })
}

pub fn remote_probe_command(req: &LaunchRequest) -> anyhow::Result<CommandSpec> {
    let LaunchTarget::Ssh { alias } = &req.target else {
        anyhow::bail!("remote workspace probe requires an SSH target");
    };
    anyhow::ensure!(!alias.trim().is_empty(), "SSH host alias is required");
    let script = remote_probe_script(req)?;
    Ok(CommandSpec {
        program: "ssh".into(),
        args: vec![
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ConnectTimeout=5".into(),
            "-o".into(),
            "NumberOfPasswordPrompts=0".into(),
            "-o".into(),
            "StrictHostKeyChecking=yes".into(),
            "--".into(),
            alias.clone(),
            "sh".into(),
            "-lc".into(),
            posix_quote(&script),
        ],
        current_dir: None,
    })
}

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
        if parts.len() != 3 {
            continue;
        }
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
        repository: repository
            .ok_or_else(|| anyhow::anyhow!("remote validation omitted repository status"))?,
        branch: branch.ok_or_else(|| anyhow::anyhow!("remote validation omitted branch status"))?,
        worktree: if wants_worktree {
            Some(
                worktree
                    .ok_or_else(|| anyhow::anyhow!("remote validation omitted worktree status"))?,
            )
        } else {
            None
        },
    })
}

pub fn password_only_rejection(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.lines().any(|line| {
        let Some((_, methods)) = line.split_once("permission denied (") else {
            return false;
        };
        let Some((methods, _)) = methods.split_once(')') else {
            return false;
        };
        methods
            .split(',')
            .any(|method| matches!(method.trim(), "password" | "keyboard-interactive"))
    })
}

fn parsed_remote_probe(output: &str, wants_worktree: bool) -> WorkspaceProbe {
    match parse_remote_probe(output, wants_worktree) {
        Ok(checked) => WorkspaceProbe::Checked(checked),
        Err(error) => WorkspaceProbe::TransportError(error.to_string()),
    }
}

const REMOTE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn terminate_and_reap(child: &mut std::process::Child) -> std::io::Result<()> {
    if let Err(error) = child.kill()
        && child.try_wait()?.is_none()
    {
        return Err(error);
    }
    child.wait().map(|_| ())
}

fn output_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<Option<std::process::Output>> {
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map(Some),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(
                    std::time::Duration::from_millis(10)
                        .min(timeout.saturating_sub(started.elapsed())),
                );
            }
            Ok(None) => {
                terminate_and_reap(&mut child)?;
                return Ok(None);
            }
            Err(error) => {
                let _ = terminate_and_reap(&mut child);
                return Err(error);
            }
        }
    }
}

fn run_remote_probe(
    command: &CommandSpec,
    wants_worktree: bool,
    timeout: std::time::Duration,
) -> WorkspaceProbe {
    let mut process = std::process::Command::new(&command.program);
    process
        .args(&command.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = &command.current_dir {
        process.current_dir(dir);
    }
    let child = match process.spawn() {
        Ok(child) => child,
        Err(error) => return WorkspaceProbe::TransportError(error.to_string()),
    };
    let output = match output_with_timeout(child, timeout) {
        Ok(Some(output)) => output,
        Ok(None) => {
            return WorkspaceProbe::TransportError("remote workspace probe timed out".into());
        }
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
    parsed_remote_probe(&String::from_utf8_lossy(&output.stdout), wants_worktree)
}

pub fn probe_workspace(req: &LaunchRequest) -> WorkspaceProbe {
    if matches!(req.target, LaunchTarget::Local) {
        return WorkspaceProbe::Checked(check_local_workspace(req));
    }
    if let Some(checked) = check_remote_input(req) {
        return WorkspaceProbe::Checked(checked);
    }
    let command = match remote_probe_command(req) {
        Ok(command) => command,
        Err(error) => return WorkspaceProbe::TransportError(error.to_string()),
    };
    run_remote_probe(
        &command,
        matches!(req.git_mode, GitMode::NewWorktree { .. }),
        REMOTE_PROBE_TIMEOUT,
    )
}

pub fn inspect_local(req: &LaunchRequest) -> anyhow::Result<LocalInspection> {
    validate_request(req)?;
    anyhow::ensure!(
        matches!(req.target, LaunchTarget::Local),
        "local inspection requires a local target"
    );
    check_local_workspace(req).require_valid()?;

    Ok(LocalInspection {
        dirty: !git(&req.repository, &["status", "--porcelain"])?.is_empty(),
    })
}

pub fn prepare_local(req: &LaunchRequest, allow_dirty: bool) -> anyhow::Result<PreparedWorkspace> {
    let inspection = inspect_local(req)?;
    if matches!(req.git_mode, GitMode::ExistingCheckout) && inspection.dirty && !allow_dirty {
        anyhow::bail!(
            "the existing checkout has uncommitted changes; confirm before creating the branch"
        );
    }

    match &req.git_mode {
        GitMode::ExistingCheckout => {
            git(&req.repository, &["switch", "-c", &req.branch])?;
            Ok(PreparedWorkspace {
                directory: req.repository.clone(),
                retained_path: req.repository.clone(),
            })
        }
        GitMode::NewWorktree { path } => {
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
            git(
                &req.repository,
                &["worktree", "add", "-b", &req.branch, path, "HEAD"],
            )?;
            Ok(PreparedWorkspace {
                directory: path.clone(),
                retained_path: path.clone(),
            })
        }
    }
}

fn diagnostic_shell(message: &str) -> String {
    format!(
        "{{ printf '%s\\n' {}; exec \"${{SHELL:-/bin/sh}}\" -l; }}",
        posix_quote(message)
    )
}

fn guarded(command: String, message: &str) -> String {
    format!("{command} || {}", diagnostic_shell(message))
}

fn remote_parent(path: &str) -> anyhow::Result<&str> {
    let (parent, name) = path
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("remote path must be absolute"))?;
    anyhow::ensure!(!name.is_empty(), "remote path has no directory name");
    Ok(if parent.is_empty() { "/" } else { parent })
}

fn remote_script(req: &LaunchRequest, slug: &str) -> anyhow::Result<String> {
    validate_request(req)?;
    let LaunchTarget::Ssh { .. } = &req.target else {
        anyhow::bail!("remote agent command requires an SSH target");
    };

    let executable = req.agent.executable();
    let repo = posix_quote(&req.repository);
    let branch = posix_quote(&req.branch);
    let reference = posix_quote(&format!("refs/heads/{}", req.branch));
    let mut script = vec![
        "set -u".to_owned(),
        guarded(
            format!("command -v {} >/dev/null 2>&1", posix_quote(executable)),
            &format!("{executable} is not installed"),
        ),
        guarded(
            "command -v 'git' >/dev/null 2>&1".to_owned(),
            "git is not installed",
        ),
        guarded(
            format!("git -C {repo} rev-parse --show-toplevel >/dev/null"),
            "repository is not a Git checkout",
        ),
        guarded(
            format!("git -C {repo} check-ref-format --branch {branch} >/dev/null"),
            "branch name is invalid",
        ),
        format!(
            "branch_status=0; git -C {repo} show-ref --verify --quiet {reference} || branch_status=$?"
        ),
        format!(
            "if [ \"$branch_status\" -eq 0 ]; then {}; fi",
            diagnostic_shell("branch already exists")
        ),
        format!(
            "if [ \"$branch_status\" -ne 1 ]; then {}; fi",
            diagnostic_shell("could not inspect remote branches")
        ),
    ];

    let directory = match &req.git_mode {
        GitMode::ExistingCheckout => {
            script.push(format!(
                "if [ -n \"$(git -C {repo} status --porcelain)\" ]; then printf '%s' 'This checkout has uncommitted changes. Carry them onto the new branch? [y/N] '; read answer; case \"$answer\" in y|Y|yes|YES) ;; *) {} ;; esac; fi",
                diagnostic_shell("launch cancelled; uncommitted changes were not moved")
            ));
            script.push(guarded(
                format!("git -C {repo} switch -c {branch}"),
                "could not create branch",
            ));
            req.repository.clone()
        }
        GitMode::NewWorktree { path } => {
            let worktree = posix_quote(path);
            let parent = posix_quote(remote_parent(path)?);
            script.push(format!(
                "if [ -e {worktree} ]; then {}; fi",
                diagnostic_shell("worktree destination already exists")
            ));
            script.push(guarded(
                format!("mkdir -p {parent}"),
                "could not create worktree parent directory",
            ));
            script.push(guarded(
                format!("git -C {repo} worktree add -b {branch} {worktree} HEAD"),
                "could not create worktree",
            ));
            path.clone()
        }
    };

    script.push(guarded(
        format!("cd -- {}", posix_quote(&directory)),
        "could not enter prepared workspace",
    ));
    let prompt = compose_prompt(req, slug);
    let agent = agent_command(req.agent, slug, &prompt, &directory);
    let mut command = format!("exec {}", posix_quote(&agent.program));
    for arg in &agent.args {
        command.push(' ');
        command.push_str(&posix_quote(arg));
    }
    script.push(command);

    Ok(script.join("\n"))
}

pub fn remote_agent_command(req: &LaunchRequest, slug: &str) -> anyhow::Result<CommandSpec> {
    let LaunchTarget::Ssh { alias } = &req.target else {
        anyhow::bail!("remote agent command requires an SSH target");
    };
    let script = remote_script(req, slug)?;
    Ok(CommandSpec {
        program: "ssh".into(),
        args: vec![
            "-t".into(),
            "-o".into(),
            "ExitOnForwardFailure=yes".into(),
            "-R".into(),
            format!("4747:127.0.0.1:{}", req.mcp_port),
            "--".into(),
            alias.clone(),
            format!("exec sh -lc {}", posix_quote(&script)),
        ],
        current_dir: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFlavor {
    MacTerminal,
    LinuxXTerminal,
    LinuxGnome,
    LinuxKonsole,
}

fn posix_command(spec: &CommandSpec) -> String {
    let mut command = spec
        .current_dir
        .as_ref()
        .map(|dir| format!("cd -- {} && ", posix_quote(dir)))
        .unwrap_or_default();
    command.push_str("exec ");
    command.push_str(&posix_quote(&spec.program));
    for arg in &spec.args {
        command.push(' ');
        command.push_str(&posix_quote(arg));
    }
    command
}

pub fn terminal_command(flavor: TerminalFlavor, child: &CommandSpec) -> CommandSpec {
    match flavor {
        TerminalFlavor::MacTerminal => CommandSpec {
            program: "osascript".into(),
            args: vec![
                "-e".into(),
                "on run argv".into(),
                "-e".into(),
                "tell application \"Terminal\" to activate".into(),
                "-e".into(),
                "tell application \"Terminal\" to do script (item 1 of argv)".into(),
                "-e".into(),
                "end run".into(),
                "--".into(),
                posix_command(child),
            ],
            current_dir: None,
        },
        TerminalFlavor::LinuxXTerminal => linux_terminal("x-terminal-emulator", &["-e"], child),
        TerminalFlavor::LinuxGnome => linux_terminal("gnome-terminal", &["--"], child),
        TerminalFlavor::LinuxKonsole => linux_terminal("konsole", &["-e"], child),
    }
}

fn linux_terminal(program: &str, prefix: &[&str], child: &CommandSpec) -> CommandSpec {
    let mut args = prefix
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<_>>();
    args.push(child.program.clone());
    args.extend(child.args.clone());
    CommandSpec {
        program: program.into(),
        args,
        current_dir: child.current_dir.clone(),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn spawn_command(spec: &CommandSpec) -> std::io::Result<()> {
    let mut command = std::process::Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(dir) = &spec.current_dir {
        command.current_dir(dir);
    }
    command.spawn().map(|_| ())
}

#[cfg(any(target_os = "linux", test))]
fn open_linux_terminal_with(
    child: &CommandSpec,
    mut spawn: impl FnMut(&CommandSpec) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    for flavor in [
        TerminalFlavor::LinuxXTerminal,
        TerminalFlavor::LinuxGnome,
        TerminalFlavor::LinuxKonsole,
    ] {
        match spawn(&terminal_command(flavor, child)) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        }
    }
    anyhow::bail!(
        "no supported terminal found; install x-terminal-emulator, gnome-terminal, or konsole"
    )
}

// Never route the child through `wt.exe`: Windows Terminal splits its command
// line on every unescaped `;` — even inside quoted arguments — so the SSH
// bootstrap script became one tab per fragment. CREATE_NEW_CONSOLE passes the
// arguments verbatim and still opens the user's default terminal.
#[cfg(target_os = "windows")]
fn spawn_windows_console(child: &CommandSpec) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let mut command = std::process::Command::new(&child.program);
    command.args(&child.args).creation_flags(CREATE_NEW_CONSOLE);
    if let Some(dir) = &child.current_dir {
        command.current_dir(dir);
    }
    command.spawn().map(|_| ())
}

// Each `#[cfg]` arm needs its own `return` because sibling arms follow it in
// source; only one compiles per target, so clippy sees the last as redundant.
#[allow(clippy::needless_return)]
pub fn open_terminal(child: &CommandSpec) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        return spawn_windows_console(child)
            .map_err(|err| anyhow::anyhow!("could not open a console window: {err}"));
    }
    #[cfg(target_os = "macos")]
    {
        return spawn_command(&terminal_command(TerminalFlavor::MacTerminal, child))
            .map_err(|err| anyhow::anyhow!("could not open Terminal.app: {err}"));
    }
    #[cfg(target_os = "linux")]
    {
        return open_linux_terminal_with(child, spawn_command);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!("this operating system has no terminal launcher")
    }
}

pub fn read_ssh_aliases() -> Vec<String> {
    let Some(base) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    read_ssh_aliases_from(&base.home_dir().join(".ssh/config"))
}

fn read_ssh_aliases_from(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| parse_ssh_aliases(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yare::parameterized;

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
                assert2::assert!(
                    std::process::Command::new("git")
                        .arg("-C")
                        .arg(&path)
                        .args(args)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            std::fs::write(path.join("README.md"), "fixture\n").unwrap();
            assert2::assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(["add", "README.md"])
                    .status()
                    .unwrap()
                    .success()
            );
            assert2::assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(["commit", "-m", "fixture"])
                    .status()
                    .unwrap()
                    .success()
            );
            Self { root, path }
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    fn git_output(repo: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert2::assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

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
            .arg("-C")
            .arg(&repo.path)
            .args(["branch", "already-there"])
            .status()
            .unwrap();
        let mut req = request(AgentKind::Claude);
        req.repository = repo.path.to_string_lossy().into_owned();
        req.branch = "already-there".into();
        req.git_mode = GitMode::NewWorktree {
            path: occupied.to_string_lossy().into_owned(),
        };
        let checked = check_local_workspace(&req);
        assert2::assert!(
            matches!(checked.branch, FieldCheck::Invalid(message) if message.contains("already exists"))
        );
        assert2::assert!(
            matches!(checked.worktree, Some(FieldCheck::Invalid(message)) if message.contains("already exists"))
        );

        req.repository = repo.root.join("missing").to_string_lossy().into_owned();
        let checked = check_local_workspace(&req);
        assert2::assert!(matches!(checked.repository, FieldCheck::Invalid(_)));
        assert2::assert!(
            checked.branch == FieldCheck::Invalid("select a valid Git repository first".into())
        );
    }

    #[test]
    fn local_workspace_check_rejects_empty_and_malformed_values() {
        let mut req = request(AgentKind::Claude);
        req.repository.clear();
        req.branch = "bad branch".into();
        req.git_mode = GitMode::NewWorktree {
            path: String::new(),
        };
        let checked = check_local_workspace(&req);
        assert2::assert!(
            checked.repository == FieldCheck::Invalid("repository is required".into())
        );
        assert2::assert!(checked.branch == FieldCheck::Invalid("branch name is invalid".into()));
        assert2::assert!(
            checked.worktree == Some(FieldCheck::Invalid("worktree path is required".into()))
        );
    }

    #[test]
    fn unchecked_workspace_cannot_pass_authoritative_validation() {
        let checked = WorkspaceCheck {
            repository: FieldCheck::Unchecked,
            branch: FieldCheck::Unchecked,
            worktree: Some(FieldCheck::Unchecked),
        };

        assert2::assert!(
            checked
                .require_valid()
                .unwrap_err()
                .to_string()
                .contains("validation is incomplete")
        );
    }

    #[test]
    fn remote_probe_is_one_bounded_noninteractive_ssh_command() {
        let mut req = request(AgentKind::Claude);
        req.target = LaunchTarget::Ssh {
            alias: "build-box".into(),
        };
        req.repository = "/srv/api".into();
        req.git_mode = GitMode::NewWorktree {
            path: "/srv/api-worktrees/feature/check".into(),
        };
        let command = remote_probe_command(&req).unwrap();
        assert2::assert!(command.program == "ssh");
        for option in [
            "BatchMode=yes",
            "ConnectTimeout=5",
            "NumberOfPasswordPrompts=0",
            "StrictHostKeyChecking=yes",
        ] {
            assert2::assert!(command.args.iter().any(|arg| arg == option));
        }
        let script = command.args.last().unwrap();
        assert2::assert!(script.contains("NODESTORM_REPOSITORY"));
        assert2::assert!(script.contains("NODESTORM_BRANCH"));
        assert2::assert!(script.contains("NODESTORM_WORKTREE"));
        assert2::assert!(!script.contains("mkdir"));
        assert2::assert!(!script.contains("worktree add"));
    }

    #[parameterized(
        empty_repository = { "", "/srv/api-worktrees/feature/check", "repository is required", None },
        relative_repository = { "srv/api", "/srv/api-worktrees/feature/check", "remote repository must be absolute", None },
        empty_worktree = { "/srv/api", "", "", Some("worktree path is required") },
        relative_worktree = { "/srv/api", "srv/api-worktrees/feature/check", "", Some("remote worktree path must be absolute") },
    )]
    fn remote_probe_reports_locally_invalid_paths_per_field_without_transport_error(
        repository: &str,
        worktree: &str,
        repository_error: &str,
        worktree_error: Option<&str>,
    ) {
        let mut req = request(AgentKind::Claude);
        req.target = LaunchTarget::Ssh {
            alias: "host-that-must-not-be-contacted".into(),
        };
        req.repository = repository.into();
        req.git_mode = GitMode::NewWorktree {
            path: worktree.into(),
        };

        let WorkspaceProbe::Checked(checked) = probe_workspace(&req) else {
            panic!("locally invalid input must not be classified as a transport failure");
        };
        if repository_error.is_empty() {
            assert2::assert!(checked.repository == FieldCheck::Unchecked);
            assert2::assert!(checked.branch == FieldCheck::Unchecked);
        } else {
            assert2::assert!(checked.repository == FieldCheck::Invalid(repository_error.into()));
            assert2::assert!(
                checked.branch == FieldCheck::Invalid("select a valid Git repository first".into())
            );
        }
        assert2::assert!(
            checked.worktree
                == Some(match worktree_error {
                    Some(message) => FieldCheck::Invalid(message.into()),
                    None => FieldCheck::Unchecked,
                })
        );
    }

    #[test]
    fn unavailable_local_git_defers_remote_branch_validation_to_ssh() {
        let mut req = request(AgentKind::Claude);
        req.target = LaunchTarget::Ssh {
            alias: "build-box".into(),
        };
        req.repository = "/srv/api".into();
        req.git_mode = GitMode::NewWorktree {
            path: "/srv/api-worktrees/feature/check".into(),
        };

        assert2::assert!(check_remote_input_with(&req, |_| None).is_none());
        let command = remote_probe_command(&req).unwrap();
        assert2::assert!(command.program == "ssh");
    }

    #[test]
    fn remote_probe_parser_preserves_independent_results() {
        let output = "banner\nNODESTORM_REPOSITORY\tvalid\t\nNODESTORM_BRANCH\tinvalid\tbranch already exists\nNODESTORM_WORKTREE\tvalid\t\n";
        assert2::assert!(
            (parse_remote_probe(output, true).unwrap())
                == (WorkspaceCheck {
                    repository: FieldCheck::Valid,
                    branch: FieldCheck::Invalid("branch already exists".into()),
                    worktree: Some(FieldCheck::Valid),
                })
        );
    }

    #[parameterized(
        missing = {
            "NODESTORM_REPOSITORY\tvalid\t\nNODESTORM_WORKTREE\tvalid\t\n",
            "omitted branch status"
        },
        duplicate = {
            "NODESTORM_REPOSITORY\tvalid\t\nNODESTORM_REPOSITORY\tvalid\t\nNODESTORM_BRANCH\tvalid\t\nNODESTORM_WORKTREE\tvalid\t\n",
            "duplicate remote repository status"
        },
        malformed = {
            "NODESTORM_REPOSITORY\tunknown\t\nNODESTORM_BRANCH\tvalid\t\nNODESTORM_WORKTREE\tvalid\t\n",
            "invalid field result"
        },
    )]
    fn remote_probe_parser_rejects_invalid_statuses(output: &str, expected: &str) {
        assert2::assert!(
            parse_remote_probe(output, true)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
        assert2::assert!(
            matches!(parsed_remote_probe(output, true), WorkspaceProbe::TransportError(message) if message.contains(expected))
        );
    }

    #[cfg(unix)]
    #[test]
    fn remote_probe_timeout_terminates_and_reaps_child() {
        let root =
            std::env::temp_dir().join(format!("nodestorm-probe-timeout-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let pid_file = root.join("pid");
        let command = CommandSpec {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "printf '%s' $$ > {}; exec sleep 10",
                    posix_quote(&pid_file.to_string_lossy())
                ),
            ],
            current_dir: None,
        };

        let result = run_remote_probe(&command, false, std::time::Duration::from_millis(250));

        assert2::assert!(
            matches!(result, WorkspaceProbe::TransportError(message) if message.contains("timed out"))
        );
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert2::assert!(
            !std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[parameterized(
        password = { "Permission denied (publickey,password).", true },
        keyboard = { "Permission denied (publickey,keyboard-interactive).", true },
        key_only = { "Permission denied (publickey).", false },
        password_in_host = { "alice@password-vault: Permission denied (publickey).", false },
        host_key = { "Host key verification failed.", false },
        timeout = { "ssh: connect to host build-box port 22: Connection timed out", false },
    )]
    fn only_interactive_auth_rejection_enables_fallback(stderr: &str, expected: bool) {
        assert2::assert!(password_only_rejection(stderr) == expected);
    }

    #[test]
    fn defaults_derive_branch_and_sibling_worktree() {
        assert2::assert!((suggest_branch("Cache Redesign")) == ("nodestorm/cache-redesign"));
        assert2::assert!(
            (suggest_worktree("/work/api", "nodestorm/cache-redesign", false).unwrap())
                == ("/work/api-worktrees/nodestorm/cache-redesign")
        );
        assert2::assert!(
            (suggest_worktree("/srv/api", "nodestorm/cache-redesign", true).unwrap())
                == ("/srv/api-worktrees/nodestorm/cache-redesign")
        );
    }

    #[parameterized(
        claude = { AgentKind::Claude, "claude", vec!["TASK"] },
        codex = { AgentKind::Codex, "codex", vec!["TASK"] },
        opencode = { AgentKind::OpenCode, "opencode", vec!["--prompt", "TASK"] },
        pi = { AgentKind::Pi, "pi", vec!["--name", "cache-redesign", "TASK"] },
    )]
    fn every_agent_gets_its_interactive_arguments(
        agent: AgentKind,
        expected_program: &str,
        expected_args: Vec<&str>,
    ) {
        let spec = agent_command(agent, "cache-redesign", "TASK", "/work/tree");
        assert2::assert!(
            spec == CommandSpec {
                program: expected_program.into(),
                args: expected_args.into_iter().map(str::to_owned).collect(),
                current_dir: Some("/work/tree".into()),
            }
        );
    }

    #[test]
    fn prompt_preserves_task_and_addresses_the_nodestorm_session() {
        let prompt = compose_prompt(&request(AgentKind::Codex), "cache-redesign");
        assert2::assert!(prompt.starts_with("Design the cache; don't run $(touch /tmp/nope)."));
        assert2::assert!(prompt.contains("Nodestorm session `cache-redesign`"));
        assert2::assert!(prompt.contains("agent identity `codex-cache-redesign`"));
        assert2::assert!(prompt.contains("installed Nodestorm skill"));
    }

    #[test]
    fn ssh_aliases_include_literals_only() {
        let config = "Host prod bastion\n  HostName prod.test\nHost *.corp !blocked\nHost dev\n";
        assert2::assert!((parse_ssh_aliases(config)) == (vec!["bastion", "dev", "prod"]));
    }

    #[parameterized(
        plain = { "plain", "'plain'" },
        embedded_quote = { "a'b", "'a'\\''b'" },
        shell_syntax_and_newline = {
            "$(touch /tmp/nope)\nnext",
            "'$(touch /tmp/nope)\nnext'"
        },
    )]
    fn posix_quote_neutralizes_shell_syntax(input: &str, expected: &str) {
        assert2::assert!(posix_quote(input) == expected);
    }

    #[test]
    fn diagnostic_shell_prints_the_quoted_error_then_opens_a_login_shell() {
        assert2::assert!(
            (diagnostic_shell("can't launch"))
                == ("{ printf '%s\\n' 'can'\\''t launch'; exec \"${SHELL:-/bin/sh}\" -l; }")
        );
    }

    #[parameterized(
        root_child = { "/repo", "/" },
        nested = { "/srv/repos/api", "/srv/repos" },
    )]
    fn remote_parent_handles_absolute_paths(input: &str, expected: &str) {
        assert2::assert!(remote_parent(input).unwrap() == expected);
    }

    #[parameterized(relative = { "relative" }, trailing_slash = { "/srv/repos/" })]
    fn remote_parent_rejects_invalid_paths(input: &str) {
        assert2::assert!(remote_parent(input).is_err());
    }

    #[test]
    fn spawning_a_missing_program_reports_not_found() {
        let spec = CommandSpec {
            program: format!("nodestorm-missing-{}", uuid::Uuid::new_v4()),
            args: Vec::new(),
            current_dir: None,
        };
        assert2::assert!(
            (spawn_command(&spec).unwrap_err().kind()) == (std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn linux_terminal_fallback_skips_only_missing_programs() {
        let child = CommandSpec {
            program: "agent".into(),
            args: Vec::new(),
            current_dir: None,
        };
        let mut attempts = 0;
        open_linux_terminal_with(&child, |_| {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::ErrorKind::NotFound.into())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert2::assert!((attempts) == (3));

        let mut attempts = 0;
        let error = open_linux_terminal_with(&child, |_| {
            attempts += 1;
            Err(std::io::ErrorKind::PermissionDenied.into())
        })
        .unwrap_err();
        assert2::assert!((attempts) == (1));
        assert2::assert!(
            (error.downcast_ref::<std::io::Error>().unwrap().kind())
                == (std::io::ErrorKind::PermissionDenied)
        );

        let error = open_linux_terminal_with(&child, |_| Err(std::io::ErrorKind::NotFound.into()))
            .unwrap_err();
        assert2::assert!(error.to_string().contains("no supported terminal found"));
    }

    #[test]
    fn ssh_alias_file_is_parsed_and_missing_files_are_empty() {
        let root =
            std::env::temp_dir().join(format!("nodestorm-ssh-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("config");
        std::fs::write(&config, "Host prod *.internal dev\n").unwrap();
        assert2::assert!((read_ssh_aliases_from(&config)) == (vec!["dev", "prod"]));
        assert2::assert!(read_ssh_aliases_from(&root.join("missing")).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validation_names_the_missing_field() {
        let mut req = request(AgentKind::Claude);
        req.task.clear();
        assert2::assert!(
            (validate_request(&req).unwrap_err().to_string()) == ("initial task is required")
        );
        req.task = "task".into();
        req.target = LaunchTarget::Ssh { alias: "".into() };
        assert2::assert!(
            (validate_request(&req).unwrap_err().to_string()) == ("SSH host alias is required")
        );
    }

    #[test]
    fn worktree_mode_creates_branch_without_switching_or_carrying_dirty_files() {
        let repo = TempRepo::new("worktree");
        std::fs::write(repo.path.join("dirty.txt"), "not committed").unwrap();
        let original = git_output(&repo.path, &["branch", "--show-current"]);
        let worktree = repo.root.join("api-worktrees/nodestorm/cache-redesign");
        let mut req = request(AgentKind::Claude);
        req.repository = repo.path.to_string_lossy().into_owned();
        req.git_mode = GitMode::NewWorktree {
            path: worktree.to_string_lossy().into_owned(),
        };

        let prepared = prepare_local(&req, false).unwrap();

        assert2::assert!((prepared.directory) == (worktree.to_string_lossy().into_owned()));
        assert2::assert!((git_output(&repo.path, &["branch", "--show-current"])) == (original));
        assert2::assert!(!worktree.join("dirty.txt").exists());
        assert2::assert!(
            (git_output(&worktree, &["branch", "--show-current"])) == ("nodestorm/cache-redesign")
        );
    }

    #[test]
    fn existing_checkout_requires_dirty_confirmation_then_switches() {
        let repo = TempRepo::new("existing");
        std::fs::write(repo.path.join("dirty.txt"), "keep me").unwrap();
        let mut req = request(AgentKind::Claude);
        req.repository = repo.path.to_string_lossy().into_owned();
        req.git_mode = GitMode::ExistingCheckout;

        assert2::assert!(inspect_local(&req).unwrap().dirty);
        assert2::assert!(
            prepare_local(&req, false)
                .unwrap_err()
                .to_string()
                .contains("uncommitted changes")
        );
        let prepared = prepare_local(&req, true).unwrap();
        assert2::assert!((prepared.directory) == (repo.path.to_string_lossy().into_owned()));
        assert2::assert!(
            (git_output(&repo.path, &["branch", "--show-current"])) == ("nodestorm/cache-redesign")
        );
        assert2::assert!(repo.path.join("dirty.txt").exists());
    }

    #[test]
    fn collisions_and_invalid_branches_do_not_mutate_repository() {
        let repo = TempRepo::new("invalid");
        let original = git_output(&repo.path, &["branch", "--show-current"]);
        let mut req = request(AgentKind::Claude);
        req.repository = repo.path.to_string_lossy().into_owned();
        req.branch = "bad branch".into();
        assert2::assert!(prepare_local(&req, false).is_err());
        assert2::assert!((git_output(&repo.path, &["branch", "--show-current"])) == (original));

        assert2::assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo.path)
                .args(["branch", "already-there"])
                .status()
                .unwrap()
                .success()
        );
        req.branch = "already-there".into();
        assert2::assert!(
            prepare_local(&req, false)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );

        let occupied = repo.root.join("occupied");
        std::fs::create_dir_all(&occupied).unwrap();
        req.branch = "free-branch".into();
        req.git_mode = GitMode::NewWorktree {
            path: occupied.to_string_lossy().into_owned(),
        };
        assert2::assert!(
            prepare_local(&req, false)
                .unwrap_err()
                .to_string()
                .contains("destination")
        );
        assert2::assert!((git_output(&repo.path, &["branch", "--show-current"])) == (original));
    }

    #[test]
    fn executable_check_reports_missing_program() {
        assert2::assert!(ensure_executable("git").is_ok());
        assert2::assert!(ensure_executable("ssh").is_ok());
        assert2::assert!(
            ensure_executable("nodestorm-definitely-not-installed")
                .unwrap_err()
                .to_string()
                .contains("unavailable")
        );
    }

    #[test]
    fn ssh_command_has_tty_tunnel_validation_and_quoted_values() {
        let mut req = request(AgentKind::Claude);
        req.target = LaunchTarget::Ssh {
            alias: "build-box".into(),
        };
        req.repository = "/srv/repo with spaces".into();
        req.git_mode = GitMode::NewWorktree {
            path: "/srv/repo-worktrees/nodestorm/cache-redesign".into(),
        };
        let spec = remote_agent_command(&req, "cache-redesign").unwrap();
        assert2::assert!((spec.program) == ("ssh"));
        assert2::assert!(
            (&spec.args[..6])
                == ([
                    "-t",
                    "-o",
                    "ExitOnForwardFailure=yes",
                    "-R",
                    "4747:127.0.0.1:8123",
                    "--"
                ])
        );
        assert2::assert!((spec.args[6]) == ("build-box"));
        let script = remote_script(&req, "cache-redesign").unwrap();
        assert2::assert!(
            (spec.args.last().unwrap()) == (&format!("exec sh -lc {}", posix_quote(&script)))
        );
        assert2::assert!(script.contains("command -v 'claude'"));
        assert2::assert!(script.contains("'/srv/repo with spaces'"));
        assert2::assert!(script.contains("worktree add -b"));
        assert2::assert!(script.contains("exec 'claude'"));
        assert2::assert!(script.contains(&posix_quote(&compose_prompt(&req, "cache-redesign"))));
    }

    #[test]
    fn ssh_existing_checkout_prompts_before_carrying_dirty_changes() {
        let mut req = request(AgentKind::Codex);
        req.target = LaunchTarget::Ssh {
            alias: "build-box".into(),
        };
        req.repository = "/srv/api".into();
        req.git_mode = GitMode::ExistingCheckout;
        let script = remote_script(&req, "cache-redesign").unwrap();
        assert2::assert!(script.contains("uncommitted changes"));
        assert2::assert!(script.contains("read answer"));
        assert2::assert!(script.contains("switch -c"));
        assert2::assert!(!script.contains("worktree add"));
    }

    #[test]
    fn terminal_wrappers_preserve_program_and_arguments() {
        let child = CommandSpec {
            program: "codex".into(),
            args: vec!["task with spaces".into()],
            current_dir: Some("/work/tree".into()),
        };
        let linux = terminal_command(TerminalFlavor::LinuxXTerminal, &child);
        assert2::assert!((linux.program) == ("x-terminal-emulator"));
        assert2::assert!((&linux.args[..2]) == (["-e", "codex"]));
        assert2::assert!((linux.current_dir.as_deref()) == (Some("/work/tree")));

        let mac = terminal_command(TerminalFlavor::MacTerminal, &child);
        assert2::assert!((mac.program) == ("osascript"));
        assert2::assert!(
            mac.args
                .last()
                .unwrap()
                .contains("cd -- '/work/tree' && exec 'codex' 'task with spaces'")
        );
    }

    #[test]
    fn remote_paths_must_be_absolute() {
        let mut req = request(AgentKind::Pi);
        req.target = LaunchTarget::Ssh {
            alias: "build-box".into(),
        };
        req.repository = "relative/repo".into();
        assert2::assert!(
            validate_request(&req)
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );
    }
}
