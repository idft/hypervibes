use crate::{
    model_catalog::options::{
        ModelPickerOption, build_model_picker_options, selection_exists_in_options,
    },
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
pub(in crate::web::routes) const WORKSPACE_MAINTENANCE_ACTIVE_WARNING: &str = "Workspace maintenance is queued or running for this agent. Run now is unavailable until it completes.";
pub(in crate::web::routes) const WORKSPACE_MAINTENANCE_DUPLICATE_WARNING: &str =
    "A workspace maintenance task is already queued or running for this agent.";
pub(in crate::web::routes) const SERVER_SHUTTING_DOWN_WARNING: &str = "Server is shutting down. Scheduled Run now is unavailable until the next start. Hook Run now is still allowed while the server drains.";
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
pub(in crate::web::routes) struct ToggleScheduleForm {
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
}
#[derive(Debug, Default, Deserialize)]
pub(in crate::web::routes) struct TimeoutErrorQuery {
    #[serde(default)]
    pub timeout_error: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
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
pub(in crate::web::routes) fn jobs_warning_redirect(agent_key: &str, message: &str) -> Response {
    Redirect::to(&format!(
        "/agents/{agent_key}/jobs?warning={}",
        urlencode(message)
    ))
    .into_response()
}
pub(in crate::web::routes) async fn load_model_picker_context(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
) -> ModelPickerContext {
    match build_model_picker_options(
        agent,
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
    options: &[ModelPickerOption],
) -> String {
    if selected.trim().is_empty() {
        return "None selected".to_string();
    }

    options
        .iter()
        .find(|option| option.value == selected)
        .map(|option| format!("{} / {}", option.provider_name, option.model_name))
        .unwrap_or_else(|| selected.to_string())
}
pub(in crate::web::routes) fn build_model_picker_view(
    input_id: &str,
    selected_value: &str,
    picker: ModelPickerContext,
) -> ModelPickerView {
    let provider_groups = build_provider_groups(&picker.options);

    ModelPickerView {
        input_id: input_id.to_string(),
        input_name: "model_selection".to_string(),
        selected_value: selected_value.to_string(),
        selected_label: selected_model_label(selected_value, &picker.options),
        empty_label: "None selected".to_string(),
        provider_groups,
        options: picker.options,
        warning: picker.warning,
        show_label: true,
        auto_submit: false,
        use_modal: false,
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
    agent: &crate::agents::model::AgentDetailRow,
    selection: Option<(String, String)>,
) -> Result<Option<(String, String)>, String> {
    let Some(selection) = selection else {
        return Ok(None);
    };

    let options = build_model_picker_options(
        agent,
        &state.opencode_base_url,
        &state.opencode_client,
        &state.model_catalog,
    )
    .await
    .map_err(|_| "Could not load configured OpenCode models. Try again.".to_string())?;
    if selection_exists_in_options(&options, &selection) {
        Ok(Some(selection))
    } else {
        Err("Select a valid model.".to_string())
    }
}
pub(in crate::web::routes) fn parse_positive_schedule_seconds(
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
