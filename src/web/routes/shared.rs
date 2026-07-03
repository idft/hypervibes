pub(in crate::web::routes) fn unique_violation_message(error: &anyhow::Error) -> Option<String> {
    let db_err = error.downcast_ref::<sqlx::Error>()?.as_database_error()?;
    if db_err.is_unique_violation() {
        let constraint = db_err.constraint().unwrap_or("unknown");
        if constraint.contains("agent_key") || constraint.contains("agents_pkey") {
            Some("An agent with this agent key already exists.".to_string())
        } else if constraint.contains("agent_runtimes_pkey") {
            Some("A backend with this runtime ID already exists.".to_string())
        } else if constraint.contains("agent_runtimes_name") {
            Some("A backend with this name already exists.".to_string())
        } else if constraint.contains("wallet") {
            Some("An agent with this wallet address and environment already exists.".to_string())
        } else if constraint.contains("api_key") {
            Some("An agent with this API key already exists.".to_string())
        } else {
            Some("This agent conflicts with an existing registry entry.".to_string())
        }
    } else {
        None
    }
}
