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

## 1. Build the blog image
Quadlet does not build images at unit-load time; build once (rebuild on code change):

```sh
podman build \
  -f .devcontainer/Dockerfile \
  --target runtime \
  -t localhost/blog:latest .
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
- **Image build**: compose `build:` is replaced by an explicit `podman build` step
  (step 1). Re-run it after code changes, then `systemctl restart blog-container`.
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

### 2. Build the blog image (Quadlet does not build images)
Flatcar ships no git/rustc toolchain, so build the distroless runtime image on a
dev machine and ship the OCI archive:

```sh
podman build -f .devcontainer/Dockerfile --target runtime -t localhost/blog:latest .
podman save -o blog.tar localhost/blog:latest
```

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
`/etc/flatcar/enabled-sysext.conf`, and writes `/etc/containers/policy.json`.

Podman is **not** in the Flatcar base image; it is an opt-in system extension
(sysext, available since 3941.0.0). The `enabled-sysext.conf` entry makes Flatcar
download and merge `flatcar-podman` at boot (requires internet on the VPS at
first boot). The extension ships a podman new enough to include the Quadlet
generator. See https://www.flatcar.org/docs/latest/provisioning/sysext/

### 4. Load the blog image and start
The loaded image and the quadlet units live under `/etc` and `/var`, which
survive Flatcar auto-update reboots.

```sh
scp blog.tar user@vps:/var/lib/web/blog.tar
ssh user@vps
sudo podman load -i /var/lib/web/blog.tar
sudo systemctl daemon-reload        # runs the quadlet generator
sudo systemctl start web-pod        # pulls in blog-container + caddy-container
sudo systemctl status web-pod
journalctl -u caddy-container -f    # watch Caddy obtain the Let's Encrypt cert
```

> `podman load` failing with `payload does not match any of the supported image
> formats` almost always means the local archive was **rejected by policy** —
> Flatcar's podman sysext ships a strict `/etc/containers/policy.json`
> (`default: reject`) that blocks the `docker-archive`/`oci-archive`
> transports. The `flatcar.bu` config writes our own `policy.json`
> (`quadlet/policy.json` → `/etc/containers/policy.json`) that permits those
> transports (plus `docker.io/library` for the Caddy pull) before the sysext
> activates, so `podman load` works out of the box. If you skip the Ignition
> policy, the on-box workaround is `sudo podman load
> --signature-policy=/etc/containers/policy.json` after editing it to allow
> `docker-archive`, or build the image on the VPS instead of loading an archive.

### Updating the blog
```sh
# dev machine
podman build -f .devcontainer/Dockerfile --target runtime -t localhost/blog:latest .
podman save -o blog.tar localhost/blog:latest
scp blog.tar user@vps:/var/lib/web/blog.tar
# vps
sudo podman load -i /var/lib/web/blog.tar
sudo systemctl restart blog-container
```

### Notes
- **Rootful**: this config runs rootful (units in `/etc/containers/systemd/`),
  so privileged ports 80/443 bind without any sysctl changes. For rootless, use
  `~/.config/containers/systemd/` + `systemctl --user`, set
  `net.ipv4.ip_unprivileged_port_start=80`, and enable lingering.
- **Persistence**: `/etc` and `/var` persist across Flatcar auto-update reboots,
  so the units, Caddyfile, named volumes (`caddy-data` / `caddy-config` → TLS
  certs and ACME leases), and the loaded `localhost/blog:latest` image all
  survive. The pod auto-starts (`Restart=always` + enabled unit).
- **Podman sysext**: podman is opt-in on Flatcar (not in the base image).
  `flatcar.bu` writes `podman` to `/etc/flatcar/enabled-sysext.conf`, so Flatcar
  pulls and merges `flatcar-podman` at first boot. The extension is versioned
  to the OS and auto-updates with Flatcar releases.
- **Caddyfile changes**: edit `/etc/web/Caddyfile` on the VPS, then
  `sudo systemctl restart caddy-container`.
