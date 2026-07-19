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
        assert!(
            ensure_executable("nodestorm-definitely-not-installed")
                .unwrap_err()
                .to_string()
                .contains("unavailable")
        );
    }
}
