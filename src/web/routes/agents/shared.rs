use crate::{
    model_catalog::options::{ModelPickerOption, build_model_picker_options},
    web::{
        AppState,
        templates::{ModelPickerProviderGroup, ModelPickerView},
    },
};
use axum::{
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::warn;
pub(in crate::web::routes) const SERVER_SHUTTING_DOWN_WARNING: &str =
    "Server is shutting down. Run now is unavailable until the next start.";
#[derive(Debug, Clone)]
pub(in crate::web::routes) struct ModelPickerContext {
    pub options: Vec<ModelPickerOption>,
    pub warning: Option<String>,
}
pub(in crate::web::routes) fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ToggleJobForm {
    pub enabled: Option<String>,
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeoutForm {
    #[serde(default)]
    pub timeout: String,
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct ModelSelectionForm {
    #[serde(default)]
    pub model_selection: String,
    #[serde(default)]
    pub model_variant: String,
}
pub(in crate::web::routes) fn timeout_error_redirect(
    detail_url: &str,
    message: String,
) -> Response {
    let encoded = urlencode(&message);
    Redirect::to(&format!("{detail_url}?timeout_error={encoded}")).into_response()
}
pub(in crate::web::routes) fn urlencode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                vec![byte as char].into_iter().collect::<Vec<_>>()
            } else {
                format!("%{byte:02X}").chars().collect::<Vec<_>>()
            }
            .into_iter()
        })
        .collect()
}
pub(in crate::web::routes) fn sub_agents_warning_redirect(
    agent_key: &str,
    message: &str,
) -> Response {
    Redirect::to(&format!(
        "/agents/{agent_key}/analysis?warning={}",
        urlencode(message)
    ))
    .into_response()
}
pub(in crate::web::routes) async fn load_model_picker_context(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
) -> ModelPickerContext {
    match build_model_picker_options(
        &state.opencode_container_workspaces_root,
        &state.opencode_base_url,
        &state.opencode_client,
        &state.model_catalog,
    )
    .await
    {
        Ok(options) => ModelPickerContext {
            options,
            warning: None,
        },
        Err(error) => {
            warn!(agent_key = %agent.agent_key, error = ?error, "failed to load model picker options");
            ModelPickerContext {
                options: Vec::new(),
                warning: Some("Could not load configured OpenCode models.".to_string()),
            }
        }
    }
}

pub(in crate::web::routes) fn selected_model_label(
    selected: &str,
    selected_variant: &str,
    options: &[ModelPickerOption],
) -> String {
    if selected.trim().is_empty() {
        return "None selected".to_string();
    }

    let label = options
        .iter()
        .find(|option| option.value == selected)
        .map(|option| format!("{} / {}", option.provider_name, option.model_name))
        .unwrap_or_else(|| selected.to_string());
    if selected_variant.trim().is_empty() {
        label
    } else {
        format!("{label} - {}", selected_variant.trim())
    }
}
pub(in crate::web::routes) fn build_model_picker_view(
    input_id: &str,
    selected_value: &str,
    selected_variant: Option<&str>,
    picker: ModelPickerContext,
) -> ModelPickerView {
    let provider_groups = build_provider_groups(&picker.options);

    ModelPickerView {
        input_id: input_id.to_string(),
        input_name: "model_selection".to_string(),
        variant_input_id: format!("{input_id}-variant"),
        variant_input_name: "model_variant".to_string(),
        selected_value: selected_value.to_string(),
        selected_variant: selected_variant.unwrap_or_default().to_string(),
        selected_label: selected_model_label(
            selected_value,
            selected_variant.unwrap_or_default(),
            &picker.options,
        ),
        empty_label: "None selected".to_string(),
        provider_groups,
        options: picker.options,
        warning: picker.warning,
        show_label: true,
        submit_on_save: true,
        lazy_options_url: None,
    }
}

fn build_provider_groups(options: &[ModelPickerOption]) -> Vec<ModelPickerProviderGroup> {
    let mut groups = Vec::new();

    for option in options {
        if let Some(group) = groups
            .iter_mut()
            .find(|group: &&mut ModelPickerProviderGroup| group.provider_id == option.provider_id)
        {
            group.options.push(option.clone());
            continue;
        }

        groups.push(ModelPickerProviderGroup {
            provider_id: option.provider_id.clone(),
            provider_name: option.provider_name.clone(),
            provider_logo_url: Some(option.provider_logo_url.clone()),
            options: vec![option.clone()],
        });
    }

    groups
}
pub(in crate::web::routes) async fn validate_model_selection_for_agent(
    state: &Arc<AppState>,
    selection: Option<(String, String)>,
    variant: &str,
) -> Result<Option<(String, String, Option<String>)>, String> {
    let variant = (!variant.trim().is_empty()).then(|| variant.trim().to_string());
    let Some(selection) = selection else {
        return if variant.is_none() {
            Ok(None)
        } else {
            Err("Select a model before choosing a thinking mode.".to_string())
        };
    };

    let options = build_model_picker_options(
        &state.opencode_container_workspaces_root,
        &state.opencode_base_url,
        &state.opencode_client,
        &state.model_catalog,
    )
    .await
    .map_err(|_| "Could not load configured OpenCode models. Try again.".to_string())?;
    validate_model_selection_in_options(&options, selection, variant)
}

fn validate_model_selection_in_options(
    options: &[ModelPickerOption],
    selection: (String, String),
    variant: Option<String>,
) -> Result<Option<(String, String, Option<String>)>, String> {
    let Some(option) = options
        .iter()
        .find(|option| option.provider_id == selection.0 && option.model_id == selection.1)
    else {
        return Err("Select a valid model.".to_string());
    };
    if let Some(ref variant) = variant
        && !option.thinking_variants.iter().any(|name| name == variant)
    {
        return Err("Select a thinking mode advertised for this model.".to_string());
    }
    Ok(Some((selection.0, selection.1, variant)))
}
pub(in crate::web::routes) fn parse_positive_job_seconds(
    raw_value: &str,
    field_name: &str,
    errors: &mut Vec<String>,
) -> Option<i32> {
    match raw_value.trim().parse::<i32>() {
        Ok(value) if value > 0 => Some(value),
        _ => {
            errors.push(format!(
                "{field_name} must be a positive number of seconds."
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option() -> ModelPickerOption {
        ModelPickerOption {
            value: "anthropic/claude-sonnet-4".to_string(),
            provider_id: "anthropic".to_string(),
            provider_name: "Anthropic".to_string(),
            provider_logo_url: "/model-catalog/logos/anthropic".to_string(),
            model_id: "claude-sonnet-4".to_string(),
            model_name: "Claude Sonnet 4".to_string(),
            metadata_text: String::new(),
            thinking_variants: vec!["high".to_string()],
        }
    }

    #[test]
    fn model_validation_accepts_default_and_advertised_variants() {
        let selection = ("anthropic".to_string(), "claude-sonnet-4".to_string());

        assert_eq!(
            validate_model_selection_in_options(&[option()], selection.clone(), None).unwrap(),
            Some((selection.0.clone(), selection.1.clone(), None))
        );
        assert_eq!(
            validate_model_selection_in_options(
                &[option()],
                selection.clone(),
                Some("high".to_string()),
            )
            .unwrap(),
            Some((selection.0, selection.1, Some("high".to_string())))
        );
    }

    #[test]
    fn model_validation_rejects_stale_or_mismatched_variants() {
        let selection = ("anthropic".to_string(), "claude-sonnet-4".to_string());

        assert!(
            validate_model_selection_in_options(
                &[option()],
                selection.clone(),
                Some("low".to_string()),
            )
            .is_err()
        );
        assert!(
            validate_model_selection_in_options(
                &[option()],
                ("anthropic".to_string(), "claude-opus-4".to_string()),
                Some("high".to_string()),
            )
            .is_err()
        );
    }
}
