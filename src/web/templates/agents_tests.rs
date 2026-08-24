use askama::Template;

use crate::{
    agents::model::CreateAgentForm, hyperliquid::live_state::AccountLiveState,
    hyperliquid::queries::AccountTransactionRow,
};

use super::*;
use crate::web::templates::test_support::*;
use chrono::Utc;

#[test]
fn agents_page_renders_base_layout_and_status_box() {
    let entry = AgentListEntry {
        row: sample_agent_list_row(),
        readiness: sample_agent_readiness(),
        account_balance: AccountBalanceView {
            total_balance: Some(rust_decimal::Decimal::new(232_6800, 4)),
            total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
            data_available: true,
        },
        api_key_last_used_iso: None,
    };
    let template = AgentsPageTemplate {
        agents: vec![entry],
        notice: None,
        current_path: "/agents".to_string(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("HyperVibes Agents"));
    assert!(!rendered.contains("Registered agents"));
    assert!(!rendered.contains("Agents persisted in the registry database."));
    assert!(rendered.contains("Account balance"));
    assert!(rendered.contains("232.6800"));
    assert!(!rendered.contains("USDC"));
    assert!(!rendered.contains("OpenCode local"));
    assert!(!rendered.contains(">opencode<"));
    assert!(rendered.contains("<th class=\"px-5 py-3 font-medium\"></th>"));
    assert!(rendered.contains("M10 4v12m-6-6h12"));
    assert!(rendered.contains("Enabled"));
}

#[test]
fn agents_page_renders_loading_placeholder_when_no_balance() {
    let entry = AgentListEntry {
        row: sample_agent_list_row(),
        readiness: sample_agent_readiness(),
        account_balance: AccountBalanceView {
            total_balance: None,
            total_u_pnl: AnimatedNumber::for_pnl(rust_decimal::Decimal::ZERO),
            data_available: false,
        },
        api_key_last_used_iso: None,
    };
    let template = AgentsPageTemplate {
        agents: vec![entry],
        notice: None,
        current_path: "/agents".to_string(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("Loading"));
    assert!(!rendered.contains("232.6800"));
}

#[test]
fn agents_page_renders_empty_state_without_table_or_header_action() {
    let template = AgentsPageTemplate {
        agents: vec![],
        notice: None,
        current_path: "/agents".to_string(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();

    assert!(rendered.contains("No Agents yet"));
    assert!(rendered.contains("Create your first Agent to get started."));
    assert!(rendered.contains("Create a new Agent"));
    assert!(rendered.contains("href=\"/agents/new\""));
    assert!(!rendered.contains("<table"));
    assert!(!rendered.contains("Create agent"));
}

#[test]
fn agents_show_page_renders_base_layout_and_delete_modal() {
    let view = sample_account_balance_view();
    let account_balance_html = AccountBalancePartialTemplate::render_view(view).unwrap();
    let positions_view = OpenPositionsView::from_live_state(AccountLiveState {
        account_address: "0x1234567890abcdef".to_string(),
        environment: "live".to_string(),
        ..Default::default()
    });
    let open_positions_html = OpenPositionsPartialTemplate::render_view(positions_view).unwrap();
    let orders_view = OpenOrdersView::from_live_state(AccountLiveState {
        account_address: "0x1234567890abcdef".to_string(),
        environment: "live".to_string(),
        ..Default::default()
    });
    let open_orders_html = OpenOrdersPartialTemplate::render_view(orders_view).unwrap();
    let sparklines = vec![
        SparklineView::from_series("24h", &[], 240, 48),
        SparklineView::from_series("30d", &[], 240, 48),
    ];
    let sparklines_html = BalanceSparklinesPartialTemplate::render_view(sparklines).unwrap();
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Positions);
    template.account_balance_html = account_balance_html;
    template.open_positions_html = open_positions_html;
    template.open_orders_html = open_orders_html;
    template.latest_trade_execution_summary_html =
        LatestTradeExecutionSummaryPartialTemplate::render_view(
            Some("Scaled out into strength".to_string()),
            Some(Utc::now()),
        )
        .unwrap();
    template.sparklines_html = sparklines_html;
    template.setup_checklist = AgentSetupChecklistView::from_readiness(&sample_agent_readiness());
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("Test Agent · HyperVibes"));
    assert!(rendered.contains("delete-modal"));
    assert!(rendered.contains("Delete agent"));
    assert!(rendered.contains("Agent sections"));
    assert!(rendered.contains("data-agent-tabs"));
    assert!(
        rendered.contains("href=\"/agents/test-agent/chat\" data-agent-tab-link title=\"Chat\"")
    );
    assert!(!rendered.contains(
        "href=\"/agents/test-agent/chat\" data-agent-tab-link hx-get=\"/agents/test-agent/chat\""
    ));
    assert!(rendered.contains("Transactions"));
    assert!(rendered.contains("Memories"));
    assert!(rendered.contains("Prompts"));
    assert!(rendered.contains("Settings"));
    assert!(rendered.contains("Balance"));
    assert!(rendered.contains("Unrealized"));
    assert!(rendered.contains("Scaled out into strength"));
    assert!(!rendered.contains("Agent setup"));
}

#[test]
fn selected_agent_workspace_template_drift_renders_a_settings_warning() {
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Positions);
    template.navbar = Navbar {
        selected_agent_workspace_template_drift: true,
        ..Default::default()
    }
    .with_selected_agent("test-agent".to_string(), "Test Agent".to_string(), true);

    let rendered = template.render().expect("render template");

    assert!(rendered.contains("data-navbar-workspace-template-drift"));
    assert!(rendered.contains("href=\"/agents/test-agent/settings\""));
    assert!(
        rendered
            .contains("title=\"Workspace template drift detected. Review workspace settings.\"")
    );
    assert!(
        rendered.contains(
            "aria-label=\"Workspace template drift detected. Review workspace settings.\""
        )
    );
    assert!(rendered.contains(
        "data-navbar-workspace-template-drift class=\"ml-auto inline-flex h-9 w-9 cursor-pointer"
    ));
}

#[test]
fn positions_page_renders_linked_incomplete_agent_setup_checklist() {
    let mut readiness = sample_agent_readiness();
    readiness.has_enabled_trading_job = false;
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Positions);
    template.setup_checklist = AgentSetupChecklistView::from_readiness(&readiness);

    let rendered = template.render().expect("render setup checklist");

    assert!(rendered.contains("Agent setup"));
    let checklist_index = rendered.find("Agent setup").expect("setup checklist");
    let tab_content_index = rendered
        .find("id=\"agent-show-tab-content\"")
        .expect("HTMX tab content");
    assert!(checklist_index > tab_content_index);
    assert!(rendered.contains("Select currencies to trade"));
    assert!(rendered.contains("BTC is selected by default"));
    assert!(rendered.contains("Enable Trading sub-agent"));
    assert!(rendered.contains("href=\"/agents/test-agent/sub-agents/2?setup=true\""));
    assert!(rendered.contains("href=\"/agents/test-agent/sub-agents/3?setup=true\""));
}

#[test]
fn opencode_agent_shows_jobs_tab_with_recent_runs() {
    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::SubAgents);
    template.jobs_loaded = true;
    template.jobs = vec![
        HarnessSubAgentView::from_row(&sample_candle_job_row(1, "analysis-15m", "analysis", true)),
        HarnessSubAgentView::from_row(&sample_candle_job_row(2, "trading-5m", "trading", false)),
        HarnessSubAgentView::from_row(&sample_event_job_row(3, true)),
    ];
    template.can_enable_all_jobs = true;
    template.can_disable_all_jobs = true;
    template.recent_runs_section.recent_runs_loaded = true;
    template.recent_runs_section.recent_runs = vec![HarnessSubAgentRunView::from_row(
        &sample_run_row(1, "succeeded", "analysis-15m"),
    )];
    let rendered = template.render().expect("render sub-agents tab");
    assert!(rendered.contains("/agents/test-agent/sub-agents"));
    assert!(rendered.contains("Enable all"));
    assert!(rendered.contains("Disable all"));
    assert!(rendered.contains("Recent Runs"));
    assert!(rendered.contains("local-datetime-ready"));
    assert!(rendered.contains("analysis-15m"));
    assert!(rendered.contains("trading-5m"));
    assert!(rendered.contains("market-analysis"));
    assert!(rendered.contains("15m"));
    assert!(rendered.contains("5m"));
    assert!(!rendered.contains(">10m<"));
    assert!(!rendered.contains(">Timeout<"));
    assert!(!rendered.contains(">Delay</th>"));
    let disabled_job_row = rendered
        .split_once("data-row-href=\"/agents/test-agent/sub-agents/2\"")
        .and_then(|(_, remainder)| remainder.split_once("</tr>"))
        .map(|(row, _)| row)
        .expect("render disabled scheduled job row");
    assert!(disabled_job_row.contains("Disabled"));
    assert!(disabled_job_row.contains("—"));
    assert!(!disabled_job_row.contains("local-datetime"));
    let event_job_row = rendered
        .split_once("data-row-href=\"/agents/test-agent/sub-agents/3\"")
        .and_then(|(_, remainder)| remainder.split_once("</tr>"))
        .map(|(row, _)| row)
        .expect("render event job row");
    assert!(event_job_row.contains("analysis batch"));
    assert!(!event_job_row.contains("local-datetime"));
    assert!(rendered.contains("anthropic/claude-3-5-sonnet"));
    assert!(rendered.contains("Run now"));
    assert!(!rendered.contains("Operator prompt</th>"));
    assert!(rendered.contains("/agents/test-agent/sub-agents/1/run"));
    assert!(rendered.contains("/agents/test-agent/sub-agents/3/run"));
    assert!(rendered.contains("/agents/test-agent/sub-agents/3"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
    assert!(rendered.contains("id=\"agent-recent-runs-stream\" hx-ext=\"sse\""));
    assert!(rendered.contains(&format!(
        "sse-connect=\"/agents/{}/sub-agents/recent-runs/stream?page=1\"",
        "test-agent"
    )));
    assert!(rendered.contains("sse-swap=\"recent-runs\""));
    assert!(rendered.contains("sse-swap=\"recent-runs\" hx-swap=\"outerHTML\""));
}

#[test]
fn jobs_page_renders_recent_run_rows() {
    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::SubAgents);
    template.recent_runs_section.recent_runs_loaded = true;
    template.recent_runs_section.recent_runs = vec![
        HarnessSubAgentRunView::from_row(&sample_run_row(1, "succeeded", "analysis-15m")),
        HarnessSubAgentRunView::from_row(&sample_run_row(2, "failed", "trading-5m")),
    ];
    template.recent_runs_section.recent_runs_page = 1;
    template.recent_runs_section.recent_runs_total_pages = 1;
    template.recent_runs_section.recent_runs_total_count = 2;
    template.recent_runs_section.recent_runs_range_start = 1;
    template.recent_runs_section.recent_runs_range_end = 2;
    let rendered = template.render().expect("render jobs page runs section");
    assert!(rendered.contains(">succeeded<"));
    assert!(rendered.contains(">failed<"));
    assert!(rendered.contains("15m"));
    assert!(rendered.contains("42s"));
    assert!(rendered.contains("/agents/test-agent/runs/1"));
    assert!(rendered.contains("Showing 1-2 of 2 runs"));
    assert!(!rendered.contains("data-running-duration"));
}

#[test]
fn jobs_page_renders_running_duration_ticker_markup() {
    let mut row = sample_run_row(3, "running", "analysis-15m");
    row.started_at = Some(Utc::now() - chrono::Duration::seconds(5));
    row.finished_at = None;
    let run = HarnessSubAgentRunView::from_row(&row);
    let fallback = run.duration_text.clone();

    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::SubAgents);
    template.recent_runs_section.recent_runs_loaded = true;
    template.recent_runs_section.recent_runs = vec![run];
    let rendered = template.render().expect("render running jobs page");

    assert!(rendered.contains("data-running-duration"));
    assert!(rendered.contains("data-started-at=\""));
    assert!(rendered.contains(&format!(">{fallback}</span>")));
}

#[test]
fn jobs_page_renders_recent_runs_pagination_controls() {
    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::SubAgents);
    template.recent_runs_section.recent_runs_loaded = true;
    template.recent_runs_section.recent_runs = vec![HarnessSubAgentRunView::from_row(
        &sample_run_row(12, "succeeded", "analysis-15m"),
    )];
    template.recent_runs_section.recent_runs_page = 2;
    template.recent_runs_section.recent_runs_total_pages = 3;
    template.recent_runs_section.recent_runs_total_count = 25;
    template.recent_runs_section.recent_runs_range_start = 11;
    template.recent_runs_section.recent_runs_range_end = 20;
    template.recent_runs_section.recent_runs_previous_page_url =
        Some("/agents/test-agent/sub-agents?page=1".to_string());
    template.recent_runs_section.recent_runs_next_page_url =
        Some("/agents/test-agent/sub-agents?page=3".to_string());
    template.recent_runs_section.stream_url =
        "/agents/test-agent/sub-agents/recent-runs/stream?page=2".to_string();

    let rendered = template.render().expect("render jobs page pagination");

    assert!(rendered.contains("Showing 11-20 of 25 runs"));
    assert!(rendered.contains("Page 2 of 3"));
    assert!(rendered.contains("/agents/test-agent/sub-agents?page=1"));
    assert!(rendered.contains("/agents/test-agent/sub-agents?page=3"));
    assert!(
        rendered
            .contains("sse-connect=\"/agents/test-agent/sub-agents/recent-runs/stream?page=2\"")
    );
    assert!(rendered.contains("id=\"agent-recent-runs\""));
    assert!(rendered.contains("hx-select=\"#agent-recent-runs-stream\""));
    assert!(rendered.contains("hx-target=\"#agent-recent-runs-stream\""));
    assert!(rendered.contains("hx-swap=\"outerHTML\""));
    assert!(rendered.contains("hx-push-url=\"true\""));
}

#[test]
fn transactions_page_renders_pagination_controls() {
    let row = AccountTransactionRow {
        event_time: Utc::now(),
        event_category: "ledger".to_string(),
        symbol: None,
        asset: None,
        fee_usdc: None,
        realized_pnl_usdc: None,
        usdc_delta: Some(rust_decimal::Decimal::new(1, 0)),
        running_balance: Some(rust_decimal::Decimal::new(5, 0)),
    };
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Transactions);
    template.transactions = vec![TransactionView::from_row(row)];
    template.transactions_page = 2;
    template.transactions_total_pages = 3;
    template.transactions_total_count = 125;
    template.transactions_range_start = 51;
    template.transactions_range_end = 100;
    template.transactions_previous_page_url =
        Some("/agents/test-agent/transactions?page=1".to_string());
    template.transactions_next_page_url =
        Some("/agents/test-agent/transactions?page=3".to_string());

    let rendered = template
        .render()
        .expect("render transactions page pagination");

    assert!(rendered.contains("Showing 51-100 of 125 transactions"));
    assert!(rendered.contains("Page 2 of 3"));
    assert!(rendered.contains("/agents/test-agent/transactions?page=1"));
    assert!(rendered.contains("/agents/test-agent/transactions?page=3"));
    assert!(rendered.contains("id=\"agent-transactions\""));
    assert!(rendered.contains("hx-select=\"#agent-transactions\""));
    assert!(rendered.contains("hx-target=\"#agent-transactions\""));
    assert!(rendered.contains("hx-swap=\"outerHTML\""));
    assert!(rendered.contains("hx-push-url=\"true\""));
}

#[test]
fn jobs_page_links_to_new_job_page() {
    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::SubAgents);
    template.jobs_loaded = true;
    let rendered = template.render().expect("render sub-agents page");
    assert!(rendered.contains("New sub-agent"));
    assert!(rendered.contains("/agents/test-agent/sub-agents/new"));
}

#[test]
fn memories_tab_renders_timeline_date_filter_and_markdown_content() {
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Memories);
    let records = [
        sample_memory_record(
            "BTC",
            Some("15m"),
            "analysis",
            "Momentum remains constructive",
            "### Readout\n\n- Wait for a pullback before adding risk.\n- Use patient entries and avoid chasing.",
        ),
        sample_memory_record(
            "ETH",
            None,
            "trade_management",
            "Tighten invalidation",
            "Trail the stop closer if funding flips and spot momentum weakens.",
        ),
    ];
    template.set_memories(
        records.iter().map(crate::memory::MemoryTimelineRecord::from).collect(),
        records.first().cloned(),
        "2026-06-20".to_string(),
        Some("Saturday, June 20, 2026".to_string()),
        None,
        Some("/agents/test-agent/memories/timeline?before_us=1&before_id=00000000-0000-0000-0000-000000000000&date=2026-06-20".to_string()),
    );

    let rendered = template.render().unwrap();
    assert!(rendered.contains("Timeline"));
    assert!(rendered.contains("Showing Saturday, June 20, 2026"));
    assert!(rendered.contains("name=\"date\""));
    assert!(rendered.contains("Momentum remains constructive"));
    assert!(rendered.contains("<h3>Readout</h3>"));
    assert!(rendered.contains("<li>Wait for a pullback before adding risk.</li>"));
    assert!(rendered.contains("metadata keys"));
    assert!(rendered.contains("data-memory-detail-loading"));
    assert!(
        rendered
            .contains("hx-trigger=\"intersect once root:.memory-timeline-scroll threshold:0.5\"")
    );
}

#[test]
fn settings_tab_omits_sync_status_section() {
    let template = AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Settings);

    let rendered = template.render().unwrap();
    assert!(rendered.contains("Settings"));
    assert!(!rendered.contains("Sync status"));
}

#[test]
fn settings_tab_renders_masked_api_key_with_wallet_actions() {
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Settings);
    template.is_main_account = false;
    template.subaccount_name = Some("vt-Test Agent".to_string());

    let rendered = template.render().unwrap();

    assert!(rendered.contains("https://arbiscan.io/address/0x1234567890abcdef"));
    assert_eq!(rendered.matches("data-copy-button").count(), 2);
    assert!(rendered.contains("aria-label=\"Copy trading account address\""));
    assert!(rendered.contains("aria-label=\"Copy API key\""));
    assert!(rendered.contains(">****<"));
    assert!(!rendered.contains("API key last used"));
    assert!(!rendered.contains("Runtime base URL"));
    assert!(rendered.contains("Account address"));
    assert!(!rendered.contains("Wallet address"));
    assert!(rendered.contains("Sub-account: vt-Test Agent"));
    assert!(!rendered.contains("Agent API wallet"));
    assert!(!rendered.contains("API wallet expiry"));
}

#[test]
fn settings_tab_omits_subaccount_label_for_main_account() {
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Settings);
    template.is_main_account = true;

    let rendered = template.render().unwrap();

    assert!(!rendered.contains("Sub-account:"));
}

#[test]
fn settings_tab_renders_workspace_template_drift() {
    let mut template =
        AgentsShowPageTemplate::new(sample_opencode_detail_row(), AgentShowTab::Settings);
    template.opencode_workspace = Some(OpenCodeWorkspaceSettingsView {
        template_drift: OpenCodeWorkspaceTemplateDriftView {
            status_text: "Template drift",
            status_class: "border-amber-900/60 bg-amber-950/30 text-amber-300",
            changed_files: vec![OpenCodeWorkspaceTemplateFileChangeView {
                status_code: "M",
                path: "AGENTS.md".to_string(),
                added_lines: 3,
                removed_lines: 1,
            }],
            is_missing: false,
        },
        maintenance_html: OpenCodeWorkspaceMaintenanceStatusTemplate::render_view(
            OpenCodeWorkspaceMaintenanceView::idle("test-agent"),
        )
        .expect("render maintenance partial"),
        maintenance: OpenCodeWorkspaceMaintenanceView::idle("test-agent"),
    });

    let rendered = template.render().unwrap();
    assert!(rendered.contains("id=\"agent-workspace-section\""));
    assert!(rendered.contains("Template drift"));
    assert!(rendered.contains("AGENTS.md"));
    assert!(rendered.contains("+3"));
    assert!(rendered.contains("-1"));
    assert!(rendered.contains("Only files generated from the workspace template are compared."));
}

#[test]
fn prompts_tab_renders_strategy_copy_and_reset_defaults_ui() {
    let mut template =
        AgentsShowPageTemplate::new(sample_agent_detail_row(), AgentShowTab::Prompts);
    template.set_prompt_editors(vec![
        PromptEditorView::new(
            "analysis",
            "Beep boop analysis.".to_string(),
            crate::agents::prompts::DEFAULT_ANALYSIS_STRATEGY_PROMPT,
        ),
        PromptEditorView::new(
            "market_analysis",
            "Beep boop market analysis.".to_string(),
            crate::agents::prompts::DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT,
        ),
        PromptEditorView::new(
            "trading",
            "Beep boop trading.".to_string(),
            crate::agents::prompts::DEFAULT_TRADING_STRATEGY_PROMPT,
        ),
        PromptEditorView::new(
            "daily_review",
            "Beep boop review.".to_string(),
            crate::agents::prompts::DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT,
        ),
    ]);

    let rendered = template.render().unwrap();
    assert!(rendered.contains("Strategy Prompts"));
    assert!(rendered.contains("Discuss prompt"));
    assert!(rendered.contains("formaction=\"/agents/test-agent/chat/conversations\""));
    assert!(rendered.contains("Analysis"));
    assert!(rendered.contains("Runs on a fixed schedule"));
    assert!(rendered.contains("Market Analysis"));
    assert!(rendered.contains("Runs after analysis sub-agents complete"));
    assert!(rendered.contains("Trading"));
    assert!(rendered.contains("Manages positions and orders based on market-analysis memories"));
    assert!(rendered.contains("Daily Review"));
    assert!(rendered.contains("Directs the daily review"));
    assert!(rendered.contains("data-agent-prompt-form=\"analysis\""));
    assert!(rendered.contains("data-agent-prompt-form=\"market_analysis\""));
    assert!(rendered.contains("data-agent-prompt-form=\"trading\""));
    assert!(rendered.contains("data-agent-prompt-form=\"daily_review\""));
    assert!(rendered.contains("data-agent-prompt-reset=\"analysis\""));
    assert!(rendered.contains("data-agent-prompt-reset=\"market_analysis\""));
    assert!(rendered.contains("data-agent-prompt-reset=\"trading\""));
    assert!(rendered.contains("data-agent-prompt-reset=\"daily_review\""));
    assert!(rendered.contains("data-agent-prompt-save=\"analysis\""));
    assert!(rendered.contains("data-agent-prompt-save=\"market_analysis\""));
    assert!(rendered.contains("data-agent-prompt-save=\"trading\""));
    assert!(rendered.contains("data-agent-prompt-save=\"daily_review\""));
    assert!(rendered.contains("default-analysis-strategy-prompt-value"));
    assert!(rendered.contains("default-market_analysis-strategy-prompt-value"));
    assert!(rendered.contains("Default analysis validity"));
    assert!(rendered.contains("Time-in-force"));
}

#[test]
fn agents_new_page_renders_base_layout_and_form() {
    let template = AgentsNewPageTemplate {
        form: CreateAgentForm::default(),
        choices: TradingAccountChoicesView {
            main_address: "0x0000000000000000000000000000000000000001".to_string(),
            main_balance: Some("100.0000".to_string()),
            main_assigned_to: None,
            subaccounts: Vec::new(),
            subaccount_capacity: Some("10 remaining of 10 total".to_string()),
            lookup_error: None,
        },
        selected_account: String::new(),
        errors: vec![],
        current_path: "/agents/new".to_string(),
        navbar: Navbar::default(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("<!DOCTYPE html>"));
    assert!(rendered.contains("Create New Agent · HyperVibes"));
    assert!(!rendered.contains("The wallet address and app API key"));
    assert!(rendered.contains(">Name</label>"));
    assert!(!rendered.contains("The agent key is derived"));
    assert!(!rendered.contains("Generate new Agent wallet"));
    assert!(!rendered.contains("Import existing Private Key"));
    assert!(
        rendered
            .contains("id=\"display_name\" name=\"display_name\" value=\"\" required autofocus")
    );
    assert!(rendered.contains("display_name"));
    assert!(rendered.contains("trading_account_selection"));
    assert!(rendered.contains("0x0000000000000000000000000000000000000001 · 100.0000"));
    assert!(!rendered.contains("Create new Sub-Account"));
    assert!(rendered.contains("data-create-new-agent-subaccount"));
    assert!(rendered.contains("/agents/new/account-choices"));
    assert!(rendered.contains("Sub-account capacity"));
    assert!(!rendered.contains("Runtime instance"));
    assert!(!rendered.contains("name=\"enabled\""));
}

#[test]
fn trading_account_choices_render_subaccount_names() {
    let template = AgentTradingAccountChoicesTemplate {
        choices: TradingAccountChoicesView {
            main_address: "0x0000000000000000000000000000000000000001".to_string(),
            main_balance: Some("100.0000".to_string()),
            main_assigned_to: None,
            subaccounts: vec![
                crate::web::templates::SubaccountChoiceView {
                    name: Some("vt-BTC".to_string()),
                    address: "0x0000000000000000000000000000000000000002".to_string(),
                    balance: None,
                    assigned_to: None,
                },
                crate::web::templates::SubaccountChoiceView {
                    name: None,
                    address: "0x0000000000000000000000000000000000000003".to_string(),
                    balance: None,
                    assigned_to: None,
                },
                crate::web::templates::SubaccountChoiceView {
                    name: Some("vt-ETH".to_string()),
                    address: "0x0000000000000000000000000000000000000004".to_string(),
                    balance: None,
                    assigned_to: Some(crate::web::templates::SubaccountAssignmentView {
                        agent_key: "eth-agent".to_string(),
                        display_name: "ETH Agent".to_string(),
                    }),
                },
            ],
            subaccount_capacity: None,
            lookup_error: None,
        },
        selected_account: "0x0000000000000000000000000000000000000002".to_string(),
    };
    let rendered = template.render().unwrap();
    assert!(rendered.contains("Sub-account: vt-BTC"));
    assert!(rendered.contains(">Sub-account</span>"));
    assert!(rendered.contains("checked"));
    assert!(rendered.contains("value=\"0x0000000000000000000000000000000000000004\"  disabled"));
    assert!(rendered.contains(
        "In use by <a href=\"/agents/eth-agent\" class=\"underline hover:text-amber-100\">ETH Agent</a>"
    ));
}

#[test]
fn new_job_page_renders_agent_navbar_with_jobs_active() {
    let agent = sample_opencode_detail_row();
    let template = AgentJobNewPageTemplate {
        tabs: build_agent_show_tabs(&agent, AgentShowTab::SubAgents),
        agent_tabs_use_htmx: false,
        agent,
        form: CreateHarnessSubAgentFormValues {
            sub_agent_kind: "analysis".to_string(),
            timeframe: "15m".to_string(),
            timeout_seconds: "900".to_string(),
            model_selection: "anthropic/claude-sonnet-4".to_string(),
            model_variant: String::new(),
            operator_prompt: "Focus on clean continuation setups".to_string(),
            enabled: true,
        },
        model_picker: ModelPickerView {
            input_id: "sub-agent-model-selection".to_string(),
            input_name: "model_selection".to_string(),
            variant_input_id: "sub-agent-model-selection-variant".to_string(),
            variant_input_name: "model_variant".to_string(),
            selected_value: "anthropic/claude-sonnet-4".to_string(),
            selected_variant: String::new(),
            selected_label: "Anthropic / Claude Sonnet 4".to_string(),
            empty_label: "None selected".to_string(),
            provider_groups: vec![ModelPickerProviderGroup {
                provider_id: "anthropic".to_string(),
                provider_name: "Anthropic".to_string(),
                provider_logo_url: Some("/model-catalog/logos/anthropic.svg".to_string()),
                options: sample_model_options(),
            }],
            options: sample_model_options(),
            warning: None,
            show_label: true,
            auto_submit: false,
            use_modal: false,
            submit_on_save: true,
            lazy_options_url: None,
        },
        market_analysis_available: false,
        analysis_coding_available: false,
        show_timeframe: true,
        errors: Vec::new(),
        current_path: "/agents/test-agent/sub-agents/new".to_string(),
        navbar: Navbar::default(),
    };

    let rendered = template.render().expect("render new sub-agent page");

    assert!(rendered.contains("Agent sections"));
    assert!(rendered.contains("agent-rail-initially-collapsed"));
    assert!(rendered.contains("agent-rail agent-rail-expanded"));
    assert!(rendered.contains("data-agent-rail-toggle"));
    assert!(rendered.contains("Expand agent navigation"));
    assert!(rendered.contains("Create sub-agent"));
    assert!(rendered.contains("action=\"/agents/test-agent/sub-agents\""));
}
