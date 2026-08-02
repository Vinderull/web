+++
title = "One"
date = "2026-08-02"
description = "My first blog post"
tags = ["web", "excess", "manifesto"]
+++

# I think I took a wrong turn

## Introductions

Now, I know what you're thinking, how'd we get here? I don't do social media. I prowl Reddit and Discord but have never once considered posting. No LinkedIn and I still have yet to make it to the Fediverse.

So, why a blog? It began as a exercise in wanting to leverage some tech stacks I don't otherwise use during the day-to-day. I am an "embedded" engineer who straddles [FPGA's](https://www.ibm.com/think/topics/field-programmable-gate-arrays) and board-support-packages are how I put bread on the table. As a result, I have little reason to ever look at [htmx](https://htmx.org/) , play around with [distroless](https://github.com/googlecontainertools/distroless) containers, explore [keyless signing](https://github.com/sigstore/cosign), ponder what CDNs do, or browse the [CNCF](https://www.cncf.io/) projects. 

More importantly, I wanted a reason to write, I wanted a reason to build, I wanted a reason to tinker, and I wanted something that I could point at and say, boy, I probably shouldn't have done this. It is like when a musician leaves their successful band to make it big solo- like [Fieldy's Dreams](https://music.youtube.com/playlist?list=OLAK5uy_m67R-U6Bbsvyn_Vus5-5E5O2QEavNu-Hg&si=ck1-YdbSkGOaZ6e5) or when Phil Collins left Genesis to make the Tarzan soundtrack.

This blog will deal with technical things, because I find them interesting, this blog will deal with music, this blog will likely talk about books and *gasp* even poetry. This blog will also talk, for a brief moment, about AI, which, while used heavily to create the server application, will never be used for blog posts.

**If you can't be bothered to write it then I can't be bothered to read it.**
I believe this firmly, so skip the source code if it is an affront to you. I won't begrudge you in the slightest. Using AI to help build things, mostly trivial, self-serving, side projects such as this, has restored a lot of my passion for playing with software. However, I remain unshaken in the belief that *pure* expression, such as the content in this blog, is one of the core pillars of what makes us human.

Words are important. Expressing the thoughts in your head with words is important. Working through the process of serializing, wrangling, making peace with and asserting dominion over that graymatter brain-pudding in your skull is borderline sacrosanct.

With that out of the way, let's talk about questionable decisions and how they get made.

## This vodka soda is pretty much just straight vodka

What actually started this? Well, I was reading about `htmx` and how it can be used to for websites to minimize JavaScript and move things back to server side. This, I am told, can help make things snappy. Also, I don't particularly enjoy writing JavaScript, so I thought, how far can I get making a website with the minimal amount of JavaScript.

I am not sure if you have noticed, but the internet has become hellish. Ads, trackers, analytics- all there to watch you, monetize you, and rarely, make some little fun widgets for your benefit. Using the web without an [adblocker](https://github.com/gorhill/uBlock) of some [kind](https://pi-hole.net/) is like driving through the city with off-road tires; sure, you'll get where you're going but god-damn can you barely hear that [Brandy](https://youtu.be/Xkj1An6Wnec?si=R5MJTm0hLLQs0Mok) mix-tape playing over the sound of the tires on asphalt.

I wanted to see just how minimal, but functional, I could make this thing while still looking *OK*. Like yeah, it probably won't win any awards but it sure does work in [lynx](https://lynx.invisible-island.net/). So if you feel like browsing from a merchant marine vessel from the middle of the Pacific ocean with terrible internet then I am here for you.

I also wanted to explore carving out an interesting place that is my own little garden. The internet still has a few neat spots left if you know where to look. There are people out there making quality content; the kind of stuff, that in the face of LinkedIn corpo-slop, can give a fella a little hope.

 To reiterate, as a rough sketch I wanted
+ htmx
+ Write a web server with fun routes
+ Containerize some stuff
+ Play around with [Flatcar Linux](https://www.flatcar.org/)
+ Write a bunch of it in Rust
+ Be fast with nearly zero client side baggage

And here is a list of things I am not going to do with this
- Add comments
- Add likes
- Build a community
- Build the next *hot* webserver library

I really don't care about exposure, or writing as a side hustle, or getting in on the techbro blog posts. I don't really care if anyone else reads this. Likely the only traffic will be Cambodian bot-nets and LLM scrapers, which, at a minimum, might spare some simulacrum of me from [Roko's Basilisk](https://en.wikipedia.org/wiki/Roko%27s_basilisk).

If you feel like engaging, go take that energy and build something, learn a musical instrument, paint, invest in a [fountain pen](https://nealstephenson.substack.com/p/writing-by-hand-is-good-for-your) and write *anything.*

Seriously, if you think you just want a website, don't do what I did. If I just wanted a blog, I would do this all over again with [Zola](https://www.getzola.org/) or [Hugo](https://gohugo.io/). Those projects more or less just work, can be hosted in all the places static websites can be hosted and are way more likely to get you *seen* by the [*cool kids*](https://seo.hugomods.com/). There may very well come a day where I port this whole thing to one of the other frameworks, but for now this monster is my own.

## What's next
I would like to get RSS/Atom feeds working in short order here, if anything just to have it done. Otherwise, I really got caught up in the technical buildout and neglected writing posts... which is probably the most onbrand thing for me I can imagine. So posts, posts would be nice.

I have settled on the technical deep dive for the [blog plumbing](https://github.com/Vinderull/web) should be its own post to keep this post **light and pithy.** 
I mean, we just met, leading with the [model trains](https://en.wikipedia.org/wiki/Rail_transport_modelling) is a real forward first date move.