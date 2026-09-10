use crate::{model::Host, runner};
use serde::Serialize;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const STATUS_LIMIT: usize = 1024 * 1024;
const DIFF_LIMIT: usize = 512 * 1024;
const FILE_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
    pub untracked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub repository: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    pub files: Vec<GitFileStatus>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiff {
    pub repository: bool,
    pub path: Option<String>,
    pub scope: Option<String>,
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffScope {
    Staged,
    Unstaged,
    Untracked,
}

impl DiffScope {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "staged" => Ok(Self::Staged),
            "unstaged" => Ok(Self::Unstaged),
            "untracked" => Ok(Self::Untracked),
            _ => Err("Git diff scope must be staged, unstaged, or untracked.".into()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
        }
    }
}

struct Output {
    status: Option<std::process::ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

pub fn status(host: &Host, cwd: &str) -> Result<GitStatus, String> {
    let Some(root) = repository_root(host, cwd)? else {
        return Ok(GitStatus {
            repository: false,
            root: None,
            branch: None,
            files: vec![],
            truncated: false,
        });
    };
    let output = run_git(
        host,
        cwd,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--renames",
            "--untracked-files=all",
        ],
        STATUS_LIMIT,
    )?;
    if output.status.is_none() {
        return Err("Git status timed out.".into());
    }
    if !output.status.is_some_and(|status| status.success()) {
        return Err(command_error("Git status failed", &output.stderr));
    }
    let (branch, mut files) = parse_status(&output.stdout);
    let truncated = output.truncated || files.len() > FILE_LIMIT;
    files.truncate(FILE_LIMIT);
    Ok(GitStatus {
        repository: true,
        root: Some(root),
        branch,
        files,
        truncated,
    })
}

pub fn diff(host: &Host, cwd: &str, path: &str, scope: &str) -> Result<GitDiff, String> {
    let scope = DiffScope::parse(scope)?;
    let Some(root) = repository_root(host, cwd)? else {
        return Ok(GitDiff {
            repository: false,
            path: None,
            scope: None,
            text: String::new(),
            truncated: false,
            binary: false,
        });
    };
    validate_path(path)?;
    let current = status(host, cwd)?;
    let file = current
        .files
        .iter()
        .find(|file| file.path == path || file.original_path.as_deref() == Some(path))
        .ok_or("Git path is not a current task-folder change.")?;
    let allowed = match scope {
        DiffScope::Staged => file.index_status != " " && !file.untracked,
        DiffScope::Unstaged => file.worktree_status != " " && !file.untracked,
        DiffScope::Untracked => file.untracked,
    };
    if !allowed {
        return Err("Git path has no change for that diff scope.".into());
    }
    let mut args: Vec<&str> = match scope {
        DiffScope::Staged => vec![
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-textconv",
            "--color=never",
            "--",
        ],
        DiffScope::Unstaged => vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--color=never",
            "--",
        ],
        DiffScope::Untracked => vec![
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--",
            "/dev/null",
            path,
        ],
    };
    if scope != DiffScope::Untracked {
        args.push(path);
        if let Some(original) = file.original_path.as_deref() {
            args.push(original);
        }
    }
    let output = run_git(host, &root, &args, DIFF_LIMIT)?;
    if output.status.is_none() {
        return Err("Git diff timed out.".into());
    }
    // git diff --no-index uses exit 1 when it finds a difference.
    if !output.status.is_some_and(|status| {
        status.success() || scope == DiffScope::Untracked && status.code() == Some(1)
    }) {
        return Err(command_error("Git diff failed", &output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let binary = text.contains("Binary files ") || text.contains("GIT binary patch");
    Ok(GitDiff {
        repository: true,
        path: Some(path.into()),
        scope: Some(scope.as_str().into()),
        text,
        truncated: output.truncated,
        binary,
    })
}

fn repository_root(host: &Host, cwd: &str) -> Result<Option<String>, String> {
    let output = run_git(host, cwd, &["rev-parse", "--show-toplevel"], 32 * 1024)?;
    if output.status.is_none() {
        return Err("Git repository check timed out.".into());
    }
    if !output.status.is_some_and(|status| status.success()) {
        if is_not_repository(&output.stderr) {
            return Ok(None);
        }
        return Err(command_error("Git repository check failed", &output.stderr));
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        Ok(None)
    } else {
        Ok(Some(root))
    }
}

fn run_git(host: &Host, cwd: &str, args: &[&str], limit: usize) -> Result<Output, String> {
    if cwd.trim().is_empty() {
        return Err("Task folder is empty.".into());
    }
    let git_args = std::iter::once("--literal-pathspecs")
        .chain(args.iter().copied())
        .collect::<Vec<_>>();
    let mut command = if host.kind == "local" {
        let mut command = Command::new("git");
        command.arg("-C").arg(cwd).args(&git_args);
        command
    } else if host.kind == "ssh" {
        let mut command = Command::new("ssh");
        runner::add_ssh_options(&mut command, host);
        let remote = remote_git_command(cwd, &git_args);
        command.arg(runner::ssh_target(host)?).arg(remote);
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start Git for this task.".to_string())?;
    let stdout = child.stdout.take().ok_or("Could not read Git output.")?;
    let stderr = child.stderr.take().ok_or("Could not read Git errors.")?;
    let stdout_receiver = read_limited(stdout, limit);
    // Git's diagnostic stream only needs enough room for a useful user-facing error.
    let stderr_receiver = read_limited(stderr, 16 * 1024);
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| "Could not read Git status.")? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let (stdout, stdout_truncated) = stdout_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "Git output did not finish.")?
        .map_err(|_| "Could not read Git output.")?;
    let (stderr, stderr_truncated) = stderr_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "Git error output did not finish.")?
        .map_err(|_| "Could not read Git errors.")?;
    Ok(Output {
        status,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
    })
}

fn read_limited<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> std::sync::mpsc::Receiver<std::io::Result<(Vec<u8>, bool)>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::with_capacity(limit.min(16 * 1024));
        let mut truncated = false;
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let remaining = limit.saturating_sub(output.len());
                    let kept = read.min(remaining);
                    output.extend_from_slice(&buffer[..kept]);
                    truncated |= kept < read;
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            }
        }
        let _ = sender.send(Ok((output, truncated)));
    });
    receiver
}

fn is_not_repository(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .to_ascii_lowercase()
        .contains("not a git repository")
}

fn command_error(prefix: &str, stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr)
        .trim()
        .replace(['\n', '\r'], " ");
    if detail.is_empty() {
        format!("{prefix}.")
    } else {
        format!("{prefix}: {detail}")
    }
}

fn remote_git_command(cwd: &str, args: &[&str]) -> String {
    std::iter::once("git".to_string())
        .chain(std::iter::once("-C".to_string()))
        .chain(std::iter::once(runner::remote_path(cwd)))
        .chain(args.iter().map(|value| runner::posix_quote(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with("~")
        || path.split('/').any(|part| part == "..")
    {
        return Err("Git path must be a repository-relative changed path.".into());
    }
    Ok(())
}

fn parse_status(raw: &[u8]) -> (Option<String>, Vec<GitFileStatus>) {
    let mut fields = raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut branch = None;
    let mut files = vec![];
    while let Some(field) = fields.next() {
        if field.starts_with(b"## ") {
            let value = String::from_utf8_lossy(&field[3..]);
            if value != "HEAD (no branch)" && !value.starts_with("HEAD detached") {
                branch = Some(value.split("...").next().unwrap_or(&value).into());
            }
            continue;
        }
        if field.len() < 3 {
            continue;
        }
        let index = field[0] as char;
        let worktree = field[1] as char;
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let renamed = matches!(index, 'R' | 'C');
        let original_path = renamed
            .then(|| {
                fields
                    .next()
                    .map(|value| String::from_utf8_lossy(value).into_owned())
            })
            .flatten();
        files.push(GitFileStatus {
            path,
            original_path,
            index_status: index.into(),
            worktree_status: worktree.into(),
            untracked: index == '?' && worktree == '?',
        });
    }
    (branch, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    fn host() -> Host {
        Host {
            id: "h".into(),
            name: "local".into(),
            kind: "local".into(),
            address: "".into(),
            user: "".into(),
            port: 0,
            identity_file: "".into(),
            default_cwd: "".into(),
            codex_path: "".into(),
            claude_path: "".into(),
            opencode_path: "".into(),
            hermes_path: "".into(),
        }
    }
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    fn repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("monitter-git-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
        git(&dir, &["config", "user.name", "Test"]);
        fs::write(dir.join("base.txt"), "base\n").unwrap();
        fs::write(dir.join("rename-source.txt"), "rename\n").unwrap();
        git(&dir, &["add", "base.txt", "rename-source.txt"]);
        git(&dir, &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn status_and_diff_cover_staged_unstaged_untracked_rename_binary_and_outside() {
        let root = repo();
        let root = root.as_path();
        fs::rename(root.join("rename-source.txt"), root.join("renamed.txt")).unwrap();
        git(root, &["add", "-A"]);
        fs::write(root.join("base.txt"), "changed\n").unwrap();
        fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
        fs::write(root.join("binary.bin"), [0, 1, 2, 0]).unwrap();
        let git_status = status(&host(), root.to_str().unwrap()).unwrap();
        assert!(
            git_status.repository
                && git_status
                    .files
                    .iter()
                    .any(|file| file.path == "renamed.txt"
                        && file.original_path.as_deref() == Some("rename-source.txt")),
            "{git_status:?}"
        );
        assert!(git_status
            .files
            .iter()
            .any(|file| file.path == "base.txt" && file.worktree_status == "M"));
        assert!(git_status
            .files
            .iter()
            .any(|file| file.path == "untracked.txt" && file.untracked));
        assert!(
            diff(&host(), root.to_str().unwrap(), "base.txt", "unstaged")
                .unwrap()
                .text
                .contains("changed")
        );
        assert!(diff(
            &host(),
            root.to_str().unwrap(),
            "untracked.txt",
            "untracked"
        )
        .unwrap()
        .text
        .contains("untracked"));
        assert!(
            diff(&host(), root.to_str().unwrap(), "binary.bin", "untracked")
                .unwrap()
                .binary
        );
        let outside =
            std::env::temp_dir().join(format!("monitter-git-outside-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&outside).unwrap();
        assert!(
            !status(&host(), outside.to_str().unwrap())
                .unwrap()
                .repository
        );
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn porcelain_z_parser_keeps_rename_paths_and_detached_branch_is_null() {
        let (branch, files) =
            parse_status(b"## HEAD (no branch)\0R  new name\0old name\0?? extra\0");
        assert!(branch.is_none());
        assert_eq!(files[0].path, "new name");
        assert_eq!(files[0].original_path.as_deref(), Some("old name"));
        assert!(files[1].untracked);
    }

    #[test]
    fn remote_command_quotes_task_folder_and_path() {
        let command = remote_git_command("~/repo with spaces", &["diff", "--", "odd '; $path.txt"]);
        assert!(command.contains("\"$HOME\"/'repo with spaces'"));
        assert!(command.contains("$path.txt'"));
        assert_eq!(command.split_whitespace().next(), Some("git"));
    }

    #[test]
    fn nested_cwd_uses_root_relative_literal_paths_and_staged_rename() {
        let root = repo();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/[odd]*.txt"), "one\n").unwrap();
        git(&root, &["add", "nested/[odd]*.txt"]);
        git(&root, &["commit", "-qm", "nested"]);
        fs::write(root.join("nested/[odd]*.txt"), "two\n").unwrap();
        fs::rename(root.join("rename-source.txt"), root.join("renamed.txt")).unwrap();
        git(&root, &["add", "-A"]);
        fs::write(root.join("nested/[odd]*.txt"), "three\n").unwrap();
        fs::write(root.join("nested/untracked.txt"), "new\n").unwrap();
        let nested = root.join("nested");
        let current = status(&host(), nested.to_str().unwrap()).unwrap();
        assert!(current.files.iter().any(|f| f.path == "nested/[odd]*.txt"));
        assert!(diff(
            &host(),
            nested.to_str().unwrap(),
            "nested/[odd]*.txt",
            "unstaged"
        )
        .unwrap()
        .text
        .contains("three"));
        assert!(diff(
            &host(),
            nested.to_str().unwrap(),
            "nested/untracked.txt",
            "untracked"
        )
        .unwrap()
        .text
        .contains("new"));
        let renamed = diff(&host(), nested.to_str().unwrap(), "renamed.txt", "staged")
            .unwrap()
            .text;
        assert!(renamed.contains("rename-source.txt") && renamed.contains("renamed.txt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn large_diff_is_drained_and_marked_truncated() {
        let root = repo();
        fs::write(root.join("large.txt"), "x\n".repeat(DIFF_LIMIT)).unwrap();
        let result = diff(&host(), root.to_str().unwrap(), "large.txt", "untracked").unwrap();
        assert!(result.truncated);
        assert!(result.text.len() <= DIFF_LIMIT);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ssh_transport_failure_is_not_misreported_as_non_repository() {
        let mut remote = host();
        remote.kind = "ssh".into();
        remote.address = "127.0.0.1".into();
        remote.port = 1;
        let error = repository_root(&remote, "/tmp").unwrap_err();
        assert!(error.contains("Git repository check failed"), "{error}");
        assert!(!is_not_repository(error.as_bytes()));
    }
}
