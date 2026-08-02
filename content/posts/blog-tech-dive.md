+++
title = "One"
date = "2026-08-01"
description = "My first blog post"
tags = ["rust", "web", "excess"]
+++

# I think I took a wrong turn

## Introductions

Now, I know what you're thinking, how'd we get here? I don't do social media. I prowl Reddit and Discord but have never once considered posting. No LinkedIn and I still have yet to make it to the Fediverse.

So, why a blog? It began as a exercise in wanting to leverage some tech stacks I don't otherwise use during the day-to-day. I am an "embedded" engineer who straddles [FPGA's](https://www.ibm.com/think/topics/field-programmable-gate-arrays) and board-support-packages are how I put bread on the table. As a result, I have little reason to ever look at [htmx](https://htmx.org/) , play around with [distroless](https://github.com/googlecontainertools/distroless) containers, explore [keyless signing](https://github.com/sigstore/cosign), ponder what CDNs do, or browse the [CNCF](https://www.cncf.io/) projects. 

More importantly, I wanted a reason to write, I wanted a reason to build, I wanted a reason to tinker, and I wanted something that I could point at and say, boy, I probably shouldn't have done this. It is like when a musician leaves their successful band to make it big solo- like [Fieldy's Dreams](https://music.youtube.com/playlist?list=OLAK5uy_m67R-U6Bbsvyn_Vus5-5E5O2QEavNu-Hg&si=ck1-YdbSkGOaZ6e5) or when Phil Collins left Genesis to make the Tarzan soundtrack.

This blog will deal with technical things, because I find them interesting, this blog will deal with music, this blog will likely talk about books and *gasp* even poetry. This blog will also talk, for a brief moment, about AI, which while used heavily to create the server application, will never be used for blog posts.

**If you can't be bothered to write it then I can't be bothered to read it.**
I believe this firmly, so skip the source code if it is an affront to you. I won't begrudge you in the slightest. Using AI to help build things, mostly trivial, self-serving, side projects such as this, has restored a lot of my passion for playing with software, but I remain unshaken in the belief that *pure* expression, such as the content in this blog, is one of the core pillars of what makes us human.

Words are important. Expressing the thoughts in your head with words is important. Working through the process of serializing, wrangling, making peace with and asserting dominion over that graymatter brain-pudding in your skull is borderline sacrosanct.

With that out of the way, let's talk about questionable decisions and how they get made.


## This vodka soda is pretty much just straight vodka
Ok, so, to start, one week ago I didn't even know what a [static site generator](https://en.wikipedia.org/wiki/Static_site_generator) was. Is that embarrassing to admit? I feel like it is. I almost built one of those, but it definitely came out as a binary application webserver that... serves static web pages?

Seriously, if you think you just want a website, don't do what I did. If I just wanted a blog, I would do this all over again with [Zola](https://www.getzola.org/) or [Hugo](https://gohugo.io/). Those projects more or less just work, can be hosted in all the places static websites can be hosted and are way more likely to get you *seen* by the [*cool kids*](https://seo.hugomods.com/). 

But I deal with Linux machines all day and the thought of *not* building an infrastructure pipeline was out of the question. Plus, it feels like the cool indie developers are all about [sovereignty](https://www.otherstrangeness.com/2026/03/14/have-a-fucking-website/) and, what can I say, I've always been a little indie curious myself.

## *Pourqoui?*


What actually started this? Well, I was reading about `htmx` and how for a lot of use cases it can eliminate most JavaScript and move things back to server side. This, I am told, can help make things snappy. I am not sure if you have noticed, but the internet has become hellish. Ads, trackers, analytics- all there to watch you, monetize you, and rarely, make some little fun widgets foryour benefit. Using the web without an [adblocker](https://github.com/gorhill/uBlock) of some [kind](https://pi-hole.net/) is like driving through the city with off-road tires; sure, you'll get where you're going but god-damn can you barely hear that [Brandy](https://youtu.be/Xkj1An6Wnec?si=R5MJTm0hLLQs0Mok) mix-tape playing over the sound of the tires on asphalt. 

So I thought, how far can I get with the bare bones amount of JS. Also, I don't particularly enjoy writing JS so the fact the `htmx` is offering a return to pure `html` was pretty inticing.


The web app itself is primarily written in Rust. Why?

You'll get the usual spiel about how Rust provides:

- Memory ([mostly](https://doc.rust-lang.org/book/ch15-06-reference-cycles.html)) safety without garbage collection
- High performance
- Excellent tooling

Mostly, I have really grown to enjoy writing Rust. Just, like, the act of writing, looking at it, and the problems it makes me think about. It scratches some weird itch, right behind the eyes. I primarly wrote Linux Kernel [C](https://www.kernel.org/doc/html/latest/process/programming-language.html) for years and was fairly skeptical of [The Cult of Rustacean](https://www.rustacean.net/) as any self-respecting apostate is want to do. But, overtime, I kept telling myself I needed to give it a fair shake, and that, *groan*, I shouldn't knock it until I tried it. Well, consider my boots good and knocked because I am a convert. If/when I am writing code these days, I start with Rust unless it is better off as a script.

The Rust [community](https://github.com/sharkdp/hyperfine) attracts a [certain](https://probe.rs/) level of [craftmanship](https://github.com/burntsushi/ripgrep), care, and passion. Most of the tools and libraries are a joy to use. Also, since I spend my days in system's programming land, there is more skill crossover for me than Go. One day I'll get into [Zig](https://ziglang.org/), likely once they `1.0.0` that puppy. Until then, I will watch [luminaries](https://andrewkelley.me/post/my-thoughts-bun-rust-rewrite.html) rain Gomorrah levels of fury down; my fellow nerds are out there [straight *gassing*](https://youtu.be/mgxDbkhQY-Q?si=H_8Or0U2CxLTbvzn).

Anyway.

I have settled on the technical deep dive should be its own post to keep this post *light and pithy.* I mean, we just met, leading with the [model trains](https://en.wikipedia.org/wiki/Rail_transport_modelling) is strong first date behavior

From there I settled on a distroless container to wrap it in because *why not.*
Doing so was actually pretty easy and drove me into the MUSL target for rust, which, was also such a nothing-burger choice.

I have been looking to leverage Landlock since reading [this](https://blog.prizrak.me/post/landlock/) and the Rust API examples served as no small inspiration. Let's take peek at the code:

### Landlock

```rust
    pub fn apply(static_dir: &Path) -> Result<()> {
        let abi = ABI::V9;
        let access_all = AccessFs::from_all(abi);
        let access_read = AccessFs::from_read(abi);

        let mut ruleset = Ruleset::default()
            .handle_access(access_all)?
            .handle_access(AccessNet::BindTcp)?
            .handle_access(AccessNet::ConnectTcp)?
            .create()?;

        // Posts are loaded into memory before sandboxing, so only the static
        // asset dir (served on-demand by ServeDir) needs read access.
        if static_dir.exists() {
            ruleset = ruleset.add_rule(PathBeneath::new(
                PathFd::new(static_dir)
                    .with_context(|| format!("opening {}", static_dir.display()))?,
                access_read,
            ))?;
        }

        let status = ruleset
            .set_compatibility(CompatLevel::BestEffort)
            .restrict_self()?;
```
This does some nice stuff, most of it total overkill for what is literally a webserver serving up markdown files as HTML, but it was a nice muscle to flex. This web server, which hangs out on the web with lions, dragons, and bears, is restricting itself and what it can do to a much smaller set of things. Normally any application run in Linux has access to anything that the user running it has access to. Well, I *know* what this server should be accessing, so why not seal it so it can only do that? It is a pretty reasonable thing, when you think about it, and there is some elegance here that I enjoy. 

## Ah, the french champagne