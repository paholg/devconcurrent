# DNS: Verification

<!-- OUTLINE — replace with prose.
Source: README-old.md "Verification" (L357–400).

- `dc proxy status` first — it's the real tool. Sample output table.
  NOTE: output was reworked in 13a0bd4 — now one line per service; the old
  README's sample (per-port rows, `443→8080`) is stale. Capture a fresh
  sample from a real run.
- Reading the columns: DNS vs RESOLV distinction; stale-settings detection;
  the TLS-column-doesn't-check-trust-store caveat; `--json`; non-zero exit.
- The port-80 check now expects the 307 navigation redirect (3c1e88c), not
  the app's status — worth a sentence so a 307 there doesn't look like a bug.
- Manual checks: `dig +short` on Linux, `dscacheutil` on macOS (and why dig
  lies there).
- `dc show hostname` / `dc show ip`; hostname list comes from compose config,
  typos error out; IPs change on recreation.
- Pointer to Workspace Variables for getting all of these at once.
-->
