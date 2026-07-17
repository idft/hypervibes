use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::workspace::OpenCodeWorkspaceConfig;

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TREE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_TREE_FILES: usize = 500;
const CODING_AGENT_PATH: &str = ".opencode/agents/analysis-coding.md";
const USER_DESCENDANT_PERMISSION: &str = "    \"scripts/user/**\": allow";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionJournalPhase {
    Prepared,
    LiveBackedUp,
    CandidatePromoted,
    SmokeTestPassed,
    Completed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionJournal {
    pub task_id: i64,
    pub agent_key: String,
    pub base_manifest_hash: String,
    pub candidate_manifest_hash: String,
    pub live_path: PathBuf,
    pub candidate_path: PathBuf,
    pub backup_path: PathBuf,
    pub retained_version_path: Option<PathBuf>,
    pub phase: PromotionJournalPhase,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingCandidate {
    pub root: PathBuf,
    pub user_root: PathBuf,
    pub base_manifest: BTreeMap<String, String>,
}

pub fn live_user_root(config: &OpenCodeWorkspaceConfig, agent_key: &str) -> Result<PathBuf> {
    validate_agent_key(agent_key)?;
    Ok(config
        .host_workspaces_root
        .join("agents")
        .join(agent_key)
        .join("scripts")
        .join("user"))
}

pub fn candidate_root(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<PathBuf> {
    validate_agent_key(agent_key)?;
    if task_id <= 0 {
        bail!("coding task id must be positive");
    }
    Ok(config
        .host_workspaces_root
        .join("coding")
        .join(agent_key)
        .join(task_id.to_string())
        .join("workspace"))
}

pub fn candidate_container_root(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<String> {
    validate_agent_key(agent_key)?;
    if task_id <= 0 {
        bail!("coding task id must be positive");
    }
    Ok(format!(
        "{}/coding/{}/{}/workspace",
        config.container_workspaces_root.trim_end_matches('/'),
        agent_key,
        task_id
    ))
}

pub fn promotion_journal_path(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<PathBuf> {
    validate_agent_key(agent_key)?;
    if task_id <= 0 {
        bail!("coding task id must be positive");
    }
    Ok(config
        .host_workspaces_root
        .join("coding")
        .join(agent_key)
        .join(task_id.to_string())
        .join("promotion-journal.json"))
}

pub fn write_promotion_journal(
    config: &OpenCodeWorkspaceConfig,
    journal: &PromotionJournal,
) -> Result<()> {
    let path = promotion_journal_path(config, &journal.agent_key, journal.task_id)?;
    let parent = path.parent().context("promotion journal has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".promotion-journal-{}.tmp", std::process::id()));
    let contents =
        serde_json::to_vec_pretty(journal).context("failed to encode promotion journal")?;
    fs::write(&temporary, contents).context("failed to write promotion journal temporary file")?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("failed to atomically publish promotion journal");
    }
    Ok(())
}

pub fn read_promotion_journal(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<Option<PromotionJournal>> {
    let path = promotion_journal_path(config, agent_key, task_id)?;
    match fs::read(&path) {
        Ok(contents) => Ok(Some(
            serde_json::from_slice(&contents).context("invalid promotion journal")?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("failed to read promotion journal"),
    }
}

pub fn update_promotion_journal_phase(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
    phase: PromotionJournalPhase,
) -> Result<()> {
    let mut journal = read_promotion_journal(config, agent_key, task_id)?
        .context("promotion journal is missing")?;
    journal.phase = phase;
    journal.updated_at = Utc::now();
    write_promotion_journal(config, &journal)
}

pub fn set_promotion_journal_retained_version(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
    retained_version_path: PathBuf,
) -> Result<()> {
    let mut journal = read_promotion_journal(config, agent_key, task_id)?
        .context("promotion journal is missing")?;
    journal.retained_version_path = Some(retained_version_path);
    journal.updated_at = Utc::now();
    write_promotion_journal(config, &journal)
}

pub fn prepare_coding_candidate(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
    display_name: &str,
    api_key: &str,
) -> Result<CodingCandidate> {
    let root = candidate_root(config, agent_key, task_id)?;
    let container_root = candidate_container_root(config, agent_key, task_id)?;
    if root.exists() {
        bail!("coding candidate already exists: {}", root.display());
    }
    fs::create_dir_all(root.join("scripts/user/tests"))?;
    fs::create_dir_all(root.join("data"))?;
    fs::create_dir_all(root.join("scratch"))?;
    for relative in [".opencode/commands", ".opencode/agents", ".opencode/skills"] {
        copy_template_tree(&config.source_root.join(relative), &root.join(relative))?;
    }
    add_candidate_permission_scope(&root, &container_root)?;
    for (template, output) in [
        ("AGENTS.md.template", "AGENTS.md"),
        ("opencode.json.template", "opencode.json"),
    ] {
        let contents = fs::read_to_string(config.source_root.join(template))?
            .replace("{{agent_key}}", agent_key)
            .replace("{{display_name}}", display_name)
            .replace("{{api_base_url}}", &config.api_base_url)
            .replace("{{workspace_container_path}}", &container_root);
        fs::write(root.join(output), contents)?;
    }
    fs::write(
        root.join(".env"),
        format!(
            "VIBETRADING_AGENT_KEY={agent_key}\nVIBETRADING_API_BASE_URL={}\nVIBETRADING_API_KEY={api_key}\nVIBETRADING_WORKSPACE={}\nVIBETRADING_ENGINEERING_TASK_ID={task_id}\n",
            config.api_base_url, container_root,
        ),
    )?;
    let user_root = root.join("scripts/user");
    let live_root = live_user_root(config, agent_key)?;
    if live_root.exists() {
        copy_user_tree(&live_root, &user_root)?;
    }
    let base_manifest = manifest_tree(&user_root)?;
    Ok(CodingCandidate {
        root,
        user_root,
        base_manifest,
    })
}

fn add_candidate_permission_scope(root: &Path, container_root: &str) -> Result<()> {
    let profile_path = root.join(CODING_AGENT_PATH);
    let profile = fs::read_to_string(&profile_path)
        .with_context(|| format!("failed to read {}", profile_path.display()))?;
    let permission_count = profile.matches(USER_DESCENDANT_PERMISSION).count();
    if permission_count != 3 {
        bail!("coding profile must contain exactly three scripts/user descendant permissions");
    }

    // Non-Git OpenCode workspaces use `/` as their project worktree, so file
    // tools authorize against this root-relative path rather than the session cwd.
    let scoped_user_root = format!(
        "{}/scripts/user",
        container_root.trim_start_matches('/').trim_end_matches('/')
    );
    let replacement = format!(
        "{USER_DESCENDANT_PERMISSION}\n    \"{scoped_user_root}\": allow\n    \"{scoped_user_root}/**\": allow"
    );
    let rendered = profile.replace(USER_DESCENDANT_PERMISSION, &replacement);
    fs::write(&profile_path, rendered)
        .with_context(|| format!("failed to write {}", profile_path.display()))?;
    Ok(())
}

pub fn manifest_tree(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut manifest = BTreeMap::new();
    let mut total = 0_u64;
    if !root.exists() {
        return Ok(manifest);
    }
    collect_files(root, root, &mut manifest, &mut total)?;
    Ok(manifest)
}

pub fn changed_paths(
    base: &BTreeMap<String, String>,
    candidate: &BTreeMap<String, String>,
) -> Vec<String> {
    base.keys()
        .chain(candidate.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| base.get(*path) != candidate.get(*path))
        .cloned()
        .collect()
}

pub fn manifest_hash(manifest: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (path, digest) in manifest {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(digest.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

/// Atomically swap the candidate's `scripts/user` directory into the live
/// workspace. Callers must hold the agent's exclusive workspace lease and
/// verify the promoted tree matches the validated candidate hash before
/// deleting the returned backup directory.
pub fn promote_user_tree(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
    expected_base: &BTreeMap<String, String>,
) -> Result<PathBuf> {
    let live = live_user_root(config, agent_key)?;
    let candidate = candidate_root(config, agent_key, task_id)?.join("scripts/user");
    let backup = config
        .host_workspaces_root
        .join("coding")
        .join(agent_key)
        .join(task_id.to_string())
        .join("backup-user");
    if manifest_tree(&live)? != *expected_base {
        bail!("live scripts/user changed while coding was running");
    }
    if !candidate.exists() {
        bail!("coding candidate user tree is missing");
    }
    remove_ignored_artifacts(&candidate)?;
    if backup.exists() {
        bail!("coding backup already exists");
    }
    let journal = PromotionJournal {
        task_id,
        agent_key: agent_key.to_string(),
        base_manifest_hash: manifest_hash(expected_base),
        candidate_manifest_hash: manifest_hash(&manifest_tree(&candidate)?),
        live_path: live.clone(),
        candidate_path: candidate.clone(),
        backup_path: backup.clone(),
        retained_version_path: None,
        phase: PromotionJournalPhase::Prepared,
        updated_at: Utc::now(),
    };
    write_promotion_journal(config, &journal)?;
    fs::create_dir_all(backup.parent().expect("backup has task parent"))?;
    if live.exists() {
        fs::rename(&live, &backup).context("failed to move live user tree to backup")?;
        update_promotion_journal_phase(
            config,
            agent_key,
            task_id,
            PromotionJournalPhase::LiveBackedUp,
        )?;
    }
    if let Err(error) = fs::rename(&candidate, &live) {
        if backup.exists() {
            let _ = fs::rename(&backup, &live);
        }
        let _ = update_promotion_journal_phase(
            config,
            agent_key,
            task_id,
            PromotionJournalPhase::RolledBack,
        );
        return Err(error).context("failed to promote candidate user tree");
    }
    update_promotion_journal_phase(
        config,
        agent_key,
        task_id,
        PromotionJournalPhase::CandidatePromoted,
    )?;
    Ok(backup)
}

pub fn rollback_user_tree(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<()> {
    let live = live_user_root(config, agent_key)?;
    let task_root = candidate_root(config, agent_key, task_id)?;
    let backup = task_root
        .parent()
        .context("coding task path has no parent")?
        .join("backup-user");
    if !backup.exists() {
        bail!("coding backup user tree is missing");
    }
    if live.exists() {
        fs::remove_dir_all(&live).context("failed to remove failed promoted user tree")?;
    }
    fs::rename(backup, live).context("failed to restore coding backup")?;
    Ok(())
}

pub fn retain_successful_version(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    task_id: i64,
) -> Result<PathBuf> {
    validate_agent_key(agent_key)?;
    let task_root = candidate_root(config, agent_key, task_id)?;
    let backup = task_root
        .parent()
        .context("coding task path has no parent")?
        .join("backup-user");
    if !backup.exists() {
        bail!("coding backup user tree is missing");
    }
    let versions_root = config.host_workspaces_root.join("versions").join(agent_key);
    let version_root = versions_root.join(task_id.to_string());
    if version_root.exists() {
        bail!("retained coding version already exists");
    }
    fs::create_dir_all(&versions_root)?;
    fs::rename(&backup, version_root.join("user")).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            fs::create_dir_all(&version_root)?;
            fs::rename(&backup, version_root.join("user"))
        } else {
            Err(error)
        }
    })?;
    let mut versions: Vec<(i64, PathBuf)> = fs::read_dir(&versions_root)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().parse::<i64>().ok()?;
            Some((id, entry.path()))
        })
        .collect();
    versions.sort_by_key(|(id, _)| *id);
    while versions.len() > 5 {
        let (_, old) = versions.remove(0);
        fs::remove_dir_all(old)?;
    }
    Ok(version_root)
}

/// Recover a journal after a process restart. Any phase before a verified
/// completed state is treated conservatively: if a backup exists, restore it
/// as the live tree rather than guessing that an unverified candidate is safe.
pub fn recover_promotion_journal(
    config: &OpenCodeWorkspaceConfig,
    journal: &PromotionJournal,
) -> Result<PromotionJournalPhase> {
    match journal.phase {
        PromotionJournalPhase::Completed | PromotionJournalPhase::RolledBack => {
            return Ok(journal.phase);
        }
        PromotionJournalPhase::Prepared => return Ok(PromotionJournalPhase::Prepared),
        PromotionJournalPhase::SmokeTestPassed => {
            if journal.retained_version_path.is_some() {
                return Ok(PromotionJournalPhase::Completed);
            }
        }
        PromotionJournalPhase::LiveBackedUp | PromotionJournalPhase::CandidatePromoted => {}
    }

    if journal.backup_path.exists() {
        if journal.live_path.exists() {
            fs::remove_dir_all(&journal.live_path)
                .context("failed to remove unverified promoted tree during recovery")?;
        }
        fs::create_dir_all(
            journal
                .live_path
                .parent()
                .context("live journal path has no parent")?,
        )?;
        fs::rename(&journal.backup_path, &journal.live_path)
            .context("failed to restore live tree during promotion recovery")?;
        update_promotion_journal_phase(
            config,
            &journal.agent_key,
            journal.task_id,
            PromotionJournalPhase::RolledBack,
        )?;
        return Ok(PromotionJournalPhase::RolledBack);
    }
    Ok(PromotionJournalPhase::Prepared)
}

pub fn list_promotion_journals(config: &OpenCodeWorkspaceConfig) -> Result<Vec<PromotionJournal>> {
    let root = config.host_workspaces_root.join("coding");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut journals = Vec::new();
    for agent_entry in fs::read_dir(root)? {
        let agent_entry = agent_entry?;
        if !agent_entry.file_type()?.is_dir() {
            continue;
        }
        let agent_key = agent_entry.file_name().to_string_lossy().into_owned();
        for task_entry in fs::read_dir(agent_entry.path())? {
            let task_entry = task_entry?;
            if !task_entry.file_type()?.is_dir() {
                continue;
            }
            let Ok(task_id) = task_entry.file_name().to_string_lossy().parse::<i64>() else {
                continue;
            };
            let Some(journal) = read_promotion_journal(config, &agent_key, task_id)? else {
                continue;
            };
            journals.push(journal);
        }
    }
    Ok(journals)
}

fn copy_user_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("live scripts/user must be a regular directory");
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "__pycache__" || name.ends_with(".pyc") || name.ends_with('~') {
            continue;
        }
        let target = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            bail!("symlinks are not allowed in scripts/user: {name}");
        }
        if metadata.is_dir() {
            fs::create_dir_all(&target)?;
            copy_user_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_FILE_BYTES {
                bail!("file exceeds coding size limit: {name}");
            }
            fs::copy(entry.path(), target)?;
        } else {
            bail!("unsupported file type in scripts/user: {name}");
        }
    }
    Ok(())
}

fn remove_ignored_artifacts(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(&path)?;
        if name == "__pycache__" {
            if metadata.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        } else if name.ends_with(".pyc") || name.ends_with('~') {
            if metadata.is_file() {
                fs::remove_file(path)?;
            }
        } else if metadata.is_dir() {
            remove_ignored_artifacts(&path)?;
        }
    }
    Ok(())
}

fn copy_template_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "workspace template path is not a directory: {}",
            source.display()
        );
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if is_template_test_file(&file_name) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(file_name);
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "workspace template contains a symlink: {}",
                source_path.display()
            );
        }
        if metadata.is_dir() {
            copy_template_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(source_path, destination_path)?;
        } else {
            bail!("workspace template contains unsupported file type");
        }
    }
    Ok(())
}

fn is_template_test_file(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with("test_") && name.ends_with(".py")
}

fn collect_files(
    root: &Path,
    current: &Path,
    manifest: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "__pycache__" || name.ends_with(".pyc") || name.ends_with('~') {
            continue;
        }
        if metadata.file_type().is_symlink() {
            bail!("symlinks are not allowed in candidate files: {name}");
        }
        if metadata.is_dir() {
            collect_files(root, &path, manifest, total)?;
            continue;
        }
        if !metadata.is_file() {
            bail!("unsupported candidate file type: {name}");
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "py" | "json" | "md") {
            bail!("candidate file extension is not allowed: {name}");
        }
        if metadata.len() > MAX_FILE_BYTES || manifest.len() >= MAX_TREE_FILES {
            bail!("candidate tree exceeds file limits");
        }
        *total = total.saturating_add(metadata.len());
        if *total > MAX_TREE_BYTES {
            bail!("candidate tree exceeds total size limit");
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        validate_relative_path(&relative)?;
        let mut file = fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        manifest.insert(relative, format!("{:x}", hasher.finalize()));
    }
    Ok(())
}

fn validate_agent_key(agent_key: &str) -> Result<()> {
    if agent_key.is_empty()
        || agent_key == "."
        || agent_key == ".."
        || agent_key.contains('/')
        || agent_key.contains('\\')
    {
        bail!("invalid agent key");
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("candidate path escapes scripts/user: {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config(root: PathBuf) -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: root.clone(),
            host_workspaces_root: root,
            container_workspaces_root: "/workspaces".into(),
            api_base_url: String::new(),
        }
    }

    #[test]
    fn manifests_are_deterministic_and_report_changes() {
        let root = std::env::temp_dir().join(format!(
            "coding-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("scripts/user")).unwrap();
        fs::write(root.join("scripts/user/a.py"), b"one").unwrap();
        let first = manifest_tree(&root.join("scripts/user")).unwrap();
        fs::write(root.join("scripts/user/a.py"), b"two").unwrap();
        let second = manifest_tree(&root.join("scripts/user")).unwrap();
        assert_eq!(changed_paths(&first, &second), vec!["a.py"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_rejects_unapproved_file_extensions() {
        let root = std::env::temp_dir().join(format!(
            "coding-extension-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), b"not allowed").unwrap();

        assert!(manifest_tree(&root).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidate_paths_reject_traversal() {
        let result = candidate_root(&config(PathBuf::from("/tmp")), "../bad", 1);
        assert!(result.is_err());
    }

    #[test]
    fn candidate_template_copy_excludes_test_files() {
        let root = std::env::temp_dir().join(format!(
            "coding-template-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), b"skill").unwrap();
        fs::write(source.join("test_skill.py"), b"test").unwrap();

        copy_template_tree(&source, &destination).unwrap();

        assert!(destination.join("SKILL.md").exists());
        assert!(!destination.join("test_skill.py").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidate_profile_allows_only_its_scoped_user_tree() {
        let root = std::env::temp_dir().join(format!(
            "coding-permissions-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile_path = root.join(CODING_AGENT_PATH);
        fs::create_dir_all(profile_path.parent().unwrap()).unwrap();
        fs::write(
            &profile_path,
            include_str!(
                "../../agent-runtime/workspace-template/.opencode/agents/analysis-coding.md"
            ),
        )
        .unwrap();

        add_candidate_permission_scope(&root, "/workspaces/coding/btc-1/18/workspace")
            .unwrap();

        let profile = fs::read_to_string(profile_path).unwrap();
        assert_eq!(
            profile
                .matches("\"workspaces/coding/btc-1/18/workspace/scripts/user\": allow")
                .count(),
            3
        );
        assert_eq!(
            profile
                .matches("\"workspaces/coding/btc-1/18/workspace/scripts/user/**\": allow")
                .count(),
            3
        );
        assert!(!profile.contains("workspaces/coding/btc-1/17"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn promotion_journal_round_trips_atomically() {
        let root = std::env::temp_dir().join(format!(
            "coding-journal-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let config = config(root.clone());
        let journal = PromotionJournal {
            task_id: 7,
            agent_key: "agent".into(),
            base_manifest_hash: "base".into(),
            candidate_manifest_hash: "candidate".into(),
            live_path: PathBuf::from("live"),
            candidate_path: PathBuf::from("candidate"),
            backup_path: PathBuf::from("backup"),
            retained_version_path: None,
            phase: PromotionJournalPhase::Prepared,
            updated_at: Utc::now(),
        };
        write_promotion_journal(&config, &journal).unwrap();
        assert_eq!(
            read_promotion_journal(&config, "agent", 7).unwrap(),
            Some(journal)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_restores_backup_after_candidate_swap_crash() {
        let root = std::env::temp_dir().join(format!(
            "coding-recovery-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = config(root.clone());
        let live = root.join("live");
        let candidate = root.join("candidate");
        let backup = root.join("backup");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        fs::write(live.join("old.py"), b"old").unwrap();
        fs::write(candidate.join("new.py"), b"new").unwrap();
        fs::rename(&live, &backup).unwrap();
        fs::rename(&candidate, &live).unwrap();
        let journal = PromotionJournal {
            task_id: 9,
            agent_key: "agent".into(),
            base_manifest_hash: "base".into(),
            candidate_manifest_hash: "candidate".into(),
            live_path: live.clone(),
            candidate_path: candidate,
            backup_path: backup,
            retained_version_path: None,
            phase: PromotionJournalPhase::CandidatePromoted,
            updated_at: Utc::now(),
        };
        write_promotion_journal(&config, &journal).unwrap();
        assert_eq!(
            recover_promotion_journal(&config, &journal).unwrap(),
            PromotionJournalPhase::RolledBack
        );
        assert!(live.join("old.py").exists());
        assert!(!live.join("new.py").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn promotion_rejects_live_manifest_drift() {
        let root = std::env::temp_dir().join(format!(
            "coding-drift-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = config(root.clone());
        let live = live_user_root(&config, "agent").unwrap();
        let candidate = candidate_root(&config, "agent", 11)
            .unwrap()
            .join("scripts/user");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        fs::write(live.join("analyze.py"), b"changed-after-snapshot").unwrap();
        fs::write(candidate.join("analyze.py"), b"candidate").unwrap();
        let expected = BTreeMap::from([("analyze.py".to_string(), "stale".to_string())]);
        assert!(promote_user_tree(&config, "agent", 11, &expected).is_err());
        assert_eq!(
            fs::read(live.join("analyze.py")).unwrap(),
            b"changed-after-snapshot"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn promotion_removes_ignored_python_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "coding-clean-promotion-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = config(root.clone());
        let live = live_user_root(&config, "agent").unwrap();
        let candidate = candidate_root(&config, "agent", 12)
            .unwrap()
            .join("scripts/user");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("analyze.py"), b"old").unwrap();
        fs::create_dir_all(candidate.join("analysis/__pycache__")).unwrap();
        fs::write(candidate.join("analyze.py"), b"new").unwrap();
        fs::write(
            candidate.join("analysis/__pycache__/module.cpython-313.pyc"),
            b"cache",
        )
        .unwrap();
        let expected = manifest_tree(&live).unwrap();

        promote_user_tree(&config, "agent", 12, &expected).unwrap();

        assert_eq!(fs::read(live.join("analyze.py")).unwrap(), b"new");
        assert!(!live.join("analysis/__pycache__").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retention_keeps_only_five_versions() {
        let root = std::env::temp_dir().join(format!(
            "coding-retention-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = config(root.clone());
        let versions = root.join("versions/agent");
        fs::create_dir_all(&versions).unwrap();
        for id in 1..=5 {
            fs::create_dir_all(versions.join(id.to_string()).join("user")).unwrap();
        }
        let task_root = candidate_root(&config, "agent", 6).unwrap();
        fs::create_dir_all(task_root.parent().unwrap()).unwrap();
        fs::create_dir_all(task_root.parent().unwrap().join("backup-user")).unwrap();
        fs::write(
            task_root.parent().unwrap().join("backup-user/analyze.py"),
            b"old",
        )
        .unwrap();
        retain_successful_version(&config, "agent", 6).unwrap();
        let remaining: Vec<_> = fs::read_dir(&versions).unwrap().collect();
        assert_eq!(remaining.len(), 5);
        assert!(!versions.join("1").exists());
        assert!(versions.join("6/user/analyze.py").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
