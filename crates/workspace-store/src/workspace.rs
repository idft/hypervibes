use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROFILE_SOURCE_RELATIVE_PATH: &str = "agent-runtime/workspace-template";

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

pub fn runtime_config_for_generated_workspace(
    generated: &GeneratedOpenCodeWorkspace,
) -> OpenCodeWorkspaceRuntimeConfig {
    OpenCodeWorkspaceRuntimeConfig {
        workspace_container_path: generated.workspace_container_path.clone(),
        profile_source: PROFILE_SOURCE_RELATIVE_PATH.to_string(),
    }
}

fn validate_agent_key(agent_key: &str) -> Result<()> {
    if agent_key.trim().is_empty() {
        bail!("agent_key must not be empty");
    }
    if agent_key.contains("..") || agent_key.contains('/') || agent_key.contains('\\') {
        bail!("agent_key contains unsafe path characters");
    }
    Ok(())
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
        assert!(profile.contains("\"python scripts/user/analyze.py *\": allow"));
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
        assert!(agent.contains("scripts/user/"));
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
}
