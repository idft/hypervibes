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
    notifications::store::count_notifications,
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AgentShowTab, AgentWorkspacePageTemplate, WorkspaceTreeEntryView, build_agent_show_tabs,
        },
    },
};

use super::show::load_selected_agent_navbar;

#[derive(Default, Deserialize)]
pub(in crate::web::routes) struct WorkspaceQuery {
    #[serde(default)]
    file: String,
}

pub(in crate::web::routes) async fn agents_show_workspace(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let navbar = load_selected_agent_navbar(&state, user.id, &agent).await?.0;
    let selected_path = query.file;
    let listing = state
        .workspace_controller
        .list_workspace_browser_entries(&agent_key)
        .await;
    let (workspace_exists, entries, listing_truncated, mut controller_unavailable) = match listing {
        Ok(listing) => (
            listing.workspace_exists,
            listing.entries,
            listing.truncated,
            false,
        ),
        Err(error) => {
            warn!(agent_key, error = ?error, "workspace browser controller unavailable");
            (false, Vec::new(), false, true)
        }
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
                warn!(agent_key, error = ?error, "workspace file preview unavailable");
                controller_unavailable = true;
            }
        }
    }

    let entries = entries
        .into_iter()
        .map(|entry| WorkspaceTreeEntryView {
            depth: entry.path.matches('/').count(),
            href: workspace_file_url(&agent_key, &entry.path),
            selected: entry.kind == WorkspaceBrowserEntryKind::File && entry.path == selected_path,
            is_directory: entry.kind == WorkspaceBrowserEntryKind::Directory,
            initially_hidden: workspace_entry_is_initially_hidden(&entry.path, &selected_path),
            name: entry
                .path
                .rsplit_once('/')
                .map(|(_, name)| name)
                .unwrap_or(&entry.path)
                .to_string(),
            path: entry.path,
        })
        .collect();
    let current_path = workspace_file_url(&agent_key, &selected_path);
    Ok(Html(
        AgentWorkspacePageTemplate {
            tabs: build_agent_show_tabs(&agent, AgentShowTab::Workspace, notification_count),
            agent_tabs_use_htmx: true,
            agent,
            navbar,
            current_path,
            entries,
            workspace_exists,
            listing_truncated,
            max_entries: WORKSPACE_BROWSER_MAX_ENTRIES,
            max_depth: WORKSPACE_BROWSER_MAX_DEPTH,
            controller_unavailable,
            selected_path,
            preview_text,
            preview_missing,
            preview_binary,
            preview_too_large,
        }
        .render()?,
    )
    .into_response())
}

fn workspace_entry_is_initially_hidden(entry_path: &str, selected_path: &str) -> bool {
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

fn workspace_file_url(agent_key: &str, path: &str) -> String {
    if path.is_empty() {
        return format!("/agents/{agent_key}/workspace");
    }
    format!(
        "/agents/{agent_key}/workspace?file={}",
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
    use super::workspace_entry_is_initially_hidden;

    #[test]
    fn workspace_tree_starts_collapsed_except_for_selected_file_ancestors() {
        assert!(!workspace_entry_is_initially_hidden(".opencode", ""));
        assert!(workspace_entry_is_initially_hidden(".opencode/agents", ""));
        assert!(workspace_entry_is_initially_hidden(
            ".opencode/agents/agent-conversations.md",
            ""
        ));
        assert!(!workspace_entry_is_initially_hidden(
            ".opencode/agents",
            ".opencode/agents/file.md"
        ));
        assert!(!workspace_entry_is_initially_hidden(
            ".opencode/agents/file.md",
            ".opencode/agents/file.md"
        ));
        assert!(workspace_entry_is_initially_hidden(
            ".opencode/commands/file.md",
            ".opencode/agents/file.md"
        ));
    }
}
