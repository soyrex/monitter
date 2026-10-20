//! Local, task-scoped linked worktrees. Never stash, reset, push or delete user work.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorktree {
    pub root: String,
    pub path: String,
    pub branch: String,
    pub base_commit: String,
    pub target_branch: String,
}

fn git_at(path: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Could not run Git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Git {} failed: {}",
            args.first().unwrap_or(&"command"),
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
                .trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
fn clean(path: &Path) -> Result<(), String> {
    if !git_at(path, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty() {
        return Err(format!(
            "Commit or move uncommitted files in {} before continuing. Nothing was stashed or reset.",
            path.display()
        ));
    }
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        let marker_path = git_at(path, &["rev-parse", "--git-path", marker])?;
        if path.join(marker_path).exists() {
            return Err("Finish the existing Git operation first.".into());
        }
    }
    Ok(())
}

fn retirement_clean(path: &Path) -> Result<(), String> {
    if !git_at(
        path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?
    .is_empty()
    {
        return Err(format!(
            "Archive cleanup stopped because {} contains uncommitted, untracked, or ignored files. Commit or move them first; nothing was deleted.",
            path.display()
        ));
    }
    clean(path)
}

fn expected_path(task_id: &str, w: &AgentWorktree) -> Result<(PathBuf, PathBuf), String> {
    uuid::Uuid::parse_str(task_id).map_err(|_| "Worktree task ID is invalid.".to_string())?;
    let root = Path::new(&w.root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let name = root
        .file_name()
        .ok_or("Invalid project worktree root.")?
        .to_string_lossy();
    let expected = root
        .parent()
        .ok_or("Invalid project parent.")?
        .join(format!("{name}-monitter-worktrees"))
        .join(task_id);
    if w.branch != format!("monitter/pane-{task_id}") || Path::new(&w.path) != expected.as_path() {
        return Err(
            "This pane's worktree metadata changed; inspect it before changing its archive state."
                .into(),
        );
    }
    Ok((root, expected))
}

fn is_registered(root: &Path, path: &Path) -> Result<bool, String> {
    Ok(git_at(root, &["worktree", "list", "--porcelain"])?
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .any(|listed| Path::new(listed) == path))
}

fn validate_active(root: &Path, path: &Path, w: &AgentWorktree) -> Result<(), String> {
    if !same_repository(
        path.to_str().ok_or("Worktree path is not UTF-8.")?,
        root.to_str().ok_or("Project path is not UTF-8.")?,
    ) {
        return Err("The pane no longer belongs to its recorded project repository.".into());
    }
    if git_at(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])? != w.branch {
        return Err(
            "The pane worktree branch changed; inspect it manually before archiving.".into(),
        );
    }
    Ok(())
}

pub fn create(cwd: &str, task_id: &str) -> Result<AgentWorktree, String> {
    // IDs form paths and branch names; never accept arbitrary path components.
    uuid::Uuid::parse_str(task_id).map_err(|_| "Worktree task ID is invalid.".to_string())?;
    let cwd = Path::new(cwd);
    let root = PathBuf::from(git_at(cwd, &["rev-parse", "--show-toplevel"])?);
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    clean(&root)?;
    let target_branch =
        git_at(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).map_err(|_| {
            "Choose a branch in the project before creating agent worktrees.".to_string()
        })?;
    let base_commit = git_at(&root, &["rev-parse", "HEAD"])?;
    let name = root
        .file_name()
        .ok_or("Repository has no folder name.")?
        .to_string_lossy();
    let parent = root
        .parent()
        .ok_or("Repository has no parent folder.")?
        .join(format!("{name}-monitter-worktrees"));
    std::fs::create_dir_all(&parent)
        .map_err(|e| format!("Could not create worktree folder: {e}"))?;
    let path = parent.join(task_id);
    let branch = format!("monitter/pane-{task_id}");
    // Git refuses an existing directory/branch, preserving recoverable orphaned worktrees.
    git_at(
        &root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_str().ok_or("Worktree path is not UTF-8.")?,
            &base_commit,
        ],
    )?;
    Ok(AgentWorktree {
        root: root.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        branch,
        base_commit,
        target_branch,
    })
}

pub fn validate(task_id: &str, cwd: &str, w: &AgentWorktree) -> Result<(), String> {
    let (_, expected) = expected_path(task_id, w)?;
    if Path::new(cwd) != expected.as_path() {
        return Err(
            "This pane's worktree metadata changed; inspect it before merging manually.".into(),
        );
    }
    Ok(())
}

/// Validate that archival can remove the checkout without losing any local
/// file. The branch and worktree metadata are deliberately retained so a
/// later restore can recreate the checkout at the same path.
pub fn can_retire(task_id: &str, cwd: &str, w: &AgentWorktree) -> Result<(), String> {
    validate(task_id, cwd, w)?;
    let (root, path) = expected_path(task_id, w)?;
    let exists = path.exists();
    let registered = is_registered(&root, &path)?;
    match (exists, registered) {
        (false, false) => return Ok(()), // already retired
        (true, true) => {}
        _ => {
            return Err(
                "The pane worktree checkout and Git registration disagree; inspect it manually before archiving."
                    .into(),
            )
        }
    }
    validate_active(&root, &path, w)?;
    retirement_clean(&path)
}

pub fn retire(task_id: &str, cwd: &str, w: &AgentWorktree) -> Result<(), String> {
    can_retire(task_id, cwd, w)?;
    let (root, path) = expected_path(task_id, w)?;
    if !is_registered(&root, &path)? {
        return Ok(());
    }
    git_at(
        &root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "remove",
            path.to_str().ok_or("Worktree path is not UTF-8.")?,
        ],
    )?;
    if path.exists() || is_registered(&root, &path)? {
        return Err("Git did not fully retire the pane worktree checkout.".into());
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

pub fn restore(task_id: &str, cwd: &str, w: &AgentWorktree) -> Result<(), String> {
    validate(task_id, cwd, w)?;
    let (root, path) = expected_path(task_id, w)?;
    let exists = path.exists();
    let registered = is_registered(&root, &path)?;
    match (exists, registered) {
        (true, true) => return validate_active(&root, &path, w),
        (false, false) => {}
        _ => {
            return Err(
                "The pane worktree checkout and Git registration disagree; inspect it manually before restoring."
                    .into(),
            )
        }
    }
    git_at(
        &root,
        &["show-ref", "--verify", &format!("refs/heads/{}", w.branch)],
    )
    .map_err(|_| {
        "The pane's retained worktree branch is missing; restore it manually before reopening the chat."
            .to_string()
    })?;
    git_at(
        &root,
        &["merge-base", "--is-ancestor", &w.base_commit, &w.branch],
    )
    .map_err(|_| {
        "The pane worktree history was rewritten past its recorded base; inspect it manually before restoring."
            .to_string()
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not recreate worktree folder: {e}"))?;
    }
    git_at(
        &root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "worktree",
            "add",
            path.to_str().ok_or("Worktree path is not UTF-8.")?,
            &w.branch,
        ],
    )?;
    validate_active(&root, &path, w)
}

pub fn project_root_matches(cwd: &str, root: &str) -> bool {
    git_at(Path::new(cwd), &["rev-parse", "--show-toplevel"])
        .ok()
        .and_then(|p| PathBuf::from(p).canonicalize().ok())
        == Path::new(root).canonicalize().ok()
        && Path::new(root).is_dir()
}

pub fn same_repository(cwd: &str, root: &str) -> bool {
    let common = |p: &str| {
        git_at(
            Path::new(p),
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .and_then(|p| PathBuf::from(p).canonicalize().ok())
    };
    match (common(cwd), common(root)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub fn merge(worktree: &AgentWorktree) -> Result<(), String> {
    let root = Path::new(&worktree.root);
    let path = Path::new(&worktree.path);
    // A lock in the Git common directory serializes merges across app instances.
    let common = PathBuf::from(git_at(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    let lock_path = common.join("monitter-worktree-merge.lock");
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path)
        .map_err(|_| "Another worktree merge is active. If a previous process crashed, inspect its lock before retrying.".to_string())?;
    struct Lock {
        path: PathBuf,
        file: Option<std::fs::File>,
    }
    impl Drop for Lock {
        fn drop(&mut self) {
            self.file.take();
            let _ = std::fs::remove_file(&self.path);
        }
    }
    let _lock = Lock {
        path: lock_path,
        file: Some(file),
    };
    clean(root)?;
    clean(path)?;
    if git_at(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])? != worktree.target_branch {
        return Err(format!(
            "Switch the project back to {} before merging.",
            worktree.target_branch
        ));
    }
    if git_at(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])? != worktree.branch {
        return Err("The pane worktree branch changed; merge it manually after review.".into());
    }
    let source_common = git_at(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if Path::new(&source_common)
        .canonicalize()
        .map_err(|e| e.to_string())?
        != common.canonicalize().map_err(|e| e.to_string())?
    {
        return Err("The pane no longer belongs to this project repository.".into());
    }
    let source = git_at(path, &["rev-parse", "HEAD"])?;
    for head in [source.as_str(), "HEAD"] {
        git_at(root, &["merge-base", "--is-ancestor", &worktree.base_commit, head])
            .map_err(|_| "The pane or project history was rewritten past its recorded base; review and merge manually.".to_string())?;
    }

    // merge-tree does not modify either index or working directory on conflict.
    git_at(root, &["merge-tree", "--write-tree", "HEAD", &source])
        .map_err(|_| "The pane conflicts with the project. Resolve it in the pane worktree, commit, then retry; the project was not changed.".to_string())?;
    git_at(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-edit",
            "--no-gpg-sign",
            "--no-overwrite-ignore",
            &source,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("monitter-agent-worktree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        git_at(&p, &["init", "-b", "main"]).unwrap();
        git_at(&p, &["config", "user.name", "Fixture"]).unwrap();
        git_at(&p, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        commit(&p, "file.txt", "base");
        p
    }
    fn commit(p: &Path, file: &str, body: &str) {
        std::fs::write(p.join(file), body).unwrap();
        git_at(p, &["add", "."]).unwrap();
        git_at(p, &["commit", "-m", "fixture"]).unwrap();
    }
    fn dispose(p: &Path, w: &[&AgentWorktree]) {
        for x in w {
            git_at(p, &["worktree", "remove", "--force", &x.path]).unwrap();
        }
        let parent = p.parent().unwrap().join(format!(
            "{}-monitter-worktrees",
            p.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir(parent);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn agent_worktree_isolates_panes_and_merges_committed_changes() {
        let p = repo();
        let a = create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).unwrap();
        let b = create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).unwrap();
        assert_ne!(a.path, b.path);
        assert_ne!(a.branch, b.branch);
        commit(Path::new(&a.path), "new.txt", "pane A");
        assert!(!p.join("new.txt").exists());
        assert!(!Path::new(&b.path).join("new.txt").exists());
        commit(&p, "main.txt", "independent project change");
        merge(&a).unwrap();
        assert_eq!(
            git_at(&p, &["rev-list", "--parents", "-n", "1", "HEAD"])
                .unwrap()
                .split_whitespace()
                .count(),
            3
        );
        assert_eq!(
            std::fs::read_to_string(p.join("main.txt")).unwrap(),
            "independent project change"
        );
        assert_eq!(
            std::fs::read_to_string(p.join("new.txt")).unwrap(),
            "pane A"
        );
        merge(&a).unwrap(); // exact repeat is harmless
        dispose(&p, &[&a, &b]);
    }
    #[test]
    fn agent_worktree_conflicts_do_not_touch_target() {
        let p = repo();
        let a = create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).unwrap();
        commit(&p, "file.txt", "main edit");
        commit(Path::new(&a.path), "file.txt", "pane edit");
        let before = git_at(&p, &["rev-parse", "HEAD"]).unwrap();
        assert!(merge(&a).unwrap_err().contains("conflicts"));
        assert_eq!(git_at(&p, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(p.join("file.txt")).unwrap(),
            "main edit"
        );
        assert!(!p.join(".git/MERGE_HEAD").exists());
        dispose(&p, &[&a]);
    }
    #[test]
    fn agent_worktree_rejects_dirty_source_target_and_wrong_branch() {
        let p = repo();
        let a = create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).unwrap();
        std::fs::write(Path::new(&a.path).join("draft"), "keep").unwrap();
        assert!(merge(&a).is_err());
        std::fs::remove_file(Path::new(&a.path).join("draft")).unwrap();
        std::fs::write(p.join("draft"), "keep").unwrap();
        assert!(merge(&a).is_err());
        assert!(create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).is_err());
        std::fs::remove_file(p.join("draft")).unwrap();
        git_at(&p, &["switch", "-c", "other"]).unwrap();
        assert!(merge(&a).is_err());
        dispose(&p, &[&a]);
    }
    #[test]
    fn agent_worktree_merge_preserves_ignored_local_files() {
        let p = repo();
        commit(&p, ".gitignore", "local-cache.txt\n");
        let a = create(p.to_str().unwrap(), &uuid::Uuid::new_v4().to_string()).unwrap();
        std::fs::write(
            Path::new(&a.path).join("local-cache.txt"),
            "tracked by pane",
        )
        .unwrap();
        git_at(Path::new(&a.path), &["add", "-f", "local-cache.txt"]).unwrap();
        git_at(Path::new(&a.path), &["commit", "-m", "tracked collision"]).unwrap();
        std::fs::write(p.join("local-cache.txt"), "keep local ignored file").unwrap();
        let before = git_at(&p, &["rev-parse", "HEAD"]).unwrap();
        assert!(merge(&a).is_err());
        assert_eq!(git_at(&p, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(p.join("local-cache.txt")).unwrap(),
            "keep local ignored file"
        );
        dispose(&p, &[&a]);
    }

    #[test]
    fn agent_worktree_checks_identity_subdirectories_and_rewritten_history() {
        let p = repo();
        let id = uuid::Uuid::new_v4().to_string();
        let a = create(p.to_str().unwrap(), &id).unwrap();
        validate(&id, &a.path, &a).unwrap();
        assert!(validate(&uuid::Uuid::new_v4().to_string(), &a.path, &a).is_err());
        std::fs::create_dir(p.join("src")).unwrap();
        assert!(same_repository(p.join("src").to_str().unwrap(), &a.root));
        assert!(same_repository(&a.path, &a.root));
        let mut wrong = a.clone();
        wrong.base_commit = "0000000000000000000000000000000000000000".into();
        assert!(merge(&wrong).unwrap_err().contains("history was rewritten"));
        dispose(&p, &[&a]);
    }

    #[test]
    fn archived_worktree_retires_checkout_and_restores_retained_branch() {
        let p = repo();
        commit(&p, ".gitignore", "cache/\n");
        let id = uuid::Uuid::new_v4().to_string();
        let a = create(p.to_str().unwrap(), &id).unwrap();
        commit(Path::new(&a.path), "pane.txt", "retained commit");
        let retained_head = git_at(Path::new(&a.path), &["rev-parse", "HEAD"]).unwrap();

        std::fs::create_dir(Path::new(&a.path).join("cache")).unwrap();
        std::fs::write(Path::new(&a.path).join("cache/generated.bin"), "preserve").unwrap();
        assert!(retire(&id, &a.path, &a)
            .unwrap_err()
            .contains("nothing was deleted"));
        assert!(Path::new(&a.path).join("cache/generated.bin").exists());
        std::fs::remove_dir_all(Path::new(&a.path).join("cache")).unwrap();

        std::fs::write(Path::new(&a.path).join("draft.txt"), "do not delete").unwrap();
        let error = retire(&id, &a.path, &a).unwrap_err();
        assert!(error.contains("nothing was deleted"));
        assert!(Path::new(&a.path).join("draft.txt").exists());
        std::fs::remove_file(Path::new(&a.path).join("draft.txt")).unwrap();

        retire(&id, &a.path, &a).unwrap();
        assert!(!Path::new(&a.path).exists());
        assert_eq!(
            git_at(&p, &["rev-parse", &a.branch]).unwrap(),
            retained_head
        );
        retire(&id, &a.path, &a).unwrap(); // exact repeat is harmless

        restore(&id, &a.path, &a).unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&a.path).join("pane.txt")).unwrap(),
            "retained commit"
        );
        restore(&id, &a.path, &a).unwrap(); // exact repeat is harmless
        retire(&id, &a.path, &a).unwrap();
        git_at(&p, &["branch", "-D", &a.branch]).unwrap();
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn agent_worktree_project_preference_is_durable_and_nonretroactive() {
        use crate::model::{CreateTaskInput, Project, ProjectWorkspace};
        let p = repo();
        let profile = std::env::temp_dir().join(format!(
            "monitter-agent-worktree-profile-{}",
            uuid::Uuid::new_v4()
        ));
        let service = crate::Service::open(None, profile.clone()).unwrap();
        let snapshot = service.snapshot().unwrap();
        let agent = snapshot.agents[0].clone();
        let project = Project { ideas: vec![], ideas_revision: 0,
            id: "fixture".into(),
            name: "Worktrees".into(),
            description: String::new(),
            icon: "folder".into(),
            color: "#3f9d6a".into(),
            workspaces: vec![ProjectWorkspace {
                host_id: agent.host_id.clone(),
                cwd: p.to_string_lossy().into_owned(),
            }],
            board_guidance_text: None,
            build_management_enabled: false,
            agent_worktrees_enabled: false,
        };
        service.save_project(project.clone()).unwrap();
        let input = || CreateTaskInput {
            agent_id: agent.id.clone(),
            title: "Pane".into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: Some(project.id.clone()),
            cwd: None,
            model_settings: None,
            sandbox: None,
        };
        let old = service.create_task(input()).unwrap();
        assert!(old.agent_worktree.is_none());
        let mut enabled = project.clone();
        enabled.agent_worktrees_enabled = true;
        service.save_project(enabled).unwrap();
        let a = service.create_task(input()).unwrap();
        let b = service.create_task(input()).unwrap();
        assert_ne!(a.cwd, b.cwd);
        assert_eq!(a.cwd, a.agent_worktree.as_ref().unwrap().path);
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == old.id)
                .unwrap()
                .cwd,
            old.cwd
        );
        service
            .mutate(None, |s| {
                s.tasks.iter_mut().find(|t| t.id == a.id).unwrap().status = "running".into();
                Ok(())
            })
            .unwrap();
        assert!(
            service
                .merge_task_worktree(&a.id)
                .unwrap_err()
                .contains("Stop agents")
        );
        service
            .mutate(None, |s| {
                s.tasks.iter_mut().find(|t| t.id == a.id).unwrap().status = "idle".into();
                Ok(())
            })
            .unwrap();
        let stopping = crate::runner::RunControl::new(false);
        stopping.reserve_cancellation();
        service
            .runs
            .lock()
            .unwrap()
            .tasks
            .insert(a.id.clone(), stopping);
        assert!(
            service
                .merge_task_worktree(&a.id)
                .unwrap_err()
                .contains("finish stopping")
        );
        service.runs.lock().unwrap().tasks.remove(&a.id);
        std::fs::create_dir(p.join("nested")).unwrap();
        service
            .mutate_data(None, |data| {
                let s = &mut data.snapshot;
                let mut host = s
                    .hosts
                    .iter()
                    .find(|h| h.id == agent.host_id)
                    .unwrap()
                    .clone();
                host.id = "other-local-host".into();
                data.task_hosts.insert(old.id.clone(), host.clone());
                s.hosts.push(host);
                let task = s.tasks.iter_mut().find(|t| t.id == old.id).unwrap();
                task.host_id = "other-local-host".into();
                task.cwd = p.join("nested").to_string_lossy().into_owned();
                task.status = "running".into();
                Ok(())
            })
            .unwrap();
        assert!(
            service
                .merge_task_worktree(&a.id)
                .unwrap_err()
                .contains("Stop agents")
        );
        service
            .mutate(None, |s| {
                s.tasks.iter_mut().find(|t| t.id == old.id).unwrap().status = "idle".into();
                Ok(())
            })
            .unwrap();
        commit(Path::new(&a.cwd), "pane.txt", "isolated");
        service.merge_task_worktree(&a.id).unwrap();
        let other = repo();
        let mut repointed = project.clone();
        repointed.workspaces[0].cwd = other.to_string_lossy().into_owned();
        service.save_project(repointed).unwrap();
        assert!(
            service
                .merge_task_worktree(&a.id)
                .unwrap_err()
                .contains("workspace changed")
        );
        dispose(&other, &[]);
        service.save_project(project.clone()).unwrap();
        let shared = service.create_task(input()).unwrap();
        assert!(shared.agent_worktree.is_none());
        let b_worktree = b.agent_worktree.as_ref().unwrap();
        std::fs::write(Path::new(&b.cwd).join("draft.txt"), "preserve").unwrap();
        assert!(service
            .set_task_archived(&b.id, true)
            .unwrap_err()
            .contains("nothing was deleted"));
        assert!(Path::new(&b.cwd).join("draft.txt").exists());
        assert!(!service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == b.id)
            .unwrap()
            .archived);
        std::fs::remove_file(Path::new(&b.cwd).join("draft.txt")).unwrap();
        service.set_task_archived(&b.id, true).unwrap();
        assert!(!Path::new(&b.cwd).exists());
        assert!(service
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == b.id)
            .unwrap()
            .archived);
        service.set_task_archived(&b.id, false).unwrap();
        assert!(Path::new(&b.cwd).exists());
        assert_eq!(
            git_at(Path::new(&b.cwd), &["symbolic-ref", "--short", "HEAD"]).unwrap(),
            b_worktree.branch
        );
        drop(service);
        let reopened = crate::Service::open(None, profile.clone()).unwrap();
        assert_eq!(
            reopened
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == a.id)
                .unwrap()
                .agent_worktree,
            a.agent_worktree
        );
        drop(reopened);
        dispose(
            &p,
            &[
                a.agent_worktree.as_ref().unwrap(),
                b.agent_worktree.as_ref().unwrap(),
            ],
        );
        std::fs::remove_dir_all(profile).unwrap();
    }
}
