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

The image is served from a **public** GHCR package (no auth token to manage or
commit), gated only by the cosign signature in `policy.json`. To build
locally for testing and feed it to the quadlet, build the `runtime`
target tagged to match `Image=`:

```sh
podman build -f .devcontainer/Dockerfile --target runtime -t ghcr.io/vinderull/web:latest .
```

## 2. Install the units

**Rootless** (production — runs as the `web` system user):

The Ignition config writes the units to
`/etc/containers/systemd/users/2000/` (matching the `web` user's pinned
UID 2000). The system manager picks them up and generates user-session
service files. On first boot `enable-linger-web.service` enables lingering
so `web`'s `systemd --user` session starts automatically.

To install on a non-Ignition box (e.g. a dev VM):

```sh
# Create the user-session quadlet directory for UID 2000
sudo mkdir -p /etc/containers/systemd/users/2000
sudo cp quadlet/*.pod quadlet/*.container quadlet/*.volume \
  /etc/containers/systemd/users/2000/
sudo systemctl daemon-reload
# Ensure the sysctl for unprivileged ports is set
sudo sysctl net.ipv4.ip_unprivileged_port_start=80
# Enable lingering for the web user
sudo loginctl enable-linger web
```

## 3. Start
```sh
# rootless — manage via web's user session
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user start web-pod
```

Starting `web-pod` brings up the pod infra container; `caddy` and
`blog` are pulled in via their `Pod=` + `Requires=`/`After=` ordering
(caddy waits for blog, blog waits for the pod).

## 4. Caddyfile target
`Caddyfile` now proxies to `127.0.0.1:3000` — the blog app is reachable on
localhost inside the shared pod namespace. Edit the domain to match yours;
reload Caddy with:

```sh
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user restart caddy
```

## Managing the rootless deployment

All commands run via `web`'s user session:

```sh
# status
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user status web-pod
# logs — as the web user. Rootless quadlet units write to the *system*
# journal tagged _UID=2000, not the per-user journal, so --user misses them.
journalctl -b _UID=2000
# filter by unit afterward if wanted (journalctl's -u can't combine with _UID)
journalctl _UID=2000 -b | grep -iE 'caddy|blog|update'
# restart a container
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user restart blog
```

## Notes / deviations from compose
- **Image source**: compose `build:` is replaced by a registry pull. CI pushes the
  `runtime` image to `ghcr.io/<owner>/web:latest` (plus a versioned `:<tag>`,
  keyless-signed with cosign) when you publish a GitHub Release (against a `v*`
  tag); `blog.container`
  pulls it via `Update=registry`, so a deploy is just
  `systemctl --user restart blog`.
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
- **`DropCapability=all`** on the blog container (scratch binary needs no
  capabilities, and it works because the image's static binary doesn't trigger
  the OCI runtime bounding-set issue). Caddy does not drop all — in rootless
  mode with crun, dropping all caps also clears the bounding set, which
  prevents the OCI runtime from exec'ing the process. The user namespace
  already strips real privileges, so the default caps are harmless. The sysctl
  `net.ipv4.ip_unprivileged_port_start=80` covers port binding instead of
  `NET_BIND_SERVICE`.
- **Healthcheck gating**: the blog app exposes `GET /healthz` → `200 ok` (added
  to `src/main.rs`). The Caddyfile's `reverse_proxy` block uses
  `health_path /healthz` so Caddy only routes to the blog once it's healthy and
  self-heals around a wedged backend. No systemd-side `HealthCmd` (the scratch
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

The image is pulled unauthenticated from the public GHCR package; the `podman`
sysext picks it up automatically, so `Update=registry` pulls succeed out of the
box. The cosign signature in `policy.json` remains the integrity gate.

### 3. Render the Ignition config
`flatcar.bu` (repo root) references the Caddyfile and quadlet units as local
file includes — there is no `auth.json` or credential anywhere in the repo.
Convert it with Butane and pass the result as the instance user-data when
provisioning the VPS:

```sh
# Install Butane: https://flatcar.org/docs/latest/provisioning/config-transpiler/
# -d . resolves the `local:` file includes relative to the repo root.
butane --pretty -d . flatcar.bu -o ignition.json
```

Most providers (Hetzner, DigitalOcean, Equinix, Vultr) accept `ignition.json` as
the user-data / custom Ignition field when booting a Flatcar stable image.
On first boot Ignition:
- Creates the `web` user (UID 2000) with auto-allocated subuid/subgid ranges
- Writes `/etc/web/Caddyfile` and the six quadlet units under
  `/etc/containers/systemd/users/2000/`
- Writes `podman` to `/etc/flatcar/enabled-sysext.conf`
- Creates `/var/lib/containers/storage-web` owned by `web`
- Writes `/etc/containers/storage.conf` (`driver = "overlay"`, `rootless_storage_path` on `/var/lib/containers/storage-web`)
- Enables the `enable-linger-web` oneshot so `web`'s user session starts at boot

Podman is **not** in the Flatcar base image; it is an opt-in system extension
(sysext, available since 3941.0.0). The `enabled-sysext.conf` entry makes Flatcar
download and merge `flatcar-podman` at boot (requires internet on the VPS at
first boot). The extension ships a podman new enough to include the Quadlet
generator. See https://www.flatcar.org/docs/latest/provisioning/sysext/

### 4. Start the pod (image pulls from GHCR)
The quadlet units live under `/etc/containers/systemd/users/2000/`, which
survives Flatcar auto-update reboots. The blog image is pulled from GHCR on
first start (`Update=registry`); the image cache lives in
`/var/lib/containers/storage-web` and also persists.

```sh
# On first boot the enable-linger-web.service has already run. The generated
# user-session services are available; reload and start via web's user session:
ssh user@vps
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user daemon-reload
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user start web-pod
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user status web-pod
# Watch logs
journalctl _UID=2000 -f
```

> If the pull fails with `image not known` / `access denied`, confirm the GHCR
> package (`ghcr.io/vinderull/web`) is public so the unauthenticated pull
> succeeds, and that the cosign signature matches `policy.json`.

### Updating the blog
Publishing a GitHub Release (against a `v*` tag) triggers CI to build, push
(`ghcr.io/vinderull/web:latest` plus a versioned `:<tag>`), and keyless-sign
the image. A production update is then a one-liner — `Update=registry`
re-pulls the `:latest` tag on restart:

```sh
# dev machine — publish a release (creates/pushes the tag + fires CI)
gh release create v1.2.3 --generate-notes
# vps (once CI's deploy job is green)
sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user restart blog
```

To pin a specific release instead of floating on `:latest`, set the tag in
`blog.container` (e.g. `Image=ghcr.io/vinderull/web:v1.2.3` — version tags are
published by the CI `deploy` job alongside `:latest`), copy the unit to
`/etc/containers/systemd/users/2000/`, `daemon-reload`, then `restart`.

### Notes
- **Rootless**: this config runs fully rootless — units in
  `/etc/containers/systemd/users/2000/` (the system manager's user-session
  directory), managed via `sudo -u web XDG_RUNTIME_DIR=/run/user/2000
  systemctl --user`. The sysctl `net.ipv4.ip_unprivileged_port_start=80` allows
  binding 80/443 without capabilities. The `web` user has no SSH access and is
  isolated from the `core` admin account.
- **Persistence**: `/etc`, `/var`, and `/var/lib/containers/storage-web` persist
  across Flatcar auto-update reboots, so the units, Caddyfile, named volumes
  (`caddy-data` / `caddy-config` → TLS certs and ACME leases), and the pulled
  `ghcr.io/vinderull/web:latest` image cache all survive.
- **Podman sysext**: podman is opt-in on Flatcar (not in the base image).
  `flatcar.bu` writes `podman` to `/etc/flatcar/enabled-sysext.conf`, so Flatcar
  pulls and merges `flatcar-podman` at first boot. The extension is versioned
  to the OS and auto-updates with Flatcar releases.
- **Caddyfile changes**: edit `/etc/web/Caddyfile` on the VPS, then
  `sudo -u web XDG_RUNTIME_DIR=/run/user/2000 systemctl --user restart caddy`.
- **Re-provisioning**: `flatcar-reset` (re-provisioning) does not preserve
  `/home` by default, but container storage is now on `/var` at
  `/var/lib/containers/storage-web`, so it survives. Rootless named volumes
  live under that graph root — do not silently point at old rootful volumes;
  re-provision fresh and let Caddy re-issue the cert (or migrate deliberately).
