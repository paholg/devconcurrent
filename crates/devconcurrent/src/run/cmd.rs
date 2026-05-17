use std::borrow::Cow;
use std::path::Path;

use serde::{Deserialize, Serialize};
use vec1::{Vec1, vec1};

use crate::run;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub(crate) enum Cmd {
    Shell(String),
    Args(Vec1<String>),
}

impl Cmd {
    pub(crate) fn as_args(&self) -> Vec<&str> {
        match self {
            Cmd::Shell(prog) => vec!["/bin/sh", "-c", prog],
            Cmd::Args(args) => args.iter().map(|s| s.as_str()).collect(),
        }
    }

    pub(crate) fn description(&self) -> Cow<'_, str> {
        match &self {
            Cmd::Shell(prog) => prog.into(),
            Cmd::Args(vec1) => vec1.join(" ").into(),
        }
    }
}

impl From<std::process::Command> for Cmd {
    fn from(cmd: std::process::Command) -> Self {
        let mut args = vec1![cmd.get_program().to_string_lossy().to_string()];
        args.extend(cmd.get_args().map(|a| a.to_string_lossy().to_string()));

        Self::Args(args)
    }
}

pub(crate) struct NamedCmd<'a> {
    pub(crate) name: &'a str,
    pub(crate) cmd: &'a Cmd,
    pub(crate) dir: Option<&'a Path>,
}

impl run::Runnable for NamedCmd<'_> {
    fn name(&self) -> Cow<'_, str> {
        self.name.into()
    }

    fn description(&self) -> Cow<'_, str> {
        self.cmd.description()
    }

    async fn run(self, _: run::Token) -> eyre::Result<()> {
        let argv = self.cmd.as_args();
        super::run_cmd(&argv, self.dir).await
    }
}
