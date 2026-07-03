use askama::Template;

use crate::agents::{
    model::{AgentRuntimeRow, CreateAgentRuntimeForm},
};

#[derive(Template)]
#[template(path = "backends.html")]
pub struct BackendsPageTemplate {
    pub runtimes: Vec<AgentRuntimeRow>,
    pub current_path: String,
}

#[derive(Template)]
#[template(path = "backends_new.html")]
pub struct BackendsNewPageTemplate {
    pub form: CreateAgentRuntimeForm,
    pub errors: Vec<String>,
    pub current_path: String,
}