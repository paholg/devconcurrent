use std::path::PathBuf;

use serde::Deserialize;

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
