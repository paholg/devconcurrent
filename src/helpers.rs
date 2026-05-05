use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;

/// Send a shell command to the calling shell (via the `dc` wrapper function).
///
/// If `DC_SHELL_FD` names an open file descriptor, write the command there and
/// the wrapper will `eval` it. Otherwise print to stdout so the user can copy
/// it (or pipe to `eval` themselves).
pub(crate) fn forward_to_shell(command: &str) -> eyre::Result<()> {
    if let Ok(fd) = std::env::var("DC_SHELL_FD") {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(format!("/dev/fd/{fd}"))?;
        writeln!(f, "{command}")?;
    } else {
        println!("{command}");
    }
    Ok(())
}

pub(crate) fn deserialize_shell_path_opt<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<PathBuf>, D::Error> {
    Option::<String>::deserialize(d)
        .map(|o| o.map(|s| PathBuf::from(shellexpand::tilde(&s).as_ref())))
}

pub(crate) fn deserialize_shell_path<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<PathBuf, D::Error> {
    let s = String::deserialize(d)?;
    Ok(PathBuf::from(shellexpand::tilde(&s).as_ref()))
}
