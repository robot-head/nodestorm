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
    Ok(parent
        .join(format!("{name}-worktrees"))
        .join(branch)
        .to_string_lossy()
        .into_owned())
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

pub fn inspect_local(req: &LaunchRequest) -> anyhow::Result<LocalInspection> {
    validate_request(req)?;
    anyhow::ensure!(
        matches!(req.target, LaunchTarget::Local),
        "local inspection requires a local target"
    );
    git(&req.repository, &["rev-parse", "--show-toplevel"])?;
    git(
        &req.repository,
        &["check-ref-format", "--branch", &req.branch],
    )?;

    let reference = format!("refs/heads/{}", req.branch);
    let branch_status = std::process::Command::new("git")
        .args([
            "-C",
            &req.repository,
            "show-ref",
            "--verify",
            "--quiet",
            &reference,
        ])
        .status()?;
    match branch_status.code() {
        Some(0) => anyhow::bail!("branch `{}` already exists", req.branch),
        Some(1) => {}
        _ => anyhow::bail!("git could not inspect branch `{}`", req.branch),
    }

    if let GitMode::NewWorktree { path } = &req.git_mode {
        anyhow::ensure!(
            !Path::new(path).exists(),
            "worktree destination `{path}` already exists"
        );
    }

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
    WindowsTerminal,
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
        TerminalFlavor::WindowsTerminal => {
            let mut args = vec!["-w".into(), "new".into(), "new-tab".into()];
            if let Some(dir) = &child.current_dir {
                args.extend(["-d".into(), dir.clone()]);
            }
            args.push(child.program.clone());
            args.extend(child.args.clone());
            CommandSpec {
                program: "wt.exe".into(),
                args,
                current_dir: None,
            }
        }
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

fn spawn_command(spec: &CommandSpec) -> std::io::Result<()> {
    let mut command = std::process::Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(dir) = &spec.current_dir {
        command.current_dir(dir);
    }
    command.spawn().map(|_| ())
}

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

// Each `#[cfg]` arm needs its own `return` because sibling arms follow it in
// source; only one compiles per target, so clippy sees the last as redundant.
#[allow(clippy::needless_return)]
pub fn open_terminal(child: &CommandSpec) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        return spawn_command(&terminal_command(TerminalFlavor::WindowsTerminal, child))
            .map_err(|err| anyhow::anyhow!("could not open Windows Terminal: {err}"));
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
                assert!(
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
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(["add", "README.md"])
                    .status()
                    .unwrap()
                    .success()
            );
            assert!(
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
        assert!(
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
            (
                AgentKind::Pi,
                "pi",
                vec!["--name", "cache-redesign", "TASK"],
            ),
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
        assert_eq!(
            posix_quote("$(touch /tmp/nope)\nnext"),
            "'$(touch /tmp/nope)\nnext'"
        );
    }

    #[test]
    fn diagnostic_shell_prints_the_quoted_error_then_opens_a_login_shell() {
        assert_eq!(
            diagnostic_shell("can't launch"),
            "{ printf '%s\\n' 'can'\\''t launch'; exec \"${SHELL:-/bin/sh}\" -l; }"
        );
    }

    #[test]
    fn remote_parent_handles_root_and_nested_paths() {
        assert_eq!(remote_parent("/repo").unwrap(), "/");
        assert_eq!(remote_parent("/srv/repos/api").unwrap(), "/srv/repos");
        assert!(remote_parent("relative").is_err());
        assert!(remote_parent("/srv/repos/").is_err());
    }

    #[test]
    fn spawning_a_missing_program_reports_not_found() {
        let spec = CommandSpec {
            program: format!("nodestorm-missing-{}", uuid::Uuid::new_v4()),
            args: Vec::new(),
            current_dir: None,
        };
        assert_eq!(
            spawn_command(&spec).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
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
        assert_eq!(attempts, 3);

        let mut attempts = 0;
        let error = open_linux_terminal_with(&child, |_| {
            attempts += 1;
            Err(std::io::ErrorKind::PermissionDenied.into())
        })
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );

        let error = open_linux_terminal_with(&child, |_| Err(std::io::ErrorKind::NotFound.into()))
            .unwrap_err();
        assert!(error.to_string().contains("no supported terminal found"));
    }

    #[test]
    fn ssh_alias_file_is_parsed_and_missing_files_are_empty() {
        let root =
            std::env::temp_dir().join(format!("nodestorm-ssh-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("config");
        std::fs::write(&config, "Host prod *.internal dev\n").unwrap();
        assert_eq!(read_ssh_aliases_from(&config), vec!["dev", "prod"]);
        assert!(read_ssh_aliases_from(&root.join("missing")).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validation_names_the_missing_field() {
        let mut req = request(AgentKind::Claude);
        req.task.clear();
        assert_eq!(
            validate_request(&req).unwrap_err().to_string(),
            "initial task is required"
        );
        req.task = "task".into();
        req.target = LaunchTarget::Ssh { alias: "".into() };
        assert_eq!(
            validate_request(&req).unwrap_err().to_string(),
            "SSH host alias is required"
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

        assert_eq!(prepared.directory, worktree.to_string_lossy().into_owned());
        assert_eq!(
            git_output(&repo.path, &["branch", "--show-current"]),
            original
        );
        assert!(!worktree.join("dirty.txt").exists());
        assert_eq!(
            git_output(&worktree, &["branch", "--show-current"]),
            "nodestorm/cache-redesign"
        );
    }

    #[test]
    fn existing_checkout_requires_dirty_confirmation_then_switches() {
        let repo = TempRepo::new("existing");
        std::fs::write(repo.path.join("dirty.txt"), "keep me").unwrap();
        let mut req = request(AgentKind::Claude);
        req.repository = repo.path.to_string_lossy().into_owned();
        req.git_mode = GitMode::ExistingCheckout;

        assert!(inspect_local(&req).unwrap().dirty);
        assert!(
            prepare_local(&req, false)
                .unwrap_err()
                .to_string()
                .contains("uncommitted changes")
        );
        let prepared = prepare_local(&req, true).unwrap();
        assert_eq!(prepared.directory, repo.path.to_string_lossy().into_owned());
        assert_eq!(
            git_output(&repo.path, &["branch", "--show-current"]),
            "nodestorm/cache-redesign"
        );
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
        assert_eq!(
            git_output(&repo.path, &["branch", "--show-current"]),
            original
        );

        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo.path)
                .args(["branch", "already-there"])
                .status()
                .unwrap()
                .success()
        );
        req.branch = "already-there".into();
        assert!(
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
        assert!(
            prepare_local(&req, false)
                .unwrap_err()
                .to_string()
                .contains("destination")
        );
        assert_eq!(
            git_output(&repo.path, &["branch", "--show-current"]),
            original
        );
    }

    #[test]
    fn executable_check_reports_missing_program() {
        assert!(ensure_executable("git").is_ok());
        assert!(ensure_executable("ssh").is_ok());
        assert!(
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
        assert_eq!(spec.program, "ssh");
        assert_eq!(
            &spec.args[..6],
            [
                "-t",
                "-o",
                "ExitOnForwardFailure=yes",
                "-R",
                "4747:127.0.0.1:8123",
                "--"
            ]
        );
        assert_eq!(spec.args[6], "build-box");
        let script = remote_script(&req, "cache-redesign").unwrap();
        assert_eq!(
            spec.args.last().unwrap(),
            &format!("exec sh -lc {}", posix_quote(&script))
        );
        assert!(script.contains("command -v 'claude'"));
        assert!(script.contains("'/srv/repo with spaces'"));
        assert!(script.contains("worktree add -b"));
        assert!(script.contains("exec 'claude'"));
        assert!(script.contains(&posix_quote(&compose_prompt(&req, "cache-redesign"))));
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
        assert!(script.contains("uncommitted changes"));
        assert!(script.contains("read answer"));
        assert!(script.contains("switch -c"));
        assert!(!script.contains("worktree add"));
    }

    #[test]
    fn terminal_wrappers_preserve_program_and_arguments() {
        let child = CommandSpec {
            program: "codex".into(),
            args: vec!["task with spaces".into()],
            current_dir: Some("/work/tree".into()),
        };
        let windows = terminal_command(TerminalFlavor::WindowsTerminal, &child);
        assert_eq!(windows.program, "wt.exe");
        assert_eq!(
            &windows.args[..5],
            ["-w", "new", "new-tab", "-d", "/work/tree"]
        );
        assert!(
            windows
                .args
                .ends_with(&["codex".into(), "task with spaces".into()])
        );

        let linux = terminal_command(TerminalFlavor::LinuxXTerminal, &child);
        assert_eq!(linux.program, "x-terminal-emulator");
        assert_eq!(&linux.args[..2], ["-e", "codex"]);
        assert_eq!(linux.current_dir.as_deref(), Some("/work/tree"));

        let mac = terminal_command(TerminalFlavor::MacTerminal, &child);
        assert_eq!(mac.program, "osascript");
        assert!(
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
        assert!(
            validate_request(&req)
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );
    }
}
