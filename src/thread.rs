//! Fetches public thread data without using Reddit's blocked comments JSON route.
//!
//! Post metadata still comes from `/by_id`. Comment trees come from the HTML
//! fragments used by Reddit's logged-out Shreddit client and are normalized to
//! the legacy listing shape consumed by `post.rs`.
//!
//! The endpoint discovery and initial parser were informed by polak14/redlib
//! commits 6aed00a7, 0baefabf, and 4055fda1 (AGPL-3.0-only). This version makes
//! the transport primary, validates and binds continuation state, bounds all
//! responses, and rebuilds comment HTML through an allowlist.

use crate::client::{json, json_uncached, CLIENT};
use crate::utils::{format_url, param};
use base64::{engine::general_purpose, Engine as _};
use cached::proc_macro::cached;
use chrono::DateTime;
use futures_lite::StreamExt;
use scraper::{ElementRef, Html, Node, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json as json_value, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::time::timeout;
use wreq::{Method, Response};

const SVC_BASE: &str = "https://www.reddit.com/svc/shreddit";
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CURSOR_TOKEN_BYTES: usize = 8192;
const MAX_UPSTREAM_CURSOR_BYTES: usize = 4096;
const CONTINUATION_VERSION: u8 = 1;

static COMMENT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("shreddit-comment").expect("valid static selector"));
static MORE: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"faceplate-partial[src*="/more-comments/"]"#).expect("valid static selector"));
static CURSOR: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"input[name="cursor"]"#).expect("valid static selector"));
static MORE_COUNT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-number[number]").expect("valid static selector"));

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ThreadRequest {
	post_id: String,
	comment_id: Option<String>,
	sort: String,
	continuation: Option<String>,
}

/// A validated thread response. The fields remain private so callers cannot
/// accidentally bypass validation before converting to the existing parser's
/// input shape.
#[derive(Clone, Debug)]
pub struct ThreadData {
	post: Value,
	comments: Value,
	continuations: usize,
}

impl ThreadData {
	pub fn into_legacy_json(self) -> Value {
		json_value!([self.post, self.comments])
	}

	pub fn continuation_count(&self) -> usize {
		self.continuations
	}

	pub fn post_id(&self) -> &str {
		self.post["data"]["children"][0]["data"]["id"].as_str().unwrap_or_default()
	}

	pub fn rendered_comment_count(&self) -> usize {
		self.comments["data"]["children"].as_array().map_or(0, |items| count_kind(items, "t1"))
	}
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ThreadError {
	Restricted(String),
	Missing,
	InvalidAnonymousState,
	RateLimited(Option<u64>),
	UpstreamBlocked,
	Transport(String),
	Malformed(String),
}

impl fmt::Display for ThreadError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Restricted(reason) => write!(f, "{reason}"),
			Self::Missing => write!(f, "Reddit did not return this thread or comment"),
			Self::InvalidAnonymousState => write!(f, "Reddit rejected the anonymous session state"),
			Self::RateLimited(Some(seconds)) => write!(f, "Reddit rate limited this request; retry after {seconds} seconds"),
			Self::RateLimited(None) => write!(f, "Reddit rate limited this request"),
			Self::UpstreamBlocked => write!(f, "Reddit blocked anonymous thread access from this connection"),
			Self::Transport(detail) => write!(f, "Unable to retrieve the thread from Reddit: {detail}"),
			Self::Malformed(detail) => write!(f, "Reddit returned malformed thread data: {detail}"),
		}
	}
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ContinuationState {
	v: u8,
	thread: String,
	sort: String,
	cursor: String,
}

/// Fetch a thread using Reddit's logged-out page transport. Successful results
/// are cached for 30 seconds; failures are never cached as empty discussions.
pub async fn fetch(path: String, quarantine: bool) -> Result<ThreadData, ThreadError> {
	let request = parse_request(&path)?;
	fetch_cached(request, quarantine).await
}

/// Uncached entry point for deliberate low-rate live diagnostics.
pub async fn fetch_uncached(path: String, quarantine: bool) -> Result<ThreadData, ThreadError> {
	let request = parse_request(&path)?;
	fetch_upstream(request, quarantine, true).await
}

#[cached(size = 100, time = 30, result = true)]
async fn fetch_cached(request: ThreadRequest, quarantine: bool) -> Result<ThreadData, ThreadError> {
	fetch_upstream(request, quarantine, false).await
}

async fn fetch_upstream(request: ThreadRequest, quarantine: bool, uncached: bool) -> Result<ThreadData, ThreadError> {
	let metadata_path = format!("/by_id/t3_{}.json?raw_json=1", request.post_id);
	let post = if uncached {
		json_uncached(metadata_path, quarantine).await
	} else {
		json(metadata_path, quarantine).await
	}
	.map_err(classify_metadata_error)?;
	validate_post(&post, &request.post_id)?;

	let post_data = &post["data"]["children"][0]["data"];
	let sub = post_data["subreddit"]
		.as_str()
		.filter(|sub| valid_subreddit(sub))
		.ok_or_else(|| ThreadError::Malformed("post subreddit was missing or invalid".into()))?;

	let html = if let Some(comment_id) = &request.comment_id {
		if request.continuation.is_some() {
			return Err(ThreadError::Malformed("continuations are not accepted on focused-comment routes".into()));
		}
		let url = format!("{SVC_BASE}/comment/t1_{comment_id}?depth=0&subredditName={sub}");
		let mut html = request_html(Method::GET, &url, quarantine, None).await?;
		html.push_str(&more_comments(sub, &request.post_id, &cursor_for(comment_id), quarantine).await?);
		html
	} else if let Some(token) = &request.continuation {
		let state = decode_continuation(token, &request.post_id, &request.sort)?;
		more_comments(sub, &request.post_id, &state.cursor, quarantine).await?
	} else {
		let url = format!("{SVC_BASE}/comments/r/{sub}/t3_{}?sort={}", request.post_id, request.sort.to_uppercase());
		request_html(Method::GET, &url, quarantine, None).await?
	};

	let parsed = parse_comments(&html, &request.post_id, &request.sort)?;
	let continuations = count_kind(&parsed, "more_comments");
	Ok(ThreadData {
		post,
		comments: listing(parsed),
		continuations,
	})
}

fn parse_request(path: &str) -> Result<ThreadRequest, ThreadError> {
	let route = path
		.split('?')
		.next()
		.ok_or_else(|| ThreadError::Malformed("thread path was empty".into()))?
		.trim_end_matches(".json")
		.trim_end_matches('/');
	let segments: Vec<&str> = route.split('/').filter(|segment| !segment.is_empty()).collect();
	let comments_at = segments
		.iter()
		.position(|segment| *segment == "comments")
		.ok_or_else(|| ThreadError::Malformed("not a comments route".into()))?;
	let post_id = segments
		.get(comments_at + 1)
		.copied()
		.filter(|id| valid_base36_id(id))
		.ok_or_else(|| ThreadError::Malformed("post ID was missing or invalid".into()))?;
	let comment_id = segments.get(comments_at + 3).copied().filter(|id| !id.is_empty()).map(str::to_owned);
	if comment_id.as_deref().is_some_and(|id| !valid_base36_id(id)) {
		return Err(ThreadError::Malformed("comment ID was invalid".into()));
	}

	let sort = param(path, "sort")
		.filter(|sort| matches!(sort.as_str(), "confidence" | "top" | "new" | "controversial" | "old"))
		.unwrap_or_else(|| "confidence".into());
	let continuation = param(path, "cursor");
	if continuation.as_ref().is_some_and(|value| value.len() > MAX_CURSOR_TOKEN_BYTES) {
		return Err(ThreadError::Malformed("continuation token was too large".into()));
	}

	Ok(ThreadRequest {
		post_id: post_id.to_owned(),
		comment_id,
		sort,
		continuation,
	})
}

fn valid_base36_id(value: &str) -> bool {
	!value.is_empty() && value.len() <= 16 && value.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn valid_subreddit(value: &str) -> bool {
	!value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_post(post: &Value, expected_id: &str) -> Result<(), ThreadError> {
	let children = post["data"]["children"]
		.as_array()
		.ok_or_else(|| ThreadError::Malformed("post listing was missing children".into()))?;
	let Some(first) = children.first() else {
		return Err(ThreadError::Missing);
	};
	if first["kind"] != "t3" || first["data"]["id"].as_str() != Some(expected_id) {
		return Err(ThreadError::Malformed("post identity did not match the requested thread".into()));
	}
	Ok(())
}

fn classify_metadata_error(error: String) -> ThreadError {
	match error.as_str() {
		"quarantined" | "gated" | "private" | "banned" => ThreadError::Restricted(error),
		_ if error.contains("rate limit") || error.contains("Too many requests") => ThreadError::RateLimited(None),
		_ if error.contains("OAuth token has expired") => ThreadError::InvalidAnonymousState,
		_ if error.contains("blocked anonymous access") => ThreadError::UpstreamBlocked,
		_ => ThreadError::Transport(error),
	}
}

fn cursor_for(comment_id: &str) -> String {
	general_purpose::STANDARD.encode(format!("commenttree:,::1:d1:t1_{comment_id}"))
}

fn cookies(quarantine: bool) -> String {
	if quarantine {
		"over18=1; _options=%7B%22pref_quarantine_optin%22%3A%20true%2C%20%22pref_gated_sr_optin%22%3A%20true%7D".into()
	} else {
		"over18=1".into()
	}
}

async fn more_comments(sub: &str, post_id: &str, cursor: &str, quarantine: bool) -> Result<String, ThreadError> {
	if cursor.is_empty() || cursor.len() > MAX_UPSTREAM_CURSOR_BYTES {
		return Err(ThreadError::Malformed("upstream continuation cursor was invalid".into()));
	}
	let url = format!("{SVC_BASE}/more-comments/{sub}/t3_{post_id}");
	let csrf = format!("{:032x}", fastrand::u128(..));
	let body = serde_urlencoded::to_string([("cursor", cursor), ("csrf_token", &csrf)]).map_err(|error| ThreadError::Malformed(error.to_string()))?;
	request_html(Method::POST, &url, quarantine, Some((csrf, body))).await
}

async fn request_html(method: Method, url: &str, quarantine: bool, form: Option<(String, String)>) -> Result<String, ThreadError> {
	// Every URL is constructed from SVC_BASE plus validated identifiers. Never
	// accept an upstream URL from a request or response.
	if !url.starts_with(SVC_BASE) {
		return Err(ThreadError::Transport("refused a non-allowlisted upstream host".into()));
	}

	let operation = async {
		let mut builder = CLIENT.request(method, url).header("Cookie", cookies(quarantine));
		if let Some((csrf, body)) = form {
			builder = builder
				.header("Cookie", format!("{}; csrf_token={csrf}", cookies(quarantine)))
				.header("Content-Type", "application/x-www-form-urlencoded")
				.body(body);
		}
		let response = builder.send().await.map_err(|error| ThreadError::Transport(error.to_string()))?;
		validate_response(response).await
	};

	timeout(REQUEST_TIMEOUT, operation)
		.await
		.map_err(|_| ThreadError::Transport("request exceeded 20 seconds".into()))?
}

async fn validate_response(response: Response) -> Result<String, ThreadError> {
	let status = response.status();
	let retry_after = response
		.headers()
		.get("retry-after")
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<u64>().ok());

	if let Some(error) = classify_status(status.as_u16(), retry_after) {
		return Err(error);
	}

	let content_type = response
		.headers()
		.get("content-type")
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default()
		.to_ascii_lowercase();
	if !content_type.starts_with("text/html") {
		return Err(ThreadError::Malformed(format!("unexpected content type {content_type:?}")));
	}
	if response
		.headers()
		.get("content-length")
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<usize>().ok())
		.is_some_and(|length| length > MAX_RESPONSE_BYTES)
	{
		return Err(ThreadError::Malformed("response exceeded 8 MiB".into()));
	}

	let mut bytes = Vec::new();
	let mut stream = response.bytes_stream();
	while let Some(chunk) = stream.next().await {
		let chunk = chunk.map_err(|error| ThreadError::Transport(error.to_string()))?;
		if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
			return Err(ThreadError::Malformed("response exceeded 8 MiB".into()));
		}
		bytes.extend_from_slice(&chunk);
	}
	String::from_utf8(bytes).map_err(|_| ThreadError::Malformed("response was not UTF-8".into()))
}

fn classify_status(status: u16, retry_after: Option<u64>) -> Option<ThreadError> {
	match status {
		200..=299 => None,
		401 => Some(ThreadError::InvalidAnonymousState),
		403 => Some(ThreadError::UpstreamBlocked),
		404 => Some(ThreadError::Missing),
		429 => Some(ThreadError::RateLimited(retry_after)),
		451 => Some(ThreadError::Restricted("restricted".into())),
		300..=399 => Some(ThreadError::Transport("Reddit returned a redirect; redirects are disabled".into())),
		_ => Some(ThreadError::Transport(format!("Reddit returned HTTP {status}"))),
	}
}

fn listing(children: Vec<Value>) -> Value {
	json_value!({ "kind": "Listing", "data": { "children": children } })
}

fn parse_comments(html: &str, post_id: &str, sort: &str) -> Result<Vec<Value>, ThreadError> {
	validate_fragment_structure(html)?;
	let document = Html::parse_fragment(html);
	let post_fullname = format!("t3_{post_id}");
	let mut items: Vec<(String, Value)> = Vec::new();
	let mut seen = HashSet::new();

	for element in document.select(&COMMENT) {
		let attr = |name: &str| element.value().attr(name).unwrap_or_default();
		let fullname = attr("thingid");
		let Some(id) = fullname.strip_prefix("t1_").filter(|id| valid_base36_id(id)) else {
			continue;
		};
		if !seen.insert(fullname.to_owned()) {
			continue;
		}
		let parent = element.value().attr("parentid").unwrap_or(&post_fullname);
		if parent != post_fullname && !parent.strip_prefix("t1_").is_some_and(valid_base36_id) {
			continue;
		}

		let deleted = element.value().attr("is-comment-deleted").is_some();
		let author = if deleted {
			"[deleted]"
		} else {
			let author = attr("author");
			if author.is_empty() {
				continue;
			}
			author
		};
		let body = if deleted {
			"<p>[deleted]</p>".to_owned()
		} else {
			let Some(body) = Selector::parse(&format!(r#"[id="{fullname}-comment-rtjson-content"]"#))
				.ok()
				.and_then(|selector| element.select(&selector).next())
			else {
				continue;
			};
			clean_html(body)
		};
		let created = DateTime::parse_from_str(attr("created"), "%Y-%m-%dT%H:%M:%S%.f%z").map_or(0, |time| time.timestamp());

		items.push((
			parent.to_owned(),
			json_value!({
				"kind": "t1",
				"data": {
					"id": id,
					"name": fullname,
					"author": author,
					"body_html": format!("<div class=\"md\">{body}</div>"),
					"created_utc": created,
					"score": attr("score").parse::<i64>().unwrap_or_default(),
					"parent_id": parent,
					"replies": "",
				}
			}),
		));
	}

	for element in document.select(&MORE) {
		let source = element.value().attr("src").unwrap_or_default();
		let Some(cursor) = element.select(&CURSOR).next().and_then(|input| input.value().attr("value")) else {
			continue;
		};
		if cursor.is_empty() || cursor.len() > MAX_UPSTREAM_CURSOR_BYTES {
			continue;
		}

		if source.contains("top-level=1") {
			let count = param(source, "comments-remaining").and_then(|count| count.parse::<i64>().ok()).unwrap_or_default();
			let token = encode_continuation(post_id, sort, cursor)?;
			items.push((
				post_fullname.clone(),
				json_value!({
					"kind": "more_comments",
					"data": { "id": token, "count": count, "parent_id": post_fullname, "continuation_sort": sort }
				}),
			));
			continue;
		}

		let Some(parent) = general_purpose::STANDARD
			.decode(cursor)
			.ok()
			.and_then(|bytes| String::from_utf8(bytes).ok())
			.and_then(|decoded| {
				decoded
					.rsplit(':')
					.next()
					.filter(|parent| parent.strip_prefix("t1_").is_some_and(valid_base36_id))
					.map(str::to_owned)
			})
		else {
			continue;
		};
		let count = element
			.select(&MORE_COUNT)
			.next()
			.and_then(|number| number.value().attr("number"))
			.and_then(|number| number.parse::<i64>().ok())
			.unwrap_or_default();
		items.push((parent.clone(), json_value!({ "kind": "more", "data": { "id": "", "count": count, "parent_id": parent } })));
	}

	let names: HashSet<String> = items.iter().filter_map(|(_, item)| item["data"]["name"].as_str().map(str::to_owned)).collect();
	let mut children: HashMap<String, Vec<Value>> = HashMap::new();
	let mut top = Vec::new();
	for (parent, item) in items {
		if names.contains(&parent) {
			children.entry(parent).or_default().push(item);
		} else {
			top.push(item);
		}
	}
	Ok(top.into_iter().map(|item| attach_replies(item, &mut children)).collect())
}

fn validate_fragment_structure(html: &str) -> Result<(), ThreadError> {
	let normalized = html.to_ascii_lowercase();
	let has_comment_data = normalized.contains("<shreddit-comment") || normalized.contains("/svc/shreddit/more-comments/");
	let has_empty_state = ["no comments", "first to comment", "nobody's responded", "nobody has responded"]
		.iter()
		.any(|marker| normalized.contains(marker));
	if has_comment_data || has_empty_state {
		Ok(())
	} else {
		Err(ThreadError::Malformed(
			"response did not contain a comment tree or a recognized empty-discussion state".into(),
		))
	}
}

fn attach_replies(mut item: Value, children: &mut HashMap<String, Vec<Value>>) -> Value {
	let name = item["data"]["name"].as_str().unwrap_or_default().to_owned();
	if let Some(replies) = children.remove(&name) {
		item["data"]["replies"] = listing(replies.into_iter().map(|reply| attach_replies(reply, children)).collect());
	}
	item
}

fn count_kind(items: &[Value], expected: &str) -> usize {
	items
		.iter()
		.map(|item| usize::from(item["kind"] == expected) + item["data"]["replies"]["data"]["children"].as_array().map_or(0, |replies| count_kind(replies, expected)))
		.sum()
}

fn encode_continuation(thread: &str, sort: &str, cursor: &str) -> Result<String, ThreadError> {
	let state = ContinuationState {
		v: CONTINUATION_VERSION,
		thread: thread.to_owned(),
		sort: sort.to_owned(),
		cursor: cursor.to_owned(),
	};
	let bytes = serde_json::to_vec(&state).map_err(|error| ThreadError::Malformed(error.to_string()))?;
	Ok(general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_continuation(token: &str, thread: &str, sort: &str) -> Result<ContinuationState, ThreadError> {
	if token.is_empty() || token.len() > MAX_CURSOR_TOKEN_BYTES {
		return Err(ThreadError::Malformed("continuation token was invalid".into()));
	}
	let bytes = general_purpose::URL_SAFE_NO_PAD
		.decode(token)
		.map_err(|_| ThreadError::Malformed("continuation token was not valid base64url".into()))?;
	let state: ContinuationState = serde_json::from_slice(&bytes).map_err(|_| ThreadError::Malformed("continuation token was not valid JSON".into()))?;
	if state.v != CONTINUATION_VERSION || state.thread != thread || state.sort != sort || state.cursor.is_empty() || state.cursor.len() > MAX_UPSTREAM_CURSOR_BYTES {
		return Err(ThreadError::Malformed("continuation token did not match this thread and sort".into()));
	}
	Ok(state)
}

const ALLOWED_TAGS: &[&str] = &[
	"p",
	"br",
	"hr",
	"a",
	"em",
	"strong",
	"b",
	"i",
	"del",
	"s",
	"sup",
	"sub",
	"code",
	"pre",
	"blockquote",
	"ul",
	"ol",
	"li",
	"h1",
	"h2",
	"h3",
	"h4",
	"h5",
	"h6",
	"table",
	"thead",
	"tbody",
	"tr",
	"th",
	"td",
	"span",
	"img",
	"figure",
];
const VOID_TAGS: &[&str] = &["br", "hr", "img"];

/// Rebuild comment markup rather than trusting upstream HTML. Event handlers,
/// scripts, tracking wrappers, unsafe URL schemes, and unknown attributes are
/// never copied.
fn clean_html(element: ElementRef<'_>) -> String {
	let mut output = String::new();
	for child in element.children() {
		match child.value() {
			Node::Text(text) => output.push_str(&htmlescape::encode_minimal(text)),
			Node::Element(node) => {
				let tag = node.name();
				if matches!(tag, "script" | "style" | "svg" | "iframe" | "object" | "embed" | "faceplate-tracker" | "shreddit-analytics") {
					continue;
				}
				let inner = ElementRef::wrap(child).map(clean_html).unwrap_or_default();
				match tag {
					"shreddit-spoiler" => output.push_str(&format!("<span class=\"md-spoiler-text\">{inner}</span>")),
					"shreddit-player" => {
						let source = safe_media_url(node.attr("src"));
						if !source.is_empty() {
							let controls = if node.attr("gif").is_some() { "autoplay loop muted playsinline" } else { "controls" };
							output.push_str(&format!("<video src=\"{source}\" {controls}></video>"));
						}
					}
					_ if !ALLOWED_TAGS.contains(&tag) => output.push_str(&inner),
					_ => {
						output.push('<');
						output.push_str(tag);
						for name in ["href", "src", "alt", "title", "align"] {
							let Some(raw_value) = node.attr(name) else { continue };
							let value = match name {
								"href" => safe_link_url(Some(raw_value)),
								"src" if tag == "img" => safe_media_url(Some(raw_value)),
								"src" => String::new(),
								_ => htmlescape::encode_minimal(raw_value),
							};
							if !value.is_empty() {
								output.push_str(&format!(" {name}=\"{value}\""));
							}
						}
						if tag == "img" {
							output.push_str(" loading=\"lazy\"");
						}
						output.push('>');
						if !VOID_TAGS.contains(&tag) {
							output.push_str(&inner);
							output.push_str("</");
							output.push_str(tag);
							output.push('>');
						}
					}
				}
			}
			_ => {}
		}
	}
	output
}

fn safe_link_url(value: Option<&str>) -> String {
	let value = value.unwrap_or_default().trim();
	if value.starts_with('/') && !value.starts_with("//") {
		return htmlescape::encode_minimal(&format_url(value));
	}
	let Ok(parsed) = url::Url::parse(value) else {
		return String::new();
	};
	if !matches!(parsed.scheme(), "http" | "https") {
		return String::new();
	}
	htmlescape::encode_minimal(&format_url(value))
}

fn safe_media_url(value: Option<&str>) -> String {
	let safe = safe_link_url(value);
	// Reddit-owned media is rewritten by format_url to a local proxy route.
	// Refuse an external src so viewing a comment cannot contact a third party.
	if safe.starts_with('/') && !safe.starts_with("//") {
		safe
	} else {
		String::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE: &str = r#"
<shreddit-comment thingid="t1_a" author="alice" created="2026-10-20T17:00:53.706000+0000" score="18">
  <div id="t1_a-comment-rtjson-content"><p onclick="bad()">Hi <a href="https://www.reddit.com/r/rust" onmouseover="bad()">r/rust</a> &amp; <a href="javascript:alert(1)">bad</a><script>bad()</script></p></div>
  <shreddit-comment thingid="t1_b" parentid="t1_a" author="bob" created="2026-10-20T17:01:00.000000+0000" score="-2">
    <div id="t1_b-comment-rtjson-content"><p>reply</p></div>
  </shreddit-comment>
  <faceplate-partial src="/svc/shreddit/more-comments/rust/t3_p"><input name="cursor" value="Y29tbWVudHRyZWU6YWJjLCwsOjE6ZDE6dDFfYQ=="><faceplate-number number="7"></faceplate-number></faceplate-partial>
</shreddit-comment>
<shreddit-comment thingid="t1_c" author="[deleted]" is-comment-deleted><div id="t1_c-comment-rtjson-content"><p>must not leak</p></div></shreddit-comment>
<shreddit-comment thingid="t1_d" parentid="invalid" author="drop"><div id="t1_d-comment-rtjson-content"><p>drop me</p></div></shreddit-comment>
<shreddit-comment thingid="t1_e" author="eve"><div id="t1_e-comment-rtjson-content"><p><shreddit-spoiler>secret</shreddit-spoiler><img src="https://preview.redd.it/x.jpeg?width=400&amp;s=1"><img src="https://tracker.example/pixel"><iframe src="https://evil.example"></iframe></p></div></shreddit-comment>
<faceplate-partial src="/svc/shreddit/more-comments/rust/t3_p?top-level=1&amp;comments-remaining=90"><input name="cursor" value="YWJj"></faceplate-partial>
"#;

	#[test]
	fn parses_thread_paths_and_supported_sorts() {
		for sort in ["confidence", "top", "new", "controversial", "old"] {
			let request = parse_request(&format!("/r/rust/comments/1xaxcyl/title/.json?sort={sort}&raw_json=1")).unwrap();
			assert_eq!(request.post_id, "1xaxcyl");
			assert_eq!(request.comment_id, None);
			assert_eq!(request.sort, sort);
		}

		let request = parse_request("/r/rust/comments/1xaxcyl/title/19re6td.json?sort=bogus&context=3").unwrap();
		assert_eq!(request.comment_id.as_deref(), Some("19re6td"));
		assert_eq!(request.sort, "confidence");
		assert!(parse_request("/r/rust/hot.json").is_err());
		assert!(parse_request("/r/rust/comments/../title.json").is_err());
	}

	#[test]
	fn parses_nested_deleted_more_and_sanitized_html() {
		let comments = parse_comments(SAMPLE, "p", "new").unwrap();
		assert_eq!(comments.len(), 4);
		let first = &comments[0]["data"];
		assert_eq!(first["author"], "alice");
		assert_eq!(first["created_utc"], 1_792_515_653_i64);
		assert_eq!(first["body_html"], r#"<div class="md"><p>Hi <a href="/r/rust">r/rust</a> &amp; <a>bad</a></p></div>"#);
		let replies = first["replies"]["data"]["children"].as_array().unwrap();
		assert_eq!(replies.len(), 2);
		assert_eq!(replies[0]["data"]["author"], "bob");
		assert_eq!(replies[1]["kind"], "more");
		assert_eq!(comments[1]["data"]["body_html"], r#"<div class="md"><p>[deleted]</p></div>"#);
		assert!(!serde_json::to_string(&comments).unwrap().contains("drop me"));
		let media = comments[2]["data"]["body_html"].as_str().unwrap();
		assert!(media.contains("md-spoiler-text"));
		assert!(media.contains("/preview/pre/x.jpeg"));
		assert!(!media.contains("tracker.example"));
		assert!(!media.contains("iframe"));
		assert_eq!(comments[3]["kind"], "more_comments");
	}

	#[test]
	fn continuation_is_opaque_and_bound_to_thread_and_sort() {
		let token = encode_continuation("abc", "new", "upstream-cursor").unwrap();
		assert!(!token.contains("upstream-cursor"));
		assert_eq!(decode_continuation(&token, "abc", "new").unwrap().cursor, "upstream-cursor");
		assert!(decode_continuation(&token, "other", "new").is_err());
		assert!(decode_continuation(&token, "abc", "top").is_err());
	}

	#[test]
	fn empty_discussion_is_valid_and_has_no_continuation() {
		let comments = parse_comments("<div>No comments yet</div>", "abc", "confidence").unwrap();
		assert!(comments.is_empty());
		let data = ThreadData {
			post: json_value!({}),
			comments: listing(comments),
			continuations: 0,
		};
		assert_eq!(data.continuation_count(), 0);
	}

	#[test]
	fn rejects_html_without_a_comment_payload() {
		assert!(parse_comments("", "abc", "confidence").is_err());
		assert!(parse_comments("<html><h1>Access denied</h1></html>", "abc", "confidence").is_err());
	}

	#[test]
	fn rejects_unsafe_url_schemes_and_external_media() {
		for value in ["javascript:alert(1)", "data:text/html,bad", "vbscript:bad", "//evil.example/x"] {
			assert!(safe_link_url(Some(value)).is_empty());
		}
		assert_eq!(safe_link_url(Some("https://example.com/path?a=1&b=2")), "https://example.com/path?a=1&amp;b=2");
		assert!(safe_media_url(Some("https://example.com/image.png")).is_empty());
	}

	#[test]
	fn classifies_redirects_expiry_blocks_missing_and_rate_limits() {
		assert_eq!(classify_status(200, None), None);
		assert_eq!(classify_status(401, None), Some(ThreadError::InvalidAnonymousState));
		assert_eq!(classify_status(403, None), Some(ThreadError::UpstreamBlocked));
		assert_eq!(classify_status(404, None), Some(ThreadError::Missing));
		assert_eq!(classify_status(429, Some(17)), Some(ThreadError::RateLimited(Some(17))));
		assert!(matches!(classify_status(302, None), Some(ThreadError::Transport(message)) if message.contains("redirect")));
	}
}
