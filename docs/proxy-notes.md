# devconcurrent-proxy — user-facing notes

Quick outline of everything a user needs to know. Source material for a README.

## What it does

- A DNS server (`127.0.0.1:43770` by default) that resolves project hostnames to each compose service's container IP.
- A per-service sidecar (created automatically) that joins the target service's network namespace and does TCP-port remapping and/or TLS termination.

DNS pulls the host to the container directly. Sidecars are only created for services that need a port remap or TLS.

## Host setup (required)

Point your system resolver at the proxy for the `.test` TLD (or whatever TLD your `domainName` template ends in).

- **NixOS / systemd-resolved**:
  ```nix
  services.resolved = {
    enable = true;
    extraConfig = ''
      DNS=127.0.0.1:43770
      Domains=~test
    '';
  };
  ```
- **NetworkManager + dnsmasq**: `server=/test/127.0.0.1#43770` in `/etc/NetworkManager/dnsmasq.d/test.conf`.

Verify with `dig @127.0.0.1 -p 43770 anything.<project>.test` — should return the container IP.

## macOS — extra setup required

Container bridge IPs aren't routable from the macOS host out of the box; install a tunnel that bridges the docker network:

- [`chipmk/docker-mac-net-connect`](https://github.com/chipmk/docker-mac-net-connect) (most common), or
- [`mac-net-connect`](https://github.com/AlmightyOatmeal/mac-net-connect) fork, or
- `stack` / equivalent.

Without it, DNS resolves correctly but connections to the container IP time out.

Same DNS setup applies (`/etc/resolver/test` with `nameserver 127.0.0.1` and `port 43770`).

## Optional: TLS

1. Install mkcert and trust the CA:
   ```
   mkcert -install
   ```
2. Find the CA dir:
   ```
   mkcert -CAROOT
   ```
3. Put the path in `~/.config/devconcurrent/config.toml`:
   ```toml
   [proxy]
   caRoot = "/Users/you/Library/Application Support/mkcert"
   ```
4. Mark TLS-enabled ports in your devcontainer config (see below).

The proxy mounts the CAROOT read-only, mints per-service leaf certs in memory, and ships them into each sidecar. The CA private key never leaves the proxy container.

## Per-project config

### Required: label your primary service in `docker-compose.yml`

The proxy identifies a compose project as belonging to a devconcurrent project via the `dev.devconcurrent.project` label on the project's primary service. `dc up` adds this label automatically via a compose override — but **VSCode opens devcontainers directly through compose without going through `dc up`**, so it doesn't get the override.

Add the label explicitly in your project's `docker-compose.yml` so VSCode-launched workspaces are also adopted:

```yaml
services:
  app:
    labels:
      - "dev.devconcurrent.project=<project-name>"
```

`<project-name>` must match the key in your `~/.config/devconcurrent/config.toml` `[projects.…]`.

### Devcontainer config

In `.devcontainer/devcontainer.json`:

```jsonc
"customizations": {
  "devconcurrent": {
    "proxy": {
      "domainName": "{{#unless root}}{{workspace}}.{{/unless}}{{service}}.{{project}}.test",
      "services": {
        "app": {
          "ports": [
            { "host": 443, "container": 3000, "tls": true }
          ]
        }
      }
    }
  }
}
```

- `domainName` is a Handlebars template. Variables: `root` (bool — true when workspace name == project name), `project`, `workspace`, `service`.
- `ports[]` entries:
  - `host` — port the sidecar listens on **inside the target's netns**.
  - `container` — port the sidecar forwards to (`127.0.0.1:<container>`).
  - `tls: true` — sidecar terminates TLS on `host` using a mkcert-signed cert and forwards plaintext to `container`.

`forwardPorts` in the standard devcontainer config also gets routed through the proxy automatically as `{ host: port, container: port }` entries.

## Port-mapping rules

- **`host == container`, no TLS**: silently skipped. DNS routes to the container IP; the app binds the port directly. Sidecar would just race the app for `0.0.0.0:port`.
- **`host == container`, `tls: true`**: errors at `dc proxy up` time. TLS termination needs the cleartext port to live somewhere different from the TLS port.
- **`host != container`**: sidecar binds `host`, forwards to `container`. Standard remap.
- **`tls: true`, `host != container`**: sidecar binds `host` with TLS, forwards plaintext to `container`. Typical: `host: 443, container: 3000`.

### Important: the sidecar binds inside the netns

The sidecar shares the target service's netns and binds `0.0.0.0:<host>`. **The app inside the container cannot bind the same port.** If you have a remap like `host: 80, container: 3000`, your app must bind 3000 (and the sidecar handles 80). The error above for `tls: true && host == container` exists because TLS forcibly needs a sidecar listener and the app can't share the port.

## CLI commands

- `dc proxy up` — start/restart the proxy container and push the current project's config. Always recreates the proxy.
- `dc up` — bring up a workspace; if the project has any proxy config, ensures the proxy is running with the latest config (only recreates when image/binds/env actually changed).
- `dc proxy down` — stop and remove the proxy + all its sidecars.
- `dc proxy status` — show proxy state and currently-tracked services.

## Things to surface in the README

- DNS port (`43770`) is configurable via `proxy.port` in `~/.config/devconcurrent/config.toml`.
- TLS is opt-in per port. No CA → all `tls: true` ports come up disabled with a warning in the proxy log.
- Sidecars die when their target dies (netns-bound). The proxy recreates them when the target restarts via docker start events.
- `dc proxy up` ALWAYS recreates the proxy; cheap. Useful for picking up config changes.
- The sidecar runs the same `devconcurrent-proxy` image as the proxy itself, just with `cmd: ["sidecar"]`. One image to ship.
