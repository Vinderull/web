# Quadlet deployment

Translates `docker-compose.yml` into systemd-managed Podman units,
grouping the blog app and Caddy reverse proxy in a single **pod**.

## Units
| File | Replaces |
|------|----------|
| `web.pod` | shared network namespace + published ports 80/443 |
| `blog.container` | `blog` service |
| `caddy.container` | `caddy` service |
| `caddy-data.volume` / `caddy-config.volume` | named `volumes:` |

The `web.network` unit from the non-pod variant is gone — a pod provides its
own internal network, and the two containers reach each other on `127.0.0.1`.

## 1. Image source
`blog.container` references `ghcr.io/vinderull/web:latest` with `Update=registry`,
so Podman pulls the image from GHCR each time the service starts — no local build
step is needed for deployment. CI (`.github/workflows/ci.yml`) builds the
`runtime` image and pushes `ghcr.io/<owner>/web:latest` (plus `:<tag>`) whenever
you publish a GitHub Release (against a `v*` tag). The image is keyless-signed
with cosign; `policy.json` rejects any pull whose signature doesn't match the
`ci.yml@refs/tags/v*` OIDC identity.

For a private GHCR package, authenticate Podman first (`podman login ghcr.io`).
On Flatcar this is handled for you via `quadlet/auth.json` → `/etc/containers/auth.json`
(see the Flatcar section); on a general host, run `podman login ghcr.io` once.

To build locally for testing and feed it to the quadlet, build the `runtime`
target tagged to match `Image=`:

```sh
podman build -f .devcontainer/Dockerfile --target runtime -t ghcr.io/vinderull/web:latest .
```

## 2. Install the units
Rootful (needed for privileged ports 80/443 without extra sysctl):

```sh
sudo cp quadlet/*.pod quadlet/*.container quadlet/*.volume \
  /etc/containers/systemd/
sudo systemctl daemon-reload
```

Rootless alternative (runs as your user):

```sh
mkdir -p ~/.config/containers/systemd
cp quadlet/* ~/.config/containers/systemd/
systemctl --user daemon-reload
# Privileged ports require:
# sudo sysctl net.ipv4.ip_unprivileged_port_start=80
```

## 3. Start
```sh
# rootful
sudo systemctl start web-pod
# rootless
systemctl --user start web-pod
```

Starting `web-pod` brings up the pod infra container; `caddy-container` and
`blog-container` are pulled in via their `Pod=` + `Requires=`/`After=` ordering
(caddy waits for blog, blog waits for the pod).

## 4. Caddyfile target
`Caddyfile` now proxies to `127.0.0.1:3000` — the blog app is reachable on
localhost inside the shared pod namespace. Edit the domain to match yours;
reload Caddy with:

```sh
sudo systemctl restart caddy-container
```

## Notes / deviations from compose
- **Image source**: compose `build:` is replaced by a registry pull. CI pushes the
  `runtime` image to `ghcr.io/<owner>/web:latest` (plus a versioned `:<tag>`,
  keyless-signed with cosign) when you publish a GitHub Release (against a `v*`
  tag); `blog.container`
  pulls it via `Update=registry`, so a deploy is just
  `systemctl restart blog-container`.
- **`expose:`**: omitted — it's informational in compose; reachability comes from
  the shared pod localhost namespace.
- **`depends_on`**: modeled with `Requires=`/`After=`/`Before=` ordering. Caddy
  additionally self-gates via `health_path /healthz` (see below) so it only routes
  to the blog once the app is actually answering.
- **`restart: unless-stopped`** → `Restart=always` (systemd still honors a manual
  `systemctl stop`).
- **ReadOnly root** (`ReadOnly=true`) on both containers — root filesystems are
  read-only; Caddy writes only to the `/data` and `/config` named volumes, the
  blog app writes nothing.
- **`DropCapability=all`** on both containers — drops all Linux capabilities
  Podman would otherwise grant. Caddy's privileged port binding (80/443) still
  works via the pod's rootful context or rootless ambient `CAP_NET_BIND_SERVICE`.
- **Healthcheck gating**: the blog app exposes `GET /healthz` → `200 ok` (added
  to `src/main.rs`). The Caddyfile's `reverse_proxy` block uses
  `health_path /healthz` so Caddy only routes to the blog once it's healthy and
  self-heals around a wedged backend. No systemd-side `HealthCmd` (the distroless
  image has no `curl`; Caddy self-gates, so it's redundant).
- **Pod infra container**: adds ~5MB overhead and a coarser restart boundary
  (an infra restart bounces both containers) vs. the separate-container variant.

## Flatcar / VPS provisioning
The steps above (1–4) assume a general-purpose host where you `cp` the units by
hand. On Flatcar Container Linux there is no package manager and `/etc` is
provisioned at first boot via Ignition — the units and Caddyfile are written in
place by the config, so the manual copy is replaced by a one-time Ignition user
data payload.

`caddy.container` bind-mounts the Caddyfile from `/etc/web/Caddyfile` (a stable
server path, not a repo checkout path), which the Ignition config writes.

### 1. Set your domain
Edit `Caddyfile` and replace `blog.example.com` with your domain. The domain's
DNS A/AAAA records must point at the VPS (Caddy completes an HTTP-01 challenge,
so port 80 must be reachable from the public internet).

### 2. Image is pulled from GHCR (no on-box build)
Flatcar ships no git/rustc toolchain, and it no longer needs one: CI builds the
`runtime` image and pushes it to `ghcr.io/vinderull/web:latest` when you
publish a GitHub Release against a `v*` tag (see
`.github/workflows/ci.yml`). `blog.container` references that
image with `Update=registry`, so Podman pulls it from GHCR when the service
starts — no `podman build`/`save`/`load`/`scp` round-trip is needed.

Authentication to GHCR is handled by `quadlet/auth.json`, which the Ignition config
writes to `/etc/containers/auth.json` (see `flatcar.bu`). The `podman` sysext picks
it up automatically, so `Update=registry` pulls succeed out of the box. (For a
public GHCR package this file is unnecessary; keep it only while the package is
private, and never commit a real token to it.)

### 3. Render the Ignition config
`flatcar.bu` (repo root) references the Caddyfile and quadlet units as local
file includes. Convert it with Butane and pass the result as the instance
user-data when provisioning the VPS:

```sh
# Install Butane: https://flatcar.org/docs/latest/provisioning/config-transpiler/
# -d . resolves the `local:` file includes relative to the repo root.
butane --pretty -d . flatcar.bu -o ignition.json
```

Most providers (Hetzner, DigitalOcean, Equinix, Vultr) accept `ignition.json` as
the user-data / custom Ignition field when booting a Flatcar stable image.
On first boot Ignition writes `/etc/web/Caddyfile` and the six quadlet units
under `/etc/containers/systemd/`, enables `web-pod.service`, writes `podman` to
`/etc/flatcar/enabled-sysext.conf`, and writes `/etc/containers/auth.json`
(GHCR credentials for the `Update=registry` pull).

Podman is **not** in the Flatcar base image; it is an opt-in system extension
(sysext, available since 3941.0.0). The `enabled-sysext.conf` entry makes Flatcar
download and merge `flatcar-podman` at boot (requires internet on the VPS at
first boot). The extension ships a podman new enough to include the Quadlet
generator. See https://www.flatcar.org/docs/latest/provisioning/sysext/

### 4. Start the pod (image pulls from GHCR)
The quadlet units live under `/etc/containers/systemd/`, which survives Flatcar
auto-update reboots. The blog image is pulled from GHCR on first start
(`Update=registry`); the image cache lives in `/var` and also persists.

```sh
ssh user@vps
sudo systemctl daemon-reload        # runs the quadlet generator
sudo systemctl start web-pod        # pulls in blog-container + caddy-container;
                                    # blog-container pulls ghcr.io/vinderull/web:latest
sudo systemctl status web-pod
journalctl -u blog-container -f     # watch the image pull + app boot
journalctl -u caddy-container -f    # watch Caddy obtain the Let's Encrypt cert
```

> If the pull fails with `image not known` / `denied`, confirm
> `/etc/containers/auth.json` has valid GHCR credentials (or that the GHCR
> package is public). It is written by `flatcar.bu` from `quadlet/auth.json`;
> re-render/re-apply the Ignition config if it's stale.

### Updating the blog
Publishing a GitHub Release (against a `v*` tag) triggers CI to build, push
(`ghcr.io/vinderull/web:latest` plus a versioned `:<tag>`), and keyless-sign
the image. A production update is then a one-liner — `Update=registry`
re-pulls the `:latest` tag on restart:

```sh
# dev machine — publish a release (creates/pushes the tag + fires CI)
gh release create v1.2.3 --generate-notes
# vps (once CI's deploy job is green)
sudo systemctl restart blog-container   # re-pulls ghcr.io/vinderull/web:latest
```

To pin a specific release instead of floating on `:latest`, set the tag in
`blog.container` (e.g. `Image=ghcr.io/vinderull/web:v1.2.3` — version tags are
published by the CI `deploy` job alongside `:latest`), copy the unit to
`/etc/containers/systemd/`, `daemon-reload`, then `restart`.

### Notes
- **Rootful**: this config runs rootful (units in `/etc/containers/systemd/`),
  so privileged ports 80/443 bind without any sysctl changes. For rootless, use
  `~/.config/containers/systemd/` + `systemctl --user`, set
  `net.ipv4.ip_unprivileged_port_start=80`, and enable lingering.
- **Persistence**: `/etc` and `/var` persist across Flatcar auto-update reboots,
  so the units, Caddyfile, named volumes (`caddy-data` / `caddy-config` → TLS
  certs and ACME leases), and the pulled `ghcr.io/vinderull/web:latest` image
  cache all survive. The pod auto-starts (`Restart=always` + enabled unit).
- **Podman sysext**: podman is opt-in on Flatcar (not in the base image).
  `flatcar.bu` writes `podman` to `/etc/flatcar/enabled-sysext.conf`, so Flatcar
  pulls and merges `flatcar-podman` at first boot. The extension is versioned
  to the OS and auto-updates with Flatcar releases.
- **Caddyfile changes**: edit `/etc/web/Caddyfile` on the VPS, then
  `sudo systemctl restart caddy-container`.
