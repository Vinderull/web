+++
title = "Diving Bell III"
date = "2026-09-09"
description = "Blog tech dive"
tags = ["landlock", "barnacles", "web"]
+++

# The Diving Bell III

Ahoy, my fine, sea-legged aristocrats. Welcome back for another post about the bowels of the blog architecture and all the weird choices I made when deciding to build a web service. First, let me regale you with some of the learnings from my last journey at sea. 


## Sea Trials II
I love to tinker, it comes with the territory. As a result, the blog is living breathing thing, or, really, a [ship of Theseus](https://en.wikipedia.org/wiki/Ship_of_Theseus) or a swanky [Amphicar](https://silodrome.com/amphicar-history/). Here are some of the more significant changes made since our last debrief.

### Blanket for a Sail
For those  of you who are blissfully unaware, every device reachable on the public internet is under constant siege from [bots](https://radar.cloudflare.com/traffic#bot-vs-human) doing all sorts of scraping and vulnerability scanning. While some are looking for admin consoles to extract secrets and steal your doubloons, I suspect a good chunk are automated tooling looking to build up [botnets](https://en.wikipedia.org/wiki/Cyber-arms_industry#Online) so criminals/state-actors can rent out their armada to the highest bidder. Their are no laws out at sea, so it best to be prepared.

After taking a look at some logs, I noticed a lot of traffic flailing at my blog with POSTs for database access and admin access. Thankfully, given container deploy nature of the website, I don't have any of that and the blog is completely read only. I went ahead and added some better handling for when roustabouts attempt to use unsupported HTTP methods on my website. This was a two-fold solution, I added some header magic to Caddy:  
```text
bloginorium.me www.bloginorium.me {
    # Security headers apply to both proxied and method-rejection responses.
    header {
        Strict-Transport-Security "max-age=63072000; includeSubDomains; preload"
        -Server
    }

    # The blog is strictly read-only. Only GET and HEAD enter the proxy path;
    # every other method is handled by the mutually exclusive fallback below.
    @allowed-method method GET HEAD
    handle @allowed-method {
        encode zstd gzip

        request_body {
            max_size 1MB
        }

        reverse_proxy 127.0.0.1:3000 {
            health_uri /healthz
            health_interval 5s
        }
    }

    handle {
        header Allow "GET, HEAD"
        respond 405
    }
}
```
This does the job and will [405](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Status/405) the sharks sniffing for chum doesn't exist. The bots won't really care, but it is a little more graceful than the [404](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Status/404) they would get previously. The `404` also lead to some log spam since the connection would close and `caddy` would log the termination leading to some alarming, but benign, red messages in the logs.

I figured, while scrapping off the barnacles, I might as well make the web server itself handle this gracefully instead of 404'ing:
```diff
@@ -12,8 +12,9 @@
 use axum::{
     Router,
     body::{Body, Bytes},
-    extract::{Path as AxumPath, Query, State},
-    http::{HeaderMap, HeaderValue, StatusCode, header},
+    extract::{Path as AxumPath, Query, Request, State},
+    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
+    middleware::{self, Next},
     response::{IntoResponse, Redirect, Response},
     routing::get,
 };
@@ -88,6 +89,23 @@
     feed_etag: HeaderValue,
 }

+/// Reject every method but GET and HEAD before routing.
+///
+/// The router only serves GET (plus HEAD via Axum's automatic GET handling),
+/// so any other method gets a bodyless 405 with an explicit `Allow` header
+/// instead of reaching a route or the 404 fallback.
+async fn method_guard(req: Request, next: Next) -> Response {
+    if matches!(*req.method(), Method::GET | Method::HEAD) {
+        next.run(req).await
+    } else {
+        (
+            StatusCode::METHOD_NOT_ALLOWED,
+            [(header::ALLOW, "GET, HEAD")],
+        )
+            .into_response()
+    }
+}
+
 /// Build the application router from loaded posts and a static-asset dir.
 ///
 /// Pre-renders the index, every post page, the tag index, and every tag page
@@ -221,7 +239,8 @@
         .route("/healthz", get(|| async { "ok" }))
         .nest_service("/static", ServeDir::new(static_dir))
         .fallback(not_found)
-        .with_state(state))
+        .with_state(state)
+        .layer(middleware::from_fn(method_guard)))
 }
```
I am a greenhorn when it comes to web development, but `middleware`, in this context, refers to Axum's builtin web service layers that allow us to hijack the HTTP request before it reaches our routes. This allows us to inspect whether this is a supported method, and if not, pack a `405` into the response. It is sort of like the ship's postmaster editing out, at receive time, all of the infidelity and family drama from the sailor's letters before delivering to the crew. We've got a ship to run; we're not filming an episode of the [Real World](https://www.youtube.com/playlist?list=PLs7rUK1K9_QjV5CZJizAlwStzPsUOiNpy)- leave that stuff on the shore. 

### Land Locked Blues
One of the, nearly inconsequential, but dear to my heart changes, was the tightening of the [Landlock](https://landlock.io/) policy. For those who haven't been more than 10 leagues from the sea in sometime, Landlock is one of the (many) [Linux Security Modules](https://docs.kernel.org/admin-guide/LSM/index.html) exposed by the kernel. On Linux, any process inherits all of the [permissions](https://prakash4844.github.io/Let-s-Learn-Linux/1.-grasshopper/6.-permissions/7.-process-permissions/index.html) as the user who started the process. This has historically been convenient but in the modern day has proved problematic for security reasons. Any process started by user `ishmael` can read/write to any file that `ishmael` in addition to any other user capabilities. If you didn't take the Landlock link/bait earlier, the brief is it allows userspace applications to seal themselves off from system access, even if they are an unprivileged process. This is, in some ways, a mirror of OpenBSD's [unveil](https://man.openbsd.org/unveil.2) and [pledge](https://man.openbsd.org/pledge.2). There is some real eloquence to those OpenBSD system calls that, *checks notes*, shivers my timbers, and I have been looking for a reason to leverage the Linux Landlock equivalent for sometime. Since, we are running in Rust, using Landlock is fairly [trivial](https://docs.rs/landlock/latest/landlock/) so it was a design goal at outset of this project to use it.

Landlock has been in use since day one, but the biggest change recently was that I was able to lock down filesystem access completely by baking some bytes directly in to the binary. I'll be honest, I stumbled on [this](https://github.com/Vinderull/web/commit/33a2844d548389a89de800dea1c7937721a0069e) path by going back in forth with AI about what dependencies were doing what and whether they were worth using at all. I hadn't really considering baking content bytes into the binary, but it made a certain kind of crooked sense once I stewed on it. I was previously serving `static_dir` content using `tower-http` and the stuff in there was the `CSS` page and the `htmx` script, with the `favicon` SVG being a recent addition. I had no real intention of ever adding *more* stuff to that directory and those artifacts are reasonably sized as is, so to hell with it, pack those bytes:

```rust
// The three static assets are embedded into the binary at compile time, so the
// request path never touches the filesystem: `include_bytes!` bakes the exact
// vendored source bytes in, and each route serves them from RAM with a static,
// correct content type. htmx's `.min.js` bytes are thus the exact SRI-pinned
// release from scripts/update-htmx.sh, byte for byte.
const STATIC_CSS: &[u8] = include_bytes!("../static/css/main.css");
const STATIC_JS: &[u8] = include_bytes!("../static/js/htmx.min.js");
const STATIC_FAVICON: &[u8] = include_bytes!("../static/favicon.svg");
```
Look at it, those Rust const strings are *raw* bytes. It is filthy and I love it. More than just the sick love a person develops after years at sea, this allowed me to close up the Landlock policy to be *very* minimal:  

```rust
    /// The newest Landlock ABI the sandbox requests (V9): basic filesystem rights
    /// plus truncate, ioctl_dev, the network access controls, and the scope
    /// domains. Kernels that only support an older ABI enforce a downgraded
    /// subset of these protections.
    const REQUESTED_ABI: ABI = ABI::V9;

    pub fn apply() -> Result<()> {
        let abi = REQUESTED_ABI;
        let access_all = AccessFs::from_all(abi);

        let ruleset = Ruleset::default()
            .handle_access(access_all)?
            .handle_access(AccessNet::BindTcp)?
            .handle_access(AccessNet::ConnectTcp)?
            .scope(Scope::AbstractUnixSocket | Scope::Signal)?
            .create()?;

        let status = ruleset
            .set_compatibility(CompatLevel::BestEffort)
            .restrict_self()?;
```
We restrict before spinning up the `tokio` runtime so every subsequent thread inherits the restricted landlocked state and after we have bound to the listening port. The restriction policy, seen with the `Ruleset` above, then seals off all FS read and writes, prevents binding or connecting to any new TCP connections, and disables UnixSocket usage and Unix Signaling. Through `podman` we are sharing a namespace `pod` with just Caddy, but the scope restrictions mean the web server can't act like an annoying sibling and poke at Caddy if compromised. The mental model for Landlock is appealing because I, as the author, know what my process needs access to in order to function. Landlock gives me the tools to enforce that contract at runtime. Again, I'll have bigger concerns if this thing ever gets compromised, but it is a nice insurance policy in the face of [fickle winds](https://github.com/V4bel/dirtyfrag/blob/master/assets/write-up.md).


## Don't fall asleep while you’re ashore
I know, I know, in the last edition of `Diving Bell` I promised to cover Flatcar Linux. I didn't, and I won't, at least not in this post. This Sea Trials had some goodies and covered some material I had been meaning to get to anyway. I didn't want to risk the whole post getting too long in the tooth so we'll lay anchor here. Until next time, my salty-dogs.
