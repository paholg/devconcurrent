//! Parser for the `${...}` variable syntax used in devcontainer.json.
//!
//! Spec: https://github.com/devcontainers/spec/blob/main/docs/specs/devcontainerjson-reference.md#variables-in-devcontainerjson
//!
//! Behaviors mirrored from the reference implementation
//! (https://github.com/devcontainers/cli/blob/main/src/spec-common/variableSubstitution.ts):
//!
//! - Single pass, non-recursive: a resolved value is not re-parsed.
//! - Unknown variable names pass through as literal text.
//! - For env-style variables, only the first colon-separated arg is the name and the second
//!   (if present) is the default; further `:`-segments are silently dropped.
//! - For no-arg variables, any provided args are ignored.
//! - Case sensitive; surrounding whitespace inside `${...}` is not tolerated.

use std::fmt;
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, preceded, repeat},
    token::{literal, take_till, take_while},
};

use crate::docker::probe::ContainerData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Variable {
    LocalEnv {
        name: String,
        default: Option<String>,
    },
    ContainerEnv {
        name: String,
        default: Option<String>,
    },
    LocalWorkspaceFolder,
    ContainerWorkspaceFolder,
    LocalWorkspaceFolderBasename,
    ContainerWorkspaceFolderBasename,
    DevcontainerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    Literal(String),
    Var(Variable),
}

#[derive(Debug, Clone)]
pub(crate) struct Context<'a> {
    local_env: IndexMap<String, String>,
    local_workspace_folder: &'a Path,
    container_workspace_folder: &'a Path,
    container: Option<ContainerData>,
}

impl<'a> Context<'a> {
    pub(crate) fn new(
        local_workspace_folder: &'a Path,
        container_workspace_folder: &'a Path,
    ) -> Self {
        Self {
            local_env: std::env::vars().collect(),
            local_workspace_folder,
            container_workspace_folder,
            container: None,
        }
    }

    pub(crate) async fn with_container(mut self, container_id: &str) -> eyre::Result<Self> {
        self.container = Some(ContainerData::inspect(container_id).await?);
        Ok(self)
    }

    #[cfg(test)]
    fn with_local_env(mut self, local_env: IndexMap<String, String>) -> Self {
        self.local_env = local_env;
        self
    }

    #[cfg(test)]
    fn with_container_data(mut self, container: ContainerData) -> Self {
        self.container = Some(container);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Template(pub(crate) Vec<Segment>);

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Variable::LocalEnv {
                name,
                default: None,
            } => write!(f, "${{localEnv:{name}}}"),
            Variable::LocalEnv {
                name,
                default: Some(d),
            } => write!(f, "${{localEnv:{name}:{d}}}"),
            Variable::ContainerEnv {
                name,
                default: None,
            } => write!(f, "${{containerEnv:{name}}}"),
            Variable::ContainerEnv {
                name,
                default: Some(d),
            } => write!(f, "${{containerEnv:{name}:{d}}}"),
            Variable::LocalWorkspaceFolder => f.write_str("${localWorkspaceFolder}"),
            Variable::ContainerWorkspaceFolder => f.write_str("${containerWorkspaceFolder}"),
            Variable::LocalWorkspaceFolderBasename => {
                f.write_str("${localWorkspaceFolderBasename}")
            }
            Variable::ContainerWorkspaceFolderBasename => {
                f.write_str("${containerWorkspaceFolderBasename}")
            }
            Variable::DevcontainerId => f.write_str("${devcontainerId}"),
        }
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for seg in &self.0 {
            match seg {
                Segment::Literal(s) => f.write_str(s)?,
                Segment::Var(v) => write!(f, "{v}")?,
            }
        }
        Ok(())
    }
}

impl Serialize for Template {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Template {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Ok(Template::parse(&s))
    }
}

impl Template {
    pub(crate) fn parse(input: &str) -> Self {
        template
            .parse(input)
            .expect("template parser should be infallible")
    }

    pub(crate) fn render(&self, context: &Context<'_>) -> String {
        let mut out = String::new();
        for segment in &self.0 {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Var(variable) => out.push_str(&variable.evaluate(context)),
            }
        }
        out
    }
}

impl Variable {
    fn evaluate(&self, context: &Context<'_>) -> String {
        let basename = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        match self {
            Variable::LocalEnv { name, default } => {
                env_lookup(&context.local_env, name, default.as_deref())
            }
            Variable::ContainerEnv { name, default } => match &context.container {
                Some(container) => env_lookup(&container.env, name, default.as_deref()),
                None => default.clone().unwrap_or_default(),
            },
            Variable::LocalWorkspaceFolder => {
                context.local_workspace_folder.to_string_lossy().into_owned()
            }
            Variable::ContainerWorkspaceFolder => context
                .container_workspace_folder
                .to_string_lossy()
                .into_owned(),
            Variable::LocalWorkspaceFolderBasename => basename(context.local_workspace_folder),
            Variable::ContainerWorkspaceFolderBasename => {
                basename(context.container_workspace_folder)
            }
            Variable::DevcontainerId => context
                .container
                .as_ref()
                .expect("${devcontainerId} requires Context::with_container")
                .devcontainer_id(),
        }
    }
}

fn env_lookup(env: &IndexMap<String, String>, name: &str, default: Option<&str>) -> String {
    env.get(name)
        .cloned()
        .or_else(|| default.map(str::to_string))
        .unwrap_or_default()
}

fn template(input: &mut &str) -> ModalResult<Template> {
    let segments: Vec<Segment> = repeat(0.., segment).parse_next(input)?;
    Ok(Template(coalesce_literals(segments)))
}

fn segment(input: &mut &str) -> ModalResult<Segment> {
    alt((
        variable.map(Segment::Var),
        literal_chunk.map(Segment::Literal),
    ))
    .parse_next(input)
}

/// Unknown variable names fail this branch so [`literal_chunk`] absorbs them as text.
fn variable(input: &mut &str) -> ModalResult<Variable> {
    let _ = literal("${").parse_next(input)?;
    let name = take_while(0.., |c: char| c.is_ascii_alphabetic()).parse_next(input)?;
    let args: Vec<&str> = repeat(
        0..,
        preceded(literal(":"), take_till(0.., |c: char| c == ':' || c == '}')),
    )
    .parse_next(input)?;
    let _ = literal("}").parse_next(input)?;

    resolve_name(name, &args)
        .ok_or_else(|| winnow::error::ErrMode::Backtrack(winnow::error::ContextError::new()))
}

fn resolve_name(name: &str, args: &[&str]) -> Option<Variable> {
    match name {
        "localEnv" if !args.is_empty() => Some(Variable::LocalEnv {
            name: args[0].to_string(),
            default: args.get(1).map(|s| s.to_string()),
        }),
        "containerEnv" if !args.is_empty() => Some(Variable::ContainerEnv {
            name: args[0].to_string(),
            default: args.get(1).map(|s| s.to_string()),
        }),
        "localWorkspaceFolder" => Some(Variable::LocalWorkspaceFolder),
        "containerWorkspaceFolder" => Some(Variable::ContainerWorkspaceFolder),
        "localWorkspaceFolderBasename" => Some(Variable::LocalWorkspaceFolderBasename),
        "containerWorkspaceFolderBasename" => Some(Variable::ContainerWorkspaceFolderBasename),
        "devcontainerId" => Some(Variable::DevcontainerId),
        _ => None,
    }
}

/// Returns Err on empty so `alt` in [`segment`] backtracks to [`variable`].
fn literal_chunk(input: &mut &str) -> ModalResult<String> {
    let mut out = String::new();
    loop {
        if input.is_empty() {
            break;
        }
        if input.starts_with("${") {
            // If this `${...}` parses as a known variable, leave it for the variable branch.
            // Otherwise absorb the whole `${...}` (up to the next `}` or EOF) as literal text.
            let mut probe = *input;
            if variable.parse_next(&mut probe).is_ok() {
                break;
            }
            let bytes = input.as_bytes();
            let mut end = 2.min(bytes.len());
            while end < bytes.len() && bytes[end] != b'}' {
                end += 1;
            }
            if end < bytes.len() {
                end += 1;
            }
            out.push_str(&input[..end]);
            *input = &input[end..];
            continue;
        }
        let next = input.chars().next().unwrap();
        out.push(next);
        *input = &input[next.len_utf8()..];
    }
    if out.is_empty() {
        Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ))
    } else {
        Ok(out)
    }
}

/// Merges adjacent `Literal` segments produced by back-to-back unknown-`${...}` runs.
fn coalesce_literals(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    for seg in segments {
        match (out.last_mut(), seg) {
            (Some(Segment::Literal(prev)), Segment::Literal(next)) => prev.push_str(&next),
            (_, seg) => out.push(seg),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Segment {
        Segment::Literal(s.to_string())
    }

    fn var(v: Variable) -> Segment {
        Segment::Var(v)
    }

    fn string_map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    struct ContextBuilder {
        local_env: IndexMap<String, String>,
        local_workspace_folder: std::path::PathBuf,
        container_workspace_folder: std::path::PathBuf,
        container: Option<ContainerData>,
    }

    impl ContextBuilder {
        fn new() -> Self {
            Self {
                local_env: IndexMap::new(),
                local_workspace_folder: std::path::PathBuf::new(),
                container_workspace_folder: std::path::PathBuf::new(),
                container: None,
            }
        }

        fn local_env(mut self, pairs: &[(&str, &str)]) -> Self {
            self.local_env = string_map(pairs);
            self
        }

        fn local_workspace_folder(mut self, path: &str) -> Self {
            self.local_workspace_folder = path.into();
            self
        }

        fn container_workspace_folder(mut self, path: &str) -> Self {
            self.container_workspace_folder = path.into();
            self
        }

        fn container(mut self, env: &[(&str, &str)], labels: &[(&str, &str)]) -> Self {
            self.container = Some(ContainerData {
                env: string_map(env),
                labels: string_map(labels),
            });
            self
        }

        fn build(&self) -> Context<'_> {
            let mut context =
                Context::new(&self.local_workspace_folder, &self.container_workspace_folder)
                    .with_local_env(self.local_env.clone());
            if let Some(ref container) = self.container {
                context = context.with_container_data(container.clone());
            }
            context
        }
    }

    fn render_with(input: &str, builder: ContextBuilder) -> String {
        Template::parse(input).render(&builder.build())
    }

    #[test]
    fn empty_string() {
        assert_eq!(Template::parse("").0, vec![]);
    }

    #[test]
    fn pure_literal() {
        assert_eq!(Template::parse("hello world").0, vec![lit("hello world")]);
    }

    #[test]
    fn lone_dollar_is_literal() {
        assert_eq!(Template::parse("price: $5").0, vec![lit("price: $5")]);
    }

    #[test]
    fn local_env_no_default() {
        assert_eq!(
            Template::parse("${localEnv:HOME}").0,
            vec![var(Variable::LocalEnv {
                name: "HOME".to_string(),
                default: None,
            })]
        );
    }

    #[test]
    fn local_env_with_default() {
        assert_eq!(
            Template::parse("${localEnv:HOME:/tmp}").0,
            vec![var(Variable::LocalEnv {
                name: "HOME".to_string(),
                default: Some("/tmp".to_string()),
            })]
        );
    }

    #[test]
    fn extra_colons_dropped() {
        assert_eq!(
            Template::parse("${localEnv:HOME:def:extra}").0,
            vec![var(Variable::LocalEnv {
                name: "HOME".to_string(),
                default: Some("def".to_string()),
            })]
        );
    }

    #[test]
    fn no_arg_variables() {
        assert_eq!(
            Template::parse("${localWorkspaceFolder}").0,
            vec![var(Variable::LocalWorkspaceFolder)]
        );
        assert_eq!(
            Template::parse("${devcontainerId}").0,
            vec![var(Variable::DevcontainerId)]
        );
    }

    #[test]
    fn no_arg_variable_ignores_args() {
        assert_eq!(
            Template::parse("${localWorkspaceFolder:foo}").0,
            vec![var(Variable::LocalWorkspaceFolder)]
        );
    }

    #[test]
    fn cross_platform_home() {
        assert_eq!(
            Template::parse("${localEnv:HOME}${localEnv:USERPROFILE}").0,
            vec![
                var(Variable::LocalEnv {
                    name: "HOME".to_string(),
                    default: None,
                }),
                var(Variable::LocalEnv {
                    name: "USERPROFILE".to_string(),
                    default: None,
                }),
            ]
        );
    }

    #[test]
    fn mixed_template_parse() {
        assert_eq!(
            Template::parse("${localWorkspaceFolder}/.cache/${localEnv:USER}").0,
            vec![
                var(Variable::LocalWorkspaceFolder),
                lit("/.cache/"),
                var(Variable::LocalEnv {
                    name: "USER".to_string(),
                    default: None,
                }),
            ]
        );
    }

    #[test]
    fn unknown_variable_is_literal() {
        assert_eq!(
            Template::parse("${nope:foo} after").0,
            vec![lit("${nope:foo} after")]
        );
    }

    #[test]
    fn whitespace_inside_braces_unrecognized() {
        assert_eq!(
            Template::parse("${ localEnv:HOME }").0,
            vec![lit("${ localEnv:HOME }")]
        );
    }

    #[test]
    fn case_sensitive() {
        assert_eq!(
            Template::parse("${LocalEnv:HOME}").0,
            vec![lit("${LocalEnv:HOME}")]
        );
    }

    #[test]
    fn unterminated_brace_is_literal() {
        assert_eq!(
            Template::parse("${localEnv:HOME").0,
            vec![lit("${localEnv:HOME")]
        );
    }

    #[test]
    fn local_env_without_arg_is_unknown() {
        // Reference impl throws here; we pass through as literal instead.
        assert_eq!(Template::parse("${localEnv}").0, vec![lit("${localEnv}")]);
    }

    #[test]
    fn empty_arg() {
        assert_eq!(
            Template::parse("${localEnv:}").0,
            vec![var(Variable::LocalEnv {
                name: String::new(),
                default: None,
            })]
        );
    }

    #[test]
    fn back_to_back_unknowns() {
        assert_eq!(Template::parse("${a}${b}").0, vec![lit("${a}${b}")]);
    }

    #[test]
    fn unknown_then_known() {
        assert_eq!(
            Template::parse("${a}${localWorkspaceFolder}").0,
            vec![lit("${a}"), var(Variable::LocalWorkspaceFolder)]
        );
    }

    #[test]
    fn render_local_env_present() {
        assert_eq!(
            render_with(
                "${localEnv:HOME}",
                ContextBuilder::new().local_env(&[("HOME", "/home/me")]),
            ),
            "/home/me",
        );
    }

    #[test]
    fn render_local_env_missing_uses_default() {
        assert_eq!(
            render_with("${localEnv:X:fallback}", ContextBuilder::new()),
            "fallback",
        );
    }

    #[test]
    fn render_local_env_missing_no_default_is_empty() {
        assert_eq!(render_with("${localEnv:X}", ContextBuilder::new()), "");
    }

    #[test]
    fn render_container_env() {
        assert_eq!(
            render_with(
                "${containerEnv:PATH}",
                ContextBuilder::new().container(&[("PATH", "/usr/bin")], &[]),
            ),
            "/usr/bin",
        );
    }

    #[test]
    fn render_workspace_folders() {
        let b = ContextBuilder::new()
            .local_workspace_folder("/host/projects/myrepo")
            .container_workspace_folder("/workspaces/myrepo");
        assert_eq!(
            render_with("${localWorkspaceFolder}", b.clone_for_test()),
            "/host/projects/myrepo",
        );
        assert_eq!(
            render_with("${localWorkspaceFolderBasename}", b.clone_for_test()),
            "myrepo",
        );
        assert_eq!(
            render_with("${containerWorkspaceFolder}", b.clone_for_test()),
            "/workspaces/myrepo",
        );
        assert_eq!(
            render_with("${containerWorkspaceFolderBasename}", b),
            "myrepo",
        );
    }

    #[test]
    fn render_devcontainer_id() {
        let labels = &[("devcontainer.local_folder", "/foo")];
        let expected = ContainerData {
            env: IndexMap::new(),
            labels: string_map(labels),
        }
        .devcontainer_id();
        assert_eq!(
            render_with(
                "${devcontainerId}",
                ContextBuilder::new().container(&[], labels),
            ),
            expected,
        );
    }

    #[test]
    #[should_panic(expected = "${devcontainerId} requires Context::with_container")]
    fn render_devcontainer_id_panics_without_container() {
        let _ = render_with("${devcontainerId}", ContextBuilder::new());
    }

    #[test]
    fn render_extra_colons_dropped_in_default() {
        assert_eq!(
            render_with("${localEnv:X:def:extra}", ContextBuilder::new()),
            "def",
        );
    }

    #[test]
    fn render_mixed_template() {
        assert_eq!(
            render_with(
                "${localWorkspaceFolder}/.cache/${localEnv:USER}",
                ContextBuilder::new()
                    .local_env(&[("USER", "paho")])
                    .local_workspace_folder("/work/myrepo"),
            ),
            "/work/myrepo/.cache/paho",
        );
    }

    #[test]
    fn render_unknown_variable_passes_through() {
        assert_eq!(
            render_with("hello ${nope:foo}!", ContextBuilder::new()),
            "hello ${nope:foo}!",
        );
    }

    #[test]
    fn render_cross_platform_home() {
        // USERPROFILE unset → "" → HOME wins.
        assert_eq!(
            render_with(
                "${localEnv:HOME}${localEnv:USERPROFILE}",
                ContextBuilder::new().local_env(&[("HOME", "/home/me")]),
            ),
            "/home/me",
        );
    }

    impl ContextBuilder {
        fn clone_for_test(&self) -> ContextBuilder {
            ContextBuilder {
                local_env: self.local_env.clone(),
                local_workspace_folder: self.local_workspace_folder.clone(),
                container_workspace_folder: self.container_workspace_folder.clone(),
                container: self.container.clone(),
            }
        }
    }
}
