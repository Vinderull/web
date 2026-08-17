+++
title = "Diving Bell II"
date = "2026-08-16"
description = "Blog tech dive"
tags = ["pods", "splish-splash", "web"]
+++

# The Diving Bell II

Welcome back, my fellow aquastronauts, for another edition of `The Diving Bell`, where I wax poetically about the infrastructure behind this blog. The last entry covered some `podman` and we will be looking a little closer at that this week, mostly because I cleaned up the rigging and we're looking sea-worthy.


## Sea Trials
First, some updates. As you, dear reader, almost certainly remember from last time, I had deployed my blog as a `distroless` container, mostly because that was the term I knew for a more minimal container setup, and also because it sounded *sexy*. However, after pumping the bilges, I was advised of the existence of `scratch` [images](https://hub.docker.com/_/scratch), a built-in Docker primitive that has nothing that you need and also nothing that you don't need. It is a container after getting [Marie Kondo'ed](https://en.wikipedia.org/wiki/Marie_Kondo). It also fits great with the blog setup. I am `musl` linked already and since I opted for a `caddy` reverse proxy in-front of the blog server, I really don't need the `ssl` certs or any of the timekeeping stuff to do the `https` handshakes. I am also comfortable with all of these posts having a suspiciously consistent posting time of `00:00:00+00:00`. So after writing the last post, I sat there for awhile and decided, [to hell with it, we ball](https://github.com/Vinderull/web/commit/1cb992329f40c081227cd8d851755191abe02968).   
 
An added bonus is it let me drop the `cosign` check on the `distroless` container in CI. This was a *recommended* step by Google, and I am sure most people don't bother at all; however, dropping it teases at one of the more important themes in iterative designs: [ripping shit out is *good*](https://www.goodreads.com/quotes/19905-perfection-is-achieved-not-when-there-is-nothing-more-to). It may one day return to this project, like the [Flying Dutchman](https://en.wikipedia.org/wiki/Flying_Dutchman), but, for now, we have shed some bulk that wasn't buying us much.

## Shatterproof Wine Bottles

I had made the decision to use `podman` mostly out of ergonomics and familiarity. I imagine `docker-compose` would have worked fine, but there are some additional benefits when using `podman` that got me *thirstin'*.

For one, I like the syntax. Here is the `container` file for the `blog`:  

```text
[Unit]
Description=Blog
# Pod= auto-generates Requires=/After= on web-pod.service.
Before=caddy.service

[Container]
Image=ghcr.io/vinderull/web:latest
ContainerName=blog
Pod=web.pod
AutoUpdate=registry
Pull=newer
# Image pulled unauthenticated from the public GHCR package; no auth file.
# Reachable inside the pod at 127.0.0.1:3000; no host publish needed.
User=65532:65532
ReadOnly=true
ReadOnlyTmpfs=false
#DropCapability=all
NoNewPrivileges=true
PodmanArgs=--memory=128m --cpus=1 --pids-limit=20 --ulimit nofile=256:256
# No systemd HealthCmd: the distroless image has no curl and no /bin/sh (podman
# runs string health checks via CMD-SHELL), so one always fails; Notify=healthy
# would wedge the service and keep Caddy (After=blog.service) from starting.
# Caddy self-gates on /healthz (Caddyfile health_uri). Don't re-add HealthCmd.

[Service]
Restart=always
TimeoutStartSec=60

[Install]
WantedBy=default.target
```
Now this lets us do some neat stuff. The big one, for me: `AutoUpdate`. This lets me have an entire deployment pipeline. I have `GitHub` build and push to the `ghcr.io` registry and now my webserver will just check a few times a day if there is a new image available. I don't need to `scp` anything over, or manually restart anything. The `quadlet` service will just track `latest` for me and pull/restart the new container after it gets published.

From here, we can further lock down the container capabilities
```text
ReadOnly=true
ReadOnlyTmpfs=false
#DropCapability=all
NoNewPrivileges=true
```
This means the container now has a read-only `rootfs` and can't gain any new privileges. The container is `scratch`, so there's not much attack surface already, but this is a nice warm, wool blanket on a cold, damp night out at sea. 


For posterity ([future sailors](https://youtu.be/UtRXK5wKZCk?si=PqAPQP5hELjynKA1) love posterity), here is the complementary `caddy.container`:  

```text
[Unit]
Description=Caddy reverse proxy (TLS terminator)
After=blog.service
Wants=blog.service
RequiresMountsFor=/etc/web/Caddyfile
# Pod= auto-generates Requires=/After= on web-pod.service.

[Container]
Image=docker.io/caddy:2.11-alpine
ContainerName=caddy
Pod=web.pod
ReadOnly=true
AutoUpdate=registry
Pull=newer
# DropCapability=all not set: in rootless mode with crun, dropping ALL also
# clears the bounding set which prevents the OCI runtime from exec'ing the
# container process. The user namespace already strips real privileges;
# NET_BIND_SERVICE is covered by the sysctl
# net.ipv4.ip_unprivileged_port_start=80. No AddCapability needed.
# Bind-mount the Caddyfile read-only. On the VPS this is /etc/web/Caddyfile
# (written by the Flatcar Ignition config); locally it's wherever you install it.
Volume=/etc/web/Caddyfile:/etc/caddy/Caddyfile:ro
# Named volumes via the Quadlet .volume units below.
Volume=caddy-data.volume:/data
Volume=caddy-config.volume:/config
PodmanArgs=--memory=128m --cpus=1 --pids-limit=64 --ulimit nofile=4096:4096

[Service]
Restart=always
TimeoutStartSec=60

[Install]
WantedBy=default.target
```

And the `pod` that ties them both together:  

```text
[Unit]
Description=Web app pod (blog + caddy)
# Co-locates the blog app and Caddy reverse proxy in one network namespace.
# Containers reach each other on 127.0.0.1; ports are published by the pod.

[Pod]
PodmanArgs=--memory=384m
PublishPort=80:80
PublishPort=443:443
# note: pod infra container adds ~5MB overhead vs. separate containers on a network.

[Service]
Restart=always
TimeoutStartSec=60

[Install]
WantedBy=default.target
```

Now `podman` ties these processes together such that they now share a localhost network and will map ports `80` and `443` to the host. From here, the Caddyfile configures `caddy` to do its reverse proxy magic for us.
```text
# Replace blog.example.com with your domain.
# Caddy automatically provisions and renews Let's Encrypt TLS certs.
bloginorium.me www.bloginorium.me {
	encode zstd gzip

	header {
		Strict-Transport-Security "max-age=63072000; includeSubDomains; preload"
		-Server
	}

	request_body {
		max_size 1MB
	}

	reverse_proxy 127.0.0.1:3000 {
		health_uri /healthz
		health_interval 5s
	}
}
```
And voilà, we've got [TLS](https://www.cloudflare.com/learning/ssl/transport-layer-security-tls/) certs, some health checks, some header voodoo, and some coupled processes working together to deliver this content into your eyeballs (or screen reader).

All of the container orchestration is expressed in a very `systemd` like way, that reads like most every other `systemd` thing. Additionally, we get `journalctl` logging ([warts and all](https://github.com/systemd/systemd/issues/15292)) for free. Now, I am not some `systemd` evangelist, and I really don't wish to be caught up in the [init](https://www.devuan.org/os/init-freedom) wars, but having a semi-coherent mental model from an admin perspective has some benefits.  

Now, I know, the whole `pod` thing is very [Kubernetes](https://kubernetes.io/docs/concepts/workloads/pods/) [coded](https://knowyourmeme.com/memes/coded-slang) and not `podman` specific, but, I'll be honest, I really haven't had much-if-any exposure to Kubernetes in my travels. My days are spent doing [RTL](https://en.wikipedia.org/wiki/Register-transfer_level) and the [systems programming](https://devopedia.org/systems-programming) that goes with it. I deal with bits, the 0s and 1s, the aughts and the *not*-aughts; none of this highfalutin web development posh.

## Who thought making a ship out of toenails was a good idea

You may have noticed the commented out `#DropCapability=all`. Readers [following along](https://youtu.be/JsntlJZ9h1U?si=3JsyJ7RmtUs0aPDQ) with the source code may have noticed I was originally running the `pod` as a `rootful` [pod](https://github.com/Vinderull/web/commit/b189342e0a5a0666009dcad1e44bb632aba2352d). This was, at the time, the fastest path to getting something stood up. I had found myself spending way more time tinkering with the infrastructure than actually writing the blog posts so I decided [MVP](https://en.wikipedia.org/wiki/Minimum_viable_product) was achieved and it was time to *ship* it. Migrating to `rootless` meant dropping capabilities had some strange interactions with `crun`, particularly with the `Caddy` container (mostly around NET_BIND). Poring over the inner workings of [crun](https://github.com/containers/crun) and [namespaces](https://access.redhat.com/articles/5946151) is way out of scope for this post, but does let me smoothly segue into the next `podman` *thirst intensifier*.

The whole thing is [daemonless](https://docs.podman.io/en/latest/), and while technical concerns about Docker's rootful `daemon` can be [mitigated](https://liudonghua123.github.io/docker-docs/engine/security/rootless/), I prefer my ship to have as few demons as possible and `podman` banishes such things as a matter of default. So, via some [Linux magic](https://wiki.archlinux.org/title/Users_and_groups), we can create a `web` user whose whole existence is to just sit in a small, dark place, and run these services. Like all [unsung heroes](https://youtu.be/tl9nVK5Lx1c?si=6cPeJ0rSCL2hUZZd), it gets inevitably forgotten in favor of [shinier toys](https://katacontainers.io/).

Until [next time](https://en.wikisource.org/wiki/Tales_of_a_Wayside_Inn/Part_Third/The_Theologian%27s_Tale/Elizabeth#IV), my salty-dogs. I may discuss more of the `flatcar` provisioning that allows me to be so *blasé* regarding [DigitalOcean's](https://www.digitalocean.com/) desperate attempts to get me to pay for server backup.