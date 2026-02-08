use bollard::Docker;

pub fn client() -> eyre::Result<Docker> {
    Ok(Docker::connect_with_local_defaults()?)
}
