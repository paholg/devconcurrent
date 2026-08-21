# HTTPS and the Proxy

<!-- OUTLINE — replace with prose.
Source: README-old.md "Proxy and HTTPS" (L496–574), heavily reshaped by the
0.0.25 proxy simplification (commits fc598a5, 9ce3d73, 3c1e88c, fc8cd75).

- Motivation: browsers/frameworks dislike non-localhost http.
- The model (much simpler than the old ports array):
  - Every port of a service is reachable raw at its hostname, always —
    postgres at `foo.db.test:5432` needs no configuration.
  - Set `services.<name>.containerPort` to the port your HTTP service
    listens on, and the proxy serves it at the hostname's ports 80 and 443,
    terminating TLS on 443. The service must speak plain HTTP.
  - `containerPort` may not be 80 or 443 (those are the proxy's); the error
    explains this.
- Port-80 behavior: browser navigations (GET/HEAD + Sec-Fetch-Mode:
  navigate, falling back to Accept: text/html) get a 307 to https;
  everything else — curl, fetch/XHR, container-to-container calls, health
  checks — splices through as plain http, since those clients don't trust
  the CA. 307 not 301, so turning TLS off doesn't fight the browser cache.
  With no CA configured, port 80 serves the service directly.
- On 443 the proxy sets X-Forwarded headers and appends
  `Content-Security-Policy: upgrade-insecure-requests`, so apps emitting
  `http://` asset URLs don't hit mixed-content blocks (an app's own CSP is
  kept).
- Certificates: mkcert, `mkcert -install`, manual browser import fallback,
  `proxy.caRoot` in config.toml.
- Enabling: `proxy.enable` plus per-service `containerPort` example.
- Proxy lifecycle (GAP — barely documented today):
  - `dc proxy up` / `dc proxy down` / `dc proxy status`.
  - When you must re-run `dc proxy up` (settings changes; status tells you).
  - Architecture: what actually runs (sidecar container? the SIDECAR status
    column implies one); one proxy across multiple projects/workspaces.
  - Which workspace's config the proxy reads (`--workspace` flag on proxy up).
-->
