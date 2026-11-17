<div align="center">

# Redlib

[![Fork of redlib-org/redlib](https://img.shields.io/badge/fork%20of-redlib--org%2Fredlib-lightgrey?logo=github)](https://github.com/redlib-org/redlib) [![ghcr.io](https://img.shields.io/badge/ghcr.io-nachtalb%2Fredlib-blue?logo=docker)](https://github.com/Nachtalb/redlib/pkgs/container/redlib) [![r.naa.gg](https://img.shields.io/endpoint?url=https://r.naa.gg/healthz/reddit.json&label=r.naa.gg)](https://r.naa.gg) [![Container build](https://github.com/Nachtalb/redlib/actions/workflows/main-docker.yml/badge.svg?branch=main)](https://github.com/Nachtalb/redlib/actions/workflows/main-docker.yml) [![Tests](https://github.com/Nachtalb/redlib/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/Nachtalb/redlib/actions/workflows/test.yml) [![Lint](https://github.com/Nachtalb/redlib/actions/workflows/lint.yml/badge.svg?branch=main)](https://github.com/Nachtalb/redlib/actions/workflows/lint.yml) [![Last commit](https://img.shields.io/github/last-commit/Nachtalb/redlib/main)](https://github.com/Nachtalb/redlib/commits/main)

An alternative private front-end to Reddit, with its origins in [Libreddit](https://github.com/libreddit/libreddit).

</div>

> [!IMPORTANT]
> **This is a fork of [redlib-org/redlib](https://github.com/redlib-org/redlib).**
> It ships multi-arch images for amd64, 386, arm64, arm/v7 and arm/v6, and includes many features and fixes from unmerged upstream pull requests plus its own. See [FORK.md](FORK.md) for the full list of changes.

> [!TIP]
> Run this fork's image, **`ghcr.io/nachtalb/redlib:latest`**, rather than upstream's `quay.io/redlib/redlib`.

> [!WARNING]
> If Reddit blocks your instance's requests, set a session in `REDLIB_REDDIT_COOKIE` ([setup](CONFIGURATION.md#reddit-session)).

![screenshot](https://i.ibb.co/18vrdxk/redlib-rust.png)

---

**10-second pitch:** Redlib is a private front-end like [Invidious](https://github.com/iv-org/invidious) but for Reddit. Browse the coldest takes of [r/unpopularopinion](https://farside.link/redlib/r/unpopularopinion) without being [tracked](#reddit).

- 🚀 Fast: written in Rust for blazing-fast speeds and memory safety
- ☁️ Light: no JavaScript, no ads, no tracking, no bloat
- 🕵 Private: all requests are proxied through the server, including media
- 🔒 Secure: strong [Content Security Policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/CSP) prevents browser requests to Reddit

---

# Instances

> [!TIP]
> 🔗 **Want to automatically redirect Reddit links to Redlib? Use [LibRedirect](https://github.com/libredirect/libredirect) or [Privacy Redirect](https://github.com/SimonBrazell/privacy-redirect)!**

An up-to-date table of instances is available in [Markdown](https://github.com/redlib-org/redlib-instances/blob/main/instances.md) and [machine-readable JSON](https://github.com/redlib-org/redlib-instances/blob/main/instances.json).

Both files are part of the [redlib-instances](https://github.com/redlib-org/redlib-instances) repository. To contribute your [self-hosted instance](#deployment) to the list, see the [redlib-instances README](https://github.com/redlib-org/redlib-instances/blob/main/README.md).

---

# Comparison

This section outlines how Redlib compares to Reddit in terms of speed and privacy.

## Speed

<details>
<summary>PageSpeed Insights results from January 12, 2024</summary>

> ⚠️ **Warning:** these numbers are outdated. Current instances may vary, as they adjust automatically to Reddit's blocking efforts.

Results from Google PageSpeed Insights ([Redlib Report](https://pagespeed.web.dev/report?url=https%3A%2F%2Fredlib.matthew.science%2F), [Reddit Report](https://pagespeed.web.dev/report?url=https://www.reddit.com)).

| Performance metric  | Redlib   | Reddit    |
|---------------------|----------|-----------|
| Speed Index         | 0.6s     | 1.9s      |
| Performance Score   | 100%     | 64%       |
| Time to Interactive | **2.8s** | **12.4s** |

</details>

## Privacy

What each side collects about you, according to Reddit's [privacy policy](https://www.redditinc.com/policies/privacy-policy) and [cookie notice](https://www.redditinc.com/policies/cookies), and Redlib's code.

| | Reddit | Redlib |
|---|:---:|:---:|
| **Logging** | | |
| IP address | 😱 | |
| User-agent string | 😱 | |
| Browser type | 😱 | |
| Operating system | 😱 | |
| Referral URLs | 😱 | |
| Device information (e.g. device IDs) | 😱 | |
| Device settings | 😱 | |
| Pages visited | 😱 | |
| Links clicked | 😱 | |
| The requested URL | 😱 | |
| Search terms | 😱 | |
| **Location** | | |
| GPS (consensual) | 😱 | |
| Bluetooth (consensual) | 😱 | |
| Content associated with a location (consensual) | 😱 | |
| Your IP address | 😱 | |
| **Cookies** | | |
| Authentication | 😱 | |
| Functionality | 😱 | 😌 your settings, nothing personal |
| Analytics and performance | 😱 | |
| Advertising | 😱 | |
| Third-party cookies | 😱 | |
| Third-party sites | 😱 | |

Redlib's server logs nothing about visitors in release builds (the binary, Docker images and public instances). Debug builds (`cargo run` without `--release`) log fetched post IDs to help with troubleshooting. Its cookies are not cross-site and only hold the settings you chose.

---

# Deployment

This section covers multiple ways of deploying Redlib. Using Docker is recommended for production.

For configuration options, see [CONFIGURATION.md](CONFIGURATION.md).

Images are on [ghcr.io](https://github.com/Nachtalb/redlib/pkgs/container/redlib) for `amd64`, `386`, `arm64`, `arm/v7` and `arm/v6`.

<details>
<summary><b>Docker Compose</b></summary>

Copy `compose.yaml` and `.env.example` (as `.env`), and adjust them, for example the port.

```bash
docker compose up -d    # start Redlib in the background
docker logs -f redlib   # follow its logs
```

</details>

<details>
<summary><b>Docker CLI</b></summary>

```bash
docker run -d --name redlib -p 8080:8080 ghcr.io/nachtalb/redlib:latest             # listen on port 8080
docker run -d --name redlib -p 127.0.0.1:8080:8080 ghcr.io/nachtalb/redlib:latest   # behind a reverse proxy: local only
docker run -d --name redlib -p 8080:8080 --env-file .env ghcr.io/nachtalb/redlib:latest  # with settings from .env
docker logs -f redlib                                                              # follow its logs
```

</details>

<details>
<summary><b>Podman (Quadlets)</b></summary>

> ❗ **Important:** This assumes a systemd-based distro with [Podman](https://podman.io/docs/installation), and `loginctl enable-linger <username>` so the service starts without you logging in.

Copy `redlib.container` and `.env.example` (as `.env`) to `~/.config/containers/systemd/` and adjust them, then reboot or:

```bash
systemctl --user daemon-reload          # pick up the new files
systemctl --user start redlib.service   # start Redlib
systemctl --user status redlib.service  # check on it
```

</details>

<details>
<summary><b>Binary (Linux)</b></summary>

On Linux, grab a static binary from [the latest release](https://github.com/Nachtalb/redlib/releases/latest), rebuilt on every push to `main`. It has builds for `x86_64`, `i686`, `aarch64`, `armv7` and `arm` (v6), plus `SHA256SUMS`.

```bash
curl -fL -o redlib https://github.com/Nachtalb/redlib/releases/latest/download/redlib-x86_64-unknown-linux-musl  # pick your architecture
sudo chmod +x redlib && sudo chown root:root redlib  # make it executable, owned by root
sudo cp ./redlib /usr/bin/redlib                     # install it
redlib                                               # listen on 0.0.0.0:8080
```

> ❗ **Important:** If you're proxying Redlib through NGINX (see [issue #122](https://github.com/libreddit/libreddit/issues/122#issuecomment-782226853)), add `proxy_http_version 1.1;` above your `proxy_pass` line.

#### Running as a systemd service

You can use the systemd service available in `contrib/redlib.service`
(install it on `/etc/systemd/system/redlib.service`).

That service can be optionally configured in terms of environment variables by
creating a file in `/etc/redlib.conf`. Use the `contrib/redlib.conf` as a
template. You can also add any of the settings in
[CONFIGURATION.md](CONFIGURATION.md).

When "Proxying using NGINX" where the proxy is on the same machine, you should
guarantee nginx waits for this service to start. Edit
`/etc/systemd/system/redlib.service.d/reverse-proxy.conf`:

```conf
[Unit]
Before=nginx.service
```

</details>

<details>
<summary><b>Replit/Heroku</b></summary>

> ⚠️ **Warning:** These are free hosting options, but they are _not_ private and will monitor server usage to prevent abuse. If you need a free and easy setup, this method may work best for you.

<a href="https://repl.it/github/Nachtalb/redlib"><img src="https://repl.it/badge/github/Nachtalb/redlib" alt="Run on Repl.it" height="32" /></a>
[![Deploy](https://www.herokucdn.com/deploy/button.svg)](https://heroku.com/deploy?template=https://github.com/Nachtalb/redlib)

</details>

<details>
<summary><b>launchd (macOS)</b></summary>

If you are on macOS, you can use the [launchd](https://en.wikipedia.org/wiki/Launchd) service available in `contrib/redlib.plist`.

Install it with `cp contrib/redlib.plist ~/Library/LaunchAgents/`.

Load and start it with `launchctl load ~/Library/LaunchAgents/redlib.plist`.

</details>

<details>
<summary><b>Cloudflare WAF rule</b></summary>

Behind Cloudflare, a custom WAF rule with the action **Managed Challenge** keeps bots and scrapers off your instance, so they don't burn through its Reddit quota. Visitors solve the challenge once and keep the `cf_clearance` cookie; static files, the health badge, `/info.json` and an optional status probe with a secret header pass without one. This is the rule r.naa.gg uses, with your host and a random secret filled in:

```
(http.request.full_uri wildcard r"https://redlib.example.com/*"
 and not http.cookie contains "cf_clearance"
 and not ends_with(http.request.uri.path, ".css")
 and not ends_with(http.request.uri.path, ".js")
 and not ends_with(http.request.uri.path, "touch-icon-iphone.png")
 and not ends_with(http.request.uri.path, "favicon.ico")
 and not ends_with(http.request.uri.path, ".woff2")
 and not ends_with(http.request.uri.path, "manifest.json")
 and http.request.uri.path ne "/healthz/reddit.json"
 and http.request.uri.path ne "/info.json"
 and not (http.request.uri.path in {"/settings" "/healthz/reddit"}
          and http.request.headers["x-status-probe"][0] eq "<random secret>"))
```

Paste it under Security → WAF → Custom rules → Edit expression. Leave out the last condition if you don't run a status page.

</details>

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

Redlib is configured with environment variables, a `redlib.toml` file or command-line flags. All settings, the Reddit session, the backend order and proxies are described in [CONFIGURATION.md](CONFIGURATION.md).

---

# Building from source

```bash
git clone https://github.com/Nachtalb/redlib && cd redlib
cargo build --release              # the binary ends up in target/release/redlib
```

Redlib talks to Reddit through [wreq](https://github.com/0x676e67/wreq), which uses [BoringSSL](https://boringssl.googlesource.com/boringssl/) with wreq's patches (via the [`btls-sys`](https://crates.io/crates/btls-sys) crate) so its TLS handshake looks like a real browser's. The BoringSSL source ships with the crate and is patched and compiled during the build, so you need `git`, `cmake`, a C/C++ compiler and `libclang` (for bindgen). Certificates are checked against Mozilla's trust store compiled into the binary.

On Debian/Ubuntu:

```bash
sudo apt install build-essential git cmake libclang-dev
```

On Windows (not tested by this fork), install MSVC, which you likely already have for Rust, then:

```pwsh
# Update your PATH afterwards; some installers don't (hence -i, interactive mode).
winget install -i Kitware.CMake
winget install -i NASM.NASM
winget install -i LLVM.LLVM
```

---

# Sister projects

- [Redlib](https://github.com/redlib-org/redlib): upstream of this fork ([Matrix](https://matrix.to/#/#redlib:matrix.org)).
- [Libreddit](https://github.com/libreddit/libreddit): Reddit, where Redlib started. Dead.
- [Teddit](https://codeberg.org/teddit/teddit): Reddit. Dead.
- [Invidious](https://github.com/iv-org/invidious): YouTube.
- [Nitter](https://github.com/zedeus/nitter): Twitter. Dead, replaced by Shitter.
- [Shitter](https://codeberg.org/shitter/shitter): Twitter.
- [Bibliogram](https://sr.ht/~cadence/bibliogram/): Instagram. Dead.
