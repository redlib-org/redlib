//! Where redlib gets Reddit data from.
//!
//! Every Reddit JSON request goes through [`json`]: it works out the request's
//! [`Kind`], then tries the backends configured for that kind one after another
//! (`REDLIB_ENDPOINTS_<KIND>`, `REDLIB_ENDPOINTS` or [`Kind::default_order`]),
//! until one returns the page or Reddit gives a real answer (not found,
//! private, …). [`score`] reorders the configured backends by how well they
//! have been doing lately. A backend is one file in this directory with a type
//! implementing [`Backend`], plus one line in [`BACKENDS`].

mod cookied_json;
pub mod gql;
mod json;
#[cfg(test)]
mod live_tests;
mod score;
pub mod scraped;
pub mod stale;

use crate::config::get_setting;
use crate::oauth::force_refresh_token;
use crate::utils::param;
use cached::proc_macro::cached;
use log::{debug, error, info, warn};
use score::{Outcome, Ranking};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

/// A place redlib can get Reddit's JSON from.
pub trait Backend: Sync {
	/// The name used in settings and logs.
	fn name(&self) -> &'static str;

	/// Whether this instance can use the backend at all (a session is set, it isn't switched off).
	fn available(&self) -> bool {
		true
	}

	fn supports(&self, kind: Kind) -> bool {
		let _ = kind;
		true
	}

	/// Logged-in sessions see quarantined and gated subreddits as such even
	/// when the visitor opted in, so those pages need a logged-out backend.
	fn logged_in(&self) -> bool {
		false
	}

	/// What one attempt costs compared to the others. Sessions cost more: they
	/// use up the account's request budget and put the account at risk.
	fn cost(&self) -> f64 {
		1.0
	}

	/// `path` (a JSON API path like `/r/rust/hot.json?raw_json=1`) in the
	/// shape Reddit's JSON API returns it.
	fn fetch<'a>(&'a self, path: &'a str, quarantine: bool) -> Fetch<'a>;
}

pub type Fetch<'a> = Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'a>>;

/// Every backend this build has.
static BACKENDS: &[&dyn Backend] = &[&json::Json, &gql::Gql, &cookied_json::CookiedJson, &scraped::Scraped];

/// The kinds of Reddit data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// Just a post (`/by_id/t3_<id>`).
	Post,
	/// A post page with its comments, "more comments" and comment permalinks.
	Comments,
	/// Posts of a subreddit, popular and the front page.
	Listing,
	/// r/all, which Reddit retired; JSON still serves it as `r/all%2B`.
	All,
	/// Subreddit info (`/r/<sub>/about`).
	Subreddit,
	/// A user's profile, overview, posts and comments.
	User,
	/// Post, comment, subreddit and user search.
	Search,
	Wiki,
	Duplicates,
}

impl Kind {
	pub const ALL: [Self; 9] = [
		Self::Post,
		Self::Comments,
		Self::Listing,
		Self::All,
		Self::Subreddit,
		Self::User,
		Self::Search,
		Self::Wiki,
		Self::Duplicates,
	];

	/// The backends tried when neither `REDLIB_ENDPOINTS_<KIND>` nor
	/// `REDLIB_ENDPOINTS` is set. GraphQL is what the Android app uses, so it
	/// goes first where its data is as good as the JSON API's; anonymous JSON is
	/// always blocked for comments, so it goes last there.
	fn default_order(self) -> &'static str {
		match self {
			Self::Post | Self::Subreddit | Self::Search => "gql, json, cookied-json",
			// Reddit answers anonymous user and wiki JSON with 403 whatever the client
			Self::User | Self::Wiki => "gql, cookied-json",
			Self::Comments => "gql, cookied-json, scraped, json",
			// Only `scraped` serves the sorts Reddit refuses to the JSON API for combined feeds
			Self::Listing => "json, gql, scraped, cookied-json",
			Self::All => "json, gql, cookied-json",
			Self::Duplicates => "json, cookied-json",
		}
	}

	pub fn name(self) -> &'static str {
		match self {
			Self::Post => "post",
			Self::Comments => "comments",
			Self::Listing => "listing",
			Self::All => "all",
			Self::Subreddit => "subreddit",
			Self::User => "user",
			Self::Search => "search",
			Self::Wiki => "wiki",
			Self::Duplicates => "duplicates",
		}
	}

	/// The JSON API path for `path`: Reddit answers `/r/all` with 403 but
	/// still serves the same feed as the "multireddit" `/r/all%2B`. A bare `+`
	/// reads as a space and gets a fixed copy of r/popular's first page.
	pub fn json_path(path: &str) -> std::borrow::Cow<'_, str> {
		match path.strip_prefix("/r/all") {
			Some(rest) if Self::of(path) == Self::All => format!("/r/all%2B{rest}").into(),
			_ => path.into(),
		}
	}

	pub fn of(path: &str) -> Self {
		let route = path.split('?').next().unwrap_or_default().trim_end_matches(".json");
		let segments: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
		match segments.as_slice() {
			["by_id", ..] => Self::Post,
			["duplicates", ..] => Self::Duplicates,
			["comments", _, ..] | [_, _, "comments", _, ..] => Self::Comments,
			["user" | "u", ..] => Self::User,
			["search", ..] | ["subreddits" | "users", "search", ..] | ["r", _, "search", ..] => Self::Search,
			["r", _, "wiki", ..] | ["wiki", ..] => Self::Wiki,
			["r", _, "about", ..] | ["r", _, "api", "widgets"] => Self::Subreddit,
			["r", "all", ..] => Self::All,
			_ => Self::Listing,
		}
	}
}

/// Why a backend didn't return the page.
#[derive(Debug)]
pub enum Failure {
	/// The backend can't serve this particular path.
	Unsupported,
	/// Reddit refused the request ("blocked by network security", refused session).
	Blocked,
	RateLimited(String),
	/// The request or the response didn't make it through.
	Network(String),
	/// A response that can't be used.
	Invalid(String),
	/// Reddit's real answer (not found, private, banned, quarantined, …).
	Final(String),
}

type Order = Vec<&'static dyn Backend>;

/// The backends named in the comma-separated `list`, without duplicates, and
/// the names that aren't backends of this build.
fn parse_order(list: &str) -> (Order, Vec<String>) {
	let mut order: Order = Vec::new();
	let mut unknown = Vec::new();
	for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
		match BACKENDS.iter().find(|backend| backend.name() == name.to_lowercase()) {
			Some(backend) if !order.iter().any(|known| known.name() == backend.name()) => order.push(*backend),
			Some(_) => {}
			None => unknown.push(name.to_string()),
		}
	}
	(order, unknown)
}

/// The configured order for `kind`: `REDLIB_ENDPOINTS_<KIND>`, else
/// `REDLIB_ENDPOINTS`, else the default. Logs names it can't use.
fn configured_order(kind: Kind) -> Order {
	let key = format!("REDLIB_ENDPOINTS_{}", kind.name().to_uppercase());
	let setting = |key: &str| get_setting(key).filter(|list| !list.trim().is_empty());
	let (setting, list) = match setting(&key) {
		Some(list) => (key, list),
		None => match setting("REDLIB_ENDPOINTS") {
			Some(list) => ("REDLIB_ENDPOINTS".to_string(), list),
			None => return parse_order(kind.default_order()).0,
		},
	};
	let (order, unknown) = parse_order(&list);
	for name in unknown {
		let known: Vec<&str> = BACKENDS.iter().map(|backend| backend.name()).collect();
		warn!("{setting}: unknown backend {name:?}, this build has {}", known.join(", "));
	}
	for backend in order.iter().filter(|backend| !backend.supports(kind)) {
		warn!("{setting}: {} can't serve {} requests, skipping it", backend.name(), kind.name());
	}
	if order.is_empty() {
		warn!("{setting} names no backend, using the default for {}", kind.name());
		return parse_order(kind.default_order()).0;
	}
	order
}

static RANKINGS: LazyLock<Vec<Mutex<Ranking>>> = LazyLock::new(|| {
	let now = Instant::now();
	Kind::ALL.map(|kind| Mutex::new(Ranking::new(configured_order(kind), now))).into()
});

fn ranking(kind: Kind) -> MutexGuard<'static, Ranking> {
	RANKINGS[kind as usize].lock().unwrap_or_else(PoisonError::into_inner)
}

/// The order backends are tried in for `kind` right now.
fn order(kind: Kind) -> Order {
	ranking(kind).order(Instant::now())
}

fn usable(kind: Kind, backend: &dyn Backend) -> bool {
	backend.available() && backend.supports(kind)
}

/// Logs the backends each kind of request uses, and any setting it can't use.
pub fn log_orders() {
	for kind in Kind::ALL {
		let names: Vec<&str> = ranking(kind).configured().iter().filter(|b| usable(kind, **b)).map(|b| b.name()).collect();
		info!("Backends for {}: {}", kind.name(), names.join(", "));
	}
}

/// Each kind with its usable backends in their current order and their
/// scores, like `gql 0.92, json 0.40`.
pub fn status() -> Vec<(&'static str, String)> {
	let now = Instant::now();
	Kind::ALL
		.iter()
		.map(|&kind| {
			let scores: Vec<String> = ranking(kind)
				.scores(now)
				.into_iter()
				.filter(|(name, _)| BACKENDS.iter().any(|b| b.name() == *name && usable(kind, *b)))
				.map(|(name, score)| format!("{name} {score:.2}"))
				.collect();
			(kind.name(), scores.join(", "))
		})
		.collect()
}

/// How a backend's attempt counts for its score; `None` doesn't count.
fn outcome(result: &Result<Value, Failure>, quarantine_hidden: bool) -> Option<Outcome> {
	match result {
		Ok(_) => Some(Outcome::Success),
		Err(Failure::Unsupported) => None,
		Err(Failure::Final(msg)) if quarantine_hidden && (msg == "quarantined" || msg == "gated") => None,
		Err(Failure::Final(_)) => Some(Outcome::Success),
		Err(Failure::Network(_)) => Some(Outcome::NetworkError),
		Err(Failure::Blocked | Failure::RateLimited(_) | Failure::Invalid(_)) => Some(Outcome::Failure),
	}
}

/// Fetches `path` from Reddit's JSON API or whichever backend serves it.
pub async fn json(path: String, quarantine: bool) -> Result<Value, String> {
	fetch(path, quarantine).await.map(|(json, _)| json)
}

/// The backend whose "more comments" cursor `path` carries. Only that backend
/// can load the batch the cursor points at, whatever the configured order.
fn cursor_owner(path: &str) -> Option<&'static str> {
	let query = format!("?{}", path.split_once('?')?.1);
	param(&query, "more_cursor")?;
	Some(match param(&query, "more_query") {
		Some(query) if query == gql::MORE_QUERY => "gql",
		_ => "scraped",
	})
}

/// The r/popular region `path` asks for, unless it's global. Only `json`
/// applies it, so it is tried first.
fn region(path: &str) -> Option<String> {
	if !path.starts_with("/r/popular") {
		return None;
	}
	param(&format!("?{}", path.split_once('?')?.1), "geo_filter").filter(|region| !region.is_empty() && region != "GLOBAL")
}

/// Fetches `path` from the first backend that serves it, and names that backend.
/// When Reddit refuses or fails, the last good response is served instead.
pub async fn fetch(path: String, quarantine: bool) -> Result<(Value, &'static str), String> {
	let found = match fetch_fresh(path.clone(), quarantine).await {
		Ok(found) => Ok(found),
		Err((msg, true)) => stale::load(&path, quarantine).ok_or(msg),
		Err((msg, false)) => Err(msg),
	};
	// Reddit echoes the region it applied; other backends' feeds have none
	if let Ok((json, _)) = &found {
		if region(&path).is_some() && json["data"]["geo_filter"].is_null() {
			stale::mark_region_ignored();
		}
	}
	found
}

/// `fetch` without the stale fallback; the error says whether Reddit failed
/// (blocked, rate limited, down) rather than answered.
#[cached(max_size = 100, ttl_secs = 30)]
async fn fetch_fresh(path: String, quarantine: bool) -> Result<(Value, &'static str), (String, bool)> {
	let kind = Kind::of(&path);
	let mut blocked = false;
	let mut failed = false;
	let mut last_error = None;

	let backends: Order = match cursor_owner(&path).filter(|_| kind == Kind::Comments) {
		Some(owner) => BACKENDS.iter().copied().filter(|backend| backend.name() == owner).collect(),
		None => {
			let mut order = order(kind);
			if region(&path).is_some() {
				order.sort_by_key(|backend| backend.name() != "json");
			}
			order
		}
	};
	for backend in backends.into_iter().filter(|backend| usable(kind, *backend)) {
		let result = backend.fetch(&path, quarantine).await;
		if let Some(outcome) = outcome(&result, quarantine && backend.logged_in()) {
			ranking(kind).record(backend.name(), outcome, Instant::now());
		}
		let error = match result {
			Ok(json) => {
				stale::store(&path, quarantine, &json, backend.name());
				return Ok((json, backend.name()));
			}
			Err(Failure::Unsupported) => {
				debug!("{} can't serve {path}", backend.name());
				continue;
			}
			Err(Failure::Final(msg)) if quarantine && backend.logged_in() && (msg == "quarantined" || msg == "gated") => msg,
			Err(Failure::Final(msg)) => return Err((msg, false)),
			Err(Failure::Blocked) => {
				blocked = true;
				failed = true;
				"blocked".to_string()
			}
			Err(Failure::RateLimited(msg) | Failure::Network(msg) | Failure::Invalid(msg)) => {
				failed = true;
				msg
			}
		};
		warn!("{} {} failed for {path}: {error}", kind.name(), backend.name());
		last_error = Some(error);
	}

	let msg = match last_error {
		Some(error) if !blocked => error,
		_ => blocked_error(&path),
	};
	Err((msg, failed))
}

/// Turns the errors Reddit reports inside a JSON body into failures.
async fn check_json(json: Value, path: &str) -> Result<Value, Failure> {
	// If user is suspended
	if let Some(data) = json.get("data") {
		if let Some(is_suspended) = data.get("is_suspended").and_then(Value::as_bool) {
			if is_suspended {
				return Err(Failure::Final("suspended".into()));
			}
		}
	}

	// If Reddit returned an error
	if json["error"].is_i64() {
		// OAuth token has expired; http status 401
		if json["message"] == "Unauthorized" {
			error!("Forcing a token refresh");
			let () = force_refresh_token().await;
			return Err(Failure::Invalid("OAuth token has expired. Please refresh the page!".to_string()));
		}

		for reason in ["quarantined", "gated", "private", "banned"] {
			if json["reason"] == reason {
				return Err(Failure::Final(reason.into()));
			}
		}

		Err(Failure::Final(format!(
			"Reddit error {} \"{}\": {} | {path}",
			json["error"], json["reason"], json["message"]
		)))
	} else {
		Ok(json)
	}
}

/// The failure for a response that Reddit did not send as JSON.
fn non_json_failure(status: u16, parse_error: &str, path: &str) -> Failure {
	let msg = non_json_response_error(status, parse_error, path);
	match status {
		403 => Failure::Blocked,
		429 => Failure::RateLimited(msg),
		500..=599 => Failure::Network(msg),
		_ => Failure::Invalid(msg),
	}
}

/// Builds the user-facing error for a response that Reddit did not send as JSON.
///
/// Reddit answers logged-out clients - for every `.json` endpoint, not just the
/// thread/comment ones - with an HTML "You've been blocked by network security"
/// page rather than JSON. Reporting that as a JSON parse error sends users
/// looking for a bug in Redlib, so name the real cause and the setting that
/// fixes it instead.
///
/// The text is the same whether or not a session is configured, so visitors
/// can't tell if the instance has one.
///
/// See <https://github.com/redlib-org/redlib/issues/591>.
fn non_json_response_error(status: u16, parse_error: &str, path: &str) -> String {
	if status == 403 {
		return blocked_error(path);
	}

	if status == 429 {
		return format!("Reddit is rate limiting this instance (429 Too Many Requests). Try again in a minute. | {path}");
	}

	if (500..=599).contains(&status) {
		return "Reddit is having issues, check if there's an outage".to_string();
	}

	format!("Failed to parse page JSON data: {parse_error} | {path}")
}

const BLOCKED_ERROR_PREFIX: &str = "Reddit blocked this request (403 Forbidden)";

fn blocked_error(path: &str) -> String {
	format!("{BLOCKED_ERROR_PREFIX}: Reddit only serves this page to logged-in sessions. | {path}")
}

/// Whether `json()` failed because Reddit only serves the page to logged-in sessions.
pub fn is_blocked_error(msg: &str) -> bool {
	msg.starts_with(BLOCKED_ERROR_PREFIX)
}

#[cfg(test)]
mod tests {
	use super::*;
	use sealed_test::prelude::*;

	fn names(order: &[&dyn Backend]) -> Vec<&'static str> {
		order.iter().map(|backend| backend.name()).collect()
	}

	#[test]
	fn test_region() {
		assert_eq!(region("/r/popular/hot.json?&raw_json=1&geo_filter=DE"), Some("DE".into()));
		assert_eq!(region("/r/popular/hot.json?&raw_json=1&geo_filter=GLOBAL"), None);
		assert_eq!(region("/r/rust/hot.json?geo_filter=DE"), None);
	}

	#[test]
	fn test_json_path() {
		assert_eq!(Kind::json_path("/r/all/new.json?raw_json=1"), "/r/all%2B/new.json?raw_json=1");
		assert_eq!(Kind::json_path("/r/all.json?raw_json=1"), "/r/all%2B.json?raw_json=1");
		for path in ["/r/allthingsdnd/hot.json", "/r/all/search.json?q=rust", "/r/all/about.json", "/r/rust/hot.json"] {
			assert_eq!(Kind::json_path(path), path);
		}
	}

	#[test]
	fn test_cursor_owner() {
		assert_eq!(cursor_owner("/r/rust/comments/abc.json?sort=top&raw_json=1"), None);
		assert_eq!(cursor_owner("/r/rust/comments/abc.json?more_cursor=xyz&more_query=gql&raw_json=1"), Some("gql"));
		assert_eq!(cursor_owner("/r/rust/comments/abc.json?more_cursor=xyz&more_query=top&raw_json=1"), Some("scraped"));
		assert_eq!(cursor_owner("/r/rust/comments/abc.json?more_cursor=xyz"), Some("scraped"));
	}

	#[test]
	fn test_parse_order() {
		let (order, unknown) = parse_order(" GQL ,json,, scraped,gql, foo ");
		assert_eq!(names(&order), ["gql", "json", "scraped"]);
		assert_eq!(unknown, ["foo"]);
	}

	#[test]
	fn test_default_orders_name_only_backends_that_serve_the_kind() {
		for kind in Kind::ALL {
			let (order, _) = parse_order(kind.default_order());
			assert!(order.iter().all(|backend| backend.supports(kind)), "{kind:?}");
		}
	}

	#[test]
	#[sealed_test]
	fn test_default_order() {
		assert_eq!(names(&order(Kind::Comments)), ["gql", "cookied-json", "scraped", "json"]);
		assert_eq!(names(&order(Kind::Listing)), ["json", "gql", "scraped", "cookied-json"]);
		assert_eq!(names(&order(Kind::Wiki)), ["gql", "cookied-json"]);
		assert_eq!(names(&order(Kind::All)), ["json", "gql", "cookied-json"]);
	}

	#[test]
	#[sealed_test(env = [("REDLIB_ENDPOINTS", "json, cookied-json"), ("REDLIB_ENDPOINTS_COMMENTS", "scraped,gql")])]
	fn test_configured_order() {
		assert_eq!(names(&order(Kind::Comments)), ["scraped", "gql"]);
		assert_eq!(names(&order(Kind::User)), ["json", "cookied-json"]);
	}

	#[test]
	#[sealed_test(env = [("REDLIB_ENDPOINTS", ""), ("REDLIB_ENDPOINTS_LISTING", "gql")])]
	fn test_empty_setting_means_unset() {
		assert_eq!(names(&order(Kind::Wiki)), ["gql", "cookied-json"]);
		assert_eq!(names(&order(Kind::Listing)), ["gql"]);
	}

	#[test]
	#[sealed_test(env = [("REDLIB_ENDPOINTS_USER", "nonsense")])]
	fn test_unusable_order_falls_back_to_the_default() {
		assert_eq!(names(&order(Kind::User)), ["gql", "cookied-json"]);
	}

	#[test]
	#[sealed_test]
	fn test_order_from_config_file() {
		std::fs::write("redlib.toml", "REDLIB_ENDPOINTS_SEARCH = \"json\"").unwrap();
		assert_eq!(names(&order(Kind::Search)), ["json"]);
	}

	#[test]
	fn test_kind_of_path() {
		for (path, kind) in [
			("/by_id/t3_abc.json?raw_json=1", Kind::Post),
			("/r/rust/comments/abc.json?sort=top&raw_json=1", Kind::Comments),
			("/r/rust/comments/abc/some_title/def.json?context=3", Kind::Comments),
			("/comments/abc.json", Kind::Comments),
			("/user/spez/comments/abc/title.json", Kind::Comments),
			("/user/spez/comments.json?sort=new", Kind::User),
			("/user/spez/about.json?raw_json=1", Kind::User),
			("/r/rust.json", Kind::Listing),
			("/r/rust+linux/new.json", Kind::Listing),
			("/.json?raw_json=1", Kind::Listing),
			("/r/all/new.json?raw_json=1", Kind::All),
			("/r/all.json", Kind::All),
			("/r/all/search.json?q=rust", Kind::Search),
			("/r/rust/about.json?raw_json=1", Kind::Subreddit),
			("/r/rust/api/widgets?raw_json=1", Kind::Subreddit),
			("/search.json?q=rust", Kind::Search),
			("/r/rust/search.json?q=async&restrict_sr=on", Kind::Search),
			("/subreddits/search.json?q=rust&limit=3", Kind::Search),
			("/users/search.json?q=rust", Kind::Search),
			("/r/rust/wiki/index.json?raw_json=1", Kind::Wiki),
			("/duplicates/abc.json", Kind::Duplicates),
		] {
			assert_eq!(Kind::of(path), kind, "{path}");
		}
	}

	#[test]
	fn test_backend_names_are_unique() {
		let mut names: Vec<&str> = BACKENDS.iter().map(|backend| backend.name()).collect();
		names.sort_unstable();
		names.dedup();
		assert_eq!(names.len(), BACKENDS.len());
	}

	#[test]
	fn test_blocked_response_reports_the_block_not_a_json_error() {
		// Reddit's block page is HTML, so serde fails on it. The message the
		// user sees must not blame JSON parsing.
		let msg = non_json_response_error(403, "expected value at line 1 column 1", "/r/politics/comments/1x9xx9e/.json?&sort=confidence&raw_json=1");
		assert!(msg.contains("403"), "{msg}");
		assert!(!msg.contains("Failed to parse page JSON data"), "{msg}");

		// The error page links the setting that makes the page work again.
		assert!(is_blocked_error(&msg), "{msg}");
		assert!(include_str!("../../templates/error.html").contains("REDLIB_REDDIT_COOKIE"));
		assert!(msg.contains("/r/politics/comments/1x9xx9e/.json"), "{msg}");
		assert_eq!(msg, blocked_error("/r/politics/comments/1x9xx9e/.json?&sort=confidence&raw_json=1"));
	}

	#[test]
	fn test_rate_limited_html_response_says_so() {
		assert_eq!(
			non_json_response_error(429, "expected value at line 1 column 1", "/r/x/hot.json"),
			"Reddit is rate limiting this instance (429 Too Many Requests). Try again in a minute. | /r/x/hot.json"
		);
	}

	#[test]
	fn test_non_json_server_error_keeps_the_outage_message() {
		assert_eq!(
			non_json_response_error(502, "expected value at line 1 column 1", "/r/x/hot.json"),
			"Reddit is having issues, check if there's an outage"
		);
	}

	#[test]
	fn test_other_non_json_responses_still_report_a_parse_error() {
		assert_eq!(
			non_json_response_error(200, "expected value at line 1 column 1", "/r/x/hot.json"),
			"Failed to parse page JSON data: expected value at line 1 column 1 | /r/x/hot.json"
		);
	}

	#[test]
	fn test_localization_popular() {
		crate::client::block_on_shared(async {
			let val = json("/r/popular/hot.json?&raw_json=1&geo_filter=GLOBAL".to_string(), false).await.unwrap();
			assert_eq!("GLOBAL", val["data"]["geo_filter"].as_str().unwrap());
		});
	}

	#[test]
	fn test_private_sub() {
		crate::client::block_on_shared(async {
			let link = json("/r/suicide/about.json?raw_json=1".into(), true).await;
			assert_eq!(link, Err("private".into()));
		});
	}

	#[test]
	fn test_banned_sub() {
		crate::client::block_on_shared(async {
			let link = json("/r/aaa/about.json?raw_json=1".into(), true).await;
			assert_eq!(link, Err("banned".into()));
		});
	}

	#[test]
	fn test_gated_sub() {
		crate::client::block_on_shared(async {
			// quarantine to false to specifically catch when we _don't_ catch it
			let link = json("/r/drugs/about.json?raw_json=1".into(), false).await;
			assert_eq!(link, Err("gated".into()));
		});
	}
}
