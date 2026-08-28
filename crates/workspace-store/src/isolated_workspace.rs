use std::{
    fs,
    io::ErrorKind,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use rustix::{
    fs::{Mode, OFlags, open, openat},
    io::Errno,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{workspace::OpenCodeWorkspaceConfig, workspace::validate_agent_key};

const RUNS_ROOT: &str = "runs";
const CONVERSATIONS_ROOT: &str = "conversations";
const WORKSPACE_ROOT: &str = "workspace";
const MAX_WORKSPACE_TRAVERSAL_DEPTH: usize = 64;
const MAX_WORKSPACE_TRAVERSAL_ENTRIES: usize = 100_000;
const MAX_WORKSPACE_TRAVERSAL_DURATION: Duration = Duration::from_secs(30);

/// A validated, controller-derived scheduled-run workspace identity. It never
/// accepts a caller-provided filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWorkspacePath {
    agent_key: String,
    run_id: i64,
}

impl RunWorkspacePath {
    pub fn new(agent_key: impl Into<String>, run_id: i64) -> Result<Self> {
        let agent_key = agent_key.into();
        validate_agent_key(&agent_key)?;
        if run_id <= 0 {
            bail!("run id must be positive");
        }
        Ok(Self { agent_key, run_id })
    }

    pub fn agent_key(&self) -> &str {
        &self.agent_key
    }

    pub const fn run_id(&self) -> i64 {
        self.run_id
    }

    pub fn root_host_path(&self, config: &OpenCodeWorkspaceConfig) -> PathBuf {
        config
            .host_workspaces_root
            .join(RUNS_ROOT)
            .join(&self.agent_key)
            .join(self.run_id.to_string())
    }

    pub fn workspace_host_path(&self, config: &OpenCodeWorkspaceConfig) -> PathBuf {
        self.root_host_path(config).join(WORKSPACE_ROOT)
    }

    pub fn workspace_container_path(&self, config: &OpenCodeWorkspaceConfig) -> String {
        join_container_path(
            &config.container_workspaces_root,
            &[
                RUNS_ROOT,
                &self.agent_key,
                &self.run_id.to_string(),
                WORKSPACE_ROOT,
            ],
        )
    }
}

/// A validated, controller-derived internal conversation workspace identity.
/// External channel keys are intentionally not part of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationWorkspacePath {
    agent_key: String,
    conversation_id: Uuid,
}

impl ConversationWorkspacePath {
    pub fn new(agent_key: impl Into<String>, conversation_id: Uuid) -> Result<Self> {
        let agent_key = agent_key.into();
        validate_agent_key(&agent_key)?;
        Ok(Self {
            agent_key,
            conversation_id,
        })
    }

    pub fn parse(agent_key: impl Into<String>, conversation_id: &str) -> Result<Self> {
        let conversation_id = Uuid::parse_str(conversation_id)
            .context("invalid conversation id for workspace path")?;
        Self::new(agent_key, conversation_id)
    }

    pub fn agent_key(&self) -> &str {
        &self.agent_key
    }

    pub const fn conversation_id(&self) -> Uuid {
        self.conversation_id
    }

    pub fn root_host_path(&self, config: &OpenCodeWorkspaceConfig) -> PathBuf {
        config
            .host_workspaces_root
            .join(CONVERSATIONS_ROOT)
            .join(&self.agent_key)
            .join(self.conversation_id.to_string())
    }

    pub fn workspace_host_path(&self, config: &OpenCodeWorkspaceConfig) -> PathBuf {
        self.root_host_path(config).join(WORKSPACE_ROOT)
    }

    pub fn workspace_container_path(&self, config: &OpenCodeWorkspaceConfig) -> String {
        join_container_path(
            &config.container_workspaces_root,
            &[
                CONVERSATIONS_ROOT,
                &self.agent_key,
                &self.conversation_id.to_string(),
                WORKSPACE_ROOT,
            ],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolatedWorkspaceCreated {
    pub workspace_container_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolatedWorkspaceInspection {
    pub workspace_exists: bool,
    pub size_bytes: u64,
    pub file_count: u64,
    pub runtime_secrets_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSecretsScrubbed {
    pub workspace_exists: bool,
    pub removed: bool,
}

pub fn create_run_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
) -> Result<IsolatedWorkspaceCreated> {
    create_isolated_workspace(
        config,
        &[RUNS_ROOT, path.agent_key(), &path.run_id().to_string()],
        path.workspace_container_path(config),
    )
}

pub fn create_conversation_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &ConversationWorkspacePath,
) -> Result<IsolatedWorkspaceCreated> {
    create_isolated_workspace(
        config,
        &[
            CONVERSATIONS_ROOT,
            path.agent_key(),
            &path.conversation_id().to_string(),
        ],
        path.workspace_container_path(config),
    )
}

pub fn inspect_run_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
) -> Result<IsolatedWorkspaceInspection> {
    inspect_isolated_workspace(
        config,
        &[RUNS_ROOT, path.agent_key(), &path.run_id().to_string()],
    )
}

pub fn inspect_conversation_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &ConversationWorkspacePath,
) -> Result<IsolatedWorkspaceInspection> {
    inspect_isolated_workspace(
        config,
        &[
            CONVERSATIONS_ROOT,
            path.agent_key(),
            &path.conversation_id().to_string(),
        ],
    )
}

pub fn scrub_run_workspace_runtime_secrets(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
) -> Result<RuntimeSecretsScrubbed> {
    scrub_isolated_workspace_runtime_secrets(
        config,
        &[RUNS_ROOT, path.agent_key(), &path.run_id().to_string()],
    )
}

pub fn scrub_conversation_workspace_runtime_secrets(
    config: &OpenCodeWorkspaceConfig,
    path: &ConversationWorkspacePath,
) -> Result<RuntimeSecretsScrubbed> {
    scrub_isolated_workspace_runtime_secrets(
        config,
        &[
            CONVERSATIONS_ROOT,
            path.agent_key(),
            &path.conversation_id().to_string(),
        ],
    )
}

pub fn delete_run_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
) -> Result<bool> {
    delete_isolated_workspace(
        config,
        &[RUNS_ROOT, path.agent_key()],
        &path.run_id().to_string(),
    )
}

pub fn delete_conversation_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &ConversationWorkspacePath,
) -> Result<bool> {
    delete_isolated_workspace(
        config,
        &[CONVERSATIONS_ROOT, path.agent_key()],
        &path.conversation_id().to_string(),
    )
}

fn create_isolated_workspace(
    config: &OpenCodeWorkspaceConfig,
    root_components: &[&str],
    workspace_container_path: String,
) -> Result<IsolatedWorkspaceCreated> {
    let mut directory = open_workspace_root(config)?;
    for component in root_components {
        directory = open_or_create_directory_at(&directory, component)?;
    }

    // Creating each component through a held descriptor makes a retry safe
    // after an interrupted mkdir sequence without trusting a persisted path.
    open_or_create_directory_at(&directory, WORKSPACE_ROOT)?;
    Ok(IsolatedWorkspaceCreated {
        workspace_container_path,
    })
}

fn inspect_isolated_workspace(
    config: &OpenCodeWorkspaceConfig,
    root_components: &[&str],
) -> Result<IsolatedWorkspaceInspection> {
    let Some(workspace) = open_existing_workspace(config, root_components)? else {
        return Ok(IsolatedWorkspaceInspection {
            workspace_exists: false,
            size_bytes: 0,
            file_count: 0,
            runtime_secrets_present: false,
        });
    };

    let mut size_bytes = 0;
    let mut file_count = 0;
    collect_workspace_stats(&workspace, &mut size_bytes, &mut file_count)?;
    Ok(IsolatedWorkspaceInspection {
        workspace_exists: true,
        size_bytes,
        file_count,
        runtime_secrets_present: directory_contains_name(&workspace, ".env")?,
    })
}

fn scrub_isolated_workspace_runtime_secrets(
    config: &OpenCodeWorkspaceConfig,
    root_components: &[&str],
) -> Result<RuntimeSecretsScrubbed> {
    let Some(workspace) = open_existing_workspace(config, root_components)? else {
        return Ok(RuntimeSecretsScrubbed {
            workspace_exists: false,
            removed: false,
        });
    };

    // The generated runtime credential is always the workspace-root `.env`.
    // Removing the directory entry never follows a malicious `.env` symlink.
    let secret_path = descriptor_path(&workspace).join(".env");
    match fs::remove_file(secret_path) {
        Ok(()) => Ok(RuntimeSecretsScrubbed {
            workspace_exists: true,
            removed: true,
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(RuntimeSecretsScrubbed {
            workspace_exists: true,
            removed: false,
        }),
        Err(error) if error.kind() == ErrorKind::IsADirectory => {
            bail!("runtime secret path is a directory")
        }
        Err(error) => Err(error).context("failed to remove workspace runtime secret"),
    }
}

fn delete_isolated_workspace(
    config: &OpenCodeWorkspaceConfig,
    parent_components: &[&str],
    workspace_identity: &str,
) -> Result<bool> {
    let mut parent = open_workspace_root(config)?;
    for component in parent_components {
        let Some(next) = open_directory_at(&parent, component)? else {
            return Ok(false);
        };
        parent = next;
    }

    let Some(root) = open_directory_at(&parent, workspace_identity)? else {
        return Ok(false);
    };
    remove_directory_contents(&root)?;
    fs::remove_dir(descriptor_path(&parent).join(workspace_identity)).with_context(|| {
        format!("failed to remove isolated workspace root for identity {workspace_identity}")
    })?;
    Ok(true)
}

fn open_existing_workspace(
    config: &OpenCodeWorkspaceConfig,
    root_components: &[&str],
) -> Result<Option<fs::File>> {
    let mut directory = open_workspace_root(config)?;
    for component in root_components {
        let Some(next) = open_directory_at(&directory, component)? else {
            return Ok(None);
        };
        directory = next;
    }
    open_directory_at(&directory, WORKSPACE_ROOT)
}

fn open_workspace_root(config: &OpenCodeWorkspaceConfig) -> Result<fs::File> {
    fs::create_dir_all(&config.host_workspaces_root).with_context(|| {
        format!(
            "failed to create workspace root {}",
            config.host_workspaces_root.display()
        )
    })?;
    match open(
        &config.host_workspaces_root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(directory) => Ok(fs::File::from(directory)),
        Err(Errno::LOOP) => bail!("workspace root is a symlink"),
        Err(error) => Err(error.into()),
    }
}

fn open_or_create_directory_at(parent: &fs::File, name: &str) -> Result<fs::File> {
    if let Some(directory) = open_directory_at(parent, name)? {
        return Ok(directory);
    }

    match fs::create_dir(descriptor_path(parent).join(name)) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create workspace directory {name}"));
        }
    }
    open_directory_at(parent, name)?
        .ok_or_else(|| anyhow::anyhow!("workspace directory disappeared"))
}

fn open_directory_at(parent: &fs::File, name: &str) -> Result<Option<fs::File>> {
    match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(directory) => Ok(Some(fs::File::from(directory))),
        Err(Errno::NOENT) => Ok(None),
        Err(Errno::LOOP) => bail!("workspace path component is a symlink"),
        Err(Errno::NOTDIR) => bail!("workspace path component is not a directory"),
        Err(error) => Err(error.into()),
    }
}

fn directory_contains_name(directory: &fs::File, name: &str) -> Result<bool> {
    match openat(
        directory,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

struct WorkspaceTraversalBudget {
    started_at: Instant,
    entries: usize,
}

impl WorkspaceTraversalBudget {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            entries: 0,
        }
    }

    fn visit_entry(&mut self) -> Result<()> {
        self.entries += 1;
        if self.entries > MAX_WORKSPACE_TRAVERSAL_ENTRIES {
            bail!("workspace traversal exceeds the entry limit");
        }
        if self.started_at.elapsed() > MAX_WORKSPACE_TRAVERSAL_DURATION {
            bail!("workspace traversal exceeds the time limit");
        }
        Ok(())
    }

    fn descend(&self, depth: usize) -> Result<()> {
        if depth > MAX_WORKSPACE_TRAVERSAL_DEPTH {
            bail!("workspace traversal exceeds the directory depth limit");
        }
        Ok(())
    }
}

fn collect_workspace_stats(
    directory: &fs::File,
    size_bytes: &mut u64,
    file_count: &mut u64,
) -> Result<()> {
    let mut budget = WorkspaceTraversalBudget::new();
    let root = directory.try_clone()?;
    // Depth-first frames keep descriptor usage bounded by the depth limit.
    let mut directories = vec![InspectDirectoryFrame {
        entries: fs::read_dir(descriptor_path(&root))?,
        directory: root,
        depth: 0,
    }];

    while !directories.is_empty() {
        let entry = {
            let frame = directories
                .last_mut()
                .ok_or_else(|| anyhow::anyhow!("workspace traversal stack is empty"))?;
            frame.entries.next().transpose()?
        };
        let Some(entry) = entry else {
            let _ = directories.pop();
            continue;
        };

        budget.visit_entry()?;
        let name = entry.file_name();
        let (probe, depth) = {
            let parent = directories
                .last()
                .ok_or_else(|| anyhow::anyhow!("workspace traversal parent disappeared"))?;
            let probe = match openat(
                &parent.directory,
                &name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(probe) => fs::File::from(probe),
                Err(Errno::NOENT) | Err(Errno::LOOP) => continue,
                Err(error) => return Err(error.into()),
            };
            (probe, parent.depth)
        };
        let metadata = probe.metadata()?;
        if metadata.is_dir() {
            let child_depth = depth + 1;
            budget.descend(child_depth)?;
            let child = {
                let parent = directories
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("workspace traversal parent disappeared"))?;
                match openat(
                    &parent.directory,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                ) {
                    Ok(child) => fs::File::from(child),
                    Err(Errno::NOENT) | Err(Errno::LOOP) => continue,
                    Err(error) => return Err(error.into()),
                }
            };
            directories.push(InspectDirectoryFrame {
                entries: fs::read_dir(descriptor_path(&child))?,
                directory: child,
                depth: child_depth,
            });
        } else if metadata.is_file() {
            *size_bytes = size_bytes.saturating_add(metadata.len());
            *file_count = file_count.saturating_add(1);
        }
    }
    Ok(())
}

struct InspectDirectoryFrame {
    directory: fs::File,
    entries: fs::ReadDir,
    depth: usize,
}

struct DeleteDirectoryFrame {
    directory: fs::File,
    entries: fs::ReadDir,
    depth: usize,
    name_in_parent: Option<std::ffi::OsString>,
}

fn remove_directory_contents(directory: &fs::File) -> Result<()> {
    let root = directory.try_clone()?;
    let mut budget = WorkspaceTraversalBudget::new();
    let mut directories = vec![DeleteDirectoryFrame {
        entries: fs::read_dir(descriptor_path(&root))?,
        directory: root,
        depth: 0,
        name_in_parent: None,
    }];

    while !directories.is_empty() {
        let entry = {
            let frame = directories
                .last_mut()
                .ok_or_else(|| anyhow::anyhow!("workspace traversal stack is empty"))?;
            frame.entries.next().transpose()?
        };
        let Some(entry) = entry else {
            let completed = directories
                .pop()
                .ok_or_else(|| anyhow::anyhow!("workspace traversal stack is empty"))?;
            let name_in_parent = completed.name_in_parent.clone();
            drop(completed);
            if let Some(name) = name_in_parent {
                let parent = directories
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("workspace traversal parent disappeared"))?;
                fs::remove_dir(descriptor_path(&parent.directory).join(name))
                    .context("failed to remove workspace directory")?;
            }
            continue;
        };

        budget.visit_entry()?;
        let name = entry.file_name();
        let (path, probe, depth) = {
            let parent = directories
                .last()
                .ok_or_else(|| anyhow::anyhow!("workspace traversal parent disappeared"))?;
            let path = descriptor_path(&parent.directory).join(&name);
            let probe = match openat(
                &parent.directory,
                &name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(probe) => fs::File::from(probe),
                Err(Errno::NOENT) => continue,
                Err(error) => return Err(error.into()),
            };
            (path, probe, parent.depth)
        };
        if probe.metadata()?.is_dir() {
            let child_depth = depth + 1;
            budget.descend(child_depth)?;
            let child = {
                let parent = directories
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("workspace traversal parent disappeared"))?;
                match openat(
                    &parent.directory,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                ) {
                    Ok(child) => fs::File::from(child),
                    Err(Errno::NOENT) => continue,
                    Err(Errno::LOOP) => bail!("workspace path component is a symlink"),
                    Err(error) => return Err(error.into()),
                }
            };
            directories.push(DeleteDirectoryFrame {
                entries: fs::read_dir(descriptor_path(&child))?,
                directory: child,
                depth: child_depth,
                name_in_parent: Some(name),
            });
        } else {
            // `remove_file` unlinks a symlink itself rather than following it.
            fs::remove_file(path).context("failed to remove workspace file")?;
        }
    }
    Ok(())
}

fn descriptor_path(directory: &fs::File) -> PathBuf {
    Path::new("/proc/self/fd").join(directory.as_raw_fd().to_string())
}

fn join_container_path(root: &str, segments: &[&str]) -> String {
    let mut path = root.trim_end_matches('/').to_string();
    for segment in segments {
        path.push('/');
        path.push_str(segment);
    }
    path
}

#[cfg(test)]
mod tests {
    use std::{
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time after unix epoch")
                .as_nanos();
            let path =
                PathBuf::from("/tmp/opencode").join(format!("{prefix}-{}-{suffix}", process::id()));
            fs::create_dir_all(&path).expect("create temp directory");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn config(root: &Path) -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: root.to_path_buf(),
            host_workspaces_root: root.to_path_buf(),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: String::new(),
        }
    }

    #[test]
    fn run_workspace_paths_are_validated_and_derived() {
        let config = config(Path::new("/tmp/workspaces"));
        let path = RunWorkspacePath::new("btc-2", 42).expect("valid run path");

        assert_eq!(
            path.workspace_host_path(&config),
            PathBuf::from("/tmp/workspaces/runs/btc-2/42/workspace")
        );
        assert_eq!(
            path.workspace_container_path(&config),
            "/workspaces/runs/btc-2/42/workspace"
        );
        assert!(RunWorkspacePath::new("../btc", 42).is_err());
        assert!(RunWorkspacePath::new("btc", 0).is_err());
    }

    #[test]
    fn conversation_workspace_identity_uses_only_the_internal_uuid() {
        let config = config(Path::new("/tmp/workspaces"));
        let id = Uuid::parse_str("b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33").expect("parse uuid");
        let path = ConversationWorkspacePath::new("btc-2", id).expect("valid conversation path");

        assert_eq!(
            path.workspace_container_path(&config),
            "/workspaces/conversations/btc-2/b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33/workspace"
        );
    }

    #[test]
    fn run_workspace_creation_is_idempotent_and_recovers_partial_directories() {
        let temp = TempDir::new("run-workspace-create");
        let config = config(&temp.path);
        let path = RunWorkspacePath::new("agent", 7).expect("path");
        fs::create_dir_all(path.root_host_path(&config)).expect("create partial root");

        let first = create_run_workspace(&config, &path).expect("recover partial workspace");
        let second = create_run_workspace(&config, &path).expect("repeat workspace create");

        assert_eq!(first, second);
        assert!(path.workspace_host_path(&config).is_dir());
        assert!(!path.workspace_host_path(&config).join("scratch").exists());
    }

    #[test]
    fn inspection_and_secret_scrub_are_scoped_to_the_workspace_root() {
        let temp = TempDir::new("run-workspace-scrub");
        let config = config(&temp.path);
        let path = RunWorkspacePath::new("agent", 8).expect("path");
        create_run_workspace(&config, &path).expect("create workspace");
        let workspace = path.workspace_host_path(&config);
        fs::create_dir_all(workspace.join("nested")).expect("create nested directory");
        fs::write(workspace.join("nested/file.txt"), b"data").expect("write data");
        fs::write(workspace.join(".env"), b"credential").expect("write credential");

        let inspection = inspect_run_workspace(&config, &path).expect("inspect workspace");
        assert!(inspection.workspace_exists);
        assert_eq!(inspection.file_count, 2);
        assert_eq!(inspection.size_bytes, 14);
        assert!(inspection.runtime_secrets_present);

        assert_eq!(
            scrub_run_workspace_runtime_secrets(&config, &path).expect("scrub credential"),
            RuntimeSecretsScrubbed {
                workspace_exists: true,
                removed: true,
            }
        );
        assert!(!workspace.join(".env").exists());
        assert!(workspace.join("nested/file.txt").exists());
    }

    #[test]
    fn deleting_a_run_workspace_is_idempotent() {
        let temp = TempDir::new("run-workspace-delete");
        let config = config(&temp.path);
        let path = RunWorkspacePath::new("agent", 9).expect("path");
        create_run_workspace(&config, &path).expect("create workspace");
        fs::write(
            path.workspace_host_path(&config).join("artifact.txt"),
            b"artifact",
        )
        .expect("write artifact");

        assert!(delete_run_workspace(&config, &path).expect("delete workspace"));
        assert!(!path.root_host_path(&config).exists());
        assert!(!delete_run_workspace(&config, &path).expect("delete missing workspace"));
    }

    #[test]
    fn inspection_and_deletion_reject_overdeep_workspace_trees() {
        let temp = TempDir::new("run-workspace-depth-limit");
        let config = config(&temp.path);
        let path = RunWorkspacePath::new("agent", 11).expect("path");
        create_run_workspace(&config, &path).expect("create workspace");

        let mut nested = path.workspace_host_path(&config);
        for _ in 0..=MAX_WORKSPACE_TRAVERSAL_DEPTH {
            nested.push("nested");
            fs::create_dir(&nested).expect("create nested directory");
        }

        assert!(inspect_run_workspace(&config, &path).is_err());
        assert!(delete_run_workspace(&config, &path).is_err());
        assert!(path.root_host_path(&config).exists());
    }

    #[cfg(unix)]
    #[test]
    fn secret_scrub_unlinks_a_symlink_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("run-workspace-symlink-secret");
        let config = config(&temp.path);
        let path = RunWorkspacePath::new("agent", 10).expect("path");
        create_run_workspace(&config, &path).expect("create workspace");
        let outside = temp.path.join("outside-secret");
        fs::write(&outside, b"keep").expect("write outside secret");
        symlink(&outside, path.workspace_host_path(&config).join(".env"))
            .expect("create secret symlink");

        scrub_run_workspace_runtime_secrets(&config, &path).expect("scrub symlink");

        assert_eq!(fs::read(&outside).expect("read outside secret"), b"keep");
        assert!(!path.workspace_host_path(&config).join(".env").exists());
    }

    #[cfg(unix)]
    #[test]
    fn creation_rejects_symlinked_path_components() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("run-workspace-symlink-path");
        let config = config(&temp.path);
        let outside = temp.path.join("outside");
        fs::create_dir_all(&outside).expect("create outside directory");
        symlink(&outside, temp.path.join(RUNS_ROOT)).expect("create runs symlink");

        let error =
            create_run_workspace(&config, &RunWorkspacePath::new("agent", 11).expect("path"))
                .expect_err("symlinked root must fail");
        assert!(error.to_string().contains("symlink") || error.to_string().contains("directory"));
    }
}
