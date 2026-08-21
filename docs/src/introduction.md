# Introduction

Welcome to `devconcurrent`, your new development environment manager!


## Value Proposition

With git worktrees, you can work on multiple branches of a project in different
directories at the same time.

With devcontainers, you _can_ have isolated development environments, but there
isn't great tooling outside of VS Code, and the reality of ports make this
harder than it should be.

Enter `devconcurrent`, a tool to easily manage worktrees + devcontainers; it can
bring them up, take them down, exec into them, etc all with simple CLI commands.

With `dc up foo` you have a brand new worktree named `foo`, running its
devcontainer, ready for you! Once you're done, just `dc destroy foo`.

On top of that, devconcurrent can give you a DNS server and TLS-terminating proxy.
With a little bit of setup, if you have some web app `app` and are working on
`feature3`, you can view it in your browser at `https://feature3.app.test`!

<!-- OUTLINE — replace with prose.
Source: README-old.md "Value Proposition" (L7–24), "Layers" (L113–128), note (L3–5).

- What devconcurrent is, in two sentences: worktrees + devcontainers + DNS/proxy,
  managed from one CLI.
- The problem: parallel work on one repo is painful. Worktrees solve directories,
  devcontainers solve isolation, but the tooling around both is weak and ports
  make everything collide.
- The "layers" concept (the ogre bit): worktrees only, devcontainers only,
  DNS/proxy only — each layer is optional. One short paragraph per layer with a
  link to its guide chapter.
- The payoff example: `dc up feature3` → https://feature3.app.test.
- Stability note: pre-0.1.0, expect breaking changes.
- Where to go next: Quick Start.
-->
