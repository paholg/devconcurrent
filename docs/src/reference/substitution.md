# Variable Substitution in devcontainer.json

<!-- OUTLINE — replace with prose.
Source: CONFIGURATION-old.md "Variables in devcontainer.json" (L44–73). Port
mostly as-is; it's already reference-shaped. Drop `defaultExec` from the
substituted-properties list and the containerEnv field list — it was removed
upstream (bc99c14).

- Which properties get substitution (the list).
- Available everywhere: localEnv, localWorkspaceFolder(Basename),
  devcontainerId.
- containerWorkspaceFolder(Basename) — everywhere except workspaceFolder.
- containerEnv — only post-container fields; misuse is a named error.
- Unknown `${...}` passes through (shell syntax survives).
-->
