# Devcontainers

<!-- OUTLINE — replace with prose.
Source: README-old.md "Devcontainers" (L174–224). defaultExec was removed
upstream (bc99c14); postAttachCommand now runs (e991ef1).

- Scope statement up front: compose-based only, no features; link to the
  Devcontainer Compatibility chapter for details and rationale.
- How the workspace commands change with a devcontainer:
  - `dc up`: brings up the container, recreates existing, runs lifecycle
    commands. GAP: define "recreate" — what's preserved, what's rebuilt, when
    images rebuild.
  - `dc destroy`: removes containers and volumes.
  - `dc status`: docker info, `--live`, `--workspace`.
  - `dc show`: one-line mention, link to CLI reference.
- New commands:
  - `dc exec` / `dc x`: runs the container's default shell with no
    arguments (defaultExec no longer exists).
  - `dc fwd` / `dc f`: one line, link to Port Forwarding chapter.
  - `dc compose` / `dc c`: passthrough, e.g. `dc c logs -f`.
- Lifecycle commands: all of them run during `dc up`, in spec order —
  initialize → onCreate → updateContent → postCreate → postStart →
  postAttach (host vs container per the spec). Failures name the command as
  written and attach a tail of its output (see Troubleshooting).
- Customizations overview (`customizations.devconcurrent`), where it can live
  (devcontainer.json vs config.toml per-user override) — brief, link to
  reference.
-->
