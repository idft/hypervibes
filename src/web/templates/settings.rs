use askama::Template;

use super::navbar::Navbar;

#[derive(Template)]
#[template(path = "server_error.html")]
pub struct ServerErrorPageTemplate {
    pub message: String,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsPageTemplate {
    pub system_prompt: String,
    pub current_path: String,
    pub navbar: Navbar,
}
