use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use rustix::{
    fs::{Mode, OFlags, open, openat},
    io::Errno,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    coding_workspace::{copy_user_tree, live_user_root, manifest_hash, manifest_tree},
    isolated_workspace::{RunWorkspacePath, create_run_workspace},
};

pub const PROFILE_SOURCE_RELATIVE_PATH: &str = "agent-runtime/workspace-template";
pub const WORKSPACE_BROWSER_MAX_DEPTH: usize = 32;
pub const WORKSPACE_BROWSER_MAX_ENTRIES: usize = 2_000;
pub const WORKSPACE_BROWSER_MAX_FILENAME_BYTES: usize = 255;
pub const WORKSPACE_BROWSER_MAX_PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBrowserListing {
    pub workspace_exists: bool,
    pub entries: Vec<WorkspaceBrowserEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBrowserEntry {
    pub path: String,
    pub kind: WorkspaceBrowserEntryKind,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceBrowserEntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFilePreview {
    pub path: String,
    pub size_bytes: u64,
    pub status: WorkspaceFilePreviewStatus,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFilePreviewStatus {
    Text,
    Binary,
    TooLarge,
    Missing,
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceConfig {
    pub source_root: PathBuf,
    pub host_workspaces_root: PathBuf,
    pub container_workspaces_root: String,
    pub api_base_url: String,
}

#[derive(Debug, Clone)]
pub struct OpenCodeWorkspaceAgent {
    pub agent_key: String,
    pub display_name: String,
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct GeneratedOpenCodeWorkspace {
    pub workspace_host_path: PathBuf,
    pub workspace_container_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceGenerationMode {
    CreateNew,
    Regenerate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenCodeWorkspaceRuntimeConfig {
    pub workspace_container_path: String,
    pub profile_source: String,
}

/// Immutable identity of the quantitative package copied into a run workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantitativePackageSnapshot {
    pub version: String,
    pub manifest_hash: String,
}

const ANALYSIS_TOOL_MANIFEST: &str = "manifest.json";
const LEGACY_ANALYSIS_TOOL_MANIFEST: &str = r#"{
  "schema_version": 1,
  "package_version": "legacy-analyze.py",
  "tools": [
    {
      "id": "analyze",
      "description": "Canonical quantitative OHLCV analysis",
      "entrypoint": "analyze.py",
      "input_kind": "ohlcv",
      "supported_timeframes": ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "8h", "12h", "1d", "3d", "1w", "1M"],
      "minimum_candles": 1,
      "required_arguments": ["symbol", "timeframe", "boundary_ms", "input", "output"],
      "output_schema": "hypervibes.quantitative.v1",
      "version": "1"
    }
  ]
}
"#;

/// Controller input for rendering a scheduled run workspace. The runtime API
/// key is deliberately absent from every response and idempotency fingerprint.
#[derive(Clone, Serialize, Deserialize)]
pub struct RunWorkspaceMaterializationInput {
    pub display_name: String,
    pub api_base_url: String,
    pub runtime_api_key: String,
    pub credential_id: String,
    pub sub_agent_kind: String,
    pub enabled_capabilities: Vec<String>,
    pub expected_quantitative_package: Option<QuantitativePackageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedRunWorkspace {
    pub workspace_container_path: String,
    pub quantitative_package: Option<QuantitativePackageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTemplateDrift {
    pub workspace_exists: bool,
    pub changed_files: Vec<WorkspaceTemplateFileChange>,
}

impl WorkspaceTemplateDrift {
    pub fn is_in_sync(&self) -> bool {
        self.workspace_exists && self.changed_files.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTemplateFileChange {
    pub path: String,
    pub status: WorkspaceTemplateFileStatus,
    pub added_lines: usize,
    pub removed_lines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTemplateFileStatus {
    Modified,
    Deleted,
}

impl OpenCodeWorkspaceRuntimeConfig {
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    pub fn into_value(self) -> Value {
        serde_json::to_value(self).expect("OpenCodeWorkspaceRuntimeConfig always serializes")
    }
}

pub fn generate_agent_workspace(
    config: &OpenCodeWorkspaceConfig,
    agent: &OpenCodeWorkspaceAgent,
    mode: WorkspaceGenerationMode,
) -> Result<GeneratedOpenCodeWorkspace> {
    let workspace_host_path = agent_workspace_host_path(config, &agent.agent_key)?;
    let workspace_container_path = join_container_path(
        &config.container_workspaces_root,
        &["agents", agent.agent_key.as_str()],
    );

    if matches!(mode, WorkspaceGenerationMode::CreateNew) && workspace_host_path.exists() {
        bail!(
            "workspace already exists for agent {}; remove it before reusing this agent key",
            agent.agent_key
        );
    }

    for path in [
        workspace_host_path.as_path(),
        &workspace_host_path.join(".opencode/commands"),
        &workspace_host_path.join(".opencode/agents"),
        &workspace_host_path.join(".opencode/skills"),
        &workspace_host_path.join("scripts/user"),
        &workspace_host_path.join("data"),
        &workspace_host_path.join("scratch"),
    ] {
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create workspace directory {}", path.display()))?;
    }

    let replacements = template_replacements(config, agent, &workspace_container_path);

    write_rendered_template(
        &config.source_root.join("opencode.json.template"),
        &workspace_host_path.join("opencode.json"),
        &replacements,
    )?;
    write_rendered_template(
        &config.source_root.join("AGENTS.md.template"),
        &workspace_host_path.join("AGENTS.md"),
        &replacements,
    )?;

    copy_tree(
        &config.source_root.join(".opencode/commands"),
        &workspace_host_path.join(".opencode/commands"),
    )?;
    copy_rendered_tree(
        &config.source_root.join(".opencode/agents"),
        &workspace_host_path.join(".opencode/agents"),
        &replacements,
    )?;
    copy_tree(
        &config.source_root.join(".opencode/skills"),
        &workspace_host_path.join(".opencode/skills"),
    )?;

    fs::write(
        workspace_host_path.join(".env"),
        format!(
            "HYPERVIBES_AGENT_KEY={}\nHYPERVIBES_API_BASE_URL={}\nHYPERVIBES_API_KEY={}\nHYPERVIBES_WORKSPACE={}\n",
            agent.agent_key, config.api_base_url, agent.api_key, workspace_container_path,
        ),
    )
    .context("failed to write workspace .env")?;

    Ok(GeneratedOpenCodeWorkspace {
        workspace_host_path,
        workspace_container_path,
    })
}

/// Inspect the active quantitative package before a scheduler binds it into a
/// run snapshot. The controller derives the source location from the validated
/// agent key; callers never provide a filesystem path.
pub fn inspect_active_quantitative_package(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
) -> Result<Option<QuantitativePackageSnapshot>> {
    let workspace_root = agent_workspace_host_path(config, agent_key)?;
    let scripts_root = workspace_root.join("scripts");
    let user_root = live_user_root(config, agent_key)?;

    match fs::symlink_metadata(&workspace_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("active workspace root is not a regular directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect active workspace root"),
    }
    match fs::symlink_metadata(&scripts_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("active scripts root is not a regular directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect active scripts root"),
    }
    match fs::symlink_metadata(&user_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("active scripts/user is not a regular directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect active scripts/user root"),
    }

    ensure_analysis_tool_manifest(&user_root)?;
    let manifest = manifest_tree(&user_root)?;
    if manifest.is_empty() {
        return Ok(None);
    }
    Ok(Some(QuantitativePackageSnapshot {
        version: analysis_package_version(&user_root)?,
        manifest_hash: manifest_hash(&manifest),
    }))
}

/// Registers the pre-manifest canonical analyzer without changing its CLI. New
/// coding candidates must maintain this manifest explicitly.
fn ensure_analysis_tool_manifest(user_root: &Path) -> Result<()> {
    let analyzer = user_root.join("analyze.py");
    let manifest = user_root.join(ANALYSIS_TOOL_MANIFEST);
    if !analyzer.is_file() || manifest.exists() {
        return Ok(());
    }
    fs::write(&manifest, LEGACY_ANALYSIS_TOOL_MANIFEST)
        .context("failed to create legacy analysis-tool manifest")
}

fn analysis_package_version(user_root: &Path) -> Result<String> {
    let manifest_path = user_root.join(ANALYSIS_TOOL_MANIFEST);
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).context("failed to read analysis-tool manifest")?,
    )
    .context("analysis-tool manifest is invalid JSON")?;
    manifest
        .get("package_version")
        .and_then(Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .map(ToString::to_string)
        .context("analysis-tool manifest has no package_version")
}

/// Render a complete run-local workspace from trusted templates and the active
/// quantitative package. This must run before the OpenCode session is created.
pub fn materialize_run_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
    input: &RunWorkspaceMaterializationInput,
) -> Result<MaterializedRunWorkspace> {
    validate_run_workspace_materialization_input(input)?;

    let active_package = inspect_active_quantitative_package(config, path.agent_key())?;
    if active_package != input.expected_quantitative_package {
        bail!("active quantitative package does not match the bound run snapshot");
    }

    let created = create_run_workspace(config, path)?;
    let workspace_root = path.workspace_host_path(config);
    ensure_regular_directory(&workspace_root, "run workspace root")?;

    for relative in [
        ".opencode/commands",
        ".opencode/agents",
        ".opencode/skills",
        "scripts",
        "scratch/ohlcv",
        "scratch/analysis-output",
        "scratch/downloads",
        "scratch/tmp",
        "scratch/trading-confirmation",
    ] {
        fs::create_dir_all(workspace_root.join(relative))
            .with_context(|| format!("failed to create run workspace directory {relative}"))?;
    }

    let template_agent = OpenCodeWorkspaceAgent {
        agent_key: path.agent_key().to_string(),
        display_name: input.display_name.clone(),
        // Template rendering currently has no API-key placeholder. Keep this
        // empty so the permanent agent key can never reach a run workspace.
        api_key: String::new(),
    };
    let mut template_config = config.clone();
    template_config.api_base_url = input.api_base_url.clone();
    let replacements = template_replacements(
        &template_config,
        &template_agent,
        &created.workspace_container_path,
    );
    write_rendered_template(
        &template_config.source_root.join("opencode.json.template"),
        &workspace_root.join("opencode.json"),
        &replacements,
    )?;
    write_rendered_template(
        &template_config.source_root.join("AGENTS.md.template"),
        &workspace_root.join("AGENTS.md"),
        &replacements,
    )?;
    copy_tree(
        &template_config.source_root.join(".opencode/commands"),
        &workspace_root.join(".opencode/commands"),
    )?;
    copy_rendered_tree(
        &template_config.source_root.join(".opencode/agents"),
        &workspace_root.join(".opencode/agents"),
        &replacements,
    )?;
    copy_tree(
        &template_config.source_root.join(".opencode/skills"),
        &workspace_root.join(".opencode/skills"),
    )?;

    render_run_capability_permissions(
        &workspace_root,
        &input.sub_agent_kind,
        &input.enabled_capabilities,
    )?;

    let destination_user_root = workspace_root.join("scripts/user");
    replace_run_user_tree(&destination_user_root)?;
    let source_user_root = live_user_root(config, path.agent_key())?;
    if active_package.is_some() {
        copy_user_tree(&source_user_root, &destination_user_root)?;
    }

    write_run_runtime_environment(
        &workspace_root,
        &created.workspace_container_path,
        path,
        input,
    )?;

    Ok(MaterializedRunWorkspace {
        workspace_container_path: created.workspace_container_path,
        quantitative_package: active_package,
    })
}

fn validate_run_workspace_materialization_input(
    input: &RunWorkspaceMaterializationInput,
) -> Result<()> {
    for (name, value) in [
        ("api_base_url", input.api_base_url.as_str()),
        ("runtime_api_key", input.runtime_api_key.as_str()),
        ("credential_id", input.credential_id.as_str()),
        ("sub_agent_kind", input.sub_agent_kind.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            bail!("{name} is invalid");
        }
    }
    uuid::Uuid::parse_str(&input.credential_id).context("credential_id is invalid")?;
    if !matches!(
        input.sub_agent_kind.as_str(),
        "analysis" | "market_analysis" | "trading" | "daily_review"
    ) {
        bail!("sub_agent_kind is not supported for a run workspace");
    }
    Ok(())
}

fn ensure_regular_directory(path: &Path, description: &str) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {description}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{description} is not a regular directory");
    }
    Ok(())
}

fn replace_run_user_tree(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("run scripts/user is not a regular directory")
        }
        Ok(_) => fs::remove_dir_all(destination).context("failed to reset run scripts/user")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("failed to inspect run scripts/user"),
    }
    fs::create_dir_all(destination).context("failed to create run scripts/user")
}

fn render_run_capability_permissions(
    workspace_root: &Path,
    sub_agent_kind: &str,
    enabled_capabilities: &[String],
) -> Result<()> {
    if enabled_capabilities.iter().any(|capability| {
        capability != "hypervibes:notification_send" && !capability.starts_with("custom-mcp:")
    }) {
        bail!("run workspace has an unsupported capability");
    }
    let profile_name = match sub_agent_kind {
        "analysis" => "analysis",
        "market_analysis" => "market-analysis",
        "trading" => "trading",
        "daily_review" => "daily-review",
        _ => bail!("unsupported run profile"),
    };
    let profile_path = workspace_root
        .join(".opencode/agents")
        .join(format!("{profile_name}.md"));
    let profile = fs::read_to_string(&profile_path)
        .with_context(|| format!("failed to read run profile {}", profile_path.display()))?;
    let mut replaced = 0;
    let action = if enabled_capabilities
        .iter()
        .any(|capability| capability == "hypervibes:notification_send")
    {
        "allow"
    } else {
        "deny"
    };
    let rendered = profile
        .lines()
        .map(|line| {
            if line
                .trim_start()
                .starts_with("hypervibes_send_notification:")
            {
                replaced += 1;
                format!("  hypervibes_send_notification: {action}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if replaced != 1 {
        bail!("run profile must contain exactly one notification permission");
    }
    fs::write(&profile_path, format!("{rendered}\n"))
        .with_context(|| format!("failed to write run profile {}", profile_path.display()))
}

fn write_run_runtime_environment(
    workspace_root: &Path,
    workspace_container_path: &str,
    path: &RunWorkspacePath,
    input: &RunWorkspaceMaterializationInput,
) -> Result<()> {
    let scratch = format!("{workspace_container_path}/scratch");
    let contents = format!(
        "HYPERVIBES_AGENT_KEY={}\nHYPERVIBES_API_BASE_URL={}\nHYPERVIBES_API_KEY={}\nHYPERVIBES_WORKSPACE={}\nHYPERVIBES_RUN_ID={}\nHYPERVIBES_RUNTIME_CREDENTIAL_ID={}\nPYTHONDONTWRITEBYTECODE=1\nHOME={}\nTMPDIR={}/tmp\nMPLCONFIGDIR={}/matplotlib\n",
        path.agent_key(),
        input.api_base_url,
        input.runtime_api_key,
        workspace_container_path,
        path.run_id(),
        input.credential_id,
        scratch,
        scratch,
        scratch,
    );
    let temporary = workspace_root.join(".env.run.tmp");
    fs::write(&temporary, contents).context("failed to write run runtime environment")?;
    fs::rename(&temporary, workspace_root.join(".env"))
        .context("failed to publish run runtime environment")
}

pub fn diff_agent_workspace_from_template(
    config: &OpenCodeWorkspaceConfig,
    agent: &OpenCodeWorkspaceAgent,
) -> Result<WorkspaceTemplateDrift> {
    let workspace_host_path = agent_workspace_host_path(config, &agent.agent_key)?;
    if !workspace_host_path.is_dir() {
        return Ok(WorkspaceTemplateDrift {
            workspace_exists: false,
            changed_files: Vec::new(),
        });
    }

    let expected_files = expected_workspace_template_files(config, agent)?;
    let mut changed_files = Vec::new();

    for expected_file in expected_files {
        let actual_path = workspace_host_path.join(&expected_file.relative_path);
        let actual_contents = match fs::read(&actual_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                changed_files.push(WorkspaceTemplateFileChange {
                    path: display_relative_path(&expected_file.relative_path),
                    status: WorkspaceTemplateFileStatus::Deleted,
                    added_lines: count_lines(&expected_file.expected_contents),
                    removed_lines: 0,
                });
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read workspace file {}", actual_path.display())
                });
            }
        };

        if actual_contents == expected_file.expected_contents {
            continue;
        }

        let (removed_lines, added_lines) = line_change_counts(
            &String::from_utf8_lossy(&actual_contents),
            &String::from_utf8_lossy(&expected_file.expected_contents),
        );
        changed_files.push(WorkspaceTemplateFileChange {
            path: display_relative_path(&expected_file.relative_path),
            status: WorkspaceTemplateFileStatus::Modified,
            added_lines,
            removed_lines,
        });
    }

    Ok(WorkspaceTemplateDrift {
        workspace_exists: true,
        changed_files,
    })
}

pub fn agent_workspace_host_path(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
) -> Result<PathBuf> {
    validate_agent_key(agent_key)?;
    Ok(config.host_workspaces_root.join("agents").join(agent_key))
}

pub fn delete_agent_workspace(config: &OpenCodeWorkspaceConfig, agent_key: &str) -> Result<bool> {
    let workspace_host_path = agent_workspace_host_path(config, agent_key)?;
    if !workspace_host_path.exists() {
        return Ok(false);
    }

    let metadata = fs::metadata(&workspace_host_path)
        .with_context(|| format!("failed to stat workspace {}", workspace_host_path.display()))?;
    if !metadata.is_dir() {
        bail!(
            "workspace path {} exists but is not a directory",
            workspace_host_path.display()
        );
    }

    let canonical_root = config
        .host_workspaces_root
        .canonicalize()
        .with_context(|| {
            format!(
                "failed to canonicalize workspace root {}",
                config.host_workspaces_root.display()
            )
        })?;
    let canonical_candidate = workspace_host_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize workspace path {}",
            workspace_host_path.display()
        )
    })?;
    let canonical_agents_root = canonical_root.join("agents");
    if !canonical_candidate.starts_with(&canonical_agents_root) {
        bail!(
            "refusing to delete workspace outside managed root: {}",
            canonical_candidate.display()
        );
    }

    fs::remove_dir_all(&workspace_host_path).with_context(|| {
        format!(
            "failed to delete workspace {}",
            workspace_host_path.display()
        )
    })?;
    Ok(true)
}

pub fn list_workspace_browser_entries(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
) -> Result<WorkspaceBrowserListing> {
    let workspace_root = agent_workspace_host_path(config, agent_key)?;
    let root = match open_workspace_browser_root(&workspace_root) {
        Ok(root) => root,
        Err(Errno::NOENT) => {
            return Ok(WorkspaceBrowserListing {
                workspace_exists: false,
                entries: Vec::new(),
                truncated: false,
            });
        }
        Err(Errno::LOOP) => bail!("workspace browser root is a symlink"),
        Err(error) => return Err(error.into()),
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    list_workspace_browser_directory(&root, "", 0, &mut entries, &mut truncated)?;
    Ok(WorkspaceBrowserListing {
        workspace_exists: true,
        entries,
        truncated,
    })
}

pub fn read_workspace_browser_file(
    config: &OpenCodeWorkspaceConfig,
    agent_key: &str,
    relative_path: &str,
) -> Result<WorkspaceFilePreview> {
    let components = workspace_browser_path_components(relative_path)?;
    let workspace_root = agent_workspace_host_path(config, agent_key)?;
    let mut directory = match open_workspace_browser_root(&workspace_root) {
        Ok(root) => root,
        Err(Errno::NOENT) => return Ok(missing_workspace_file_preview(relative_path)),
        Err(Errno::LOOP) => bail!("workspace browser root is a symlink"),
        Err(error) => return Err(error.into()),
    };

    for component in &components[..components.len() - 1] {
        directory = match openat(
            &directory,
            *component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(directory) => fs::File::from(directory),
            Err(Errno::NOENT) => return Ok(missing_workspace_file_preview(relative_path)),
            Err(Errno::LOOP) => bail!("workspace browser path component is a symlink"),
            Err(error) => return Err(error.into()),
        };
        if !directory.metadata()?.is_dir() {
            bail!("workspace browser path component is not a directory");
        }
    }

    let file = match openat(
        &directory,
        *components
            .last()
            .expect("validated path has a final component"),
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => fs::File::from(file),
        Err(Errno::NOENT) => return Ok(missing_workspace_file_preview(relative_path)),
        Err(Errno::LOOP) => bail!("workspace browser path is a symlink"),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("workspace browser path is not a regular file");
    }
    let size_bytes = metadata.len();
    if size_bytes > WORKSPACE_BROWSER_MAX_PREVIEW_BYTES {
        return Ok(WorkspaceFilePreview {
            path: relative_path.to_owned(),
            size_bytes,
            status: WorkspaceFilePreviewStatus::TooLarge,
            text: None,
        });
    }

    let mut contents = Vec::with_capacity(size_bytes as usize + 1);
    file.take(WORKSPACE_BROWSER_MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut contents)?;
    if contents.len() as u64 > WORKSPACE_BROWSER_MAX_PREVIEW_BYTES {
        return Ok(WorkspaceFilePreview {
            path: relative_path.to_owned(),
            size_bytes: contents.len() as u64,
            status: WorkspaceFilePreviewStatus::TooLarge,
            text: None,
        });
    }
    if contents.contains(&0) {
        return Ok(WorkspaceFilePreview {
            path: relative_path.to_owned(),
            size_bytes: contents.len() as u64,
            status: WorkspaceFilePreviewStatus::Binary,
            text: None,
        });
    }
    match String::from_utf8(contents) {
        Ok(text) => Ok(WorkspaceFilePreview {
            path: relative_path.to_owned(),
            size_bytes: text.len() as u64,
            status: WorkspaceFilePreviewStatus::Text,
            text: Some(text),
        }),
        Err(error) => Ok(WorkspaceFilePreview {
            path: relative_path.to_owned(),
            size_bytes: error.into_bytes().len() as u64,
            status: WorkspaceFilePreviewStatus::Binary,
            text: None,
        }),
    }
}

pub fn runtime_config_for_generated_workspace(
    generated: &GeneratedOpenCodeWorkspace,
) -> OpenCodeWorkspaceRuntimeConfig {
    OpenCodeWorkspaceRuntimeConfig {
        workspace_container_path: generated.workspace_container_path.clone(),
        profile_source: PROFILE_SOURCE_RELATIVE_PATH.to_string(),
    }
}

pub(crate) fn validate_agent_key(agent_key: &str) -> Result<()> {
    if agent_key.trim().is_empty() {
        bail!("agent_key must not be empty");
    }
    if matches!(agent_key, "." | "..")
        || agent_key.contains("..")
        || agent_key.contains('/')
        || agent_key.contains('\\')
    {
        bail!("agent_key contains unsafe path characters");
    }
    Ok(())
}

fn open_workspace_browser_root(root: &Path) -> rustix::io::Result<fs::File> {
    open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(fs::File::from)
}

fn list_workspace_browser_directory(
    directory: &fs::File,
    prefix: &str,
    depth: usize,
    entries: &mut Vec<WorkspaceBrowserEntry>,
    truncated: &mut bool,
) -> Result<()> {
    // Reading through this descriptor path keeps enumeration anchored to the
    // already opened directory even if its name is concurrently renamed.
    let descriptor_path = format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(directory)
    );
    let mut candidates = Vec::new();
    for entry in fs::read_dir(descriptor_path)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_workspace_browser_name_allowed(&name) {
            continue;
        }
        let probe = match openat(
            directory,
            &name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(probe) => fs::File::from(probe),
            Err(Errno::NOENT) => continue,
            Err(error) => return Err(error.into()),
        };
        let metadata = probe.metadata()?;
        let kind = if metadata.is_dir() {
            WorkspaceBrowserEntryKind::Directory
        } else if metadata.is_file() {
            WorkspaceBrowserEntryKind::File
        } else {
            continue;
        };
        let opened = if kind == WorkspaceBrowserEntryKind::Directory {
            match openat(
                directory,
                &name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(opened) => Some(fs::File::from(opened)),
                Err(Errno::NOENT) | Err(Errno::LOOP) => continue,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        candidates.push((name, kind, metadata.len(), opened));
    }
    candidates.sort_by(|left, right| {
        let left_kind = matches!(left.1, WorkspaceBrowserEntryKind::File);
        let right_kind = matches!(right.1, WorkspaceBrowserEntryKind::File);
        left_kind
            .cmp(&right_kind)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });

    for (name, kind, size_bytes, opened) in candidates {
        if entries.len() == WORKSPACE_BROWSER_MAX_ENTRIES {
            *truncated = true;
            return Ok(());
        }
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        entries.push(WorkspaceBrowserEntry {
            path: path.clone(),
            kind,
            size_bytes: (kind == WorkspaceBrowserEntryKind::File).then_some(size_bytes),
        });
        if kind == WorkspaceBrowserEntryKind::Directory {
            if depth == WORKSPACE_BROWSER_MAX_DEPTH {
                *truncated = true;
            } else {
                list_workspace_browser_directory(
                    opened
                        .as_ref()
                        .expect("directories have an open descriptor"),
                    &path,
                    depth + 1,
                    entries,
                    truncated,
                )?;
                if *truncated {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn workspace_browser_path_components(relative_path: &str) -> Result<Vec<&str>> {
    if relative_path.is_empty()
        || Path::new(relative_path).is_absolute()
        || relative_path.contains('\\')
    {
        bail!("invalid workspace browser path");
    }
    let components: Vec<_> = relative_path.split('/').collect();
    if components
        .iter()
        .any(|component| !is_workspace_browser_name_allowed(component))
    {
        bail!("invalid workspace browser path");
    }
    Ok(components)
}

fn is_workspace_browser_name_allowed(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name != ".env"
        && !matches!(name, ".node_modules" | "node_modules")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && name.len() <= WORKSPACE_BROWSER_MAX_FILENAME_BYTES
}

fn missing_workspace_file_preview(path: &str) -> WorkspaceFilePreview {
    WorkspaceFilePreview {
        path: path.to_owned(),
        size_bytes: 0,
        status: WorkspaceFilePreviewStatus::Missing,
        text: None,
    }
}

fn join_container_path(root: &str, segments: &[&str]) -> String {
    let mut path = root.trim_end_matches('/').to_string();
    for segment in segments {
        path.push('/');
        path.push_str(segment);
    }
    path
}

struct ExpectedWorkspaceFile {
    relative_path: PathBuf,
    expected_contents: Vec<u8>,
}

fn expected_workspace_template_files(
    config: &OpenCodeWorkspaceConfig,
    agent: &OpenCodeWorkspaceAgent,
) -> Result<Vec<ExpectedWorkspaceFile>> {
    let workspace_container_path = join_container_path(
        &config.container_workspaces_root,
        &["agents", agent.agent_key.as_str()],
    );
    let replacements = template_replacements(config, agent, &workspace_container_path);
    let mut files = vec![
        ExpectedWorkspaceFile {
            relative_path: PathBuf::from("opencode.json"),
            expected_contents: render_template_file(
                &config.source_root.join("opencode.json.template"),
                &replacements,
            )?
            .into_bytes(),
        },
        ExpectedWorkspaceFile {
            relative_path: PathBuf::from("AGENTS.md"),
            expected_contents: render_template_file(
                &config.source_root.join("AGENTS.md.template"),
                &replacements,
            )?
            .into_bytes(),
        },
    ];

    for relative_root in [
        Path::new(".opencode/commands"),
        Path::new(".opencode/agents"),
        Path::new(".opencode/skills"),
    ] {
        collect_expected_workspace_files(
            &config.source_root,
            relative_root,
            &replacements,
            &mut files,
        )?;
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_expected_workspace_files(
    source_root: &Path,
    relative_root: &Path,
    replacements: &BTreeMap<&str, String>,
    files: &mut Vec<ExpectedWorkspaceFile>,
) -> Result<()> {
    let source_path = source_root.join(relative_root);
    let entries = fs::read_dir(&source_path)
        .with_context(|| format!("failed to read directory {}", source_path.display()))?;

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source_path.display()))?;
        let file_name = entry.file_name();
        if is_workspace_excluded_artifact(&file_name) {
            continue;
        }
        let relative_path = relative_root.join(&file_name);
        let entry_path = source_root.join(&relative_path);
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", entry_path.display()))?;
        if metadata.is_dir() {
            collect_expected_workspace_files(source_root, &relative_path, replacements, files)?;
            continue;
        }
        if !metadata.is_file() || file_name == ".gitkeep" {
            continue;
        }
        files.push(ExpectedWorkspaceFile {
            relative_path,
            expected_contents: if relative_root.starts_with(Path::new(".opencode/agents")) {
                render_template_file(&entry_path, replacements)?.into_bytes()
            } else {
                fs::read(&entry_path).with_context(|| {
                    format!("failed to read template file {}", entry_path.display())
                })?
            },
        });
    }

    Ok(())
}

fn template_replacements(
    config: &OpenCodeWorkspaceConfig,
    agent: &OpenCodeWorkspaceAgent,
    workspace_container_path: &str,
) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("agent_key", agent.agent_key.clone()),
        ("display_name", agent.display_name.clone()),
        ("api_base_url", config.api_base_url.clone()),
        (
            "workspace_container_path",
            workspace_container_path.to_string(),
        ),
        (
            "workspace_permission_root",
            workspace_container_path.trim_start_matches('/').to_string(),
        ),
    ])
}

fn write_rendered_template(
    source_path: &Path,
    destination_path: &Path,
    replacements: &BTreeMap<&str, String>,
) -> Result<()> {
    let rendered = render_template_file(source_path, replacements)?;
    fs::write(destination_path, rendered)
        .with_context(|| format!("failed to write {}", destination_path.display()))?;
    Ok(())
}

fn render_template_file(
    source_path: &Path,
    replacements: &BTreeMap<&str, String>,
) -> Result<String> {
    let template = fs::read_to_string(source_path)
        .with_context(|| format!("failed to read template {}", source_path.display()))?;
    render_template(&template, replacements)
        .with_context(|| format!("failed to render template {}", source_path.display()))
}

fn render_template(template: &str, replacements: &BTreeMap<&str, String>) -> Result<String> {
    let mut output = String::with_capacity(template.len());
    let mut remainder = template;

    while let Some(start) = remainder.find("{{") {
        output.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("}}") else {
            bail!("unterminated template placeholder");
        };
        let key = after_start[..end].trim();
        let value = replacements
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("unknown template placeholder: {key}"))?;
        output.push_str(value);
        remainder = &after_start[end + 2..];
    }

    if remainder.contains("}}") {
        bail!("template contains a closing placeholder without an opening marker");
    }

    output.push_str(remainder);
    Ok(output)
}

fn copy_tree(source_root: &Path, destination_root: &Path) -> Result<()> {
    let entries = fs::read_dir(source_root)
        .with_context(|| format!("failed to read directory {}", source_root.display()))?;

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source_root.display()))?;
        if is_workspace_excluded_artifact(&entry.file_name()) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination_root.join(entry.file_name());
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", source_path.display()))?;
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)
                .with_context(|| format!("failed to create {}", destination_path.display()))?;
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn copy_rendered_tree(
    source_root: &Path,
    destination_root: &Path,
    replacements: &BTreeMap<&str, String>,
) -> Result<()> {
    let entries = fs::read_dir(source_root)
        .with_context(|| format!("failed to read directory {}", source_root.display()))?;

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source_root.display()))?;
        if is_workspace_excluded_artifact(&entry.file_name()) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination_root.join(entry.file_name());
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", source_path.display()))?;
        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)
                .with_context(|| format!("failed to create {}", destination_path.display()))?;
            copy_rendered_tree(&source_path, &destination_path, replacements)?;
        } else if metadata.is_file() {
            write_rendered_template(&source_path, &destination_path, replacements)?;
        }
    }

    Ok(())
}

fn is_workspace_excluded_artifact(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name == "__pycache__"
        || name.ends_with(".pyc")
        || (name.starts_with("test_") && name.ends_with(".py"))
}

fn display_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn count_lines(contents: &[u8]) -> usize {
    String::from_utf8_lossy(contents).lines().count()
}

fn line_change_counts(expected: &str, actual: &str) -> (usize, usize) {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let mut lcs = vec![vec![0usize; actual_lines.len() + 1]; expected_lines.len() + 1];

    for (expected_index, expected_line) in expected_lines.iter().enumerate() {
        for (actual_index, actual_line) in actual_lines.iter().enumerate() {
            lcs[expected_index + 1][actual_index + 1] = if expected_line == actual_line {
                lcs[expected_index][actual_index] + 1
            } else {
                lcs[expected_index][actual_index + 1].max(lcs[expected_index + 1][actual_index])
            };
        }
    }

    let shared_lines = lcs[expected_lines.len()][actual_lines.len()];
    (
        expected_lines.len().saturating_sub(shared_lines),
        actual_lines.len().saturating_sub(shared_lines),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        env, process,
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
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn source_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(PROFILE_SOURCE_RELATIVE_PATH)
    }

    fn sample_config(root: &Path) -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: source_root(),
            host_workspaces_root: root.to_path_buf(),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
        }
    }

    fn sample_agent() -> OpenCodeWorkspaceAgent {
        OpenCodeWorkspaceAgent {
            agent_key: "btc-2".to_string(),
            display_name: "BTC 2".to_string(),
            api_key: "vta_test_123".to_string(),
        }
    }

    #[derive(Debug)]
    enum PermissionRule {
        Action(String),
        Scoped(Vec<(String, String)>),
    }

    #[derive(Debug)]
    struct ProfilePermissions {
        rules: Vec<(String, PermissionRule)>,
    }

    fn rendered_profile(generated: &GeneratedOpenCodeWorkspace, name: &str) -> ProfilePermissions {
        let profile = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/agents")
                .join(format!("{name}.md")),
        )
        .expect("read generated profile");
        let front_matter = profile
            .strip_prefix("---\n")
            .and_then(|profile| {
                profile
                    .split_once("\n---\n")
                    .map(|(front_matter, _)| front_matter)
            })
            .expect("profile has YAML front matter");
        parse_profile_permissions(front_matter)
    }

    fn parse_profile_permissions(front_matter: &str) -> ProfilePermissions {
        let lines: Vec<_> = front_matter.lines().collect();
        let mut index = lines
            .iter()
            .position(|line| *line == "permission:")
            .expect("profile includes permissions")
            + 1;
        let mut rules = Vec::new();

        while let Some(line) = lines.get(index) {
            let Some(entry) = line.strip_prefix("  ") else {
                break;
            };
            if entry.starts_with("  ") {
                panic!("scoped permission has no parent rule");
            }
            let (pattern, action) = parse_permission_entry(entry);
            index += 1;

            if !action.is_empty() {
                rules.push((pattern, PermissionRule::Action(action)));
                continue;
            }

            let mut scoped_rules = Vec::new();
            while let Some(scoped) = lines.get(index).and_then(|line| line.strip_prefix("    ")) {
                scoped_rules.push(parse_permission_entry(scoped));
                index += 1;
            }
            assert!(
                !scoped_rules.is_empty(),
                "scoped permission {pattern} must contain rules"
            );
            rules.push((pattern, PermissionRule::Scoped(scoped_rules)));
        }

        ProfilePermissions { rules }
    }

    fn parse_permission_entry(entry: &str) -> (String, String) {
        let (pattern, action) = entry
            .split_once(':')
            .expect("permission entry has an action separator");
        (
            pattern.trim().trim_matches('"').to_string(),
            action.trim().to_string(),
        )
    }

    fn profile_permission_action(
        profile: &ProfilePermissions,
        tool: &str,
        subject: &str,
    ) -> String {
        let mut action = None;
        for (pattern, rule) in &profile.rules {
            if !wildcard_matches(pattern, tool) {
                continue;
            }

            match rule {
                PermissionRule::Action(rule_action) => action = Some(rule_action.as_str()),
                PermissionRule::Scoped(scoped_rules) => {
                    for (scoped_pattern, scoped_action) in scoped_rules {
                        if wildcard_matches(scoped_pattern, subject) {
                            action = Some(scoped_action);
                        }
                    }
                }
            }
        }
        action.unwrap_or("allow").to_string()
    }

    fn profile_permissions_json(profile: &ProfilePermissions) -> Value {
        let mut permissions = serde_json::Map::new();
        for (pattern, rule) in &profile.rules {
            let rule = match rule {
                PermissionRule::Action(action) => Value::String(action.clone()),
                PermissionRule::Scoped(scoped_rules) => Value::Object(
                    scoped_rules
                        .iter()
                        .map(|(pattern, action)| (pattern.clone(), Value::String(action.clone())))
                        .collect(),
                ),
            };
            permissions.insert(pattern.clone(), rule);
        }
        Value::Object(permissions)
    }

    fn wildcard_matches(pattern: &str, value: &str) -> bool {
        let pattern = pattern.as_bytes();
        let value = value.as_bytes();
        let mut pattern_index = 0;
        let mut value_index = 0;
        let mut star_index = None;
        let mut star_value_index = 0;

        while value_index < value.len() {
            if pattern_index < pattern.len()
                && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
            {
                pattern_index += 1;
                value_index += 1;
            } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
                star_index = Some(pattern_index);
                pattern_index += 1;
                star_value_index = value_index;
            } else if let Some(star_index) = star_index {
                pattern_index = star_index + 1;
                star_value_index += 1;
                value_index = star_value_index;
            } else {
                return false;
            }
        }

        while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            pattern_index += 1;
        }
        pattern_index == pattern.len()
    }

    fn assert_profile_action(
        profile_name: &str,
        profile: &ProfilePermissions,
        tool: &str,
        subject: &str,
        expected: &str,
    ) {
        assert_eq!(
            profile_permission_action(profile, tool, subject),
            expected,
            "{profile_name} profile should {expected} {tool} for {subject:?}"
        );
    }

    #[test]
    fn generates_expected_workspace_layout() {
        let temp = TempDir::new("opencode-layout");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");

        for path in [
            generated.workspace_host_path.join("opencode.json"),
            generated.workspace_host_path.join("AGENTS.md"),
            generated.workspace_host_path.join(".env"),
            generated
                .workspace_host_path
                .join(".opencode/commands/hypervibes-analysis.md"),
            generated
                .workspace_host_path
                .join(".opencode/commands/hypervibes-market-analysis.md"),
            generated
                .workspace_host_path
                .join(".opencode/agents/analysis.md"),
            generated
                .workspace_host_path
                .join(".opencode/agents/market-analysis.md"),
            generated.workspace_host_path.join("scripts/user"),
            generated.workspace_host_path.join("scratch"),
        ] {
            assert!(path.exists(), "missing {}", path.display());
        }

        assert!(
            !generated.workspace_host_path.join("generated").exists(),
            "generated/ should not be created into workspaces"
        );

        // The workspace-local Python API client has been removed in favor
        // of the `hypervibes` MCP server, which is installed by the custom
        // OpenCode image at a fixed path.
        assert!(
            !generated.workspace_host_path.join("hypervibes").exists(),
            "hypervibes/ should not be generated into workspaces"
        );
        assert!(
            !generated
                .workspace_host_path
                .join("requirements.txt")
                .exists(),
            "root requirements.txt should not be generated into workspaces"
        );
    }

    #[test]
    fn materialized_run_workspace_uses_only_the_runtime_credential() {
        let temp = TempDir::new("opencode-run-materialize");
        let config = sample_config(&temp.path);
        generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
            .expect("generate active workspace");
        let path = RunWorkspacePath::new("btc-2", 42).expect("run path");
        let runtime_key = "vtr_test_runtime_credential".to_string();

        let materialized = materialize_run_workspace(
            &config,
            &path,
            &RunWorkspaceMaterializationInput {
                display_name: "BTC 2".to_string(),
                api_base_url: "http://host.containers.internal:3003".to_string(),
                runtime_api_key: runtime_key.clone(),
                credential_id: "b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33".to_string(),
                sub_agent_kind: "analysis".to_string(),
                enabled_capabilities: Vec::new(),
                expected_quantitative_package: None,
            },
        )
        .expect("materialize run workspace");

        assert_eq!(
            materialized.workspace_container_path,
            "/workspaces/runs/btc-2/42/workspace"
        );
        let workspace = path.workspace_host_path(&config);
        let environment = fs::read_to_string(workspace.join(".env")).expect("read run environment");
        assert!(environment.contains(&runtime_key));
        assert!(!environment.contains("vta_test_123"));
        assert!(workspace.join("scripts/user").is_dir());
        let profile = fs::read_to_string(workspace.join(".opencode/agents/analysis.md"))
            .expect("read run analysis profile");
        assert!(profile.contains("hypervibes_send_notification: deny"));
    }

    #[test]
    fn generated_opencode_json_registers_hypervibes_mcp_server() {
        let temp = TempDir::new("opencode-mcp-config");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let raw = fs::read_to_string(generated.workspace_host_path.join("opencode.json"))
            .expect("read opencode.json");
        let parsed: Value = serde_json::from_str(&raw).expect("parse opencode.json");

        let mcp = parsed
            .get("mcp")
            .and_then(Value::as_object)
            .expect("mcp object");
        let server = mcp
            .get("hypervibes")
            .and_then(Value::as_object)
            .expect("hypervibes mcp entry");
        assert_eq!(server.get("type").and_then(Value::as_str), Some("local"));
        assert_eq!(server.get("enabled").and_then(Value::as_bool), Some(true));

        let command = server
            .get("command")
            .and_then(Value::as_array)
            .expect("command array");
        let command_strs: Vec<&str> = command.iter().filter_map(Value::as_str).collect();
        assert!(
            command_strs
                .iter()
                .any(|part| part.contains("hypervibes/mcp/.venv/bin/python")),
            "command should use the MCP venv python; got {command_strs:?}"
        );
        assert!(
            command_strs
                .iter()
                .any(|part| part.ends_with("hypervibes/mcp/server.py")),
            "command should launch the MCP server script; got {command_strs:?}"
        );

        assert_eq!(server.get("cwd").and_then(Value::as_str), Some("."));

        // The tool schema is server-defined, but the config itself must
        // never embed a credential.
        assert!(!raw.contains("HYPERVIBES_API_KEY"));
        assert!(!raw.contains("vta_"));
    }

    #[test]
    fn generated_trading_profile_allows_only_conditional_confirmation_commands() {
        let temp = TempDir::new("opencode-trading-permissions");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let profile = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/agents/trading.md"),
        )
        .expect("read trading profile");

        assert!(profile.contains(
            "\"*\": deny\n    \"python .opencode/skills/hyperliquid-data/fetch_ohlcv.py *\": allow"
        ));
        assert!(!profile.contains("\"python scripts/user/analyze.py *\": allow"));
        assert!(profile.contains("hypervibes_run_analysis_tool: allow"));
        assert!(
            profile.contains("\"workspaces/agents/btc-2/scratch/trading-confirmation\": allow")
        );
        assert!(
            profile.contains("\"workspaces/agents/btc-2/scratch/trading-confirmation/**\": allow")
        );
        assert!(!profile.contains("\"scratch/trading-confirmation/**\": allow"));
        assert!(profile.contains("hyperliquid-data: allow"));
        assert!(profile.contains("Do not use `ls`, shell composition, or directory reads"));
        assert!(profile.contains("Do not read `scripts/user/analyze.py`."));
        assert!(profile.contains("webfetch: deny"));
        assert!(profile.contains("task: deny"));
    }

    #[test]
    fn generated_profiles_enforce_the_role_permission_matrix() {
        let temp = TempDir::new("opencode-profile-permissions");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let analysis = rendered_profile(&generated, "analysis");
        let market_analysis = rendered_profile(&generated, "market-analysis");
        let trading = rendered_profile(&generated, "trading");
        let daily_review = rendered_profile(&generated, "daily-review");
        let analysis_coding = rendered_profile(&generated, "analysis-coding");
        let conversations = rendered_profile(&generated, "agent-conversations");

        for (name, profile) in [
            ("analysis", &analysis),
            ("market-analysis", &market_analysis),
            ("trading", &trading),
            ("daily-review", &daily_review),
            ("analysis-coding", &analysis_coding),
            ("agent-conversations", &conversations),
        ] {
            assert_profile_action(name, profile, "unlisted_tool", "", "deny");
            assert_profile_action(name, profile, "todowrite", "", "deny");
        }

        assert_profile_action(
            "analysis",
            &analysis,
            "bash",
            "python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m",
            "allow",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "bash",
            "python scripts/user/analyze.py --symbol BTC",
            "deny",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "bash",
            "python -c 'print(1)'",
            "deny",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "read",
            "workspaces/agents/btc-2/scratch/ohlcv/input.json",
            "allow",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "read",
            "workspaces/agents/btc-2/scripts/user/analyze.py",
            "deny",
        );
        assert_profile_action("analysis", &analysis, "skill", "hyperliquid-data", "allow");
        assert_profile_action("analysis", &analysis, "skill", "analysis-coding", "deny");
        assert_profile_action("analysis", &analysis, "hypervibes_get_account", "", "allow");
        assert_profile_action(
            "analysis",
            &analysis,
            "hypervibes_run_analysis_tool",
            "",
            "allow",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "hypervibes_send_notification",
            "",
            "deny",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "hypervibes_submit_orders",
            "",
            "deny",
        );

        assert_profile_action(
            "market-analysis",
            &market_analysis,
            "hypervibes_get_latest_analysis",
            "",
            "allow",
        );
        assert_profile_action(
            "market-analysis",
            &market_analysis,
            "hypervibes_write_memory",
            "",
            "allow",
        );
        assert_profile_action(
            "market-analysis",
            &market_analysis,
            "bash",
            "python x.py",
            "deny",
        );

        assert_profile_action(
            "trading",
            &trading,
            "hypervibes_run_analysis_tool",
            "",
            "allow",
        );
        assert_profile_action(
            "market-analysis",
            &market_analysis,
            "edit",
            "scripts/user/analyze.py",
            "deny",
        );
        assert_profile_action(
            "market-analysis",
            &market_analysis,
            "hypervibes_send_notification",
            "",
            "deny",
        );

        assert_profile_action(
            "daily-review",
            &daily_review,
            "hypervibes_list_account_transactions",
            "",
            "allow",
        );
        assert_profile_action(
            "daily-review",
            &daily_review,
            "hypervibes_write_memory",
            "",
            "allow",
        );
        assert_profile_action("daily-review", &daily_review, "bash", "python x.py", "deny");
        assert_profile_action(
            "daily-review",
            &daily_review,
            "read",
            "scratch/data.json",
            "deny",
        );
        assert_profile_action(
            "daily-review",
            &daily_review,
            "hypervibes_update_strategy_prompt",
            "",
            "deny",
        );
        assert_profile_action(
            "daily-review",
            &daily_review,
            "hypervibes_send_notification",
            "",
            "deny",
        );

        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "edit",
            "scripts/user/analyze.py",
            "allow",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "read",
            "scripts/user/tests/test_indicator.py",
            "allow",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "glob",
            "scripts/user/**",
            "allow",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "edit",
            "AGENTS.md",
            "deny",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "read",
            "scratch/input.json",
            "deny",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "bash",
            "python x.py",
            "deny",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "skill",
            "analysis-coding",
            "allow",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "skill",
            "python-analysis",
            "deny",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "hypervibes_coding_validate_candidate",
            "",
            "allow",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "hypervibes_update_strategy_prompt",
            "",
            "deny",
        );
        assert_profile_action(
            "analysis-coding",
            &analysis_coding,
            "hypervibes_send_notification",
            "",
            "deny",
        );

        assert_profile_action(
            "trading",
            &trading,
            "bash",
            "python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m",
            "allow",
        );
        assert_profile_action(
            "trading",
            &trading,
            "read",
            "workspaces/agents/btc-2/scratch/trading-confirmation/result.json",
            "allow",
        );
        assert_profile_action(
            "trading",
            &trading,
            "read",
            "workspaces/agents/btc-2/scratch/ohlcv/input.json",
            "deny",
        );
        assert_profile_action("trading", &trading, "hypervibes_submit_orders", "", "allow");
        assert_profile_action(
            "trading",
            &trading,
            "hypervibes_send_notification",
            "",
            "allow",
        );

        assert_profile_action(
            "agent-conversations",
            &conversations,
            "read",
            "AGENTS.md",
            "deny",
        );
        assert_profile_action(
            "agent-conversations",
            &conversations,
            "hypervibes_list_strategy_prompts",
            "",
            "allow",
        );
        assert_profile_action(
            "agent-conversations",
            &conversations,
            "hypervibes_submit_orders",
            "",
            "deny",
        );
        assert_profile_action(
            "agent-conversations",
            &conversations,
            "hypervibes_send_notification",
            "",
            "deny",
        );
    }

    #[test]
    fn container_chat_profile_matches_workspace_template_permissions() {
        let temp = TempDir::new("opencode-container-chat-profile");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let workspace_profile = rendered_profile(&generated, "agent-conversations");
        let workspace_permissions = profile_permissions_json(&workspace_profile);

        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../agent-runtime/container/opencode.jsonc");
        let config = fs::read_to_string(config_path).expect("read container OpenCode config");
        let config = config
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let config: Value = serde_json::from_str(&config).expect("parse container OpenCode config");
        let container_permissions = config
            .get("agent")
            .and_then(Value::as_object)
            .and_then(|agents| agents.get("agent-conversations"))
            .and_then(Value::as_object)
            .and_then(|profile| profile.get("permission"))
            .expect("container chat permissions");

        assert_eq!(container_permissions, &workspace_permissions);
    }

    #[test]
    fn renders_agents_template_placeholders() {
        let temp = TempDir::new("opencode-agents-template");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let rendered = fs::read_to_string(generated.workspace_host_path.join("AGENTS.md"))
            .expect("read AGENTS.md");

        assert!(rendered.contains("btc-2"));
        assert!(rendered.contains("http://host.containers.internal:3003"));
        assert!(rendered.contains("/workspaces/agents/btc-2"));
    }

    #[test]
    fn writes_env_with_agent_credentials() {
        let temp = TempDir::new("opencode-env");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let env_text =
            fs::read_to_string(generated.workspace_host_path.join(".env")).expect("read .env");

        assert!(env_text.contains("HYPERVIBES_API_KEY=vta_test_123"));
        assert!(env_text.contains("HYPERVIBES_API_BASE_URL=http://host.containers.internal:3003"));
        assert!(env_text.contains("HYPERVIBES_AGENT_KEY=btc-2"));
        assert!(env_text.contains("HYPERVIBES_WORKSPACE=/workspaces/agents/btc-2"));
    }

    #[test]
    fn copies_commands_and_agents_into_project_opencode_dir() {
        let temp = TempDir::new("opencode-copy");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");

        let commands = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/commands/hypervibes-market-analysis.md"),
        )
        .expect("read command");
        let agent = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/agents/market-analysis.md"),
        )
        .expect("read agent");

        assert!(commands.contains("HyperVibes"));
        assert!(agent.contains("Analysis coding owns reusable quantitative code."));
        assert!(agent.contains("steps: 100"));
    }

    #[test]
    fn preserves_existing_scripts_user_file_on_regeneration() {
        let temp = TempDir::new("opencode-preserve");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");
        let custom_script = generated.workspace_host_path.join("scripts/user/custom.py");
        fs::write(&custom_script, "print('hello')\n").expect("write custom script");

        generate_agent_workspace(
            &config,
            &sample_agent(),
            WorkspaceGenerationMode::Regenerate,
        )
        .expect("regenerate workspace");

        assert_eq!(
            fs::read_to_string(custom_script).expect("read custom script"),
            "print('hello')\n"
        );
    }

    #[test]
    fn hard_reset_deletes_user_managed_files_before_regeneration() {
        let temp = TempDir::new("opencode-hard-reset");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");
        let workspace_root = &generated.workspace_host_path;

        fs::write(
            workspace_root.join("scripts/user/custom.py"),
            "print('hello')\n",
        )
        .expect("write custom script");
        fs::write(workspace_root.join("data/cache.json"), "{}\n").expect("write data file");
        fs::write(workspace_root.join("scratch/note.txt"), "scratch\n")
            .expect("write scratch file");
        fs::write(workspace_root.join("extra.txt"), "extra\n").expect("write extra file");

        let deleted = delete_agent_workspace(&config, &sample_agent().agent_key)
            .expect("delete workspace for hard reset");
        assert!(deleted);

        let regenerated = generate_agent_workspace(
            &config,
            &sample_agent(),
            WorkspaceGenerationMode::Regenerate,
        )
        .expect("recreate workspace after hard reset");

        assert!(
            !regenerated
                .workspace_host_path
                .join("scripts/user/custom.py")
                .exists()
        );
        assert!(
            !regenerated
                .workspace_host_path
                .join("data/cache.json")
                .exists()
        );
        assert!(
            !regenerated
                .workspace_host_path
                .join("scratch/note.txt")
                .exists()
        );
        assert!(!regenerated.workspace_host_path.join("extra.txt").exists());
    }

    #[test]
    fn hard_reset_recreates_template_managed_files() {
        let temp = TempDir::new("opencode-hard-reset-template");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");
        fs::remove_file(generated.workspace_host_path.join("AGENTS.md")).expect("remove AGENTS.md");

        delete_agent_workspace(&config, &sample_agent().agent_key)
            .expect("delete workspace for hard reset");

        let regenerated = generate_agent_workspace(
            &config,
            &sample_agent(),
            WorkspaceGenerationMode::Regenerate,
        )
        .expect("recreate workspace after hard reset");

        assert!(regenerated.workspace_host_path.join("AGENTS.md").exists());
        assert!(
            regenerated
                .workspace_host_path
                .join(".opencode/commands/hypervibes-analysis.md")
                .exists()
        );
        assert!(
            regenerated
                .workspace_host_path
                .join(".opencode/skills/analysis-coding/SKILL.md")
                .exists()
        );
    }

    #[test]
    fn workspace_template_diff_is_clean_for_new_workspace() {
        let temp = TempDir::new("opencode-diff-clean");
        let config = sample_config(&temp.path);
        generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
            .expect("generate workspace");

        let drift =
            diff_agent_workspace_from_template(&config, &sample_agent()).expect("diff workspace");

        assert!(drift.workspace_exists);
        assert!(drift.is_in_sync());
        assert!(drift.changed_files.is_empty());
    }

    #[test]
    fn workspace_template_diff_reports_modified_and_missing_template_files() {
        let temp = TempDir::new("opencode-diff-dirty");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");

        fs::write(
            generated.workspace_host_path.join("AGENTS.md"),
            "user-modified\n",
        )
        .expect("modify AGENTS.md");
        fs::remove_file(
            generated
                .workspace_host_path
                .join(".opencode/agents/trading.md"),
        )
        .expect("remove trading agent file");

        let drift =
            diff_agent_workspace_from_template(&config, &sample_agent()).expect("diff workspace");

        assert!(drift.workspace_exists);
        let agents_md = drift
            .changed_files
            .iter()
            .find(|file| file.path == "AGENTS.md")
            .expect("AGENTS.md changed");
        assert_eq!(agents_md.status, WorkspaceTemplateFileStatus::Modified);
        assert!(agents_md.added_lines > 0);
        assert!(agents_md.removed_lines > 0);

        let trading_md = drift
            .changed_files
            .iter()
            .find(|file| file.path == ".opencode/agents/trading.md")
            .expect("trading.md changed");
        assert_eq!(trading_md.status, WorkspaceTemplateFileStatus::Deleted);
        assert!(trading_md.added_lines > 0);
        assert_eq!(trading_md.removed_lines, 0);
    }

    #[test]
    fn line_change_counts_match_template_sync_direction() {
        let (removed_lines, added_lines) = line_change_counts("keep\nold\n", "keep\nnew\nextra\n");

        assert_eq!(removed_lines, 1);
        assert_eq!(added_lines, 2);
    }

    #[test]
    fn workspace_template_diff_ignores_agent_generated_files() {
        let temp = TempDir::new("opencode-diff-ignores-user-files");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");
        fs::write(
            generated.workspace_host_path.join("scripts/user/custom.py"),
            "print('hello')\n",
        )
        .expect("write custom file");

        let drift =
            diff_agent_workspace_from_template(&config, &sample_agent()).expect("diff workspace");

        assert!(drift.workspace_exists);
        assert!(drift.changed_files.is_empty());
    }

    #[test]
    fn workspace_template_cache_artifacts_are_not_copied_or_collected() {
        let temp = TempDir::new("opencode-cache-artifacts");
        let source = temp.path.join("source");
        let destination = temp.path.join("destination");
        let skills = source.join("skills");
        fs::create_dir_all(skills.join("__pycache__")).expect("create cache directory");
        fs::create_dir_all(&destination).expect("create destination directory");
        fs::write(skills.join("SKILL.md"), "skill\n").expect("write skill");
        fs::write(skills.join("test_skill.py"), "test\n").expect("write test");
        fs::write(skills.join("fetch_ohlcv.cpython-314.pyc"), "cache").expect("write cache file");
        fs::write(
            skills.join("__pycache__/test_fetch_ohlcv.cpython-314.pyc"),
            "cache",
        )
        .expect("write nested cache file");

        copy_tree(&skills, &destination).expect("copy template tree");
        assert!(destination.join("SKILL.md").exists());
        assert!(!destination.join("test_skill.py").exists());
        assert!(!destination.join("fetch_ohlcv.cpython-314.pyc").exists());
        assert!(!destination.join("__pycache__").exists());

        let mut files = Vec::new();
        let replacements = BTreeMap::new();
        collect_expected_workspace_files(&source, Path::new("skills"), &replacements, &mut files)
            .expect("collect template files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, PathBuf::from("skills/SKILL.md"));

        assert!(is_workspace_excluded_artifact(std::ffi::OsStr::new(
            "__pycache__"
        )));
        assert!(is_workspace_excluded_artifact(std::ffi::OsStr::new(
            "fetch_ohlcv.cpython-314.pyc"
        )));
        assert!(is_workspace_excluded_artifact(std::ffi::OsStr::new(
            "test_fetch_ohlcv.py"
        )));
        assert!(!is_workspace_excluded_artifact(std::ffi::OsStr::new(
            "fetch_ohlcv.py"
        )));
    }

    #[test]
    fn rejects_unsafe_agent_keys() {
        let temp = TempDir::new("opencode-unsafe");
        let config = sample_config(&temp.path);

        for bad_key in ["../btc", "btc/2", ""] {
            let error = generate_agent_workspace(
                &config,
                &OpenCodeWorkspaceAgent {
                    agent_key: bad_key.to_string(),
                    ..sample_agent()
                },
                WorkspaceGenerationMode::CreateNew,
            )
            .expect_err("unsafe key should fail");
            assert!(error.to_string().contains("agent_key"));
        }
    }

    #[test]
    fn fails_on_unknown_template_placeholder() {
        let temp = TempDir::new("opencode-unknown-placeholder");
        let source_temp = TempDir::new("opencode-source");
        let source_root = source_temp.path.join("profile");
        fs::create_dir_all(source_root.join(".opencode/commands")).expect("create commands");
        fs::create_dir_all(source_root.join(".opencode/agents")).expect("create agents");
        fs::create_dir_all(source_root.join(".opencode/skills")).expect("create skills");
        fs::write(
            source_root.join("opencode.json.template"),
            "{\"plugin\": [\"x\"]}\n",
        )
        .expect("write opencode template");
        fs::write(
            source_root.join("AGENTS.md.template"),
            "{{unknown_placeholder}}\n",
        )
        .expect("write agents template");
        fs::write(
            source_root.join(".opencode/commands/hypervibes-analysis.md"),
            "test\n",
        )
        .expect("write command");
        fs::write(source_root.join(".opencode/agents/analysis.md"), "test\n").expect("write agent");

        let error = generate_agent_workspace(
            &OpenCodeWorkspaceConfig {
                source_root,
                host_workspaces_root: temp.path.clone(),
                container_workspaces_root: "/workspaces".to_string(),
                api_base_url: "http://host.containers.internal:3003".to_string(),
            },
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect_err("unknown placeholder should fail");

        assert!(format!("{error:#}").contains("unknown template placeholder"));
    }

    #[test]
    fn renders_opencode_json_without_workspace_plugin_config() {
        let temp = TempDir::new("opencode-json");
        let generated = generate_agent_workspace(
            &sample_config(&temp.path),
            &sample_agent(),
            WorkspaceGenerationMode::CreateNew,
        )
        .expect("generate workspace");
        let rendered = fs::read_to_string(generated.workspace_host_path.join("opencode.json"))
            .expect("read opencode.json");

        assert!(rendered.contains("https://opencode.ai/config.json"));
        assert!(!rendered.contains("@aeondave/opencode-dotenv@latest"));
        assert!(rendered.contains("hypervibes"));
    }

    #[test]
    fn delete_agent_workspace_removes_existing_directory() {
        let temp = TempDir::new("opencode-delete-existing");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect("generate workspace");

        let deleted =
            delete_agent_workspace(&config, &sample_agent().agent_key).expect("delete workspace");

        assert!(deleted);
        assert!(!generated.workspace_host_path.exists());
    }

    #[test]
    fn delete_agent_workspace_returns_false_for_missing_directory() {
        let temp = TempDir::new("opencode-delete-missing");
        let deleted = delete_agent_workspace(&sample_config(&temp.path), &sample_agent().agent_key)
            .expect("delete missing workspace");

        assert!(!deleted);
    }

    #[test]
    fn delete_agent_workspace_rejects_unsafe_keys() {
        let temp = TempDir::new("opencode-delete-unsafe");
        let error = delete_agent_workspace(&sample_config(&temp.path), "../btc")
            .expect_err("unsafe key should fail");

        assert!(error.to_string().contains("agent_key"));
    }

    #[test]
    fn create_new_workspace_fails_when_directory_already_exists() {
        let temp = TempDir::new("opencode-existing-dir");
        let config = sample_config(&temp.path);
        let workspace_path =
            agent_workspace_host_path(&config, &sample_agent().agent_key).expect("workspace path");
        fs::create_dir_all(&workspace_path).expect("create existing workspace dir");
        fs::write(workspace_path.join("sentinel.txt"), "keep").expect("write sentinel");

        let error =
            generate_agent_workspace(&config, &sample_agent(), WorkspaceGenerationMode::CreateNew)
                .expect_err("existing directory should fail");

        assert!(error.to_string().contains("workspace already exists"));
        assert!(workspace_path.join("sentinel.txt").exists());
    }

    fn browser_workspace(config: &OpenCodeWorkspaceConfig) -> PathBuf {
        let path = agent_workspace_host_path(config, &sample_agent().agent_key)
            .expect("agent workspace path");
        fs::create_dir_all(&path).expect("create browser workspace");
        path
    }

    #[test]
    fn workspace_browser_lists_safe_entries_in_stable_order() {
        let temp = TempDir::new("workspace-browser-list");
        let config = sample_config(&temp.path);
        let workspace = browser_workspace(&config);
        fs::create_dir_all(workspace.join(".opencode")).expect("create opencode directory");
        fs::create_dir_all(workspace.join("nested")).expect("create nested directory");
        fs::write(workspace.join(".opencode/config.json"), "{}\n").expect("write config");
        fs::write(workspace.join("nested/file.txt"), "nested\n").expect("write nested file");
        fs::write(workspace.join("root.txt"), "root\n").expect("write root file");
        fs::write(workspace.join(".env"), "secret\n").expect("write root env");
        fs::write(workspace.join("nested/.env"), "secret\n").expect("write nested env");
        fs::create_dir_all(workspace.join("node_modules/package")).expect("create node modules");
        fs::write(workspace.join("node_modules/package/index.js"), "hidden\n")
            .expect("write node module");

        let listing = list_workspace_browser_entries(&config, "btc-2").expect("list workspace");

        assert!(listing.workspace_exists);
        assert!(!listing.truncated);
        assert_eq!(
            listing
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            [
                ".opencode",
                ".opencode/config.json",
                "nested",
                "nested/file.txt",
                "root.txt",
            ]
        );
        assert_eq!(
            listing.entries[0].kind,
            WorkspaceBrowserEntryKind::Directory
        );
        assert_eq!(listing.entries[1].size_bytes, Some(3));
        assert!(
            listing
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("node_modules"))
        );
    }

    #[test]
    fn workspace_browser_file_previews_report_text_binary_large_and_missing() {
        let temp = TempDir::new("workspace-browser-preview");
        let config = sample_config(&temp.path);
        let workspace = browser_workspace(&config);
        fs::write(workspace.join("text.txt"), "hello\n").expect("write text");
        fs::write(workspace.join("binary.bin"), [b'a', 0, b'b']).expect("write binary");
        fs::write(
            workspace.join("large.txt"),
            vec![b'x'; WORKSPACE_BROWSER_MAX_PREVIEW_BYTES as usize + 1],
        )
        .expect("write large file");

        let text = read_workspace_browser_file(&config, "btc-2", "text.txt").expect("read text");
        assert_eq!(text.status, WorkspaceFilePreviewStatus::Text);
        assert_eq!(text.text.as_deref(), Some("hello\n"));

        let binary =
            read_workspace_browser_file(&config, "btc-2", "binary.bin").expect("read binary");
        assert_eq!(binary.status, WorkspaceFilePreviewStatus::Binary);
        assert!(binary.text.is_none());

        let large = read_workspace_browser_file(&config, "btc-2", "large.txt").expect("read large");
        assert_eq!(large.status, WorkspaceFilePreviewStatus::TooLarge);
        assert!(large.text.is_none());

        let missing =
            read_workspace_browser_file(&config, "btc-2", "missing.txt").expect("read missing");
        assert_eq!(missing.status, WorkspaceFilePreviewStatus::Missing);
    }

    #[test]
    fn workspace_browser_rejects_unsafe_paths_and_agent_keys() {
        let temp = TempDir::new("workspace-browser-invalid");
        let config = sample_config(&temp.path);
        browser_workspace(&config);

        for path in [
            "",
            "/text.txt",
            "nested//text.txt",
            "./text.txt",
            "../text.txt",
            ".env",
            "a/\0b",
        ] {
            assert!(
                read_workspace_browser_file(&config, "btc-2", path).is_err(),
                "path should be rejected: {path:?}"
            );
        }
        for agent_key in [".", "..", "../btc", "btc/2", ""] {
            assert!(
                list_workspace_browser_entries(&config, agent_key).is_err(),
                "agent key should be rejected: {agent_key:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_browser_omits_and_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("workspace-browser-symlink");
        let config = sample_config(&temp.path);
        let workspace = browser_workspace(&config);
        let outside = temp.path.join("outside.txt");
        fs::write(&outside, "outside\n").expect("write outside file");
        symlink(&outside, workspace.join("outside-link")).expect("create outside symlink");

        let listing = list_workspace_browser_entries(&config, "btc-2").expect("list workspace");
        assert!(listing.entries.is_empty());
        assert!(read_workspace_browser_file(&config, "btc-2", "outside-link").is_err());
    }

    #[test]
    fn workspace_browser_reports_missing_workspace_and_truncated_trees() {
        let temp = TempDir::new("workspace-browser-limits");
        let config = sample_config(&temp.path);
        let missing = list_workspace_browser_entries(&config, "btc-2").expect("list missing");
        assert!(!missing.workspace_exists);
        assert_eq!(
            read_workspace_browser_file(&config, "btc-2", "missing.txt")
                .expect("read missing workspace")
                .status,
            WorkspaceFilePreviewStatus::Missing
        );

        let workspace = browser_workspace(&config);
        for index in 0..=WORKSPACE_BROWSER_MAX_ENTRIES {
            fs::write(workspace.join(format!("{index:03}.txt")), "x").expect("write entry");
        }
        let entry_limited = list_workspace_browser_entries(&config, "btc-2").expect("list entries");
        assert!(entry_limited.truncated);
        assert_eq!(entry_limited.entries.len(), WORKSPACE_BROWSER_MAX_ENTRIES);

        fs::remove_dir_all(&workspace).expect("remove entry-limited workspace");
        fs::create_dir_all(&workspace).expect("recreate workspace");
        let mut nested = workspace;
        for index in 0..=WORKSPACE_BROWSER_MAX_DEPTH {
            nested = nested.join(format!("level-{index}"));
            fs::create_dir_all(&nested).expect("create nested directory");
        }
        let depth_limited = list_workspace_browser_entries(&config, "btc-2").expect("list depth");
        assert!(depth_limited.truncated);
    }
}
