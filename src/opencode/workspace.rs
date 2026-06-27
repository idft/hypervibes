use std::{
    collections::BTreeMap,
    env, fs,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenCodeWorkspaceRuntimeConfig {
    pub workspace_host_path: String,
    pub workspace_container_path: String,
    pub profile_source: String,
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
) -> Result<GeneratedOpenCodeWorkspace> {
    validate_agent_key(&agent.agent_key)?;

    let workspace_host_path = config
        .host_workspaces_root
        .join("agents")
        .join(&agent.agent_key);
    let workspace_container_path = join_container_path(
        &config.container_workspaces_root,
        &["agents", agent.agent_key.as_str()],
    );

    for path in [
        workspace_host_path.as_path(),
        &workspace_host_path.join(".opencode/commands"),
        &workspace_host_path.join(".opencode/agents"),
        &workspace_host_path.join(".opencode/skills"),
        &workspace_host_path.join("scripts/generated"),
        &workspace_host_path.join("scripts/user"),
        &workspace_host_path.join("data"),
        &workspace_host_path.join("scratch"),
    ] {
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create workspace directory {}", path.display()))?;
    }

    let replacements = BTreeMap::from([
        ("agent_key", agent.agent_key.clone()),
        ("display_name", agent.display_name.clone()),
        ("api_base_url", config.api_base_url.clone()),
        ("workspace_container_path", workspace_container_path.clone()),
    ]);

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
    copy_tree(
        &config.source_root.join(".opencode/agents"),
        &workspace_host_path.join(".opencode/agents"),
    )?;
    copy_tree(
        &config.source_root.join(".opencode/skills"),
        &workspace_host_path.join(".opencode/skills"),
    )?;
    copy_tree(
        &config.source_root.join("scripts/generated"),
        &workspace_host_path.join("scripts/generated"),
    )?;

    fs::write(
        workspace_host_path.join(".env"),
        format!(
            "VIBETRADING_AGENT_KEY={}\nVIBETRADING_API_BASE_URL={}\nVIBETRADING_API_KEY={}\nVIBETRADING_WORKSPACE={}\n",
            agent.agent_key, config.api_base_url, agent.api_key, workspace_container_path,
        ),
    )
    .context("failed to write workspace .env")?;

    Ok(GeneratedOpenCodeWorkspace {
        workspace_host_path,
        workspace_container_path,
    })
}

pub fn runtime_config_for_generated_workspace(
    generated: &GeneratedOpenCodeWorkspace,
) -> OpenCodeWorkspaceRuntimeConfig {
    OpenCodeWorkspaceRuntimeConfig {
        workspace_host_path: display_workspace_host_path(&generated.workspace_host_path),
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

fn write_rendered_template(
    source_path: &Path,
    destination_path: &Path,
    replacements: &BTreeMap<&str, String>,
) -> Result<()> {
    let template = fs::read_to_string(source_path)
        .with_context(|| format!("failed to read template {}", source_path.display()))?;
    let rendered = render_template(&template, replacements)
        .with_context(|| format!("failed to render template {}", source_path.display()))?;
    fs::write(destination_path, rendered)
        .with_context(|| format!("failed to write {}", destination_path.display()))?;
    Ok(())
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

fn display_workspace_host_path(path: &Path) -> String {
    match env::current_dir() {
        Ok(current_dir) => path
            .strip_prefix(&current_dir)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().into_owned()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(PROFILE_SOURCE_RELATIVE_PATH)
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
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
            .expect("generate workspace");

        for path in [
            generated.workspace_host_path.join("opencode.json"),
            generated.workspace_host_path.join("AGENTS.md"),
            generated.workspace_host_path.join(".env"),
            generated
                .workspace_host_path
                .join(".opencode/commands/vibetrading-analysis.md"),
            generated
                .workspace_host_path
                .join(".opencode/agents/analysis.md"),
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
        // of the `vibetrading` MCP server, which is installed by the custom
        // OpenCode image at a fixed path.
        assert!(
            !generated.workspace_host_path.join("vibetrading").exists(),
            "vibetrading/ should not be generated into workspaces"
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
    fn generated_opencode_json_registers_vibetrading_mcp_server() {
        let temp = TempDir::new("opencode-mcp-config");
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
            .expect("generate workspace");
        let raw = fs::read_to_string(generated.workspace_host_path.join("opencode.json"))
            .expect("read opencode.json");
        let parsed: Value = serde_json::from_str(&raw).expect("parse opencode.json");

        let mcp = parsed
            .get("mcp")
            .and_then(Value::as_object)
            .expect("mcp object");
        let server = mcp
            .get("vibetrading")
            .and_then(Value::as_object)
            .expect("vibetrading mcp entry");
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
                .any(|part| part.contains("vibetrading/mcp/.venv/bin/python")),
            "command should use the MCP venv python; got {command_strs:?}"
        );
        assert!(
            command_strs
                .iter()
                .any(|part| part.ends_with("vibetrading/mcp/server.py")),
            "command should launch the MCP server script; got {command_strs:?}"
        );

        assert_eq!(server.get("cwd").and_then(Value::as_str), Some("."));

        // The tool schema is server-defined, but the config itself must
        // never embed a credential.
        assert!(!raw.contains("VIBETRADING_API_KEY"));
        assert!(!raw.contains("vta_"));
    }

    #[test]
    fn renders_agents_template_placeholders() {
        let temp = TempDir::new("opencode-agents-template");
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
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
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
            .expect("generate workspace");
        let env_text =
            fs::read_to_string(generated.workspace_host_path.join(".env")).expect("read .env");

        assert!(env_text.contains("VIBETRADING_API_KEY=vta_test_123"));
        assert!(env_text.contains("VIBETRADING_API_BASE_URL=http://host.containers.internal:3003"));
        assert!(env_text.contains("VIBETRADING_AGENT_KEY=btc-2"));
        assert!(env_text.contains("VIBETRADING_WORKSPACE=/workspaces/agents/btc-2"));
    }

    #[test]
    fn copies_commands_and_agents_into_project_opencode_dir() {
        let temp = TempDir::new("opencode-copy");
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
            .expect("generate workspace");

        let commands = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/commands/vibetrading-analysis.md"),
        )
        .expect("read command");
        let agent = fs::read_to_string(
            generated
                .workspace_host_path
                .join(".opencode/agents/analysis.md"),
        )
        .expect("read agent");

        assert!(commands.contains("Vibetrading"));
        assert!(agent.contains("scripts/user/"));
        assert!(agent.contains("steps: 100"));
    }

    #[test]
    fn preserves_existing_scripts_user_file_on_regeneration() {
        let temp = TempDir::new("opencode-preserve");
        let config = sample_config(&temp.path);
        let generated =
            generate_agent_workspace(&config, &sample_agent()).expect("generate workspace");
        let custom_script = generated.workspace_host_path.join("scripts/user/custom.py");
        fs::write(&custom_script, "print('hello')\n").expect("write custom script");

        generate_agent_workspace(&config, &sample_agent()).expect("regenerate workspace");

        assert_eq!(
            fs::read_to_string(custom_script).expect("read custom script"),
            "print('hello')\n"
        );
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
        fs::create_dir_all(source_root.join("scripts/generated"))
            .expect("create scripts/generated");
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
            source_root.join(".opencode/commands/vibetrading-analysis.md"),
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
        )
        .expect_err("unknown placeholder should fail");

        assert!(format!("{error:#}").contains("unknown template placeholder"));
    }

    #[test]
    fn renders_opencode_json_without_workspace_plugin_config() {
        let temp = TempDir::new("opencode-json");
        let generated = generate_agent_workspace(&sample_config(&temp.path), &sample_agent())
            .expect("generate workspace");
        let rendered = fs::read_to_string(generated.workspace_host_path.join("opencode.json"))
            .expect("read opencode.json");

        assert!(rendered.contains("https://opencode.ai/config.json"));
        assert!(!rendered.contains("@aeondave/opencode-dotenv@latest"));
        assert!(rendered.contains("vibetrading"));
    }
}
