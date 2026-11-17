# AGENTS.md

## What this is

A fork of [Redlib](https://github.com/redlib-org/redlib), the private Reddit front-end. It starts from [Nachtalb/redlib](https://github.com/Nachtalb/redlib), which keeps working while Reddit blocks anonymous requests, and adds fixes from upstream pull requests and other forks. `FORK.md` lists every change against upstream.

## Direction

- Stay close to Nachtalb's fork and upstream. Every local change makes the next sync harder, so prefer taking a change upstream or to Nachtalb over carrying it here.
- Prefer fixes that keep an instance working against Reddit's blocks over new features.
- New features default off and add no runtime dependency.
- Keep memory and binary size small.

## Syncing

- `upstream` is redlib-org/redlib, `nachtalb` is Nachtalb/redlib. Merge both into a branch and land it through a PR. Do not rebase: the history carries other people's commits.
- When a sync conflicts, keep the side that builds and port the other side's fix in its own commit.
- Each change taken from a PR or fork gets a line in `FORK.md` with its source link.

## Glossary

- **Upstream**: redlib-org/redlib.
- **Backend**: one way of fetching a page from Reddit (`json`, `cookied-json`, `gql`, `scraped`). See `CONFIGURATION.md`.
- **Session**: a logged-in `reddit_session` cookie set in `REDLIB_REDDIT_COOKIE`.
