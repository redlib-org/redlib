# Redlib

> An alternative private front-end to Reddit, with its origins in [Libreddit](https://github.com/libreddit/libreddit).

![screenshot](https://i.ibb.co/18vrdxk/redlib-rust.png)

---

**10-second pitch:** Redlib is a private front-end like [Invidious](https://github.com/iv-org/invidious) but for Reddit. Browse the coldest takes of [r/unpopularopinion](https://redlib.matthew.science/r/unpopularopinion) without being [tracked](#reddit).

- 🚀 Fast: written in Rust for blazing-fast speeds and memory safety
- ☁️ Light: no JavaScript, no ads, no tracking, no bloat
- 🕵 Private: all requests are proxied through the server, including media
- 🔒 Secure: strong [Content Security Policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/CSP) prevents browser requests to Reddit

---

## Table of Contents

1. [Redlib](#redlib)
2. [Instances](#instances)
3. [About](#about)
   - [Built with](#built-with)
   - [How is it different from other Reddit front ends?](#how-is-it-different-from-other-reddit-front-ends)
     - [Teddit](#teddit)
     - [Libreddit](#libreddit)
4. [Comparison](#comparison)
   - [Speed](#speed)
   - [Privacy](#privacy)
     - [Reddit](#reddit)
     - [Redlib](#redlib-1)
       - [Server](#server)
       - [Official instance (redlib.matthew.science)](#official-instance-redlibmatthewscience)
5. [Deployment](#deployment)
   - [Docker](#docker)
     - [Docker Compose](#docker-compose)
     - [Docker CLI](#docker-cli)
   - Podman 
      - Quadlets

   - [Binary](#binary)
     - [Running as a systemd service](#running-as-a-systemd-service)
   - [Building from source](#building-from-source)
   - [Replit/Heroku/Glitch](#replit-heroku-glitch)
   - [launchd (macOS)](#launchd-macos)
6. [Configuration](#configuration)
   - [Instance settings](#instance-settings)
   - [Default user settings](#default-user-settings)

---

# Instances

> [!TIP]
> 🔗 **Want to automatically redirect Reddit links to Redlib? Use [LibRedirect](https://github.com/libredirect/libredirect) or [Privacy Redirect](https://github.com/SimonBrazell/privacy-redirect)!**

An up-to-date table of instances is available in [Markdown](https://github.com/redlib-org/redlib-instances/blob/main/instances.md) and [machine-readable JSON](https://github.com/redlib-org/redlib-instances/blob/main/instances.json).

Both files are part of the [redlib-instances](https://github.com/redlib-org/redlib-instances) repository. To contribute your [self-hosted instance](#deployment) to the list, see the [redlib-instances README](https://github.com/redlib-org/redlib-instances/blob/main/README.md).

For information on instance uptime, see the [Uptime Robot status page](https://stats.uptimerobot.com/mpmqAs1G2Q).

---

# About

> [!NOTE]
> Find Redlib on 💬 [Matrix](https://matrix.to/#/#redlib:matrix.org), 🐋 [Quay.io](https://quay.io/repository/redlib/redlib), :octocat: [GitHub](https://github.com/redlib-org/redlib), and 🦊 [GitLab](https://gitlab.com/redlib/redlib).

Redlib hopes to provide an easier way to browse Reddit, without the ads, trackers, and bloat. Redlib was inspired by other alternative front-ends to popular services such as [Invidious](https://github.com/iv-org/invidious) for YouTube, [Nitter](https://github.com/zedeus/nitter) for Twitter, and [Bibliogram](https://sr.ht/~cadence/bibliogram/) for Instagram.

Redlib currently implements most of Reddit's (signed-out) functionalities but still lacks [a few features](https://github.com/redlib-org/redlib/issues).

## Built with

- [Rust](https://www.rust-lang.org/) - Programming language
- [Hyper](https://github.com/hyperium/hyper) - HTTP server and client
- [Rinja](https://github.com/rinja-rs/rinja) - Templating engine
- [Rustls](https://github.com/rustls/rustls) - TLS library

## How is it different from other Reddit front ends?

### Teddit

Teddit is another awesome open source project designed to provide an alternative frontend to Reddit. There is no connection between the two, and you're welcome to use whichever one you favor. Competition fosters innovation and Teddit's release has motivated me to build Redlib into an even more polished product.

If you are looking to compare, the biggest differences I have noticed are:

- Redlib is themed around Reddit's redesign whereas Teddit appears to stick much closer to Reddit's old design. This may suit some users better as design is always subjective.
- Redlib is written in [Rust](https://www.rust-lang.org) for speed and memory safety. It uses [Hyper](https://hyper.rs), a speedy and lightweight HTTP server/client implementation.

### Libreddit

While originating as a fork of Libreddit, the name "Redlib" was adopted to avoid legal issues, as Reddit only allows the use of their name if structured as "XYZ For Reddit".

Several technical improvements have also been made, including:

- **OAuth token spoofing**: To circumvent rate limits imposed by Reddit, OAuth token spoofing is used to mimick the most common iOS and Android clients. While spoofing both iOS and Android clients was explored, only the Android client was chosen due to content restrictions when using an anonymous iOS client.
- **Token refreshing**: The authentication token is refreshed every 24 hours, emulating the behavior of the official Android app.
- **HTTP header mimicking**: Efforts are made to send along as many of the official app's headers as possible to reduce the likelihood of Reddit's crackdown on Redlib's requests.

---

# Comparison

This section outlines how Redlib compares to Reddit in terms of speed and privacy.

## Speed

Last tested on January 12, 2024.

Results from Google PageSpeed Insights ([Redlib Report](https://pagespeed.web.dev/report?url=https%3A%2F%2Fredlib.matthew.science%2F), [Reddit Report](https://pagespeed.web.dev/report?url=https://www.reddit.com)).

| Performance metric  | Redlib   | Reddit    |
| ------------------- | -------- | --------- |
| Speed Index         | 0.6s     | 1.9s      |
| Performance Score   | 100%     | 64%       |
| Time to Interactive | **2.8s** | **12.4s** |

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

- **Logging:** In production (when running the binary, hosting with docker, or using the official instances), Redlib logs nothing. When debugging (running from source without `--release`), Redlib logs post IDs fetched to aid with troubleshooting.

- **Cookies:** Redlib uses optional cookies to store any configured settings in [the settings menu](https://redlib.matthew.science/settings). These are not cross-site cookies and the cookies hold no personal data.

#### Official instance (redlib.matthew.science)

The official instance is hosted at https://redlib.matthew.science.

- **Server:** The official instance runs a production binary, and thus logs nothing.

- **DNS:** The domain for the official instance uses Cloudflare as the DNS resolver. However, this site is not proxied through Cloudflare, and thus Cloudflare doesn't have access to user traffic.

- **Hosting:** The official instance is hosted on [Replit](https://replit.com/), which monitors usage to prevent abuse. I can understand if this invalidates certain users' threat models, and therefore, self-hosting, using unofficial instances, and browsing through Tor are welcomed.

---

# Deployment

This section covers multiple ways of deploying Redlib. Using [Docker](#docker) is recommended for production.

For configuration options, see the [Configuration section](#Configuration).

## Docker

[Docker](https://www.docker.com) lets you run containerized applications. Containers are loosely isolated environments that are lightweight and contain everything needed to run the application, so there's no need to rely on what's installed on the host.

Container images for Redlib are available at [quay.io](https://quay.io/repository/redlib/redlib), with support for `amd64`, `arm64`, and `armv7` platforms.

### Docker Compose

> [!IMPORTANT]
> These instructions assume the [Compose plugin](https://docs.docker.com/compose/migrate/#what-are-the-differences-between-compose-v1-and-compose-v2) has already been installed. If not, follow these [instructions on the Docker Docs](https://docs.docker.com/compose/install) for how to do so.

Copy `compose.yaml` and modify any relevant values (for example, the ports Redlib should listen on).

Start Redlib in detached mode (running in the background):

```bash
docker compose up -d
```

Stream logs from the Redlib container:

```bash
docker logs -f redlib
```

### Docker CLI

Deploy Redlib:

```bash
docker pull quay.io/redlib/redlib:latest
docker run -d --name redlib -p 8080:8080 quay.io/redlib/redlib:latest
```

Deploy using a different port on the host (in this case, port 80):

```bash
docker pull quay.io/redlib/redlib:latest
docker run -d --name redlib -p 80:8080 quay.io/redlib/redlib:latest
```

If you're using a reverse proxy in front of Redlib, prefix the port numbers with `127.0.0.1` so that Redlib only listens on the host port **locally**. For example, if the host port for Redlib is `8080`, specify `127.0.0.1:8080:8080`.

Stream logs from the Redlib container:

```bash
docker logs -f redlib
```
## Podman 

[Podman](https://podman.io/) lets you run containerized applications in a rootless fashion. Containers are loosely isolated environments that are lightweight and contain everything needed to run the application, so there's no need to rely on what's installed on the host.

Container images for Redlib are available at [quay.io](https://quay.io/repository/redlib/redlib), with support for `amd64`, `arm64`, and `armv7` platforms.

### Quadlets

> [!IMPORTANT]
> These instructions assume that you are on a systemd based distro with [podman](https://podman.io/). If not, follow these [instructions on podman's website](https://podman.io/docs/installation) for how to do so. 
> It also assumes you have used `loginctl enable-linger <username>` to enable the service to start for your user without logging in. 

Copy the `redlib.container` and `.env.example` files to `.config/containers/systemd/` and modify any relevant values (for example, the ports Redlib should listen on, renaming the .env file and editing its values, etc.).

To start Redlib either reboot or follow the instructions below:

Notify systemd of the new files
```bash
systemctl --user daemon-reload
```

Start the newly generated service file

```bash
systemctl --user start redlib.service
```

You can check the status of your container by using the following command:
```bash 
systemctl --user status redlib.service
```

## Binary

If you're on Linux, you can grab a binary from [the newest release](https://github.com/redlib-org/redlib/releases/latest) from GitHub.

Download the binary using [Wget](https://www.gnu.org/software/wget/):

```bash
wget https://github.com/redlib-org/redlib/releases/download/v0.31.0/redlib
```

Make the binary executable and change its ownership to `root`:

```bash
sudo chmod +x redlib && sudo chown root:root redlib
```

Copy the binary to `/usr/bin`:

```bash
sudo cp ./redlib /usr/bin/redlib
```

Deploy Redlib to `0.0.0.0:8080`:

```bash
redlib
```

> [!IMPORTANT]
> If you're proxying Redlib through NGINX (see [issue #122](https://github.com/libreddit/libreddit/issues/122#issuecomment-782226853)), add
>
> ```nginx
> proxy_http_version 1.1;
> ```
>
> to your NGINX configuration file above your `proxy_pass` line.

### Running as a systemd service

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

## Building from source

To deploy Redlib with changes not yet included in the latest release, you can build the application from source.

```bash
git clone https://github.com/redlib-org/redlib && cd redlib
cargo run
```

## Replit/Heroku

> [!WARNING]
> These are free hosting options, but they are _not_ private and will monitor server usage to prevent abuse. If you need a free and easy setup, this method may work best for you.

<a href="https://repl.it/github/redlib-org/redlib"><img src="https://repl.it/badge/github/redlib-org/redlib" alt="Run on Repl.it" height="32" /></a>
[![Deploy](https://www.herokucdn.com/deploy/button.svg)](https://heroku.com/deploy?template=https://github.com/redlib-org/redlib)

## launchd (macOS)

If you are on macOS, you can use the [launchd](https://en.wikipedia.org/wiki/Launchd) service available in `contrib/redlib.plist`.

Install it with `cp contrib/redlib.plist ~/Library/LaunchAgents/`.

Load and start it with `launchctl load ~/Library/LaunchAgents/redlib.plist`.

<!-- ## Cargo

Make sure Rust stable is installed along with `cargo`, Rust's package manager.

```bash
cargo install libreddit
``` -->

<!-- ## AUR

For ArchLinux users, Redlib is available from the AUR as [`libreddit-git`](https://aur.archlinux.org/packages/libreddit-git).

```bash
yay -S libreddit-git
```
## NetBSD/pkgsrc

For NetBSD users, Redlib is available from the official repositories.

```bash
pkgin install libreddit
```

Or, if you prefer to build from source

```bash
cd /usr/pkgsrc/libreddit
make install
``` -->

---

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
```

> [!NOTE]
> If you're deploying Redlib using the **Docker CLI or Docker Compose**, environment variables can be defined in a [`.env` file](https://docs.docker.com/compose/environment-variables/set-environment-variables/), allowing you to centralize and manage configuration in one place.
>
> To configure Redlib using a `.env` file, copy the `.env.example` file to `.env` and edit it accordingly.
>
> If using the Docker CLI, add ` --env-file .env` to the command that runs Redlib. For example:
>
> ```bash
> docker run -d --name redlib -p 8080:8080 --env-file .env quay.io/redlib/redlib:latest
> ```
>
> If using Docker Compose, no changes are needed as the `.env` file is already referenced in `compose.yaml` via the `env_file: .env` line.

## Command Line Flags

Redlib supports the following command line flags:

- `-4`, `--ipv4-only`: Listen on IPv4 only.
- `-6`, `--ipv6-only`: Listen on IPv6 only.
- `-r`, `--redirect-https`: Redirect all HTTP requests to HTTPS (no longer functional).
- `-a`, `--address <ADDRESS>`: Sets address to listen on. Default is `[::]`.
- `-p`, `--port <PORT>`: Port to listen on. Default is `8080`.
- `-H`, `--hsts <EXPIRE_TIME>`: HSTS header to tell browsers that this site should only be accessed over HTTPS. Default is `604800`.

## Instance settings

Assign a default value for each instance-specific setting by passing environment variables to Redlib in the format `REDLIB_{X}`. Replace `{X}` with the setting name (see list below) in capital letters.

| Name                      | Possible values | Default value          | Description                                                                                               |
| ------------------------- | --------------- | ----------------       | --------------------------------------------------------------------------------------------------------- |
| `SFW_ONLY`                | `["on", "off"]` | `off`                  | Enables SFW-only mode for the instance, i.e. all NSFW content is filtered.                                |
| `BANNER`                  | String          | (empty)                | Allows the server to set a banner to be displayed. Currently this is displayed on the instance info page. |
| `ROBOTS_DISABLE_INDEXING` | `["on", "off"]` | `off`                  | Disables indexing of the instance by search engines.                                                      |
| `PUSHSHIFT_FRONTEND`      | String          | `undelete.pullpush.io` | Allows the server to set the Pushshift frontend to be used with "removed" links.                          |
| `PORT`                    | Integer 0-65535 | `8080`                 | The **internal** port Redlib listens on.                                                                  |
| `ENABLE_RSS`              | `["on", "off"]` | `off`                  | Enables RSS feed generation.                                                                              |
| `FULL_URL`                | String          | (empty)                | Allows for proper URLs (for now, only needed by RSS)
## Default user settings

Assign a default value for each user-modifiable setting by passing environment variables to Redlib in the format `REDLIB_DEFAULT_{Y}`. Replace `{Y}` with the setting name (see list below) in capital letters.

| Name                                | Possible values                                                                                                                    | Default value |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| `THEME`                             | `["system", "light", "dark", "black", "dracula", "nord", "laserwave", "violet", "gold", "rosebox", "gruvboxdark", "gruvboxlight", "tokyoNight", "icebergDark", "doomone", "libredditBlack", "libredditDark", "libredditLight"]` | `system`      |
| `FRONT_PAGE`                        | `["default", "popular", "all"]`                                                                                                    | `default`     |
| `LAYOUT`                            | `["card", "clean", "compact"]`                                                                                                     | `card`        |
| `WIDE`                              | `["on", "off"]`                                                                                                                    | `off`         |
| `POST_SORT`                         | `["hot", "new", "top", "rising", "controversial"]`                                                                                 | `hot`         |
| `COMMENT_SORT`                      | `["confidence", "top", "new", "controversial", "old"]`                                                                             | `confidence`  |
| `BLUR_SPOILER`                      | `["on", "off"]`                                                                                                                    | `off`         |
| `SHOW_NSFW`                         | `["on", "off"]`                                                                                                                    | `off`         |
| `BLUR_NSFW`                         | `["on", "off"]`                                                                                                                    | `off`         |
| `USE_HLS`                           | `["on", "off"]`                                                                                                                    | `off`         |
| `HIDE_HLS_NOTIFICATION`             | `["on", "off"]`                                                                                                                    | `off`         |
| `AUTOPLAY_VIDEOS`                   | `["on", "off"]`                                                                                                                    | `off`         |
| `SUBSCRIPTIONS`                     | `+`-delimited list of subreddits (`sub1+sub2+sub3+...`)                                                                            | _(none)_      |
| `HIDE_AWARDS`                       | `["on", "off"]`                                                                                                                    | `off`         |
| `DISABLE_VISIT_REDDIT_CONFIRMATION` | `["on", "off"]`                                                                                                                    | `off`         |
| `HIDE_SCORE`                        | `["on", "off"]`                                                                                                                    | `off`         |
| `HIDE_SIDEBAR_AND_SUMMARY`          | `["on", "off"]`                                                                                                                    | `off`         |
| `FIXED_NAVBAR`                      | `["on", "off"]`                                                                                                                    | `on`          |
| `REMOVE_DEFAULT_FEEDS`              | `["on", "off"]`                                                                                                                    | `off`         |