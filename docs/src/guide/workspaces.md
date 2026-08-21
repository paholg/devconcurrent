# Workspaces

<!-- OUTLINE — replace with prose.
Source: README-old.md "Workspaces" (L156–172).

- What a workspace is (worktree + optional devcontainer); root workspace.
- `dc up NAME` / `-g`; `dc destroy NAME`; `dc status`; `dc go NAME`.
- `dc up` branch flags (new upstream): by default the branch is named after
  the workspace; `-b/--branch` picks a different branch, `-d/--detach`
  detaches instead of creating a branch, `-x/--exec` runs a command once up.
- Cattle-not-pets: one workspace = one branch = one PR; link to Cache Volumes
  for making creation cheap.
- Alias suggestions (`d` for `dc go`, completions make paths irrelevant).
- GAPS (undocumented today, worth writing here):
  - Branch semantics of `dc up`: from what base; what happens if the branch
    already exists.
  - `dc destroy` safety check: refuses when git is dirty; `--force` overrides.
    (Exists in code, absent from old README.)
  - What operations mean on the root workspace (can you destroy it? does
    `dc up` with no args do anything?).
  - Where worktrees live by default (`worktreeFolder`, platform data dir).
-->
