# Changes compared to upstream Redlib

This fork is based on [Nachtalb/redlib](https://github.com/Nachtalb/redlib), itself a fork of [redlib-org/redlib](https://github.com/redlib-org/redlib), and tracks the `main` branch of both. It adds unmerged upstream pull requests, changes from [Sparronator9999/redlib](https://github.com/Sparronator9999/redlib), pull requests from contributors, and its own work.

## Container images

| | Upstream | This fork |
|---|---|---|
| Registry | `quay.io/redlib/redlib` | `ghcr.io/project516/redlib` (forks publish to `ghcr.io/<owner>/<repo>`) |
| Platforms | `linux/amd64`, `linux/arm64`, `linux/arm/v7` | `linux/amd64`, `linux/386`, `linux/arm64`, `linux/arm/v7`, `linux/arm/v6` |
| Base | Alpine | Distroless `static:nonroot`: static musl binary, no shell or package manager, runs as uid 65532, ~16 MB |
| Tags | `latest` | `latest` (from `master`) and `sha-<short commit>` |
| Health check | Docker `HEALTHCHECK` with `wget /settings` | `GET /healthz` for the orchestrator to probe (the image has no `wget`) |
| Binaries | Tarballs on versioned releases | The image's static binaries for all five platforms on the [`latest` release](https://github.com/Project516/redlib/releases/latest), refreshed on every push to `master` ([#44](https://github.com/Nachtalb/redlib/issues/44)) |

All platforms are cross-compiled natively on the build host and published under one multi-arch tag, so `docker pull` picks the right one.

```bash
docker run -d --name redlib -p 8080:8080 ghcr.io/project516/redlib:latest
```

Without Docker, download a binary from the [`latest` release](https://github.com/Project516/redlib/releases/latest) or build from source with `cargo build --release`.

## Fixes

Bugs present in upstream Redlib that are fixed here:

- **Blocked crawlers could not read `robots.txt`**: with `REDLIB_ROBOTS_DISABLE_INDEXING=on`, known crawlers got a 403 for `/robots.txt` too, so they never saw its `Disallow: /`. They now get the file ([redlib-org#613](https://github.com/redlib-org/redlib/pull/613)).
- **Rate-limit counter wrapped around**: requests near the end of the quota could push it below zero to 65535, which skipped the token refresh ([redlib-org#586](https://github.com/redlib-org/redlib/pull/586)).
- **The fallback token login was dead**: after 5 failed Android app logins, Redlib asked `/api/v1/access_token` for a token with a web client ID, which Reddit answers with `401 Unauthorized`. It now gets the anonymous `token_v2` the website gets, by answering the JS challenge on `www.reddit.com`.
- **`/commits.atom` and `/instances.json` crashed the request** when GitHub was unreachable. They now serve the last good copy, or a 502 ([redlib-org#396](https://github.com/redlib-org/redlib/pull/396), ported).
- **Error pages called fork builds outdated**: the update check compared the instance's commit with upstream's commit list, so every fork build was "at least 20 commits old". It now checks the repository in `REDLIB_SOURCE_URL`.
- **Error pages were cluttered**: the whole message filled the heading above six spaced-out lines, and "Report an issue" opened upstream. They now show the error, one line of detail, the requested Reddit URL and one link row (Reddit status, report an issue on this fork, try another instance). An outdated instance gets a red bar above the footer instead of a status line.
- **Post pages failed** with `Failed to parse page JSON data` when Reddit blocked them for anonymous clients. A block is now reported as a block instead of a JSON error.
- **Text after a `redditstatic.com` link was mangled**: the rewrite regex matched to the end of the line and also caught other hosts (upstream `6aef54e`, `42c5d6f`).
- **Subscriptions and filters were wiped** when saving a single setting ([redlib-org#562](https://github.com/redlib-org/redlib/issues/562)).
- **Instance default post sort ignored** until the user saved their settings once ([redlib-org#521](https://github.com/redlib-org/redlib/pull/521)). With no default set, listings fall back to `hot` ([polak14 `19c6606`](https://github.com/polak14/redlib/commit/19c6606417babb0c082d9ee4948de5008475f45a)).
- **Crash on a deleted or invalid comment link**: out-of-bounds access when a thread has no comments ([redlib-org#542](https://github.com/redlib-org/redlib/pull/542)).
- **Posts deleted by their author had no undelete link**: only posts removed by moderators linked to the Pushshift frontend. Author-deleted posts now get a "view deleted post" link too, like deleted comments.
- **Share links to posts on a user's profile** (`/u/<name>/s/<id>`) showed "Nothing here" ([redlib-org#560](https://github.com/redlib-org/redlib/pull/560)).
- **Videos transcoded by Reddit** (`rich:video`, e.g. external GIF hosts) had no player; CMAF video format is supported ([redlib-org#549](https://github.com/redlib-org/redlib/pull/549)).
- **External videos and packaged media didn't play**: Reddit's copies of Streamable and other external clips come as `external-packaged-media.redd.it` MP4s, which Redlib left unproxied (blocked by the CSP) and announced as HLS. They are now proxied under `/packaged-external/` (and `packaged-media.redd.it` under `/packaged/`), and the non-HLS fallback uses Reddit's muxed MP4, which has sound ([polak14 `29f1835`](https://github.com/polak14/redlib/commit/29f18352cf54be4ffe579f773552acd0c45cdd08)).
- **Video Speed Controller overlays landed in the wrong place**: post media, comment images and the comment sidebar had no positioned container ([Sparronator9999 `ed63495`](https://github.com/Sparronator9999/redlib/commit/ed63495b12cf5d6b66d87c0ed43055e498423a08)).
- **Inconsistent margins** inside posts and comments, and headings without margins; empty zero-width-space paragraphs no longer add gaps ([redlib-org#413](https://github.com/redlib-org/redlib/pull/413)).
- **Autoplaying videos played with sound**: with "Autoplay videos" on, every video on a listing could start with audio at once. Autoplay is now muted.
- **HLS videos sometimes failed to load**: v.redd.it answered the playlist with `403` while its link carried Reddit's query, often for several minutes. Redlib now requests the playlist without the query.
- **Pausing an HLS video while it loads** was ignored and the video started playing anyway.
- **"Enable HLS to view with audio" showed on every video**, also when the MP4 played without HLS has sound or the video has none. It now only shows when HLS is the only way to hear it.
- **Tapping a video on an iPhone could open the post below it**: Safari shrank the video when it started loading, so the posts below jumped up under the finger. Videos now keep their size ([#137](https://github.com/Nachtalb/redlib/issues/137)).
- **HLS videos stayed dead after going back** in Safari when it restored the page from its back/forward cache. The next tap now loads them again ([#137](https://github.com/Nachtalb/redlib/issues/137)).
- **Settings codes broke on every new setting**: codes exported before a new field was added failed to restore. See [Settings codes](#settings-codes).
- **Comment search had no submit button.**
- **Large numbers** show as `10K` instead of `10.0K`.
- **Comment collapse arrow** has a bigger touch area on mobile ([redlib-org#520](https://github.com/redlib-org/redlib/pull/520)).
- **Collapsibles didn't look clickable**: the Feeds dropdown, the `/info` score explanation, the settings "Why?" and the subreddit and sidebar toggles kept the default cursor on hover; they now show the pointer.
- **LibreJS blocked scripts** without license tags (copy settings code, update check, highlighted comment). Every script now carries one ([redlib-org#410](https://github.com/redlib-org/redlib/pull/410), extended to all scripts).
- **Search box focus outline** covers the whole search form, not just the input ([redlib-org#524](https://github.com/redlib-org/redlib/pull/524)).
- **`REDLIB_ENABLE_RSS=off` still served feeds**: any value turned feeds on, while the RSS buttons only showed for `on`. Feeds now need `on` ([#16](https://github.com/Nachtalb/redlib/pull/16)).
- **RSS feeds for `r/popular` and `r/all` failed** with a Reddit 404: they have no subreddit metadata to fetch ([#16](https://github.com/Nachtalb/redlib/pull/16)).
- **`redlib -h` and `--help` were rejected** as unknown arguments: the argument parser was built without its help ([#156](https://github.com/Nachtalb/redlib/issues/156)).
- **r/all showed the same page after Next**: Reddit answers a bare `/r/all+.json` with a fixed copy of r/popular's first page, so it is requested as `/r/all%2B.json`, which pages and sorts.
- **New and Rising of combined feeds were blocked** (`r/a+b`, the front page of your subscriptions): Reddit refuses these sorts to its JSON API, with or without a session, and GraphQL has no combined feeds. The `scraped` backend now reads the post IDs from the old listing page Reddit still serves logged-out visitors for these feeds and loads the posts from the other backends; `cookied-json` does the same with a session, for hosts Reddit blocks logged out ([#155](https://github.com/Nachtalb/redlib/issues/155)). See [Scrape fallback](CONFIGURATION.md#scrape-fallback).
- **Top and controversial showed all-time posts** while the timeframe menu said "day": Reddit's API reads a missing `t` as all time, so `t=day` is sent when the URL has none, as on reddit.com.
- **RSS text was escaped or decoded wrongly**: subreddit feed titles showed `&amp;` for `&`, and user feeds turned escaped text like `Vec&lt;T&gt;` into markup that readers hid ([#16](https://github.com/Nachtalb/redlib/pull/16)).
- **Shutdown cut off running downloads**: on SIGTERM or Ctrl+C, the server now finishes in-flight requests before exiting.
- **One number or boolean in `redlib.toml` dropped the whole file** without a message, e.g. `REDLIB_SCRAPE_COMMENT_PREFETCH = 0`. Numbers are read as text and booleans as `on`/`off`, and a file that still can't be read is logged ([#46](https://github.com/Nachtalb/redlib/issues/46)).
- **Big subreddits showed 0 members**: Reddit sends large counts as floats (`31604051.0` for r/news). "Active", which Reddit now always reports as 0, is replaced by the weekly visitors and contributions new Reddit shows.
- **Generic subreddit icon**: subreddits without an icon get Reddit's "r/" icon, in the subreddit info and in search results, instead of an empty circle or nothing.
- **Settings written in another case, like `REDLIB_ENABLE_RSS=ON` or `REDLIB_DEFAULT_LAYOUT=Card`, were ignored** while `/info` showed them as set. Switches, choices, backend lists, the geo filter and the Pushshift frontend's host are now read in any case; free text, subreddit names, URLs and sessions stay as written.
- **`seccomp-redlib.json` stopped the container from starting**: current runc needs syscalls the profile blocked, and without `recvmsg` DNS lookups failed so Redlib never started listening. The profile now covers the image on amd64.

## Features

From upstream pull requests that are not merged upstream:

- **RedGifs videos** play inline, proxied through the instance ([redlib-org#507](https://github.com/redlib-org/redlib/pull/507)). Can be turned off with `REDLIB_ENABLE_REDGIFS=off`. Its token is renewed when Redlib switches browser identity ([fartek](https://github.com/fartek), [#61](https://github.com/Nachtalb/redlib/issues/61)), and RedGifs is reached over IPv4 only unless Redlib runs IPv6-only.
- **GIPHY GIFs in comments** are proxied and embedded ([redlib-org#561](https://github.com/redlib-org/redlib/pull/561)). Can be turned off with `REDLIB_ENABLE_GIPHY=off`.
- **Keyboard navigation** on post pages ([redlib-org#422](https://github.com/redlib-org/redlib/pull/422)): `j` / `k` next / previous comment, `t` / `p` next / previous thread, `Enter` open the post link, `Shift+Enter` open it in a new tab.
- **Collapse a comment** by clicking its indent line ([redlib-org#291](https://github.com/redlib-org/redlib/pull/291)).
- **Lazy-loaded post images** ([redlib-org#539](https://github.com/redlib-org/redlib/pull/539)).
- **r/popular region** (the geo filter) as a user setting instead of a URL parameter ([redlib-org#566](https://github.com/redlib-org/redlib/pull/566)), with country flags, and each country named in its own language with the English name after it, sorted by the native name. Only Reddit's JSON API applies it, so with a region set r/popular is asked from `json` first, and a bar at the top says when another backend served the global feed instead.
- **Configurable source code link** in the footer ([redlib-org#568](https://github.com/redlib-org/redlib/pull/568)).
- **Better RSS media**: images, galleries and videos render in RSS readers, and content is HTML-escaped ([redlib-org#572](https://github.com/redlib-org/redlib/pull/572)).
- **Catppuccin themes**: Latte, Frappé, Macchiato and Mocha ([redlib-org#538](https://github.com/redlib-org/redlib/pull/538)).
- **Light and dark theme for System**: with the System theme, any theme can be picked for the browser's light mode and another for dark mode, instead of the built-in light and dark.
- **Theme preview**: picking a theme on the settings page shows it at once, before saving.
- **Readable theme names** in settings, like "Gruvbox Dark" and "Catppuccin Frappé" instead of "Gruvboxdark" and "CatppuccinFrappe".
- **Download all images of a gallery** from the post page as individual files ([redlib-org#546](https://github.com/redlib-org/redlib/pull/546)). Needs JavaScript.
- **Post pages without a Reddit session**: the `scraped` backend takes the post from `/by_id` and scrapes the comments from the HTML Reddit serves logged-out visitors ([redlib-org#599](https://github.com/redlib-org/redlib/pull/599), used only as a fallback here). Turn it off with `REDLIB_ENABLE_SCRAPE_FALLBACK=off`. See [Scrape fallback](CONFIGURATION.md#scrape-fallback).

From [Sparronator9999/redlib](https://github.com/Sparronator9999/redlib):

- **Clean URLs**: optional setting that strips tracking parameters (`utm_*`, `si=`, `fbclid`, …) from external links (upstream [redlib-org#460](https://github.com/redlib-org/redlib/pull/460)).
- **Posts per page** (1–100) setting.
- **Maximum comment thread depth** setting.
- **Return to the previous page** after saving settings.

From [VictorVow1](https://github.com/VictorVow1) ([#16](https://github.com/Nachtalb/redlib/pull/16)):

- **RSS feeds for every listing Reddit has feeds for**, using the same URLs as Reddit. See [RSS feeds](#rss-feeds).

From [0dragosh](https://github.com/0dragosh) ([patch](https://gist.github.com/0dragosh/6afe28f5e4f8659f5ae04ac2b418402b), [redlib-org#446](https://github.com/redlib-org/redlib/issues/446)):

- **Logged-in Reddit sessions**: Reddit blocks post pages for logged-out clients. The `cookied-json` backend requests them from `www.reddit.com` with the session cookie in `REDLIB_REDDIT_COOKIE`. Subreddit listings and search, which Reddit refuses to sessions there, come from `oauth.reddit.com` with a token minted for the session like the Android app does; each host is the other's fallback. A missing or disabled page (a 404, or a 403 Reddit explains in JSON) is Reddit's answer, not a refusal. The cookie never shows on `/info` and is never logged. See [Reddit session](CONFIGURATION.md#reddit-session).

From [polak14](https://github.com/polak14) ([`a63460b`](https://github.com/polak14/redlib/commit/a63460b4ac29c27401e3e2aaab34b68dca02991e)):

- **Comments from GraphQL**: when Reddit blocks a thread's JSON, the comments come from the Android app's GraphQL API, which still answers anonymously. Tried before session cookies and scraping; turn it off with `REDLIB_ENABLE_GQL=off`.

Own features:

- **More pages from GraphQL**: the same fallback also covers posts by ID, subreddit/popular/all listings, search, subreddit info and sidebar, user pages (a user's overview is one merged page of posts and comments) and wiki pages ([#144](https://github.com/Nachtalb/redlib/issues/144)). Wiki pages get Reddit's heading ids and table of contents, so `#wiki_...` section links still work. The active Posts or Wiki tab links to that tab's first page, so a deeper page or a wiki subpage is one click from the start.
- **Backend order**: every Reddit request tries a list of backends (anonymous JSON, session JSON, GraphQL, scraping) in an order set per kind of request with `REDLIB_ENDPOINTS_<KIND>`. By default GraphQL goes first except for listings and duplicates. While running, backends that keep failing move down and ones that work move up; `/info` shows the current order and each backend's score, with how it is calculated. See [Backend order](CONFIGURATION.md#backend-order).
- **Several Reddit sessions**: `REDLIB_REDDIT_COOKIE` takes several cookies separated by `|`. Blocked requests use them in turn, and a refused session hands the request to the next. A session whose request budget is used up (`x-ratelimit-remaining` 0 or a 429) rests until Reddit resets it, instead of being asked and refused on every request. Sessions that are only out of budget make a rate limit, not a block, and don't switch the browser emulation.
- **The block message doesn't reveal whether a session is set**: visitors see the same 403 text either way, telling the instance host to set or renew `REDLIB_REDDIT_COOKIE`.
- **Blocked combined feeds say why**: feeds that combine subreddits (the front page, `r/a+b`) only load through Reddit's JSON API, since GraphQL has no combined feed. When Reddit blocks that, the error page says so and links each subreddit, which still load one by one ([#118](https://github.com/Nachtalb/redlib/issues/118)).
- **Scraped comments load on demand**: a scraped post page shows Reddit's first batch of comments. "More comments" and "More replies" open a page with the next batch, one request to Reddit per click, with a link back to all comments. `REDLIB_SCRAPE_COMMENT_PREFETCH` loads more up front.
- **"More replies" and "More comments" load in place**: the comments appear where the link was instead of opening another page. Without JavaScript the link opens the page as before.
- **Always open replies in thread view**: a setting that makes "More replies" open the parent comment's page (thread view) instead of loading in place, for narrow screens where deep threads get squeezed ([#107](https://github.com/Nachtalb/redlib/issues/107)). With it off, replies load in place, and open in thread view only when they would get less than half of the comments' width (2/3 on screens up to 800px). With JavaScript, replies already on the page that would be that narrow are replaced by a "More replies" link too, when the page loads and after loading in place.
- **Threads open without a page load**: with JavaScript, "Show parent comment", "View all comments" and "More replies" that open a thread swap in that page's comments, with a working address and back button.
- **Last good page while Reddit fails**: when every backend is blocked, rate limited or down, the last good copy from a compressed RAM cache is shown, with a bar at the top saying how old it is. See [Stale cache](CONFIGURATION.md#stale-cache).

- **GIF posts play like GIFs**: muted, looping, no controls, independent of the autoplay setting.
- **Advertiser badge**: posts made with Reddit's ad tools get an "Advertiser" badge next to the author, and so do that author's comments under them. Reddit marks the posts, not the accounts: the JSON API's `is_created_from_ads_ui`, while GraphQL only gives away ads with a call to action or a disclaimer. User pages have no badge, since no API says an account advertises.
- **One video at a time**: with autoplay on, only the video most in view plays. It pauses when scrolled away and the next one takes over.
- **Instance info shows the values in use**: `/info` shows the default for settings that aren't configured, marked `(default)`, instead of `Unset` or `None`.
- **`/healthz` endpoint**: returns `200 ok` without contacting Reddit.
- **`/healthz/reddit` endpoint**: for status pages. Reports how the last 20 post pages visitors opened went (`ok`, `scraped`, `blocked`) and returns `503` when fewer than half got Reddit's JSON. It never contacts Reddit itself. `/healthz/reddit.json` serves the same status as a [shields.io endpoint badge](https://shields.io/badges/endpoint-badge).
- **The footer shows the running commit**: a GitHub icon and the short commit hash link to the source repository, in place of the version number and the separate "Code" link. `/info` shows the same: a Source row and the short commit linked to the commit itself (full hash on hover), with no crate version, and the deploy date to the second. A hidden HTML comment keeps the `id="version">` marker so upstream's instance list still picks the instance up.
- **Search tabs like Reddit's**: Posts, Comments, Media, Communities and People.
  Each tab is a plain link with only its own filters (post type, sort, time),
  so they work without JavaScript. Results come from `DynamicSearch`, the query
  the Android app uses for its search tabs. "Show more" adds the next results
  below, or opens them as the next page without JavaScript
  ([#69](https://github.com/Nachtalb/redlib/issues/69)).
- **New Reddit's sidebar**: subreddits show the widgets new Reddit shows (rules you can expand, related communities with their member counts, text boxes, images, link menus) instead of the old-reddit sidebar text mods often left stale. Each box folds on its title and only the rules start open, so big sidebars like r/europe stay short. Subreddits without widgets keep the old sidebar.
- **`--cacert <FILE>`** or `REDLIB_CACERT`: trusts only the CA certificates in that file for requests to Reddit, to inspect them through a proxy like mitmproxy ([#157](https://github.com/Nachtalb/redlib/issues/157)).

## Configuration

### New instance settings

| Variable | Default | Description |
|---|---|---|
| `REDLIB_ENABLE_REDGIFS` | `on` | Proxy RedGifs videos and play them inline. `off` leaves RedGifs as plain links. |
| `REDLIB_ENABLE_GIPHY` | `on` | Proxy GIPHY GIFs in comments and play them inline. `off` leaves them as plain links. |
| `REDLIB_ENABLE_SCRAPE_FALLBACK` | `on` | Allow the `scraped` backend: comments and combined feeds scraped from Reddit's HTML. `off` shows the block error instead. |
| `REDLIB_ENABLE_GQL` | `on` | Allow the `gql` backend: Reddit's GraphQL API. `off` never contacts GraphQL. |
| `REDLIB_ENDPOINTS`, `REDLIB_ENDPOINTS_<KIND>` | per kind | Which backends load Reddit data, in which order. |
| `REDLIB_SCRAPE_COMMENT_PREFETCH` | `0` | Extra requests a scraped post page makes to load more comments up front. `0` loads them only when asked. |
| `REDLIB_GQL_FEED_REQUESTS` | `2` | Requests the `gql` backend makes to fill one r/all page. Reddit sends only about 8 posts for the first. At least `1`. |
| `REDLIB_REDDIT_COOKIE` | (empty) | `Cookie` header of a logged-in reddit.com session, several separated by `\|`, for the `cookied-json` backend. **Secret.** |
| `REDLIB_SOURCE_URL` | `https://github.com/Project516/redlib` | Source code link in the footer, and the GitHub repository whose default branch the update check on error pages compares against. |
| `REDLIB_LISTEN_PORT`, `REDLIB_LISTEN_ADDRESS`, `REDLIB_HSTS`, `REDLIB_IPV4_ONLY`, `REDLIB_IPV6_ONLY` | `8080`, `[::]`, `604800`, `off`, `off` | The listener options, so they also work in `redlib.toml`. The flags and `PORT`/`IPV4_ONLY`/`IPV6_ONLY` take precedence. Not `REDLIB_PORT`: Kubernetes sets that to `tcp://…` for a Service named `redlib`. |

### New user settings

Each has an instance default via `REDLIB_DEFAULT_<NAME>`:

| Setting | Variable | Default |
|---|---|---|
| r/Popular region | `REDLIB_DEFAULT_GEO_FILTER` | `GLOBAL` |
| Clean URLs | `REDLIB_DEFAULT_CLEAN_URLS` | `off` |
| Posts per page | `REDLIB_DEFAULT_POSTS_PER_PAGE` | `25` |
| Max comment thread depth | `REDLIB_DEFAULT_MAX_COMMENT_THREAD_DEPTH` | `0` (unlimited) |
| Always open replies in thread view | `REDLIB_DEFAULT_MORE_REPLIES_NEW_PAGE` | `off` |
| Light theme, dark theme (for the System theme) | `REDLIB_DEFAULT_LIGHT_THEME`, `REDLIB_DEFAULT_DARK_THEME` | empty (built-in light/dark) |

All of them are documented in [CONFIGURATION.md](CONFIGURATION.md), `.env.example` and `app.json`.

### Settings codes

Settings codes (Settings → export / restore) now carry a format revision:

- **New codes** start with a marker byte and are revisioned. A code exported before a setting existed restores that setting to its default.
- **Old codes** from upstream Redlib still restore. Settings they don't contain get their defaults.
- Settings added in this fork belong to revision 2, except "Always open replies in thread view" in revision 3 and the light and dark theme in revision 4: older revisions' codes were already in use, and a new field in them would stop those codes from restoring.

Codes exported from this fork can't be restored on upstream Redlib.

### RSS feeds

With `REDLIB_ENABLE_RSS=on`, append `.rss` to a page URL, as on Reddit. Both `/r/rust/top.rss` and `/r/rust/top/.rss` work. Query parameters are passed through, so `?t=year`, `?sort=top` and `?limit=100` work as on Reddit.

| Feed | Example |
|---|---|
| Front page | `/.rss`, `/top.rss?t=day` |
| Subreddit or multireddit | `/r/rust.rss`, `/r/SkincareAddiction/top.rss?t=year`, `/r/rust+linux/new.rss` |
| Subreddit comments | `/r/rust/comments.rss` |
| Post and its comments | `/r/rust/comments/abc123.rss`, `/comments/abc123/title/.rss?sort=top` |
| User | `/user/spez.rss`, `/u/spez/submitted.rss?sort=top&t=all`, `/user/spez/comments.rss` |
| Search | `/search.rss?q=rust&sort=new`, `/r/rust/search.rss?q=async&restrict_sr=on` |

- Feeds are built from Reddit's JSON API, not proxied from Reddit's `.rss` feeds, so they keep working if Reddit drops those.
- Post feeds list comments newest first by default.
- The RSS button on subreddit and user pages links to the feed for the current sort, timeframe and listing. Every page with a feed advertises it with `<link rel="alternate">`, so feed readers find it from the page URL.
- Feed readers don't send cookies, so feeds use the instance defaults. On the front page, the advertised feed points at the user's subscriptions or chosen front page directly.
- With `REDLIB_FULL_URL` set, relative links and images in post and comment bodies are made absolute so they load in feed readers.
- On SFW-only instances, NSFW posts and comments are left out of feeds.

## Dependencies and networking

Upgraded in the hope of reducing Reddit's TLS/SSL and fingerprinting errors (rate limits, failed token refreshes, broken media):

- **wreq** `6.0.0-rc.28` → `0.16.3` and **wreq-util** `3.0.0-rc.10` → `0.2.0`, the stable line of the HTTP client that emulates browser TLS and HTTP/2 fingerprints. The BoringSSL bindings move from `boring2 5.0.0-alpha.13` to `btls 0.5.6`. 0.16.x also applies the total timeout to the response body, uses the configured timer for connect timeouts, and no longer compresses range requests.
- The client emulates a random browser (Chrome, Firefox, Edge, Opera or Safari) on a platform it ships on, mostly versions Reddit serves both anonymous JSON and GraphQL to. When Reddit blocks the current one, it leaves it out for a few hours and switches to another, as it does when Reddit refuses every session (at most once a minute). Before blaming the browser, it asks for a listing every served browser gets, since Reddit refuses some paths like `/new` to every browser with the same page ([#141](https://github.com/Nachtalb/redlib/issues/141)). Sessions refused on subreddit listings or search don't switch it, since Reddit refuses those to every browser.
- **hyper** `0.14` → `1.x` for the server, with graceful shutdown.
- **Spoofed Android app versions** refreshed from Nov 2024 builds to Reddit 2026.33–2026.42 on Android 12–16, with a working `update_oauth_resources.sh` ([polak14 `e19df3b`](https://github.com/polak14/redlib/commit/e19df3b54e2a9b7628a09453bad1afb64b399df8)).
- **hls.js** `1.5.1` → `1.7.3`.
- All other Rust dependencies are updated (askama 0.16, cached 4, revision 0.30, toml 1.0, brotli 9, base64 0.23, …). The minimum Rust version is 1.98.

## Removals

- **quay.io image** and the **Alpine image** (`Dockerfile.alpine`) are replaced by the multi-arch distroless image on GHCR.
- **`Dockerfile` and `Dockerfile.ubuntu`**: they packaged upstream release binaries, which this fork doesn't publish. `Dockerfile.distroless` is the only image build.
- **Docker `HEALTHCHECK`** in `compose.yaml` and `compose.dev.yaml`: it used `wget`, which the image doesn't have. Probe `/healthz` instead.
- **`%2fcomments` path encoding** of thread requests ([redlib-org#592](https://github.com/redlib-org/redlib/pull/592), [#591](https://github.com/redlib-org/redlib/issues/591)): Reddit decodes the path before blocking it, so it only marked requests as coming from Redlib.
- **`/info.txt` and `/info.yaml`** are gone; `/info` and `/info.json` remain.
- **Upstream's release binary and pull-request workflows** are disabled (`*.disabled`). The container build publishes the binaries, and Lint and Tests run on pull requests.

## Development

- Formatting is enforced with pre-commit and a Lint workflow: `cargo fmt`, [djhtml](https://github.com/rtts/djhtml) for templates, Prettier for CSS and JS, plus whitespace and line-ending checks. Run `pre-commit install` once.
- `.editorconfig`: tabs for Rust, 4 spaces for templates, CSS and JS, 2 spaces for YAML and JSON.
- CI builds each platform in its own job, with the Cargo registry and `target/` cached between runs.
- A Tests workflow runs `cargo test` on every pull request and push to `master`, including the tests that need reddit.com. Those share one runtime (`client::block_on_shared`), because `CLIENT`'s pooled connections die with the runtime that opened them. Only `test_generic_web_backend`, `test_scraped_combined_feed` and `test_cookied_json_combined_feed` are `#[ignore]`d, because `www.reddit.com` answers GitHub's runners with 403; run them with `cargo test -- --include-ignored`. The tests only emulate a fixed list of browsers Reddit serves anonymous `oauth.reddit.com` JSON to, so they don't depend on the draw. `backends::live_tests` calls each backend directly (every GraphQL query, the session backends) so one can't break unnoticed behind another; the session backends use a throwaway account's `REDLIB_REDDIT_COOKIE` from a repository secret and skip without it.
- A manual Fingerprints workflow asks Reddit about every browser the client can emulate, writes the table to the run summary, and fails when the tests' list has one Reddit blocks.
- A new push cancels the Lint, Tests and Container build runs still going for the same branch.
- CI only runs when files it checks change: a commit that touches only Markdown runs Lint but no Tests or Container build.
- Dependabot opens weekly update PRs for crates, GitHub Actions and pre-commit hooks, grouped into one PR each.
- Every place Reddit data comes from (anonymous JSON, session JSON, GraphQL, scraping) is one file in `src/backends/` implementing the `Backend` trait; one loop tries them in turn.
- The Docker build context is an allowlist (`.dockerignore`); the commit hash comes in as the `GIT_HASH` build argument.
