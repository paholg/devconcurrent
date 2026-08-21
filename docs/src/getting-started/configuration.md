# Configuration

Devconcurrent requires a small config file, where you must at least list the
projects with which you wish to use it.

Until you create this file, any `devconcurrent` command will return an error
with the expected path; if you are unsure where standard config files live on
your platform, that's the easiest way to see.

The config is a [toml](https://toml.io/en/) file at `devconcurrent/config.toml`
in your platform's standard configuration directory. If you're unsure where that
is, run `devconcurrent` -- it will print an error with the expected file
location.

At a minimum, it must list a project with its location:

```toml
[projects.foo]
path = "~/src/foo"
```

Note: `devconcurrent` has a published config schema. I recommend a tool like
[tombi](https://github.com/tombi-toml/tombi), which will highlight errors and
give you auto-completion.

Most project configuration lives in the project's `devcontainer.json`, in
`customizations.devconcurrent`, but we _also_ let you specify devcontainer
settings here, letting you specify per-user overrides, or even specifying a
full devcontainer configuration for a project that does not have one.

Example:

```toml
[projects.foo]
path = "~/src/foo"
devcontainer = {
  mounts = [{
    type = "bind",
    source = "/my/special/mount",
    target = "its/too/special"
  }]
}
```

See the [Config Reference](../reference/config-toml.md) for the full set of
options.
