# Troubleshooting

<!-- OUTLINE — replace with prose. NEW CHAPTER (gap): diagnostics are
scattered through the old README. Symptom-first organization.

- First stop, always: `dc proxy status` (link to Verification chapter for
  column-by-column reading).
- Symptoms:
  - Hostname doesn't resolve → RESOLV column; per-platform DNS setup links;
    macOS dig-vs-dscacheutil trap.
  - Browser warns about certificate → CA not in trust store; TLS column
    caveat; mkcert manual import.
  - Proxy running stale settings → re-run `dc proxy up`.
  - Ports going to the wrong workspace → `dc show ports`, fwd move semantics.
  - Prompt hook "does nothing" → `dc show env` directly; silent outside a
    workspace.
  - git broken inside container → mountGit.
  - Port conflict on `dc up` → static ports in compose (link Project Setup →
    Ports).
  - Lifecycle command failed → since 21cd746 the error names the command as
    written (script, container, user, cwd) and attaches a tail of its
    output; explain how to read it and that "no output" is stated
    explicitly.
  - "why does the browser jump to https but curl stays on http?" → by
    design: only navigations (Sec-Fetch-Mode: navigate / Accept:
    text/html) get the 307; plain tool traffic splices through — link to
    the proxy chapter.
- GAP: where logs live / how to get verbose output from dc itself (does a
  -v/RUST_LOG exist? document it).
- Where to file issues.
-->
