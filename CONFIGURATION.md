# Configuration

You can configure Redlib further using environment variables. For example:

```bash
REDLIB_DEFAULT_SHOW_NSFW=on redlib
```

```bash
REDLIB_DEFAULT_WIDE=on REDLIB_DEFAULT_THEME=dark redlib -r
```

You can also configure Redlib with a configuration file named `redlib.toml`. For example:

```toml
REDLIB_DEFAULT_WIDE = "on"
REDLIB_DEFAULT_USE_HLS = "on"
REDLIB_SCRAPE_COMMENT_PREFETCH = 0
```

Numbers and booleans work as well as strings; `true`/`false` mean `on`/`off`.

> [!NOTE]
> If you're deploying Redlib using the **Docker CLI or Docker Compose**, environment variables can be defined in a [`.env` file](https://docs.docker.com/compose/environment-variables/set-environment-variables/), allowing you to centralize and manage configuration in one place.
>
> To configure Redlib using a `.env` file, copy the `.env.example` file to `.env` and edit it accordingly.
>
> If using the Docker CLI, add ` --env-file .env` to the command that runs Redlib. For example:
>
> ```bash
> docker run -d --name redlib -p 8080:8080 --env-file .env ghcr.io/project516/redlib:latest
> ```
>
> If using Docker Compose, no changes are needed as the `.env` file is already referenced in `compose.yaml` via the `env_file: .env` line.

## Command Line Flags

Redlib supports the following command line flags. They take precedence over environment variables and `redlib.toml`; environment variables in turn take precedence over `redlib.toml`.

- `-4`, `--ipv4-only`: Listen on IPv4 only.
- `-6`, `--ipv6-only`: Listen on IPv6 only.
- `-r`, `--redirect-https`: Redirect all HTTP requests to HTTPS (no longer functional).
- `-a`, `--address <ADDRESS>`: Sets address to listen on. Default is `[::]`.
- `-p`, `--port <PORT>`: Port to listen on. Default is `8080`.
- `-H`, `--hsts <EXPIRE_TIME>`: HSTS header to tell browsers that this site should only be accessed over HTTPS. Default is `604800`.
- `--cacert <FILE>`: Trust only the CA certificates in this PEM file for requests to Reddit and the other upstream hosts. For debugging: `ALL_PROXY=http://127.0.0.1:8080 redlib --cacert ~/.mitmproxy/mitmproxy-ca-cert.pem` shows the decrypted traffic in mitmproxy. `REDLIB_CACERT` sets the same; the flag takes precedence.
- `-h`, `--help`: Print the flags and exit.
- `-V`, `--version`: Print the version and exit.

## Instance settings

Assign a default value for each instance-specific setting by passing environment variables to Redlib in the format `REDLIB_{X}`. Replace `{X}` with the setting name (see list below) in capital letters.

| Name                      | Possible values | Default value                          | Description                                                                                               |
|---------------------------|-----------------|----------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `SFW_ONLY`                | `["on", "off"]` | `off`                                  | Enables SFW-only mode for the instance, i.e. all NSFW content is filtered.                                |
| `BANNER`                  | String          | (empty)                                | Allows the server to set a banner to be displayed. Currently this is displayed on the instance info page. |
| `ROBOTS_DISABLE_INDEXING` | `["on", "off"]` | `off`                                  | Disables indexing of the instance by search engines.                                                      |
| `PUSHSHIFT_FRONTEND`      | String          | `undelete.pullpush.io`                 | Allows the server to set the Pushshift frontend to be used with "removed" links.                          |
| `PORT`                    | Integer 0-65535 | `8080`                                 | The **internal** port Redlib listens on.                                                                  |
| `ENABLE_RSS`              | `["on", "off"]` | `off`                                  | Enables RSS feed generation.                                                                              |
| `ENABLE_REDGIFS`          | `["on", "off"]` | `on`                                   | Proxies RedGifs videos and plays them inline. Set to `off` to leave RedGifs as plain links.               |
| `ENABLE_GIPHY`            | `["on", "off"]` | `on`                                   | Proxies GIPHY GIFs in comments and plays them inline. Set to `off` to leave them as plain links.          |
| `FULL_URL`                | String          | (empty)                                | Allows for proper URLs (for now, only needed by RSS)                                                      |
| `SOURCE_URL`              | String          | `https://github.com/Project516/redlib`   | Link to the source code for this instance. The update check on error pages compares against this GitHub repository's default branch. |
| `ENABLE_SCRAPE_FALLBACK`  | `["on", "off"]` | `on`                                   | Allows the `scraped` backend: comments and combined feeds scraped from Reddit's HTML. See [Scrape fallback](#scrape-fallback). |
| `ENABLE_GQL`              | `["on", "off"]` | `on`                                   | Allows the `gql` backend: Reddit's GraphQL API. `off` never contacts it. |
| `ENDPOINTS`, `ENDPOINTS_<KIND>` | List      | per kind                               | Which backends load Reddit data, in which order. See [Backend order](#backend-order). |
| `SCRAPE_COMMENT_PREFETCH` | Integer         | `0`                                    | Extra requests a scraped post page makes to load more comments up front. See [Scrape fallback](#scrape-fallback). |
| `GQL_FEED_REQUESTS`       | Integer         | `2`                                    | Requests the `gql` backend makes to fill one r/all page. Reddit sends only about 8 posts for the first. |
| `REDDIT_COOKIE`           | String          | (empty)                                | Logged-in `reddit_session` cookies, `|`-separated, for the `cookied-json` backend. Use throwaway accounts. See [Reddit session](#reddit-session). |
| `REDDIT_COOKIE_FILE`      | String          | (empty)                                | File with more sessions, one per line, reread while running. See [Reddit session](#reddit-session). |
| `STALE_CACHE_MB`          | Integer         | `32`                                   | RAM for the last good copy of each page, served while Reddit refuses or fails. `0` turns it off. See [Stale cache](#stale-cache). |
| `STALE_CACHE_MAX_AGE`     | Integer         | `360`                                  | Minutes a copy stays usable for the stale cache. |
| `LISTEN_PORT`             | Integer         | `8080`                                 | Port to listen on. `-p` and the `PORT` environment variable take precedence. |
| `LISTEN_ADDRESS`          | String          | `[::]`                                 | Address to listen on. `-a` takes precedence. |
| `HSTS`                    | Integer         | `604800`                               | `max-age` of the HSTS header in seconds. `-H` takes precedence. |
| `IPV4_ONLY`               | `["on", "off"]` | `off`                                  | Listens on IPv4 only, like `-4` or `IPV4_ONLY`. |
| `IPV6_ONLY`               | `["on", "off"]` | `off`                                  | Listens on IPv6 only, like `-6` or `IPV6_ONLY`. |
| `CACERT`                  | String          | (empty)                                | PEM file with the only CA certificates trusted for requests to Reddit, for debugging through a proxy. `--cacert` takes precedence. |

### Reddit session

Reddit only serves post pages to logged-in sessions. Set `REDLIB_REDDIT_COOKIE` to the `reddit_session` cookie of a logged-in reddit.com browser (DevTools → Application → Cookies); no other cookie is needed. Separate several sessions with `|` to rotate between them:

```bash
REDLIB_REDDIT_COOKIE="reddit_session=...|reddit_session=..."
```

The `reddit_session=` prefix is optional. A whole `Cookie` header works too, as long as it has `reddit_session` in it. Sessions whose `reddit_session` has expired (it lasts about six months) are skipped with a warning in the log.

Reddit's website refuses subreddit listings and search to sessions, so for those Redlib mints a token for the session the way the Android app does (valid for a day, minted again when it expires or Reddit stops accepting it) and asks Reddit's API with it. Posts and comments still go to the website with the cookie, because the API refuses those to the token; each is the other's fallback. Neither serves `/new` to sessions, so it has to come from another backend. A session that can't get a token, or whose token the API refuses, waits five minutes before the next try and uses the website meanwhile. Each account has about 100 requests per window, shared by both; a session that used them up rests until Reddit resets them.

To add or remove sessions without a restart, put them in a file and point `REDLIB_REDDIT_COOKIE_FILE` at it. Each line is one session in the same form, lines starting with `#` are comments. Redlib checks the file for changes every 10 seconds; if it can't be read, the sessions read last stay in use. Both settings can be combined.

```bash
# /etc/redlib/sessions.txt
# throwaway 1
reddit_session=...
# throwaway 2
reddit_session=...
```

With Docker, mount the file read-only, e.g. `-v ./sessions.txt:/sessions.txt:ro` with `REDLIB_REDDIT_COOKIE_FILE=/sessions.txt`. Mount its directory instead if your editor replaces the file when saving: a single-file mount keeps showing the old one.

**Use a throwaway account**, never your main one: the instance's traffic runs through it and Reddit may ban it. Keep the value secret. Pressing "Log out" on reddit.com ends the session.

### Stale cache

Pages are cached for 30 seconds as before. Besides that, Redlib keeps the last good copy of each Reddit response in RAM, zstd-compressed, least recently used dropped first once `REDLIB_STALE_CACHE_MB` is full. A copy is only used when every backend fails because Reddit blocks, rate limits, errors or doesn't answer; the page then shows a bar saying how old it is. Real answers like a deleted post or a private subreddit are never covered up. Nothing is written to disk, so a restart empties it.

A listing page takes about 20 KB, a post with its comments 5–10 KB, so the default fits a few thousand pages.

### Scrape fallback

When the backends before it fail, the `scraped` backend loads a post page's comments from the HTML Reddit serves logged-out visitors. It shows Reddit's first batch; "More comments" and "More replies" fetch more when clicked. To load more up front, set `REDLIB_SCRAPE_COMMENT_PREFETCH` to the number of extra requests per page, e.g. `5`.

It also loads combined feeds (`r/a+b`, the front page of your subscriptions) when the JSON API refuses them, which Reddit does for New and Rising: the post IDs come from the old listing page Reddit still serves logged-out visitors for these feeds, the posts themselves from the other backends. That takes two requests per page. Reddit allows an address about 100 of these pages per 10 minutes. Where Reddit refuses the page to logged-out visitors, `cookied-json` asks for it with a session from `REDLIB_REDDIT_COOKIE`.

Scraped pages cost Reddit extra requests and may break when Reddit changes its pages. Set `REDLIB_ENABLE_SCRAPE_FALLBACK=off` to show the block error instead.

### Backend order

<details>
<summary>Which backend loads what, and how to change the order</summary>

Redlib tries backends one after another until one returns the page or Reddit gives a real answer (not found, private, banned). Set the order per kind of request with `REDLIB_ENDPOINTS_<KIND>`, or for all kinds with `REDLIB_ENDPOINTS`. Backends left out are never used; ones without a session, or not in the build, are skipped. The log lists each kind's order at startup.

```bash
REDLIB_ENDPOINTS_COMMENTS="cookied-json,gql,scraped"
```

While running, Redlib reorders the listed backends by how often each worked in the last half hour or so, counting session backends as twice as expensive (they use up the account's request budget). Your order is where it starts after a restart, and where a backend drifts back to once its failures are old, so a blocked backend gets tried again later. `/info` shows the current order and each backend's score.

`json` is Reddit's API without an account, `cookied-json` the same with a session from `REDLIB_REDDIT_COOKIE` (from Reddit's website with the cookie, and for listings and search from the API with a token minted for the session like the app does), `gql` Reddit's GraphQL API (what the Android app uses), and `scraped` comments and combined feeds from the HTML Reddit serves logged-out visitors.

| `<KIND>`     | `json`     | `cookied-json` | `gql` | `scraped` | Default order |
| ------------ | ---------- | -------------- | ----- | --------- | ------------- |
| `POST`       | ✅         | ✅             | ✅    | ❌        | gql, json, cookied-json |
| `COMMENTS`   | ⚠️ blocked | ✅             | ✅    | ✅        | gql, cookied-json, scraped, json |
| `LISTING`    | ✅         | ✅             | ⚠️¹   | ⚠️⁴       | json, gql, scraped, cookied-json |
| `ALL`        | ✅²        | ✅²            | ⚠️³   | ❌        | json, gql, cookied-json |
| `SUBREDDIT`  | ✅         | ✅             | ✅    | ❌        | gql, json, cookied-json |
| `USER`       | ⚠️ blocked | ✅             | ✅    | ❌        | gql, cookied-json |
| `SEARCH`     | ✅         | ✅             | ✅    | ❌        | gql, json, cookied-json |
| `WIKI`       | ⚠️ blocked | ✅             | ✅    | ❌        | gql, cookied-json |
| `DUPLICATES` | ✅         | ✅             | ❌    | ❌        | json, cookied-json |

¹ Skipped without a request for feeds of several subreddits (`r/a+b`, the front page of your subscriptions).

² Reddit [retired r/all](https://www.reddit.com/r/reddithelp/comments/1rfmh8h/a_helpful_link_to_rall/) and answers `/r/all.json` with 403, but still serves the same feed as `/r/all%2B.json`, which is requested instead. A bare `/r/all+.json` gets a fixed copy of r/popular's first page.

³ The old GraphQL feed sends about 8 posts per request, so a page takes up to `REDLIB_GQL_FEED_REQUESTS` requests.

⁴ Only feeds of several subreddits; skipped without a request for everything else.

</details>

## Default user settings

Assign a default value for each user-modifiable setting by passing environment variables to Redlib in the format `REDLIB_DEFAULT_{Y}`. Replace `{Y}` with the setting name (see list below) in capital letters.

| Name                                | Possible values                                                                                                                                                                                                                 | Default value |
|-------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------|
| `THEME`                             | `["system", "light", "dark", "black", "dracula", "nord", "laserwave", "violet", "gold", "rosebox", "gruvboxdark", "gruvboxlight", "tokyoNight", "icebergDark", "doomone", "libredditBlack", "libredditDark", "libredditLight", "catppuccinLatte", "catppuccinFrappe", "catppuccinMacchiato", "catppuccinMocha"]` | `system`      |
| `FRONT_PAGE`                        | `["default", "popular", "all"]`                                                                                                                                                                                                 | `default`     |
| `LAYOUT`                            | `["card", "clean", "compact"]`                                                                                                                                                                                                  | `card`        |
| `WIDE`                              | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `POST_SORT`                         | `["hot", "new", "top", "rising", "controversial"]`                                                                                                                                                                              | `hot`         |
| `COMMENT_SORT`                      | `["confidence", "top", "new", "controversial", "old"]`                                                                                                                                                                          | `confidence`  |
| `BLUR_SPOILER`                      | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `SHOW_NSFW`                         | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `BLUR_NSFW`                         | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `USE_HLS`                           | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `HIDE_HLS_NOTIFICATION`             | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `AUTOPLAY_VIDEOS`                   | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `SUBSCRIPTIONS`                     | `+`-delimited list of subreddits (`sub1+sub2+sub3+...`)                                                                                                                                                                         | _(none)_      |
| `HIDE_AWARDS`                       | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `DISABLE_VISIT_REDDIT_CONFIRMATION` | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `HIDE_SCORE`                        | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `HIDE_SIDEBAR_AND_SUMMARY`          | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `FIXED_NAVBAR`                      | `["on", "off"]`                                                                                                                                                                                                                 | `on`          |
| `REMOVE_DEFAULT_FEEDS`              | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `CLEAN_URLS`                        | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `POSTS_PER_PAGE`                    | Integer 1-100                                                                                                                                                                                                                   | 25            |
| `MAX_COMMENT_THREAD_DEPTH`          | Unsigned integer (0 = unlimited/Reddit default)                                                                                                                                                                                 | 0             |
| `MORE_REPLIES_NEW_PAGE`             | `["on", "off"]`                                                                                                                                                                                                                 | `off`         |
| `LIGHT_THEME`                       | Any `THEME` except `"system"`, used by `system` in light mode (empty = built-in light)                                                                                                                                          | `""`          |
| `DARK_THEME`                        | Any `THEME` except `"system"`, used by `system` in dark mode (empty = built-in dark)                                                                                                                                            | `""`          |

## Forward proxies

Redlib's HTTP client, [wreq](https://docs.rs/wreq/latest/wreq/#proxies), uses the standard proxy variables, so all Reddit traffic can go through a proxy. Set `ALL_PROXY` for everything, or `HTTP_PROXY` / `HTTPS_PROXY` per scheme, and `NO_PROXY` for hosts to reach directly.

```bash
ALL_PROXY=socks5h://user:pass@proxy.example:1080 redlib
```

Schemes: `http://`, `https://`, `socks4://`, `socks4a://`, `socks5://` and `socks5h://` (DNS through the proxy).
