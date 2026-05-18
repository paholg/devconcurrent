use std::net::{IpAddr, Ipv4Addr};

use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de};

/// Default hostname template.
pub(crate) const DEFAULT_TEMPLATE: &str =
    "{{#unless root}}{{workspace}}.{{/unless}}{{service}}.{{project}}.test";

/// Per-project proxy configuration.
///
/// Ports from `forwardPorts` are picked up automatically
#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ProxyOptions {
    /// Handlebars template for the proxied domain name.
    ///
    /// The default TLD (.test) is one guaranteed to never be registered on the
    /// internet, but whatever you use, you will need a DNS entry to point to
    /// the devconcurrent proxy.
    ///
    /// Available variables:
    /// - `root` (bool) — whether this is the root workspace
    /// - `project` — project name
    /// - `workspace` — workspace name
    /// - `service` — name of the service from compose
    ///
    /// Default:
    /// ```
    /// {{#unless root}}{{workspace}}.{{/unless}}{{service}}.{{project}}.test
    /// ```
    pub(crate) domain_name: Option<Template>,

    /// Per-compose service configuration.
    ///
    /// In addition, any ports specified in `forwardPorts` will be proxied.
    pub(crate) services: IndexMap<String, ProxyService>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ProxyService {
    /// Port mappings.
    pub(crate) ports: Vec<ProxyPort>,
}

/// Port mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ProxyPort {
    /// The IP address to listen on. Defaults to 0.0.0.0, allowing traffic in
    /// from any source.
    #[serde(default = "default_ip")]
    pub(crate) ip: IpAddr,
    pub(crate) host: u16,
    pub(crate) container: u16,
}

fn default_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

/// A Handlebars hostname template, compiled at deserialization time so
/// syntax errors surface as config-load errors rather than at first use.
#[derive(Clone, Debug)]
pub(crate) struct Template {
    source: String,
    compiled: handlebars::Template,
}

impl Template {
    // Consumed by `devconcurrent-service` once it lands; allowed for now.
    #[allow(dead_code)]
    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    #[allow(dead_code)]
    pub(crate) fn compiled(&self) -> &handlebars::Template {
        &self.compiled
    }

    fn compile(source: String) -> Result<Self, handlebars::TemplateError> {
        let compiled = handlebars::Template::compile(&source)?;
        Ok(Self { source, compiled })
    }
}

impl Default for Template {
    fn default() -> Self {
        Self::compile(DEFAULT_TEMPLATE.to_string())
            .expect("default template is a valid Handlebars template")
    }
}

// The compiled AST is derived from `source`, so equality on source is
// equivalent.
impl PartialEq for Template {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Template {}

impl<'de> Deserialize<'de> for Template {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::compile(s).map_err(de::Error::custom)
    }
}

impl Serialize for Template {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl JsonSchema for Template {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProxyHostnameTemplate".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description":
                "Handlebars template for the proxied hostname. \
                Variables: `root` (bool), `project`, `workspace`, `service`.",
        })
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn deserializes_valid_template() {
        let t: Template = serde_json::from_str("\"{{project}}.test\"").unwrap();
        assert_eq!(t.source(), "{{project}}.test");
    }

    #[test]
    fn rejects_invalid_template() {
        assert!(serde_json::from_str::<Template>("\"{{#unclosed\"").is_err());
    }

    #[test]
    fn default_matches_constant() {
        assert_eq!(Template::default().source(), DEFAULT_TEMPLATE);
    }

    #[test]
    fn round_trips_through_string() {
        let input = "\"{{workspace}}.{{project}}.test\"";
        let t: Template = serde_json::from_str(input).unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), input);
    }
}
