+++
title = "Diving Bell I"
date = "2026-08-09"
description = "Blog tech dive"
tags = ["rust", "containers", "boats"]
+++

# The Diving Bell I
Welcome to the Diving Bell. A reccuring series where I discuss some of the technical elements behind this blog. It is like VH1's Behind The Music but for... blog architecture. Also, I keep tinkering and one long blog post would probably be *unreadable.* 

Why Diving Bell? Well, rule one: it sounded cool. Additionally, it will let me drop sea shanty references without an excessive amount of cognitive gymnastics.  

For those of you interested, I don't know how to swim because amphibians can't be trusted. Whoever said irony is dead was a liar.

As with all software projects, the [source code](https://github.com/Vinderull/web) is a pretty good starting place too. So strap into your diving dress and let's descend.

## It's simply bathymetric down here

As referenced in the first post, maybe [don't](https://gohugo.io/) take this approach for a blog. If you use Medium, Substack, or just handspun a static site it will keep you out of the VPS game, the DNS registrar space, deployment pipelines, you'll get email newsletters basically for free. All good things if you are looking for exposure, but much like [GG Allin](https://en.wikipedia.org/wiki/GG_Allin), the only thing we'll be exposing around here is our nautical-themed fluids. No, wait, not like that.

The web app itself is primarily written in Rust. Why?
Online you'll get the usual spiel about how Rust provides:

- Memory safety without garbage collection
- High performance
- Excellent tooling

Wider cultural nitpicks around what memory safety [means](https://doc.rust-lang.org/book/ch15-06-reference-cycles.html) aside, I have really grown to enjoy writing Rust. Just, like, the act of writing, looking at it, and the problems it makes me think about. It scratches some weird itch, right behind the eyes- the sort of itch that might actually be the whispers of some half-mad, antediluvian, [god](https://www.hplovecraft.com/writings/texts/fiction/wid.aspx).

I primarily wrote Linux Kernel [C](https://www.kernel.org/doc/html/latest/process/programming-language.html) for years and was fairly skeptical of [The Cult of Rustacean](https://www.rustacean.net/) as any self-respecting apostate is wont to do. But, over time, I kept telling myself I needed to give it a fair shake, and that, *groan*, I shouldn't knock it until I tried it. Well, consider my boots good and knocked because I am a convert.

The Rust [community](https://github.com/sharkdp/hyperfine) attracts a [certain](https://probe.rs/) level of [craftsmanship](https://github.com/burntsushi/ripgrep), care, and passion. Most of the tools and libraries are a joy to use. Also, since I spend my days in system's programming land, there is more skill crossover for me than Go. One day I'll get into [Zig](https://ziglang.org/), likely once they `1.0.0` that puppy. Until then, I will watch [luminaries](https://andrewkelley.me/post/my-thoughts-bun-rust-rewrite.html) rain Gomorrah levels of fury down; my fellow nerds are out there [straight *gassing*](https://youtu.be/mgxDbkhQY-Q?si=H_8Or0U2CxLTbvzn).


## Pretty in Art Deco

With my mind settled on some sort of Rust server, I was left with some choices about how to host. Most of them were already made for me, because I had settled on [containers](https://www.docker.com/resources/what-container/) already. Containers are nice ways to bundle things up and ship them, they bring promises of separation of concerns, reproducibility, and all the things most major software projects tend to promise but [never quite deliver](https://en.wikipedia.org/wiki/Fundamental_theorem_of_software_engineering). For me, they are a somewhat underutilized tool in my toolbelt and I wanted a chance to use them.

More particularly, I wanted to leverage a [distroless](https://github.com/GoogleContainerTools/distroless/blob/main/README.md) container to wrap it in because *why not.* They sound exotic, mystical, the sort of deep arcana that erudites go blind by beholding.
In reality, they are stripped down, minimal, containers. You see, most containers end up being pretty beefy boys (a technical term) if you aren't careful. Google has done some work to strip everything but the bare minimum from these distroless containers. Minimal is nice, just bring what you need. I am writing a website, dammit, not *glamping*. So in the spirit of minimalism, I settled on the `static` distroless variant; `glibc` can wait in the drydock until needed.

Now, the more experienced, and salty sea-hands, of this audience might be quick to point out:  

*"Ryan, you landlubber, why didn't you do a [scratch](https://hub.docker.com/_/scratch/) image?"*   
And I would reply:   
*"Because I didn't know that was a thing, you scurvy dog, and also I think I might need timezone stuff eventually, but your point is taken and I'll look into it."*  
 
It is this sort of repartee that would gain me considerable clout aboard any sea-faring vessel. 

Either approach, distroless or scratch, is actually pretty easy, it turns out.  We are helped heavily by the Rust tooling/ecosystem. [Cargo](https://doc.rust-lang.org/cargo/) makes building the [musl](https://musl.libc.org/) target *easy.* And for those of us who spent some years supporting legacy products, side-stepping the `libc` version mismatch life is quite nice. A common tripping point is specifying the linker and build setup, so watch out for that, here are the relevant portions of the `.cargo/config.toml`

```toml
[build]
target = "x86_64-unknown-linux-musl"

[target.x86_64-unknown-linux-musl]
linker = "musl-gcc"
```



For those poking around in the code, I have a [devcontainer](https://containers.dev/). They let you do some nice stuff easily and I have found working with them to be pleasant. I spend most of my days on [immutable Linux boxes](https://universal-blue.org/) and the devcontainer flow fits well there. For those of you with a similar love/hate relationship with VSCode, you will be glad to hear that there is a [devcontainer-cli](https://code.visualstudio.com/docs/devcontainers/devcontainer-cli) available which is perfectly serviceable.

 Here is our Dockerfile:

```text
ARG VARIANT="trixie"
ARG MUSL_TARGET="x86_64-unknown-linux-musl"

# ---- Dev stage (used by devcontainer) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS dev
ARG MUSL_TARGET
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add ${MUSL_TARGET}

# ---- Builder stage (compiles release binary) ----
FROM mcr.microsoft.com/devcontainers/rust:2-1-${VARIANT} AS builder
ARG MUSL_TARGET
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add ${MUSL_TARGET}
WORKDIR /build
COPY . .
# Buildkit cache mounts persist the cargo registry + target dir across builds,
# so redeploys only recompile changed crates instead of the full tree.
# The target dir is a cache mount (ephemeral, not in the image layer), so the
# binary is copied out of it within the same RUN; a later COPY can't see it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked --target ${MUSL_TARGET} \
    && cp target/${MUSL_TARGET}/release/web /web

# ---- Runtime stage (for deployment) ----
# Distroless static: no shell, no package manager, no libc (binary is musl-static).
# :nonroot runs as UID 65532 without needing adduser.
FROM gcr.io/distroless/static-debian13:nonroot AS runtime
WORKDIR /app
COPY --from=builder /web /app/web
COPY --from=builder /build/content /app/content
COPY --from=builder /build/static /app/static
ENV CONTENT_DIR=/app/content \
    STATIC_DIR=/app/static
USER nonroot:nonroot
EXPOSE 3000
CMD ["/app/web"]
```

It is a glorious monstrosity that let's us do some nice things. All of our development happens on the same image the release binary gets built in. That is pretty cool. This same image gets used in CI to build and publish, which is also, quite nice. To keep up with the nautical theme, lest you forgot, this is like saying the rum tastes the same whether we are docked in Tortuga or Halifax. Boats!

## Carnival Cruise or Underwater Libertarian Allegory

Now that we are speaking of containers, it is probably worth talking container runtimes. This project started as Docker based, and it still uses the Dockerfile language for expressing everything.

However, because I am a Fedora convert by way of [bluefin](https://projectbluefin.io/), and I have had my software life changed for the better via [distrobox](https://github.com/89luca89/distrobox), I have spent a fair amount of time in and around [podman](https://podman.io/). `podman` brings a lot to the table, and as of right now, the biggest selling point is its [pods,](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/8/html/building_running_and_managing_containers/assembly_working-with-pods_building-running-and-managing-containers) which let us weave together containers so they can share a [namespace](https://en.wikipedia.org/wiki/Linux_namespaces). For us, this means easily coupling our webserver app to a [caddy](https://caddyserver.com/) proxy for easy HTTPS/TLS- no relying on Cloudflare proxies for encryption in my undersea ideological paradise. 

Additionally, `podman` lets us leverage [quadlets](https://www.redhat.com/en/blog/quadlet-podman) which means managing our server deployment is as easy as `sudo systemctl restart web-pod`. I use `systemd` heavily throughout the day, and having the server deployment and [update](https://docs.podman.io/en/v5.0.1/markdown/podman-auto-update.1.html) cadence be driven through `systemd` lets me leverage some pre-existing muscle memory. Here is some free advice, ergonomics matter when being creative- fast iterative cycles are how good work happens; not because you have so many good ideas in your head, but simply because it lets you work through the mountains of bad to mediocre output *faster*. [Prince](https://en.wikipedia.org/wiki/Prince_singles_discography) wrote somewhere between 500-1000 songs and his exceptionality lies in just how many of them were *good*. The road to [good taste](https://www.goodreads.com/quotes/309485-nobody-tells-this-to-people-who-are-beginners-i-wish) is long, but it is ultimately the only road worth taking.

*Phew*, did you guys know that [Kubernetes'](https://kubernetes.io/) logo is a ship's wheel, Docker's mascot is a [whale](https://hub.docker.com/r/docker/whalesay), and podman's mascot is a pod of seals... or [sealions?](https://www.fisheries.noaa.gov/feature-story/it-seal-or-sea-lion) These nautical jokes basically write themselves, people, and that is part of being an artist: getting lucky and passing it off as anything but.