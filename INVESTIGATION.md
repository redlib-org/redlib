# Anonymous Reddit thread investigation

Status: verified source fix and separately built artifact. Deployment remains a
later, user-controlled operation.

## Scope and source state

- Investigation began 2026-10-22 on Debian 13 from Redlib commit
  `ba116c1186f6b0f9d45f0ae50c827e13eb2f5ea2` (v0.40.0).
- Upstream was rechecked at 2026-10-22T23:02:00Z. The same commit was still
  the head of `redlib-org/redlib:main`; no later upstream fix existed.
- Upstream was checked again immediately before handoff on 2026-10-23. Main
  was still `ba116c11`; #598 remained open. Its new comments independently
  reported both that the encoding workaround was blocked and that builds of
  the polak14 Shreddit fork were loading comments successfully.
- The running Docker deployment was not inspected, restarted, rebuilt, or
  modified.
- No account, login, authenticated user session, alternate server, proxy, VPN,
  browser automation, or official Reddit application was used.

Relevant upstream reports and changes:

- [#591](https://github.com/redlib-org/redlib/issues/591) established the
  route-specific failure: listings succeeded while comments JSON returned an
  HTML 403 response.
- [PR #592](https://github.com/redlib-org/redlib/pull/592) and
  [commit 0a3e9cde](https://github.com/redlib-org/redlib/commit/0a3e9cde456b9ce92008d0e3846301c5efcd6565)
  introduced the encoded-separator transport workaround.
- [#598](https://github.com/redlib-org/redlib/issues/598) and the latest
  comments in [#446](https://github.com/redlib-org/redlib/issues/446) showed
  that Reddit blocked that workaround again on 2026-10-22.

## Control gate and live observations

Direct `curl` received 403 responses from both listing and thread candidates.
It therefore failed the listing control and was not used to reject any
comments-specific transport. Redlib's TLS/client emulation is materially part
of the request contract.

At approximately 2026-10-22T22:52Z, an isolated public build containing the
Shreddit experiment was run on `127.0.0.1:18080`. It used Redlib's `wreq`
client and a freshly acquired anonymous Android OAuth identity. The process was
stopped after the checks. The installed Docker deployment was untouched.

The same client and network produced:

| Check | Result |
| --- | --- |
| OAuth listing control | Success; rate-limit headers were present |
| Legacy comments JSON | HTTP 403 HTML block response |
| Supplied Northern Ireland thread | Rendered successfully; legitimately reported zero comments |
| `r/mildlyinfuriating` thread from #598 | 25 parsed comments; post declared 5.4k |
| `r/politics` thread from #591 | 25 parsed comments; post declared 926 |
| `sort=new` | 25 parsed comments |
| Top-level continuation | Success; 10 additional comments |
| Focused-comment route | Success; two comments rendered and target highlighted |

The machine-readable, sanitized record is in
[`investigation-results.json`](investigation-results.json). It contains no
headers, cookies, tokens, bodies, device identifiers, or tracking identifiers.

## Candidate conclusions

### Selected: logged-out Shreddit

Reddit's logged-out page calls these public, Reddit-owned services:

- `GET /svc/shreddit/comments/r/{subreddit}/t3_{post}?sort={SORT}`
- `GET /svc/shreddit/comment/t1_{comment}?depth=0&subredditName={subreddit}`
- `POST /svc/shreddit/more-comments/{subreddit}/t3_{post}`

Post metadata remains available through anonymous OAuth at `/by_id/t3_{id}`.
The Shreddit responses contain stable structural elements for comment IDs,
parent IDs, bodies, scores, timestamps, and continuation cursors. This
candidate passed post identity, nested-parent, sort, focused-comment, empty
discussion, and continuation checks.

The initial endpoint/parser evidence came from the AGPL-3.0-only fork commits
[`6aed00a7`](https://github.com/polak14/redlib/commit/6aed00a7cadba3fbf24979322936d92859968ee8),
[`0baefabf`](https://github.com/polak14/redlib/commit/0baefabf4e806006e62c30a3b8766fa458bf08d2),
and [`4055fda1`](https://github.com/polak14/redlib/commit/4055fda1a5355f1507b23108795d827a16794b58).
The local implementation adds bounded reads, fixed-host enforcement, explicit
error categories, opaque thread/sort-bound continuation state, cache control,
credential-log redaction, and stricter URL/media sanitization.

### Rejected as the durable primary: encoded legacy comments JSON

The `%2Fcomments` variant was a routing discrepancy, not a supported anonymous
interface. It worked on 2026-10-21 and was blocked on 2026-10-22. It remains
useful evidence that the OAuth identity itself was valid, but adding another
encoding mutation would repeat the same failure mode.

### Not selected: anonymous GraphQL

No known-valid current comments operation and persisted-query hash was found in
the reviewed public source before Shreddit passed every candidate gate. Empty
or guessed GraphQL requests were deliberately not treated as authentication
tests. Per the investigation plan, further alternatives were not probed after
the first candidate qualified.

### Controls only: `/by_id` and `/duplicates`

`/by_id` supplies post metadata but not a comment tree. `/duplicates` supplies
related posts, not comments. Neither can restore thread semantics alone.

## Implementation contract

- `src/thread.rs` is the only thread-specific upstream adapter.
- `src/post.rs` consumes a validated `ThreadData` normalized into Redlib's
  existing post/comment representation. Generic listing requests are unchanged.
- Requests use hard-coded Reddit hosts, do not follow redirects, time out after
  20 seconds, and reject responses above 8 MiB.
- HTTP 401, 403, 404, 429, 451, transport failures, and malformed data are
  distinct internal outcomes. A comments 403 never triggers token refresh.
- Comment HTML is structurally parsed and rebuilt from an allowlist. Scripts,
  event attributes, embedded documents, unsafe URL schemes, third-party media
  loads, and known tracking elements are removed before `safe` rendering.
- Successful logical thread requests are cached for 30 seconds. Failures are
  not converted into or cached as empty discussions.
- Continuations are base64url-encoded local state containing a version, thread
  ID, sort, and bounded upstream cursor. The thread and sort must match before a
  fixed-endpoint POST is made; arbitrary upstream URLs are never accepted.
- Anonymous tokens, response session headers, device UUIDs, and cookies are no
  longer emitted by OAuth info/trace logging.

## Verification of this implementation

Build environment:

- Rust/Cargo 1.99.1, rustfmt 1.10.0, Clippy 0.1.99
- GCC 14.2.0, CMake 3.31.6, libclang 19.1.7
- Debian build-essential 12.12

Completed checks:

- `cargo fmt --all -- --check`: pass
- `cargo test --locked --offline thread::tests`: pass, seven tests
- `cargo clippy --locked --offline --all-targets -- -D warnings`: pass
- `cargo build --locked --offline --release --bins`: pass
- Release artifacts:
  - `target/release/redlib` (14,509,024 bytes), SHA-256
    `c12ca6f566c8a3261ec5187cc715be9ede0053f18e2bc51c83ad374cdaaa6fe6`
  - `target/release/redlib-probe` (10,166,176 bytes), SHA-256
    `0d93931cefb905a1e91dad7cb2f6b6c4bddd576d6698e02fc488039e27ee7cc6`
- Four separate executions of the release probe, each creating a fresh
  anonymous identity: all listing controls and all 12 thread checks passed.
- Every probe listing contained 25 children. On each identity, the supplied
  thread validated as empty and both populated controls produced validated
  comments. The first three identities received 10-comment fragments; the
  final, exact checksummed artifact received 25-comment fragments. The large
  thread exposed a continuation throughout.
- Isolated release server on `127.0.0.1:18081`: listing, empty thread, populated
  thread, `sort=new`, continuation, focused-comment context, local comment
  search, and user filtering all returned successful HTML without an error
  page. The continuation produced nine further comments; the focused route
  rendered 11 comments and exactly one highlighted target.
- Replaying a continuation with a different sort produced the expected local
  malformed-request error rather than an upstream request.
- Rendered live pages contained no script tags or inline event attributes.
  Image/video rewriting and unsafe media removal are additionally covered by
  the structural sanitizer fixture.
- The loopback process was stopped after validation. The Docker deployment was
  never changed.

An explicit Rust 1.81 build could not be completed because the unmodified
v0.40.0 lockfile already contains `cpufeatures 0.3.1`, whose Edition 2024
manifest cannot be parsed by Cargo 1.81. This is a pre-existing declared-MSRV
drift, not introduced by the `scraper` addition; the selected `scraper 0.24`
itself uses Edition 2021. Current stable Rust builds cleanly.

## Reproduction and maintenance

Opt-in probe, after building:

```sh
cargo run --locked --bin redlib-probe
```

The probe first requires a successful listing control, uses uncached Redlib
requests, runs sequentially with at least two seconds between thread checks,
and emits sanitized JSON Lines. Pass explicit local thread paths as arguments
to replace the fixed set. Redirects remain disabled. A 429 is reported with its
`Retry-After` value and is not retried early.

Offline checks:

```sh
cargo fmt --all -- --check
cargo test --locked --offline thread::tests
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --locked --release
```

Maintenance procedure when Reddit changes Shreddit markup:

1. Run the probe and require the listing control to pass.
2. Capture only a deliberately sanitized fragment; never commit cookies,
   headers, tokens, cursors from a personal context, or tracking identifiers.
3. Update the selectors and normalization in `src/thread.rs` with a minimal
   fixture reproducing the changed structure.
4. Re-run sanitizer, nesting, focus, sort, continuation, empty-discussion, and
   malformed-input tests before any live check.
5. Confirm all outbound URLs remain compile-time-selected Reddit hosts.

## Remaining uncertainty

No six-hour, 30-minute-interval soak was completed after the final binary was
available. The four-identity validation and end-to-end run covered the request
contract but are not a time-based stability result. The earlier candidate test
and the final build both worked from the evening of 2026-10-22 through the
morning of 2026-10-23, but they were not periodic samples of the same artifact
and are not represented as a soak.

Reddit can still deliberately remove or gate its logged-out web interface. The
fix is durable in the engineering sense—supported-client transport, bounded and
validated requests, localized adaptation, diagnostics, and regression tests—
not a guarantee against a future upstream policy change.
