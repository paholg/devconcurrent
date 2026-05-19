//! Hostname template rendering.

use handlebars::Handlebars;
use serde::Serialize;
use shared::{DEFAULT_DOMAIN_TEMPLATE, ProxyOptions};

#[derive(Serialize)]
struct TemplateContext<'a> {
    root: bool,
    project: &'a str,
    workspace: &'a str,
    service: &'a str,
}

/// Render the hostname for one (project, workspace, service) tuple using the
/// project's `domainName` template, falling back to the default when unset.
/// Logs and returns `None` if the template fails.
pub fn render_hostname(
    opts: &ProxyOptions,
    project: &str,
    workspace: &str,
    service: &str,
    root: bool,
) -> Option<String> {
    let template_src = opts
        .domain_name
        .as_ref()
        .map(|t| t.source())
        .unwrap_or(DEFAULT_DOMAIN_TEMPLATE);
    let mut hbs = Handlebars::new();
    hbs.set_strict_mode(false);
    let ctx = TemplateContext {
        root,
        project,
        workspace,
        service,
    };
    match hbs.render_template(template_src, &ctx) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(
                project,
                template = %template_src,
                "failed to render domain template: {e}"
            );
            None
        }
    }
}
