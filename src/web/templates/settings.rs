use askama::Template;

use super::navbar::Navbar;

#[derive(Template)]
#[template(path = "errors/server-error.html")]
pub struct ServerErrorPageTemplate {
    pub message: String,
    pub current_path: String,
    pub navbar: Navbar,
}

#[derive(Template)]
#[template(path = "errors/not-found.html")]
pub struct NotFoundPageTemplate {
    pub current_path: String,
    pub navbar: Navbar,
}
