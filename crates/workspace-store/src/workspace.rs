use std::{
    fs,
    path::{Path, PathBuf},
};

use super::isolated_workspace::{
    ConversationWorkspacePath, RunWorkspacePath, create_conversation_workspace,
    create_run_workspace,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Path of the rendered workspace template, relative to the repository root.
/// Run and conversation workspaces render from this source tree.
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

/// Request-scoped OpenCode runtime directory configuration. The harness
/// backend derives the `workspace_container_path` of a specific run or
/// conversation OpenCode session from this value; it is never agent-level
/// durable metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenCodeWorkspaceRuntimeConfig {
    pub workspace_container_path: String,
    pub profile_source: String,
}

impl OpenCodeWorkspaceRuntimeConfig {
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    pub fn into_value(self) -> serde_json::Value {
        serde_json::to_value(self).expect("OpenCodeWorkspaceRuntimeConfig always serializes")
    }
}

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedRunWorkspace {
    pub workspace_container_path: String,
}

/// Controller input for rendering a conversation workspace. The API key is
/// deliberately absent from every response and idempotency fingerprint.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationWorkspaceMaterializationInput {
    pub display_name: String,
    pub api_base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializedConversationWorkspace {
    pub workspace_container_path: String,
}

/// Render a complete run-local workspace from trusted templates. This must run
/// before the OpenCode session is created.
pub fn materialize_run_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &RunWorkspacePath,
    input: &RunWorkspaceMaterializationInput,
) -> Result<MaterializedRunWorkspace> {
    validate_run_workspace_materialization_input(input)?;

    let created = create_run_workspace(config, path)?;
    let workspace_root = path.workspace_host_path(config);
    ensure_regular_directory(&workspace_root, "run workspace root")?;

    for relative in [
        ".opencode/commands",
        ".opencode/agents",
        ".opencode/skills",
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

    write_run_runtime_environment(
        &workspace_root,
        &created.workspace_container_path,
        path,
        input,
    )?;

    Ok(MaterializedRunWorkspace {
        workspace_container_path: created.workspace_container_path,
    })
}

/// Render a complete conversation-local workspace from trusted templates. This
/// must run before the OpenCode conversation session is created so the session
/// directory registers the HyperVibes MCP server and carries the runtime
/// credential the MCP server reads from `.env`.
pub fn materialize_conversation_workspace(
    config: &OpenCodeWorkspaceConfig,
    path: &ConversationWorkspacePath,
    input: &ConversationWorkspaceMaterializationInput,
) -> Result<MaterializedConversationWorkspace> {
    validate_conversation_workspace_materialization_input(input)?;

    let created = create_conversation_workspace(config, path)?;
    let workspace_root = path.workspace_host_path(config);
    ensure_regular_directory(&workspace_root, "conversation workspace root")?;

    for relative in [".opencode/commands", ".opencode/agents", ".opencode/skills"] {
        fs::create_dir_all(workspace_root.join(relative)).with_context(|| {
            format!("failed to create conversation workspace directory {relative}")
        })?;
    }

    let template_agent = OpenCodeWorkspaceAgent {
        agent_key: path.agent_key().to_string(),
        display_name: input.display_name.clone(),
        // Template rendering currently has no API-key placeholder. Keep this
        // empty so the permanent agent key can never reach a conversation
        // workspace.
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

    write_conversation_runtime_environment(
        &workspace_root,
        &created.workspace_container_path,
        path,
        input,
    )?;

    Ok(MaterializedConversationWorkspace {
        workspace_container_path: created.workspace_container_path,
    })
}

fn validate_conversation_workspace_materialization_input(
    input: &ConversationWorkspaceMaterializationInput,
) -> Result<()> {
    for (name, value) in [
        ("api_base_url", input.api_base_url.as_str()),
        ("api_key", input.api_key.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            bail!("{name} is invalid");
        }
    }
    Ok(())
}

fn write_conversation_runtime_environment(
    workspace_root: &Path,
    workspace_container_path: &str,
    path: &ConversationWorkspacePath,
    input: &ConversationWorkspaceMaterializationInput,
) -> Result<()> {
    let contents = format!(
        "HYPERVIBES_AGENT_KEY={}\nHYPERVIBES_API_BASE_URL={}\nHYPERVIBES_API_KEY={}\nHYPERVIBES_WORKSPACE={}\n",
        path.agent_key(),
        input.api_base_url,
        input.api_key,
        workspace_container_path,
    );
    let temporary = workspace_root.join(".env.conversation.tmp");
    fs::write(&temporary, contents).context("failed to write conversation runtime environment")?;
    fs::rename(&temporary, workspace_root.join(".env"))
        .context("failed to publish conversation runtime environment")
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
        "analysis" | "trading" | "review"
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

fn render_run_capability_permissions(
    workspace_root: &Path,
    sub_agent_kind: &str,
    enabled_capabilities: &[String],
) -> Result<()> {
    if enabled_capabilities.iter().any(|capability| {
        !matches!(
            capability.as_str(),
            "hypervibes:notification_send"
                | "hypervibes:prompt_revision_submit"
                | "hypervibes:review_prompt_update"
        ) && !capability.starts_with("custom-mcp:")
    }) {
        bail!("run workspace has an unsupported capability");
    }
    let profile_name = match sub_agent_kind {
        "analysis" => "analysis",
        "trading" => "trading",
        "review" => "review",
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

fn template_replacements(
    config: &OpenCodeWorkspaceConfig,
    agent: &OpenCodeWorkspaceAgent,
    workspace_container_path: &str,
) -> std::collections::BTreeMap<&'static str, String> {
    std::collections::BTreeMap::from([
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
    replacements: &std::collections::BTreeMap<&str, String>,
) -> Result<()> {
    let rendered = render_template_file(source_path, replacements)?;
    fs::write(destination_path, rendered)
        .with_context(|| format!("failed to write {}", destination_path.display()))?;
    Ok(())
}

fn render_template_file(
    source_path: &Path,
    replacements: &std::collections::BTreeMap<&str, String>,
) -> Result<String> {
    let template = fs::read_to_string(source_path)
        .with_context(|| format!("failed to read template {}", source_path.display()))?;
    render_template(&template, replacements)
        .with_context(|| format!("failed to render template {}", source_path.display()))
}

fn render_template(
    template: &str,
    replacements: &std::collections::BTreeMap<&str, String>,
) -> Result<String> {
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
    replacements: &std::collections::BTreeMap<&str, String>,
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

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
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
            .join("agent-runtime/workspace-template")
    }

    fn sample_config(root: &Path) -> OpenCodeWorkspaceConfig {
        OpenCodeWorkspaceConfig {
            source_root: source_root(),
            host_workspaces_root: root.to_path_buf(),
            container_workspaces_root: "/workspaces".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
        }
    }

    fn valid_materialization_input() -> RunWorkspaceMaterializationInput {
        RunWorkspaceMaterializationInput {
            display_name: "BTC 2".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
            runtime_api_key: "vtr_test_runtime_credential".to_string(),
            credential_id: "b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33".to_string(),
            sub_agent_kind: "analysis".to_string(),
            enabled_capabilities: Vec::new(),
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

    fn profile_permissions_json(profile: &ProfilePermissions) -> serde_json::Value {
        let mut permissions = serde_json::Map::new();
        for (pattern, rule) in &profile.rules {
            let rule = match rule {
                PermissionRule::Action(action) => serde_json::Value::String(action.clone()),
                PermissionRule::Scoped(scoped_rules) => serde_json::Value::Object(
                    scoped_rules
                        .iter()
                        .map(|(pattern, action)| {
                            (pattern.clone(), serde_json::Value::String(action.clone()))
                        })
                        .collect(),
                ),
            };
            permissions.insert(pattern.clone(), rule);
        }
        serde_json::Value::Object(permissions)
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

    fn rendered_profile_from(workspace_root: &Path, name: &str) -> ProfilePermissions {
        let profile = fs::read_to_string(
            workspace_root
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

    #[test]
    fn materialized_run_workspace_uses_only_the_runtime_credential() {
        let temp = TempDir::new("opencode-run-materialize");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 42).expect("run path");
        let runtime_key = "vtr_test_runtime_credential".to_string();
        let mut input = valid_materialization_input();
        input.runtime_api_key = runtime_key.clone();

        let materialized =
            materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");

        assert_eq!(
            materialized.workspace_container_path,
            "/workspaces/runs/btc-2/42/workspace"
        );
        let workspace = path.workspace_host_path(&config);
        let environment = fs::read_to_string(workspace.join(".env")).expect("read run environment");
        assert!(environment.contains(&runtime_key));
        assert!(!environment.contains("vta_test_123"));
        assert!(!workspace.join("scripts/user").exists());
        let profile = fs::read_to_string(workspace.join(".opencode/agents/analysis.md"))
            .expect("read run analysis profile");
        assert!(profile.contains("hypervibes_send_notification: deny"));
    }

    #[test]
    fn generated_opencode_json_registers_hypervibes_mcp_server() {
        let temp = TempDir::new("opencode-mcp-config");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 45).expect("run path");
        let input = valid_materialization_input();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");

        let raw = fs::read_to_string(path.workspace_host_path(&config).join("opencode.json"))
            .expect("read opencode.json");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse opencode.json");

        let mcp = parsed
            .get("mcp")
            .and_then(serde_json::Value::as_object)
            .expect("mcp object");
        let server = mcp
            .get("hypervibes")
            .and_then(serde_json::Value::as_object)
            .expect("hypervibes mcp entry");
        assert_eq!(
            server.get("type").and_then(serde_json::Value::as_str),
            Some("local")
        );
        assert_eq!(
            server.get("enabled").and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let command = server
            .get("command")
            .and_then(serde_json::Value::as_array)
            .expect("command array");
        let command_strs: Vec<&str> = command
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
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

        assert_eq!(
            server.get("cwd").and_then(serde_json::Value::as_str),
            Some(".")
        );
        assert!(!raw.contains("HYPERVIBES_API_KEY"));
        assert!(!raw.contains("vta_"));
    }

    #[test]
    fn generated_trading_profile_preserves_restrictions() {
        let temp = TempDir::new("opencode-trading-permissions");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 46).expect("run path");
        let mut input = valid_materialization_input();
        input.sub_agent_kind = "trading".to_string();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let profile = fs::read_to_string(
            path.workspace_host_path(&config)
                .join(".opencode/agents/trading.md"),
        )
        .expect("read trading profile");

        assert!(profile.contains("  bash: deny"));
        assert!(profile.contains("  read: deny"));
        assert!(!profile.contains("trading-confirmation"));
        assert!(!profile.contains("hyperliquid-data"));
        assert!(profile.contains("webfetch: deny"));
        assert!(profile.contains("task: deny"));
    }

    #[test]
    fn review_workspace_accepts_prompt_revision_capability() {
        let temp = TempDir::new("opencode-review-prompt-revision");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 48).expect("run path");
        let mut input = valid_materialization_input();
        input.sub_agent_kind = "review".to_string();
        input.enabled_capabilities = vec!["hypervibes:prompt_revision_submit".to_string()];

        materialize_run_workspace(&config, &path, &input).expect("materialize review workspace");
    }

    #[test]
    fn generated_profiles_enforce_the_role_permission_matrix() {
        let temp = TempDir::new("opencode-profile-permissions");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 47).expect("run path");
        let input = valid_materialization_input();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let workspace_root = path.workspace_host_path(&config);
        let analysis = rendered_profile_from(&workspace_root, "analysis");
        let trading = rendered_profile_from(&workspace_root, "trading");
        let review = rendered_profile_from(&workspace_root, "review");

        for (name, profile) in [
            ("analysis", &analysis),
            ("trading", &trading),
            ("review", &review),
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
            "python -c 'print(1)'",
            "deny",
        );
        assert_profile_action(
            "analysis",
            &analysis,
            "read",
            "workspaces/runs/btc-2/47/workspace/scratch/ohlcv/input.json",
            "allow",
        );
        assert_profile_action("analysis", &analysis, "skill", "hyperliquid-data", "allow");
        assert_profile_action("analysis", &analysis, "skill", "analysis-coding", "deny");
        assert_profile_action("analysis", &analysis, "hypervibes_get_account", "", "allow");
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
            "review",
            &review,
            "hypervibes_list_account_transactions",
            "",
            "allow",
        );
        assert_profile_action("review", &review, "hypervibes_write_memory", "", "allow");
        assert_profile_action("review", &review, "bash", "python x.py", "deny");
        assert_profile_action("review", &review, "read", "scratch/data.json", "deny");
        assert_profile_action(
            "review",
            &review,
            "hypervibes_update_strategy_prompt",
            "",
            "deny",
        );
        assert_profile_action(
            "review",
            &review,
            "hypervibes_send_notification",
            "",
            "deny",
        );

        assert_profile_action("trading", &trading, "hypervibes_submit_orders", "", "allow");
        assert_profile_action("trading", &trading, "hypervibes_get_account", "", "allow");
        assert_profile_action(
            "trading",
            &trading,
            "bash",
            "python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m",
            "deny",
        );
        assert_profile_action(
            "trading",
            &trading,
            "read",
            "workspaces/runs/btc-2/47/workspace/scratch/trading-confirmation/result.json",
            "deny",
        );
        assert_profile_action(
            "trading",
            &trading,
            "read",
            "workspaces/runs/btc-2/47/workspace/scratch/ohlcv/input.json",
            "deny",
        );
        assert_profile_action(
            "trading",
            &trading,
            "hypervibes_send_notification",
            "",
            "allow",
        );
    }

    #[test]
    fn container_chat_profile_matches_workspace_template_permissions() {
        let temp = TempDir::new("opencode-container-chat-profile");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 48).expect("run path");
        let mut input = valid_materialization_input();
        input.sub_agent_kind = "analysis".to_string();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let workspace_profile =
            rendered_profile_from(&path.workspace_host_path(&config), "agent-conversations");
        let workspace_permissions = profile_permissions_json(&workspace_profile);

        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../agent-runtime/container/opencode.jsonc");
        let config = fs::read_to_string(config_path).expect("read container OpenCode config");
        let config = config
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let config: serde_json::Value =
            serde_json::from_str(&config).expect("parse container OpenCode config");
        let container_permissions = config
            .get("agent")
            .and_then(serde_json::Value::as_object)
            .and_then(|agents| agents.get("agent-conversations"))
            .and_then(serde_json::Value::as_object)
            .and_then(|profile| profile.get("permission"))
            .expect("container chat permissions");

        assert_eq!(container_permissions, &workspace_permissions);
    }

    #[test]
    fn materialized_conversation_workspace_registers_mcp_and_runtime_env() {
        let temp = TempDir::new("opencode-conversation-materialize");
        let config = sample_config(&temp.path);
        let id = uuid::Uuid::parse_str("b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33").expect("parse uuid");
        let path = ConversationWorkspacePath::new("btc-2", id).expect("conversation path");
        let input = ConversationWorkspaceMaterializationInput {
            display_name: "BTC 2".to_string(),
            api_base_url: "http://host.containers.internal:3003".to_string(),
            api_key: "vta_test_123".to_string(),
        };

        let materialized =
            materialize_conversation_workspace(&config, &path, &input).expect("materialize");
        assert_eq!(
            materialized.workspace_container_path,
            "/workspaces/conversations/btc-2/b3ce59a8-f3b6-448d-a0c8-d44ea9d23a33/workspace"
        );

        let workspace = path.workspace_host_path(&config);
        let raw = fs::read_to_string(workspace.join("opencode.json")).expect("read opencode.json");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse opencode.json");
        let mcp = parsed
            .get("mcp")
            .and_then(serde_json::Value::as_object)
            .expect("mcp object");
        assert!(mcp.get("hypervibes").is_some());

        let environment = fs::read_to_string(workspace.join(".env")).expect("read .env");
        assert!(environment.contains("HYPERVIBES_AGENT_KEY=btc-2"));
        assert!(environment.contains("HYPERVIBES_API_KEY=vta_test_123"));
        assert!(
            environment.contains("HYPERVIBES_API_BASE_URL=http://host.containers.internal:3003")
        );

        let profile = fs::read_to_string(workspace.join(".opencode/agents/agent-conversations.md"))
            .expect("read conversation profile");
        assert!(profile.contains("hypervibes_list_memories: allow"));
    }

    #[test]
    fn renders_agents_template_placeholders() {
        let temp = TempDir::new("opencode-agents-template");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 49).expect("run path");
        let input = valid_materialization_input();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let rendered = fs::read_to_string(path.workspace_host_path(&config).join("AGENTS.md"))
            .expect("read AGENTS.md");

        assert!(rendered.contains("btc-2"));
        assert!(rendered.contains("http://host.containers.internal:3003"));
        assert!(rendered.contains("/workspaces/runs/btc-2/49/workspace"));
    }

    #[test]
    fn copies_commands_and_agents_into_project_opencode_dir() {
        let temp = TempDir::new("opencode-copy");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 50).expect("run path");
        let input = valid_materialization_input();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let workspace_root = path.workspace_host_path(&config);

        let commands =
            fs::read_to_string(workspace_root.join(".opencode/commands/hypervibes-trading.md"))
                .expect("read command");
        let agent = fs::read_to_string(workspace_root.join(".opencode/agents/trading.md"))
            .expect("read agent");

        assert!(commands.contains("HyperVibes"));
        assert!(agent.contains("steps: 100"));
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

        let path = RunWorkspacePath::new("btc-2", 51).expect("run path");
        let input = valid_materialization_input();
        let error = materialize_run_workspace(
            &OpenCodeWorkspaceConfig {
                source_root,
                host_workspaces_root: temp.path.clone(),
                container_workspaces_root: "/workspaces".to_string(),
                api_base_url: "http://host.containers.internal:3003".to_string(),
            },
            &path,
            &input,
        )
        .expect_err("unknown placeholder should fail");

        assert!(format!("{error:#}").contains("unknown template placeholder"));
    }

    #[test]
    fn renders_opencode_json_without_workspace_plugin_config() {
        let temp = TempDir::new("opencode-json");
        let config = sample_config(&temp.path);
        let path = RunWorkspacePath::new("btc-2", 52).expect("run path");
        let input = valid_materialization_input();
        materialize_run_workspace(&config, &path, &input).expect("materialize run workspace");
        let rendered = fs::read_to_string(path.workspace_host_path(&config).join("opencode.json"))
            .expect("read opencode.json");

        assert!(rendered.contains("https://opencode.ai/config.json"));
        assert!(!rendered.contains("@aeondave/opencode-dotenv@latest"));
        assert!(rendered.contains("hypervibes"));
    }

    #[test]
    fn workspace_template_cache_artifacts_are_not_copied() {
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
}
