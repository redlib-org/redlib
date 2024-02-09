# Redlib

> An alternative private front-end to Reddit, with its origins in [Libreddit](https://github.com/libreddit/libreddit).

![screenshot](https://i.ibb.co/QYbqTQt/libreddit-rust.png)

---

**10-second pitch:** Redlib is a private front-end like [Invidious](https://github.com/iv-org/invidious) but for Reddit. Browse the coldest takes of [r/unpopularopinion](https://redlib.matthew.science/r/unpopularopinion) without being [tracked](#reddit).

- 🚀 Fast: written in Rust for blazing-fast speeds and memory safety
- ☁️ Light: no JavaScript, no ads, no tracking, no bloat
- 🕵 Private: all requests are proxied through the server, including media
- 🔒 Secure: strong [Content Security Policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/CSP) prevents browser requests to Reddit

---

# Instances

🔗 **Want to automatically redirect Reddit links to Redlib? Use [LibRedirect](https://github.com/libredirect/libredirect) or [Privacy Redirect](https://github.com/SimonBrazell/privacy-redirect)!**

[Follow this link](https://github.com/redlib-org/redlib-instances/blob/main/instances.md) for an up-to-date table of instances in Markdown format. This list is also available as [a machine-readable JSON](https://github.com/redlib-org/redlib-instances/blob/main/instances.json).

Both files are part of the [redlib-instances](https://github.com/redlib-org/redlib-instances) repository. To contribute your [self-hosted instance](#deployment) to the list, see the [redlib-instances README](https://github.com/redlib-org/redlib-instances/blob/main/README.md).

---

# About

Find Redlib on 💬 [Matrix](https://matrix.to/#/#redlib:matrix.org), 🐋 [Quay.io](https://quay.io/repository/redlib/redlib), :octocat: [GitHub](https://github.com/redlib-org/redlib), and 🦊 [GitLab](https://gitlab.com/redlib/redlib).

## Built with

- [Rust](https://www.rust-lang.org/) - Programming language
- [Hyper](https://github.com/hyperium/hyper) - HTTP server and client
- [Askama](https://github.com/djc/askama) - Templating engine
- [Rustls](https://github.com/rustls/rustls) - TLS library

## Info
Redlib hopes to provide an easier way to browse Reddit, without the ads, trackers, and bloat. Redlib was inspired by other alternative front-ends to popular services such as [Invidious](https://github.com/iv-org/invidious) for YouTube, [Nitter](https://github.com/zedeus/nitter) for Twitter, and [Bibliogram](https://sr.ht/~cadence/bibliogram/) for Instagram.

Redlib currently implements most of Reddit's (signed-out) functionalities but still lacks [a few features](https://github.com/redlib-org/redlib/issues).

## How does it compare to Teddit?

Teddit is another awesome open source project designed to provide an alternative frontend to Reddit. There is no connection between the two, and you're welcome to use whichever one you favor. Competition fosters innovation and Teddit's release has motivated me to build Redlib into an even more polished product.

If you are looking to compare, the biggest differences I have noticed are:
- Redlib is themed around Reddit's redesign whereas Teddit appears to stick much closer to Reddit's old design. This may suit some users better as design is always subjective.
- Redlib is written in [Rust](https://www.rust-lang.org) for speed and memory safety. It uses [Hyper](https://hyper.rs), a speedy and lightweight HTTP server/client implementation.

---

# Comparison

This section outlines how Redlib compares to Reddit.

## Speed

Lasted tested Jan 12, 2024.

Results from Google PageSpeed Insights ([Redlib Report](https://pagespeed.web.dev/report?url=https%3A%2F%2Fredlib.matthew.science%2F), [Reddit Report](https://pagespeed.web.dev/report?url=https://www.reddit.com)).

|                        | Redlib   | Reddit    |
|------------------------|-------------|-----------|
| Speed Index            | 0.6s        | 1.9s      |
| Performance Score      | *100%*      | *64%*     |
| Time to Interactive    | **2.8s**    | **12.4s** |

## Privacy

### Reddit

**Logging:** According to Reddit's [privacy policy](https://www.redditinc.com/policies/privacy-policy), they "may [automatically] log information" including:
- IP address
- User-agent string
- Browser type
- Operating system
- Referral URLs
- Device information (e.g., device IDs)
- Device settings
- Pages visited
- Links clicked
- The requested URL
- Search terms

**Location:** The same privacy policy goes on to describe that location data may be collected through the use of:
- GPS (consensual)
- Bluetooth (consensual)
- Content associated with a location (consensual)
- Your IP Address

**Cookies:** Reddit's [cookie notice](https://www.redditinc.com/policies/cookies) documents the array of cookies used by Reddit including/regarding:
- Authentication
- Functionality
- Analytics and Performance
- Advertising
- Third-Party Cookies
- Third-Party Site

### Redlib

For transparency, I hope to describe all the ways Redlib handles user privacy.

#### Server

* **Logging:** In production (when running the binary, hosting with docker, or using the official instances), Redlib logs nothing. When debugging (running from source without `--release`), Redlib logs post IDs fetched to aid with troubleshooting.

* **Cookies:** Redlib uses optional cookies to store any configured settings in [the settings menu](https://redlib.matthew.science/settings). These are not cross-site cookies and the cookies hold no personal data.

#### Official instance (redlib.matthew.science)

The official instance is hosted at https://redlib.matthew.science.

* **Server:** The official instance runs a production binary, and thus logs nothing.

* **DNS:** The domain for the official instance uses Cloudflare as the DNS resolver. However, this site is not proxied through Cloudflare, and thus Cloudflare doesn't have access to user traffic.

* **Hosting:** The official instance is hosted on [Replit](https://replit.com/), which monitors usage to prevent abuse. I can understand if this invalidates certain users' threat models, and therefore, self-hosting, using unofficial instances, and browsing through Tor are welcomed.

---

# Installation

<!-- ## 1) Cargo

Make sure Rust stable is installed along with `cargo`, Rust's package manager.

```
cargo install libreddit
``` -->

## 2) Docker

[Docker](https://www.docker.com) lets you run containerized applications. Containers are loosely isolated environments that are lightweight and contain everything needed to run the application, so there's no need to rely on what's installed on the host.

Docker images for Redlib are available at [quay.io](https://quay.io/repository/redlib/redlib), with support for `amd64`, `arm64`, and `armv7` platforms.

For configuration options, see the [Deployment section](#Deployment).

### Docker CLI

Deploy Redlib:

```
docker pull quay.io/redlib/redlib:latest
docker run -d --name redlib -p 8080:8080 quay.io/redlib/redlib:latest
```

Deploy using a different port on the host (in this case, port 80):

```
docker pull quay.io/redlib/redlib:latest
docker run -d --name redlib -p 80:8080 quay.io/redlib/redlib:latest
```

If you're using a reverse proxy in front of Redlib, prefix the port numbers with `127.0.0.1` so that Redlib only listens on the host port **locally**. For example, if the host port for Redlib is `8080`, specify `127.0.0.1:8080:8080`. 

If deploying on:

- an `arm64` platform, use the `quay.io/redlib/redlib:latest-arm` image instead.
- an `armv7` platform, use the `quay.io/redlib/redlib:latest-armv7` image instead.

### Docker Compose

Copy `compose.yaml` and modify any relevant values (for example, the ports Redlib should listen on).

Start Redlib in detached mode (running in the background):

```bash
docker compose up -d
```

<!-- ## 3) AUR

For ArchLinux users, Redlib is available from the AUR as [`libreddit-git`](https://aur.archlinux.org/packages/libreddit-git).

```
yay -S libreddit-git
```
## 4) NetBSD/pkgsrc

For NetBSD users, Redlib is available from the official repositories.

```
pkgin install libreddit
```

Or, if you prefer to build from source

```
cd /usr/pkgsrc/libreddit
make install
``` -->

## 5) GitHub Releases

If you're on Linux and none of these methods work for you, you can grab a Linux binary from [the newest release](https://github.com/redlib-org/redlib/releases/latest).

## 6) Replit/Heroku/Glitch

> **Warning**
>
> These are free hosting options, but they are *not* private and will monitor server usage to prevent abuse. If you need a free and easy setup, this method may work best for you.

<a href="https://repl.it/github/redlib-org/redlib"><img src="https://repl.it/badge/github/redlib-org/redlib" alt="Run on Repl.it" height="32" /></a>
[![Deploy](https://www.herokucdn.com/deploy/button.svg)](https://heroku.com/deploy?template=https://github.com/redlib-org/redlib)

---

# Deployment

Once installed, deploy Redlib to `0.0.0.0:8080` by running:

```
redlib
```

## Configuration

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
```

### For Docker deployments

If you're deploying Redlib using the **Docker CLI or Docker Compose**, environment variables can be defined in a [`.env` file](https://docs.docker.com/compose/environment-variables/set-environment-variables/), allowing you to centralize and manage configuration in one place.

To configure Redlib using a `.env` file, copy the `.env.example` file to `.env` and edit it accordingly.

If using the Docker CLI, add ` --env-file .env` to the command that runs Redlib. For example:
```bash
docker run -d --name redlib -p 8080:8080 --env-file .env quay.io/redlib/redlib:latest
```

If using Docker Compose, no change is needed as the `.env` file is already referenced in `compose.yaml` via the `env_file: .env` line.

### Instance settings

Assign a default value for each instance-specific setting by passing environment variables to Redlib in the format `REDLIB_{X}`. Replace `{X}` with the setting name (see list below) in capital letters.

| Name                      | Possible values | Default value    | Description                                                                                               |
|---------------------------|-----------------|------------------|-----------------------------------------------------------------------------------------------------------|
| `SFW_ONLY`                | `["on", "off"]` | `off`            | Enables SFW-only mode for the instance, i.e. all NSFW content is filtered.                                |
| `BANNER`                  | String          | (empty)          | Allows the server to set a banner to be displayed. Currently this is displayed on the instance info page. | 
| `ROBOTS_DISABLE_INDEXING` | `["on", "off"]` | `off`            | Disables indexing of the instance by search engines.                                                      |
| `PUSHSHIFT_FRONTEND`      | String          | `www.unddit.com` | Allows the server to set the Pushshift frontend to be used with "removed" links.                          |

### Default User Settings

Assign a default value for each user-modifiable setting by passing environment variables to Redlib in the format `REDLIB_DEFAULT_{Y}`. Replace `{Y}` with the setting name (see list below) in capital letters.

| Name                                | Possible values                                                                                                                    | Default value |
|-------------------------------------|------------------------------------------------------------------------------------------------------------------------------------|---------------|
| `THEME`                             | `["system", "light", "dark", "black", "dracula", "nord", "laserwave", "violet", "gold", "rosebox", "gruvboxdark", "gruvboxlight"]` | `system`      |
| `FRONT_PAGE`                        | `["default", "popular", "all"]`                                                                                                    | `default`     |
| `LAYOUT`                            | `["card", "clean", "compact"]`                                                                                                     | `card`        |
| `WIDE`                              | `["on", "off"]`                                                                                                                    | `off`         |
| `POST_SORT`                         | `["hot", "new", "top", "rising", "controversial"]`                                                                                 | `hot`         |
| `COMMENT_SORT`                      | `["confidence", "top", "new", "controversial", "old"]`                                                                             | `confidence`  |
| `SHOW_NSFW`                         | `["on", "off"]`                                                                                                                    | `off`         |
| `BLUR_NSFW`                         | `["on", "off"]`                                                                                                                    | `off`         |
| `USE_HLS`                           | `["on", "off"]`                                                                                                                    | `off`         |
| `HIDE_HLS_NOTIFICATION`             | `["on", "off"]`                                                                                                                    | `off`         |
| `AUTOPLAY_VIDEOS`                   | `["on", "off"]`                                                                                                                    | `off`         |
| `SUBSCRIPTIONS`                     | `+`-delimited list of subreddits (`sub1+sub2+sub3+...`)                                                                            | _(none)_      | 
| `HIDE_AWARDS`                       | `["on", "off"]`                                                                                                                    | `off`         |
| `DISABLE_VISIT_REDDIT_CONFIRMATION` | `["on", "off"]`                                                                                                                    | `off`         |
| `HIDE_SCORE`                        | `["on", "off"]`                                                                                                                    | `off`         |
| `FIXED_NAVBAR`                      | `["on", "off"]`                                                                                                                    | `on`          |

## Proxying using NGINX

> **Note**
>
> If you're [proxying Redlib through an NGINX Reverse Proxy](https://github.com/libreddit/libreddit/issues/122#issuecomment-782226853), add
> ```nginx
> proxy_http_version 1.1;
> ```
> to your NGINX configuration file above your `proxy_pass` line.

## systemd

You can use the systemd service available in `contrib/redlib.service`
(install it on `/etc/systemd/system/redlib.service`).

That service can be optionally configured in terms of environment variables by
creating a file in `/etc/redlib.conf`. Use the `contrib/redlib.conf` as a
template. You can also add the `REDLIB_DEFAULT__{X}` settings explained
above.

When "Proxying using NGINX" where the proxy is on the same machine, you should
guarantee nginx waits for this service to start. Edit
`/etc/systemd/system/redlib.service.d/reverse-proxy.conf`:

```conf
[Unit]
Before=nginx.service
```

## launchd

If you are on macOS, you can use the launchd service available in `contrib/redlib.plist`.

Install it with `cp contrib/redlib.plist ~/Library/LaunchAgents/`.

Load and start it with `launchctl load ~/Library/LaunchAgents/redlib.plist`.

## Building

```
git clone https://github.com/redlib-org/redlib
cd redlib
cargo run
```
