# Reference: customizations

<!-- OUTLINE — replace this comment with prose; the option list below is
generated and stays.

- Where these live: devcontainer.json (repo-wide) or
  projects.NAME.devcontainer (per-user), under
  `customizations.devconcurrent`.
- Worth prose beyond the generated list: containerPort may not be 80/443
  (the proxy's own ports; the error explains), and env templates render
  strictly (link to Templates).
- The generated section below comes from the config JSON schema
  (`just gen`); improve it by editing doc comments on `DcOptions` in
  crates/cli/src/devcontainer/dc_options.rs and the types in crates/shared.
-->

{{#include generated/customizations.md}}
