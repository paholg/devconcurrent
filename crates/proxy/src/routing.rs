//! Hostname template rendering.

use handlebars::Handlebars;
use serde::Serialize;
use shared::ProjectProxyConfig;

#[derive(Serialize)]
struct TemplateContext<'a> {
    root: bool,
    project: &'a str,
    workspace: &'a str,
    service: &'a str,
}

/// Render the hostname for one (project, workspace, service) tuple using the
/// project's domain template. Logs and returns `None` if the template fails.
pub fn render_hostname(
    cfg: &ProjectProxyConfig,
    workspace: &str,
    service: &str,
    root: bool,
) -> Option<String> {
    let mut hbs = Handlebars::new();
    hbs.set_strict_mode(false);
    let ctx = TemplateContext {
        root,
        project: &cfg.project,
        workspace,
        service,
    };
    match hbs.render_template(&cfg.domain_template, &ctx) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(
                project = %cfg.project,
                template = %cfg.domain_template,
                "failed to render domain template: {e}"
            );
            None
        }
    }
}
