use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use tracing::warn;
use workspace_store::workspace::{
    WORKSPACE_BROWSER_MAX_DEPTH, WORKSPACE_BROWSER_MAX_ENTRIES, WorkspaceBrowserEntryKind,
    WorkspaceFilePreviewStatus,
};

use crate::{
    agents::store::get_agent,
    harness::model::AgentMaintenanceTaskRow,
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentCodingPageTemplate, AgentShowTab, CodingPackageStatusView,
            CodingTaskStatusPartialTemplate, CodingTaskStatusView, CodingTreeEntryView,
            build_agent_show_tabs,
        },
    },
};

use super::show::load_selected_agent_navbar;

#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct CodingQuery {
    #[serde(default)]
    file: String,
}

pub(in crate::web::routes) async fn agents_show_coding(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<CodingQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?;
    let selected_path = query.file;

    // Package inspection: no package yet, a valid manifest-backed package, or
    // an invalid/unavailable package with a safe error message.
    let inspection = state
        .workspace_controller
        .inspect_active_quantitative_package(&agent_key)
        .await;
    let (package_status, package_exists, mut controller_unavailable) = match inspection {
        Ok(Some(snapshot)) => (
            CodingPackageStatusView::Valid {
                version: snapshot.version,
                manifest_hash: snapshot.manifest_hash,
            },
            true,
            false,
        ),
        Ok(None) => (CodingPackageStatusView::Missing, false, false),
        Err(error) => {
            warn!(agent_key, error = ?error, "coding package inspection unavailable");
            (
                CodingPackageStatusView::Invalid {
                    message: "The Coding package could not be inspected.".to_string(),
                },
                false,
                true,
            )
        }
    };

    let listing = if controller_unavailable {
        None
    } else {
        Some(
            state
                .workspace_controller
                .list_workspace_browser_entries(&agent_key)
                .await,
        )
    };
    let (listing_exists, entries, listing_truncated) = match listing {
        Some(Ok(listing)) => (listing.workspace_exists, listing.entries, listing.truncated),
        Some(Err(error)) => {
            warn!(agent_key, error = ?error, "coding package browser controller unavailable");
            controller_unavailable = true;
            (false, Vec::new(), false)
        }
        None => (false, Vec::new(), false),
    };

    let mut preview_text = None;
    let mut preview_missing = false;
    let mut preview_binary = false;
    let mut preview_too_large = false;
    if !selected_path.is_empty() && !controller_unavailable {
        match state
            .workspace_controller
            .read_workspace_browser_file(&agent_key, &selected_path)
            .await
        {
            Ok(preview) => match preview.status {
                WorkspaceFilePreviewStatus::Text => preview_text = preview.text,
                WorkspaceFilePreviewStatus::Missing => preview_missing = true,
                WorkspaceFilePreviewStatus::Binary => preview_binary = true,
                WorkspaceFilePreviewStatus::TooLarge => preview_too_large = true,
            },
            Err(error) => {
                warn!(agent_key, error = ?error, "coding package file preview unavailable");
                controller_unavailable = true;
            }
        }
    }

    let entries = entries
        .into_iter()
        .map(|entry| CodingTreeEntryView {
            depth: entry.path.matches('/').count(),
            href: coding_file_url(&agent_key, &entry.path),
            selected: entry.kind == WorkspaceBrowserEntryKind::File && entry.path == selected_path,
            is_directory: entry.kind == WorkspaceBrowserEntryKind::Directory,
            initially_hidden: coding_entry_is_initially_hidden(&entry.path, &selected_path),
            name: entry
                .path
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(&entry.path)
                .to_string(),
            path: entry.path,
        })
        .collect();

    let latest_task =
        crate::harness::store::get_latest_analysis_coding_task(&state.db_pool, &agent.agent_key)
            .await
            .inspect_err(|error| {
                warn!(agent_key, error = ?error, "failed to load latest analysis-coding task");
            })
            .unwrap_or(None);
    let (report_summary, changed_paths) =
        load_coding_report_summary(&state, latest_task.as_ref()).await;
    let task_status = CodingTaskStatusView::from_task(latest_task, report_summary, changed_paths);
    let task_status_html = CodingTaskStatusPartialTemplate::render_view(
        task_status,
        coding_task_status_url(&agent_key),
    )
    .unwrap_or_default();

    let current_path = coding_file_url(&agent_key, &selected_path);
    Ok(Html(
        AgentCodingPageTemplate {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Coding, notification_count),
            agent_tabs_use_htmx: true,
            agent,
            navbar,
            current_path,
            entries,
            package_exists: package_exists && listing_exists,
            package_status,
            listing_truncated,
            max_entries: WORKSPACE_BROWSER_MAX_ENTRIES,
            max_depth: WORKSPACE_BROWSER_MAX_DEPTH,
            controller_unavailable,
            selected_path,
            preview_text,
            preview_missing,
            preview_binary,
            preview_too_large,
            task_status_html,
        }
        .render()?,
    )
    .into_response())
}

/// Polling partial for the latest analysis-coding task. Only rendered task
/// state is returned; terminal task information remains visible without
/// polling because `should_poll` is false once the task is terminal.
pub(in crate::web::routes) async fn agents_coding_task_status(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let latest_task =
        crate::harness::store::get_latest_analysis_coding_task(&state.db_pool, &agent.agent_key)
            .await
            .inspect_err(|error| {
                warn!(agent_key, error = ?error, "failed to load latest analysis-coding task");
            })
            .unwrap_or(None);
    let (report_summary, changed_paths) =
        load_coding_report_summary(&state, latest_task.as_ref()).await;
    let task_status = CodingTaskStatusView::from_task(latest_task, report_summary, changed_paths);
    Ok(Html(CodingTaskStatusPartialTemplate::render_view(
        task_status,
        coding_task_status_url(&agent_key),
    )?)
    .into_response())
}

async fn load_coding_report_summary(
    state: &Arc<AppState>,
    task: Option<&AgentMaintenanceTaskRow>,
) -> (Option<String>, Vec<String>) {
    let Some(task) = task else {
        return (None, Vec::new());
    };
    let report = state
        .workspace_controller
        .inspect_candidate(&task.agent_key, task.id)
        .await
        .ok()
        .and_then(|inspection| inspection.report);
    (
        report
            .as_ref()
            .and_then(|value| value.get("summary"))
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        report
            .as_ref()
            .and_then(|value| value.get("changed_paths"))
            .and_then(serde_json::Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|path| path.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

fn coding_task_status_url(agent_key: &str) -> String {
    format!("/agents/{agent_key}/coding/task-status")
}

fn coding_entry_is_initially_hidden(entry_path: &str, selected_path: &str) -> bool {
    let expanded_directories: Vec<_> = selected_path
        .split('/')
        .scan(String::new(), |path, component| {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(component);
            Some(path.clone())
        })
        .collect();
    let mut ancestors = entry_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .into_iter()
        .flat_map(|parent| {
            parent.split('/').scan(String::new(), |path, component| {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(component);
                Some(path.clone())
            })
        });
    ancestors.any(|ancestor| !expanded_directories.iter().any(|path| path == &ancestor))
}

fn coding_file_url(agent_key: &str, path: &str) -> String {
    if path.is_empty() {
        return format!("/agents/{agent_key}/coding");
    }
    format!(
        "/agents/{agent_key}/coding?file={}",
        percent_encode_query_value(path)
    )
}

fn percent_encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => unreachable!("a byte nibble is at most 15"),
    }
}

#[cfg(test)]
mod tests {
    use super::{coding_entry_is_initially_hidden, coding_file_url};

    #[test]
    fn coding_tree_starts_collapsed_except_for_selected_file_ancestors() {
        assert!(!coding_entry_is_initially_hidden("strategies", ""));
        assert!(coding_entry_is_initially_hidden("strategies/trend.py", ""));
        assert!(!coding_entry_is_initially_hidden(
            "strategies",
            "strategies/trend.py"
        ));
        assert!(!coding_entry_is_initially_hidden(
            "strategies/trend.py",
            "strategies/trend.py"
        ));
        assert!(coding_entry_is_initially_hidden(
            "shared/indicators.py",
            "strategies/trend.py"
        ));
    }

    #[test]
    fn coding_file_urls_are_page_relative() {
        assert_eq!(coding_file_url("btc-2", ""), "/agents/btc-2/coding");
        assert_eq!(
            coding_file_url("btc-2", "strategies/trend.py"),
            "/agents/btc-2/coding?file=strategies%2Ftrend.py"
        );
    }
}
