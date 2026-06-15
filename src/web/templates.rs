use askama::Template;

use crate::{
    agents::model::{AgentDetailRow, AgentListRow, CreateAgentForm},
    hyperliquid::{queries::AccountTransactionRow, sync_state::{SyncStateRow, SyncStatus}},
};

#[derive(Debug, Clone)]
pub struct SummaryCard {
    pub label: &'static str,
    pub value: String,
    pub detail: &'static str,
}

#[derive(Template)]
#[template(path = "agents.html")]
pub struct AgentsPageTemplate {
    pub summary_cards: Vec<SummaryCard>,
    pub agents: Vec<AgentListRow>,
}

#[derive(Template)]
#[template(path = "agents_new.html")]
pub struct AgentsNewPageTemplate {
    pub form: CreateAgentForm,
    pub errors: Vec<String>,
}

#[derive(Template)]
#[template(path = "agents_show.html")]
pub struct AgentsShowPageTemplate {
    pub agent: AgentDetailRow,
    pub transactions: Vec<AccountTransactionRow>,
    pub sync_state: Vec<SyncStateRow>,
}

#[derive(Template)]
#[template(path = "server_error.html")]
pub struct ServerErrorPageTemplate {
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_agent_list_row() -> AgentListRow {
        AgentListRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
        }
    }

    fn sample_agent_detail_row() -> AgentDetailRow {
        let now = Utc::now();
        AgentDetailRow {
            display_name: "Test Agent".to_string(),
            agent_key: "test-agent".to_string(),
            enabled: true,
            prompt: "Beep boop.".to_string(),
            wallet_address: "0x1234567890abcdef".to_string(),
            environment: "live".to_string(),
            api_key: "vt_test_key".to_string(),
            api_key_last_used_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn agents_page_renders_base_layout_and_status_box() {
        let template = AgentsPageTemplate {
            summary_cards: vec![],
            agents: vec![sample_agent_list_row()],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Vibetrading Agents"));
        assert!(rendered.contains("Web server online"));
        assert!(rendered.contains("Registered agents"));
    }

    #[test]
    fn agents_show_page_renders_base_layout_and_delete_modal() {
        let template = AgentsShowPageTemplate {
            agent: sample_agent_detail_row(),
            transactions: vec![],
            sync_state: vec![],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Test Agent · Vibetrading"));
        assert!(rendered.contains("delete-modal"));
        assert!(rendered.contains("Delete agent"));
    }

    #[test]
    fn agents_new_page_renders_base_layout_and_form() {
        let template = AgentsNewPageTemplate {
            form: CreateAgentForm::default(),
            errors: vec![],
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("Create agent · Vibetrading"));
        assert!(rendered.contains("display_name"));
    }

    #[test]
    fn server_error_page_renders_base_layout() {
        let template = ServerErrorPageTemplate {
            message: "Internal server error: boom".to_string(),
        };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("<!DOCTYPE html>"));
        assert!(rendered.contains("500 Server Error"));
        assert!(rendered.contains("Internal server error: boom"));
    }
}
