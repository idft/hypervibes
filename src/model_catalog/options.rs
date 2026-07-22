use anyhow::{Result, anyhow};

use crate::{
    agents::model::AgentDetailRow,
    model_catalog::models_dev::{ModelsDevCatalog, ModelsDevModel, ModelsDevProvider},
    opencode::{
        client::{OpenCodeClient, OpenCodeProviderInfo, OpenCodeProvidersResponse},
        workspace::OpenCodeWorkspaceRuntimeConfig,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPickerOption {
    pub value: String,
    pub provider_id: String,
    pub provider_name: String,
    pub provider_logo_url: String,
    pub model_id: String,
    pub model_name: String,
    pub metadata_text: String,
}

pub async fn build_model_picker_options(
    agent: &AgentDetailRow,
    opencode_base_url: &str,
    opencode_client: &OpenCodeClient,
    model_catalog: &ModelsDevCatalog,
) -> Result<Vec<ModelPickerOption>> {
    let workspace = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
        .ok_or_else(|| anyhow!("OpenCode workspace metadata is missing for this agent"))?;
    let response = opencode_client
        .list_providers(opencode_base_url, &workspace.workspace_container_path)
        .await?;
    let snapshot = model_catalog.snapshot().await.ok();
    Ok(build_model_picker_options_from_response(
        &response,
        snapshot.as_ref(),
    ))
}

pub fn build_model_picker_options_from_response(
    response: &OpenCodeProvidersResponse,
    catalog: Option<&crate::model_catalog::models_dev::ModelsDevCatalogSnapshot>,
) -> Vec<ModelPickerOption> {
    let connected_only = !response.connected.is_empty();
    let mut options = Vec::new();

    for provider in &response.all {
        if connected_only && !response.connected.iter().any(|id| id == &provider.id) {
            continue;
        }

        let catalog_provider = catalog.and_then(|snapshot| snapshot.providers.get(&provider.id));
        let provider_name = provider_display_name(provider, catalog_provider);

        for (model_key, model_info) in &provider.models {
            let model_id = model_info.id.as_deref().unwrap_or(model_key).to_string();
            let catalog_model =
                catalog_provider.and_then(|provider| provider.models.get(&model_id));
            let model_name = catalog_model
                .map(|model| model.name.clone())
                .or_else(|| model_info.name.clone())
                .unwrap_or_else(|| model_id.clone());

            options.push(ModelPickerOption {
                value: format!("{}/{}", provider.id, model_id),
                provider_id: provider.id.clone(),
                provider_name: provider_name.clone(),
                provider_logo_url: format!("/model-catalog/logos/{}", provider.id),
                model_id,
                model_name,
                metadata_text: build_metadata_text(catalog_model),
            });
        }
    }

    options.sort_by(|left, right| {
        left.provider_name
            .cmp(&right.provider_name)
            .then(left.model_name.cmp(&right.model_name))
    });
    options
}

pub fn parse_model_selection(raw: &str) -> Result<Option<(String, String)>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let Some((provider, model)) = raw.split_once('/') else {
        return Err("Select a valid model.".to_string());
    };
    if provider.trim().is_empty() || model.trim().is_empty() {
        return Err("Select a valid model.".to_string());
    }
    Ok(Some((provider.to_string(), model.to_string())))
}

pub fn selection_exists_in_options(
    options: &[ModelPickerOption],
    selection: &(String, String),
) -> bool {
    options
        .iter()
        .any(|option| option.provider_id == selection.0 && option.model_id == selection.1)
}

fn provider_display_name(
    provider: &OpenCodeProviderInfo,
    catalog_provider: Option<&ModelsDevProvider>,
) -> String {
    catalog_provider
        .map(|provider| provider.name.clone())
        .or_else(|| provider.name.clone())
        .unwrap_or_else(|| provider.id.clone())
}

fn build_metadata_text(model: Option<&ModelsDevModel>) -> String {
    let Some(model) = model else {
        return String::new();
    };

    let mut parts = Vec::new();
    if let Some(limit) = model.limit.as_ref().and_then(|limit| limit.context_window) {
        parts.push(format_context_window(limit));
    }
    if model.tool_call.unwrap_or(false) {
        parts.push("tools".to_string());
    }
    if model.reasoning.unwrap_or(false) {
        parts.push("reasoning".to_string());
    }
    if let Some(cost) = model.cost.as_ref()
        && let (Some(input), Some(output)) = (cost.input, cost.output)
    {
        parts.push(format!("${input}/${output}"));
    }
    parts.join(" · ")
}

fn format_context_window(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}M ctx", value / 1_000_000)
    } else if value >= 1_000 {
        format!("{}K ctx", value / 1_000)
    } else {
        format!("{value} ctx")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        model_catalog::models_dev::{
            ModelsDevCatalogSnapshot, ModelsDevCost, ModelsDevLimit, ModelsDevProvider,
        },
        opencode::client::{OpenCodeModelInfo, OpenCodeProviderInfo, OpenCodeProvidersResponse},
    };

    use super::*;

    #[test]
    fn parses_model_selection_values() {
        assert_eq!(parse_model_selection("").unwrap(), None);
        assert_eq!(
            parse_model_selection("anthropic/claude-sonnet-4-6").unwrap(),
            Some(("anthropic".to_string(), "claude-sonnet-4-6".to_string()))
        );
        assert_eq!(
            parse_model_selection("openrouter/google/gemini-2.5-flash").unwrap(),
            Some((
                "openrouter".to_string(),
                "google/gemini-2.5-flash".to_string()
            ))
        );
        assert!(parse_model_selection("anthropic").is_err());
    }

    #[test]
    fn merge_prefers_models_dev_names_and_falls_back_to_opencode() {
        let response = OpenCodeProvidersResponse {
            all: vec![OpenCodeProviderInfo {
                id: "anthropic".to_string(),
                name: Some("Anthropic Runtime".to_string()),
                models: BTreeMap::from([(
                    "claude-sonnet-4".to_string(),
                    OpenCodeModelInfo {
                        id: None,
                        name: Some("Runtime Sonnet".to_string()),
                    },
                )]),
            }],
            connected: vec!["anthropic".to_string()],
        };
        let snapshot = ModelsDevCatalogSnapshot {
            providers: BTreeMap::from([(
                "anthropic".to_string(),
                ModelsDevProvider {
                    name: "Anthropic".to_string(),
                    models: BTreeMap::from([(
                        "claude-sonnet-4".to_string(),
                        ModelsDevModel {
                            name: "Claude Sonnet 4".to_string(),
                            reasoning: Some(true),
                            tool_call: Some(true),
                            limit: Some(ModelsDevLimit {
                                context_window: Some(1_000_000),
                            }),
                            cost: Some(ModelsDevCost {
                                input: Some(3.0),
                                output: Some(15.0),
                            }),
                        },
                    )]),
                },
            )]),
            fetched_at: chrono::Utc::now(),
            stale: false,
        };

        let options = build_model_picker_options_from_response(&response, Some(&snapshot));
        assert_eq!(options[0].provider_name, "Anthropic");
        assert_eq!(options[0].model_name, "Claude Sonnet 4");
        assert_eq!(
            options[0].metadata_text,
            "1M ctx · tools · reasoning · $3/$15"
        );
    }

    #[test]
    fn selection_exists_checks_provider_and_model() {
        let options = vec![ModelPickerOption {
            value: "anthropic/claude-sonnet-4".to_string(),
            provider_id: "anthropic".to_string(),
            provider_name: "Anthropic".to_string(),
            provider_logo_url: "/model-catalog/logos/anthropic".to_string(),
            model_id: "claude-sonnet-4".to_string(),
            model_name: "Claude Sonnet 4".to_string(),
            metadata_text: String::new(),
        }];

        assert!(selection_exists_in_options(
            &options,
            &("anthropic".to_string(), "claude-sonnet-4".to_string())
        ));
        assert!(!selection_exists_in_options(
            &options,
            &("anthropic".to_string(), "claude-opus-4".to_string())
        ));
    }
}
