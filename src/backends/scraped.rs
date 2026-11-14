//! Pages scraped from the HTML reddit.com serves logged-out visitors.
//!
//! Comment trees: Reddit's edge blocks the OAuth comments API for anonymous
//! clients, but its web frontend ("shreddit") still serves HTML fragments of
//! comment trees. This module crawls those fragments and converts them into
//! the API's `Listing` JSON, so comment parsing and rendering stay unchanged.
//!
//! Combined feeds (`r/a+b`): Reddit refuses some sorts to the JSON API, but
//! serves the old listing page. Its post IDs are read and loaded from `/by_id`.

use super::{is_blocked_error, json as reddit_json, Backend, Failure, Fetch, Kind};
use crate::client::{ALTERNATIVE_REDDIT_URL_BASE, CLIENT, DESKTOP_CLIENT};
use crate::utils::{enable_scrape_fallback, param, scrape_comment_prefetch};
use cached::proc_macro::cached;
use chrono::DateTime;
use log::{trace, warn};
use scraper::{ElementRef, Html, Selector};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, LazyLock};
use wreq::cookie::Jar;
use wreq::header as wreq_header;

/// How much of a comment tree is crawled beyond the first fragment.
#[derive(Clone, Copy)]
struct CrawlLimits {
	/// Stop expanding "more comments" placeholders once this many comments are loaded.
	max_comments: usize,
	/// Upper bound on placeholder expansions.
	max_expansions: usize,
	/// Number of placeholder expansions fetched concurrently.
	concurrency: usize,
}

/// `max_expansions` comes from `REDLIB_SCRAPE_COMMENT_PREFETCH`, which
/// defaults to 0: further comments are only loaded when a visitor asks.
const CRAWL_LIMITS: CrawlLimits = CrawlLimits {
	max_comments: 200,
	max_expansions: 0,
	concurrency: 10,
};

const MORE_COMMENTS_PATH_PREFIX: &str = "/svc/shreddit/more-comments/";
const SHREDDIT_PATH_PREFIX: &str = "/svc/shreddit/";
// Asks for the bare fragment instead of a full page shell, roughly halving the response.
const SHREDDIT_PARTIAL_ACCEPT: &str = "text/vnd.reddit.partial+html, text/html;q=0.9";
// Reddit answers rate-limited shreddit requests with this reCAPTCHA page and a 200 status.
const SHREDDIT_CHALLENGE_TITLE: &str = "<title>Reddit - Prove your humanity</title>";

// Logged-out reddit.com session shared by shreddit requests. Reddit challenges
// cookieless clients after about ten requests, but gives each session a budget
// of a couple hundred requests. It is kept apart from the OAuth API requests on purpose.
static SHREDDIT_SESSION: LazyLock<Arc<Jar>> = LazyLock::new(Arc::default);
// Leaves room for the requests already in flight when the session is rotated.
const SHREDDIT_RATELIMIT_ROTATE_BELOW: f32 = 20.0;

static COMMENT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("shreddit-comment").unwrap());
static COMMENT_BODY: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"[slot="comment"]"#).unwrap());
static COMMENT_META: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"[slot="commentMeta"]"#).unwrap());
static SPAN: LazyLock<Selector> = LazyLock::new(|| Selector::parse("span").unwrap());
static TIMEAGO: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-timeago[ts]").unwrap());
static MORE_COMMENTS: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-partial[src]").unwrap());
static CURSOR: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"input[name="cursor"]"#).unwrap());
static COUNT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-number[number]").unwrap());
static FEED: LazyLock<Selector> = LazyLock::new(|| Selector::parse("#siteTable").unwrap());
static FEED_POST: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"#siteTable .thing[data-fullname^="t3_"]"#).unwrap());
static FEED_NEXT: LazyLock<Selector> = LazyLock::new(|| Selector::parse(".next-button a[href]").unwrap());

/// A "more comments" placeholder to load: its cursor and the query string of
/// its `/svc/shreddit/more-comments/` path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct More {
	pub cursor: String,
	pub query: String,
}

pub struct Scraped;

impl Backend for Scraped {
	fn name(&self) -> &'static str {
		"scraped"
	}

	fn available(&self) -> bool {
		enable_scrape_fallback()
	}

	fn supports(&self, kind: Kind) -> bool {
		matches!(kind, Kind::Comments | Kind::Listing)
	}

	fn fetch<'a>(&'a self, path: &'a str, quarantine: bool) -> Fetch<'a> {
		Box::pin(async move {
			match Kind::of(path) {
				Kind::Listing => combined_feed(path, quarantine, None).await,
				_ => thread(path, quarantine).await.map_err(Failure::Invalid),
			}
		})
	}
}

/// The listing for a combined feed's API `path` (`/r/a+b/new.json`): the old
/// listing page gives the post IDs and the next page's cursor, `/by_id` the posts.
/// The page is asked for logged out, or as the session cookie `session`.
pub(super) async fn combined_feed(path: &str, quarantine: bool, session: Option<&str>) -> Result<Value, Failure> {
	let page = combined_feed_page(path).ok_or(Failure::Unsupported)?;
	let html = feed_page(&page, session).await?;
	let (ids, after) = parse_feed_page(&html).ok_or_else(|| Failure::Invalid(format!("Reddit didn't send the listing page | {page}")))?;
	if ids.is_empty() {
		return Ok(listing(Vec::new()));
	}

	let mut posts = reddit_json(format!("/by_id/{}.json?raw_json=1", ids.join(",")), quarantine).await.map_err(|msg| {
		if is_blocked_error(&msg) {
			Failure::Blocked
		} else {
			Failure::Invalid(msg)
		}
	})?;
	if let Some(children) = posts["data"]["children"].as_array_mut() {
		children.sort_by_key(|post| ids.iter().position(|id| id.strip_prefix("t3_") == post["data"]["id"].as_str()));
	}
	posts["data"]["after"] = after.into();
	Ok(posts)
}

/// The reddit.com page for a combined feed's API `path`, `None` for other
/// listings: Reddit serves single subreddits as shreddit pages without posts.
fn combined_feed_page(path: &str) -> Option<String> {
	let (route, query) = path.split_once('?').unwrap_or((path, ""));
	let segments: Vec<&str> = route.trim_end_matches(".json").split('/').filter(|s| !s.is_empty()).collect();
	let ["r", subs, sort @ ("hot" | "new" | "rising" | "top" | "controversial")] = segments.as_slice() else {
		return None;
	};
	// The page only exists with a literal `+`
	let subs = subs.replace("%2B", "+");
	if !subs.contains('+') || !subs.chars().all(|c| c.is_ascii_alphanumeric() || "_-+".contains(c)) {
		return None;
	}
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(url::form_urlencoded::parse(query.as_bytes()).filter(|(key, value)| ["after", "limit", "t"].contains(&&**key) && !value.is_empty()))
		.finish();
	// Only the bare path is the old page for hot; `/hot/` is a shreddit page
	let sort = if *sort == "hot" { String::new() } else { format!("{sort}/") };
	Some(format!("/r/{subs}/{sort}?{query}"))
}

/// Fetches a listing page as a visitor who confirmed their age, which Reddit
/// asks logged-out ones for when a subreddit of the feed is NSFW. Phones are
/// sent to the front page instead, so this asks as a desktop browser.
async fn feed_page(page: &str, session: Option<&str>) -> Result<String, Failure> {
	let cookie = session.map_or("over18=1".to_string(), |session| format!("{session}; over18=1"));
	let mut url = format!("{ALTERNATIVE_REDDIT_URL_BASE}{page}");
	// Reddit redirects once, to its own spelling and order of the names
	for _ in 0..2 {
		let response = DESKTOP_CLIENT
			.get(&url)
			.header(wreq_header::ACCEPT, "text/html")
			.header(wreq_header::COOKIE, &cookie)
			.send()
			.await
			.map_err(|e| Failure::Network(format!("Couldn't send request to Reddit: {e} | {page}")))?;
		let status = response.status();
		if status.is_redirection() {
			let location = response.headers().get(wreq_header::LOCATION).and_then(|location| location.to_str().ok());
			match location.filter(|location| location.starts_with(&format!("{ALTERNATIVE_REDDIT_URL_BASE}/r/"))) {
				Some(location) => url = location.to_string(),
				None => return Err(Failure::Invalid(format!("Reddit redirected the listing page to {location:?} | {page}"))),
			}
			continue;
		}
		return match status.as_u16() {
			200 => response
				.text()
				.await
				.map_err(|e| Failure::Network(format!("Failed receiving body from Reddit: {e} | {page}"))),
			403 => Err(Failure::Blocked),
			429 => Err(Failure::RateLimited(format!(
				"Reddit is rate limiting listing pages (429 Too Many Requests). Try again in a minute. | {page}"
			))),
			500..=599 => Err(Failure::Network(format!("Reddit returned {status} | {page}"))),
			_ => Err(Failure::Invalid(format!("Reddit returned {status} | {page}"))),
		};
	}
	Err(Failure::Invalid(format!("Reddit keeps redirecting the listing page | {page}")))
}

/// The post IDs (`t3_…`) of a listing page in its order and the cursor of the
/// next page, `None` when the page isn't a listing (a challenge, a login wall).
fn parse_feed_page(html: &str) -> Option<(Vec<String>, Option<String>)> {
	let document = Html::parse_document(html);
	document.select(&FEED).next()?;
	let ids = document
		.select(&FEED_POST)
		.filter(|post| post.value().attr("data-promoted") != Some("true"))
		.filter_map(|post| post.value().attr("data-fullname"))
		.map(str::to_string)
		.collect();
	let after = document
		.select(&FEED_NEXT)
		.next()
		.and_then(|next| next.value().attr("href")?.split_once('?'))
		.and_then(|(_, query)| param(&format!("?{query}"), "after"));
	Some((ids, after))
}

/// The thread's API response (`[post listing, comment listing]`) for a
/// `/comments/` API `path`: the post comes from `/by_id`, the comments are
/// scraped from the HTML Reddit serves logged-out visitors.
async fn thread(path: &str, quarantine: bool) -> Result<Value, String> {
	let (route, query) = path.split_once('?').unwrap_or((path, ""));
	let query = format!("?{query}");
	let segments: Vec<&str> = route.trim_end_matches(".json").split('/').filter(|s| !s.is_empty()).collect();
	let Some(at) = segments.iter().position(|s| *s == "comments") else {
		return Err(format!("Not a post page | {path}"));
	};
	let id = segments.get(at + 1).map(ToString::to_string).unwrap_or_default();
	let comment_id = segments.get(at + 3).map(ToString::to_string);
	let sort = param(&query, "sort").unwrap_or_default();
	let more = param(&query, "more_cursor").map(|cursor| More {
		cursor,
		query: param(&query, "more_query").unwrap_or_default(),
	});

	let post = reddit_json(format!("/by_id/t3_{id}.json?raw_json=1"), quarantine).await?;
	let Some(sub) = post["data"]["children"][0]["data"]["subreddit"].as_str() else {
		return Err("Post not found".to_string());
	};

	let comments = comments(sub.to_string(), id, comment_id, sort, param(&query, "context"), more).await?;
	Ok(json!([post, comments]))
}

/// Fetch the comment tree of a post as a Reddit API comment `Listing`.
/// Unloaded comments become `more` stubs carrying their `cursor` and
/// `more_query`, so they can be loaded later with `more`.
///
/// `more` loads one placeholder instead of the first page; its comments are
/// returned as top-level comments. When `comment_id` is set, only that
/// comment's thread is returned, preceded by up to `context` ancestors,
/// mirroring the API's comment permalinks.
#[cached(max_size = 100, ttl_secs = 30)]
pub async fn comments(subreddit: String, post_id: String, comment_id: Option<String>, sort: String, context: Option<String>, more: Option<More>) -> Result<Value, String> {
	let html = match &more {
		Some(more) => shreddit_fragment(more_path(&subreddit, &post_id, &more.query), Some(more.cursor.clone())).await?,
		None => shreddit_fragment(comments_path(&subreddit, &post_id, comment_id.as_deref(), &sort), None).await?,
	};

	let mut tree = CommentTree::new(format!("t3_{post_id}"), comment_id.map(|id| format!("t1_{id}")));
	tree.add(parse_fragment(&html));
	let limits = CrawlLimits {
		max_expansions: scrape_comment_prefetch(),
		..CRAWL_LIMITS
	};
	expand(&mut tree, limits, |path, cursor| shreddit_fragment(path, Some(cursor))).await;

	let context = context.and_then(|context| context.parse().ok()).unwrap_or(0);
	Ok(tree.into_listing(context))
}

/// The query comes from the visitor's URL, so it is parsed and re-encoded
/// rather than pasted into the path.
fn more_path(subreddit: &str, post_id: &str, query: &str) -> String {
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(url::form_urlencoded::parse(query.as_bytes()))
		.finish();
	format!("{MORE_COMMENTS_PATH_PREFIX}{subreddit}/t3_{post_id}?{query}")
}

fn comments_path(subreddit: &str, post_id: &str, comment_id: Option<&str>, sort: &str) -> String {
	let mut path = format!("/svc/shreddit/comments/r/{subreddit}/t3_{post_id}");
	if let Some(comment_id) = comment_id {
		path.push_str(&format!("/t1_{comment_id}"));
	}
	if !sort.is_empty() {
		path.push_str(&format!(
			"?{}",
			url::form_urlencoded::Serializer::new(String::new()).append_pair("sort", &sort.to_uppercase()).finish()
		));
	}
	path
}

/// Fetch a server-rendered HTML fragment from Reddit's web frontend.
///
/// These are the fragments reddit.com loads for logged-out visitors, so they
/// are sent without the spoofed OAuth client's headers. A `cursor` turns the
/// request into the form POST used to expand a "more comments" placeholder.
async fn shreddit_fragment(path: String, cursor: Option<String>) -> Result<String, String> {
	if !path.starts_with(SHREDDIT_PATH_PREFIX) {
		return Err(format!("Refusing to fetch non-shreddit path | {path}"));
	}

	if let Some(body) = shreddit_request(&path, cursor.as_deref()).await? {
		return Ok(body);
	}

	warn!("Reddit rate limited the shreddit session, starting a new one");
	SHREDDIT_SESSION.clear();
	shreddit_request(&path, cursor.as_deref())
		.await?
		.ok_or_else(|| format!("Reddit is rate limiting comment requests. Try refreshing in a few seconds. | {path}"))
}

/// Returns `Ok(None)` when Reddit rate limits or challenges the session.
async fn shreddit_request(path: &str, cursor: Option<&str>) -> Result<Option<String>, String> {
	let url = format!("{ALTERNATIVE_REDDIT_URL_BASE}{path}");

	let builder = match cursor {
		Some(cursor) => CLIENT
			.load_full()
			.post(&url)
			.header(wreq_header::CONTENT_TYPE, "application/x-www-form-urlencoded")
			.body(url::form_urlencoded::Serializer::new(String::new()).append_pair("cursor", cursor).finish()),
		None => CLIENT.load_full().get(&url),
	};

	let response = builder
		.header(wreq_header::ACCEPT, SHREDDIT_PARTIAL_ACCEPT)
		.cookie_provider(Arc::clone(&SHREDDIT_SESSION))
		.send()
		.await
		.map_err(|e| format!("Couldn't send request to Reddit: {e} | {path}"))?;

	let status = response.status();
	if status == wreq::StatusCode::TOO_MANY_REQUESTS {
		return Ok(None);
	}
	if !status.is_success() {
		return Err(format!("Reddit returned {status} | {path}"));
	}

	let remaining = response
		.headers()
		.get("x-ratelimit-remaining")
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<f32>().ok());
	if remaining.is_some_and(|remaining| remaining < SHREDDIT_RATELIMIT_ROTATE_BELOW) {
		trace!("Shreddit session rate limit is low ({remaining:?} remaining), starting a new one");
		SHREDDIT_SESSION.clear();
	}

	let body = response.text().await.map_err(|e| format!("Failed receiving body from Reddit: {e} | {path}"))?;
	Ok((!body.contains(SHREDDIT_CHALLENGE_TITLE)).then_some(body))
}

/// Expand "more comments" placeholders breadth-first in concurrent batches.
/// Placeholders that fail or exceed `limits` stay in the tree as stubs.
async fn expand<F, Fut>(tree: &mut CommentTree, limits: CrawlLimits, fetch: F)
where
	F: Fn(String, String) -> Fut,
	Fut: Future<Output = Result<String, String>> + Send + 'static,
{
	let mut expansions = 0;
	while tree.len() < limits.max_comments && expansions < limits.max_expansions {
		let batch = tree.next_batch(limits.concurrency.min(limits.max_expansions - expansions));
		if batch.is_empty() {
			break;
		}
		expansions += batch.len();

		let requests: Vec<_> = batch.iter().map(|more| tokio::spawn(fetch(more.path.clone(), more.cursor.clone()))).collect();
		for (more, request) in batch.into_iter().zip(requests) {
			match request.await.map_err(|e| e.to_string()).and_then(|result| result) {
				Ok(html) => tree.add(parse_fragment(&html)),
				Err(e) => {
					warn!("Couldn't expand more comments: {e}");
					tree.skip(more);
				}
			}
		}
	}
}

/// A comment parsed from a fragment, before its parent is resolved.
struct ParsedComment {
	id: String,
	parent_id: Option<String>,
	depth: u32,
	position: String,
	parent_positions: String,
	data: Value,
}

/// A "more comments" placeholder that can be expanded with a cursor.
struct MoreComments {
	path: String,
	cursor: String,
	count: i64,
	/// Position path of the parent comment; `None` for more top-level comments.
	parent_positions: Option<String>,
}

struct Fragment {
	comments: Vec<ParsedComment>,
	more: Vec<MoreComments>,
}

enum Child {
	Comment(String),
	More(MoreComments),
}

/// Comments accumulated across fragments, keyed by fullname (`t1_...`).
struct CommentTree {
	post: String,
	focus: Option<String>,
	data: HashMap<String, Value>,
	parents: HashMap<String, String>,
	children: HashMap<String, Vec<Child>>,
	/// Shreddit addresses comments by their path of sibling positions, e.g. `0,2`.
	positions: HashMap<String, String>,
	pending: VecDeque<MoreComments>,
	skipped: Vec<MoreComments>,
}

impl CommentTree {
	fn new(post: String, focus: Option<String>) -> Self {
		Self {
			post,
			focus,
			data: HashMap::new(),
			parents: HashMap::new(),
			children: HashMap::new(),
			positions: HashMap::new(),
			pending: VecDeque::new(),
			skipped: Vec::new(),
		}
	}

	fn len(&self) -> usize {
		self.data.len()
	}

	fn add(&mut self, fragment: Fragment) {
		for comment in fragment.comments {
			if self.data.contains_key(&comment.id) {
				continue;
			}
			let path = if comment.parent_positions.is_empty() {
				comment.position.clone()
			} else {
				format!("{},{}", comment.parent_positions, comment.position)
			};
			// A parent that isn't loaded (the top of a single loaded
			// placeholder) makes the comment top-level, so it isn't lost.
			let parent = comment
				.parent_id
				.filter(|id| self.data.contains_key(id))
				.or_else(|| (comment.depth > 0).then(|| self.positions.get(&comment.parent_positions).cloned()).flatten())
				.unwrap_or_else(|| self.post.clone());

			let mut data = comment.data;
			data["parent_id"] = Value::String(parent.clone());

			self.positions.insert(path, comment.id.clone());
			self.children.entry(parent.clone()).or_default().push(Child::Comment(comment.id.clone()));
			self.parents.insert(comment.id.clone(), parent);
			self.data.insert(comment.id, data);
		}
		self.pending.extend(fragment.more);
	}

	/// Take up to `size` placeholders worth expanding, breadth-first.
	fn next_batch(&mut self, size: usize) -> Vec<MoreComments> {
		let mut batch = Vec::new();
		while batch.len() < size {
			let Some(more) = self.pending.pop_front() else { break };
			if self.wants(&more) {
				batch.push(more);
			} else {
				self.skipped.push(more);
			}
		}
		batch
	}

	fn skip(&mut self, more: MoreComments) {
		self.skipped.push(more);
	}

	/// In single-thread mode, only expand placeholders inside the focused thread.
	fn wants(&self, more: &MoreComments) -> bool {
		let Some(focus) = &self.focus else { return true };
		self.lineage(&self.placeholder_parent(more)).any(|id| id == focus)
	}

	/// `id` followed by its ancestor comments, nearest first.
	fn lineage<'a>(&'a self, id: &'a String) -> impl Iterator<Item = &'a String> {
		std::iter::successors(Some(id), |id| self.parents.get(*id))
			.take_while(|id| **id != self.post)
			// Malformed `parentId` attributes could form a cycle.
			.take(self.data.len() + 1)
	}

	fn placeholder_parent(&self, more: &MoreComments) -> String {
		more
			.parent_positions
			.as_ref()
			.and_then(|positions| self.positions.get(positions))
			.cloned()
			.unwrap_or_else(|| self.post.clone())
	}

	fn into_listing(mut self, context: usize) -> Value {
		let unexpanded: Vec<MoreComments> = self.pending.drain(..).chain(self.skipped.drain(..)).collect();
		for more in unexpanded {
			let parent = self.placeholder_parent(&more);
			self.children.entry(parent).or_default().push(Child::More(more));
		}

		let thread = self.focus.as_ref().filter(|focus| self.data.contains_key(*focus)).map(|focus| {
			let mut thread: Vec<String> = self.lineage(focus).take(context + 1).cloned().collect();
			thread.reverse();
			thread
		});

		match thread.as_deref() {
			Some([root, rest @ ..]) => listing(vec![self.child(&self.parents[root], &Child::Comment(root.clone()), rest)]),
			_ => self.listing(&self.post, &[]),
		}
	}

	/// Build the `Listing` of `parent`'s replies. While `thread` is non-empty,
	/// only its next comment is kept at each level.
	fn listing(&self, parent: &str, thread: &[String]) -> Value {
		let children = self.children.get(parent).map(Vec::as_slice).unwrap_or_default();
		listing(match thread.split_first() {
			Some((next, rest)) => children
				.iter()
				.filter(|child| matches!(child, Child::Comment(id) if id == next))
				.map(|child| self.child(parent, child, rest))
				.collect(),
			None => children.iter().map(|child| self.child(parent, child, &[])).collect(),
		})
	}

	fn child(&self, parent: &str, child: &Child, thread: &[String]) -> Value {
		match child {
			Child::Comment(id) => {
				let mut data = self.data[id].clone();
				let has_replies = self.children.get(id).is_some_and(|replies| !replies.is_empty());
				data["replies"] = if has_replies { self.listing(id, thread) } else { Value::String(String::new()) };
				json!({ "kind": "t1", "data": data })
			}
			Child::More(more) => {
				// A reply placeholder whose parent isn't loaded (the end of a
				// loaded batch of replies) is still a reply.
				let parent = if parent == self.post && more.parent_positions.is_some() { "t1_" } else { parent };
				json!({
					"kind": "more",
					"data": {
						"count": more.count,
						"parent_id": parent,
						"id": "_",
						"children": [],
						"cursor": more.cursor,
						"more_query": more.path.split_once('?').map(|(_, query)| query).unwrap_or_default(),
					}
				})
			}
		}
	}
}

fn listing(children: Vec<Value>) -> Value {
	json!({ "kind": "Listing", "data": { "children": children } })
}

fn parse_fragment(html: &str) -> Fragment {
	let document = Html::parse_document(html);
	Fragment {
		comments: document.select(&COMMENT).filter_map(parse_comment).collect(),
		more: document.select(&MORE_COMMENTS).filter_map(parse_more_comments).collect(),
	}
}

fn parse_comment(element: ElementRef<'_>) -> Option<ParsedComment> {
	let attr = |name: &str| element.value().attr(name);
	let id = attr("thingid")?.to_string();
	let author = attr("author").unwrap_or("[deleted]");
	let is_moderator = attr("is-mod-distinguished").is_some();

	let content = comment_content(element, &id);
	let body_html = content.map(|content| content.inner_html()).unwrap_or_default();
	let body_text = content.map(|content| content.text().collect::<String>().trim().to_string()).unwrap_or_default();

	Some(ParsedComment {
		parent_id: attr("parentid").map(str::to_string),
		depth: attr("depth").and_then(|depth| depth.parse().ok()).unwrap_or(0),
		position: attr("comment-position").unwrap_or_default().to_string(),
		parent_positions: attr("comment-parent-positions").unwrap_or_default().trim_matches(['[', ']']).to_string(),
		data: json!({
			"id": id.trim_start_matches("t1_"),
			"name": id,
			"author": author,
			"body": body_text,
			"body_html": format!("<div class=\"md\">{body_html}</div>"),
			"score": attr("score").and_then(|score| score.parse::<i64>().ok()).unwrap_or(0),
			"created_utc": attr("created").and_then(unix_time).unwrap_or_default(),
			"edited": edited_time(element).map_or(Value::Bool(false), Value::from),
			"distinguished": if is_moderator { Value::from("moderator") } else { Value::Null },
			// Shreddit only marks stickied mod comments by collapsing them.
			"stickied": is_moderator && attr("collapsed").is_some(),
			"permalink": attr("permalink").unwrap_or_default(),
		}),
		id,
	})
}

/// The element holding a comment's rendered body, if it has one.
fn comment_content<'a>(element: ElementRef<'a>, id: &str) -> Option<ElementRef<'a>> {
	// A comment without a body would otherwise match the first reply's body.
	let body_id = format!("{id}-comment-rtjson-content");
	let body = element.select(&COMMENT_BODY).next().filter(|body| body.value().id() == Some(body_id.as_str()))?;
	Some(
		body
			.child_elements()
			.find(|child| child.value().id().is_some_and(|child_id| child_id.ends_with("-post-rtjson-content")))
			.unwrap_or(body),
	)
}

fn edited_time(element: ElementRef<'_>) -> Option<f64> {
	element
		.select(&COMMENT_META)
		.next()?
		.select(&SPAN)
		.find(|span| span.text().collect::<String>().trim_start().starts_with("Edited"))?
		.select(&TIMEAGO)
		.next()
		.and_then(|timeago| unix_time(timeago.value().attr("ts")?))
}

fn parse_more_comments(element: ElementRef<'_>) -> Option<MoreComments> {
	let path = element.value().attr("src")?;
	if !path.starts_with(MORE_COMMENTS_PATH_PREFIX) {
		return None;
	}
	let cursor = element.select(&CURSOR).next()?.value().attr("value")?;
	let query: HashMap<String, String> = path
		.split_once('?')
		.map(|(_, query)| url::form_urlencoded::parse(query.as_bytes()).into_owned().collect())
		.unwrap_or_default();

	let count = element
		.select(&COUNT)
		.next()
		.and_then(|count| count.value().attr("number"))
		.or_else(|| query.get("comments-remaining").map(String::as_str))
		.and_then(|count| count.parse().ok())
		.unwrap_or(0);

	Some(MoreComments {
		path: path.to_string(),
		cursor: cursor.to_string(),
		count,
		parent_positions: query.get("commentParentPositions").cloned(),
	})
}

/// Parse shreddit's RFC 3339 timestamps (`...Z` or `...+0000`) into Unix seconds.
fn unix_time(timestamp: &str) -> Option<f64> {
	DateTime::parse_from_rfc3339(timestamp)
		.or_else(|_| DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.f%z"))
		.ok()
		.map(|time| time.timestamp_millis() as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_shreddit_fragment_rejects_other_paths() {
		for path in ["/api/v1/me", "https://evil.example/svc/shreddit/x", "/svc/shreddit"] {
			let error = shreddit_fragment(path.to_string(), None).await.unwrap_err();
			assert!(error.starts_with("Refusing to fetch non-shreddit path"), "{path}: {error}");
		}
	}

	/// An old listing page cut down to what is read from it.
	const FEED_PAGE: &str = r#"
<body class="listing-page multi-page new-page"><div class="side"><div class="thing" data-fullname="t5_2s7lj"></div></div>
<div id="siteTable" class="sitetable linklisting">
  <div class=" thing id-t3_c promoted link" data-fullname="t3_c" data-promoted="true"></div>
  <div class=" thing id-t3_a odd link self" data-fullname="t3_a" data-subreddit="rust" data-promoted="false"><a class="title" href="/r/rust/comments/a/x/">A</a></div>
  <div class="clearleft"></div>
  <div class=" thing id-t3_b even link" data-fullname="t3_b" data-subreddit="golang" data-promoted="false"></div>
  <div class="nav-buttons"><span class="nextprev">view more:
    <span class="prev-button"><a href="https://www.reddit.com/r/golang+rust/new/?count=26&amp;before=t3_a" rel="nofollow prev">prev</a></span>
    <span class="next-button"><a href="https://www.reddit.com/r/golang+rust/new/?count=50&amp;after=t3_b" rel="nofollow next">next</a></span>
  </span></div>
</div></body>"#;

	#[test]
	fn test_parse_feed_page() {
		let (ids, after) = parse_feed_page(FEED_PAGE).unwrap();
		assert_eq!(ids, ["t3_a", "t3_b"], "posts in page order, without the ad and the sidebar");
		assert_eq!(after.as_deref(), Some("t3_b"));

		let last = FEED_PAGE.replace("next-button", "gone");
		assert_eq!(parse_feed_page(&last).unwrap().1, None, "the last page has no cursor");
		assert_eq!(parse_feed_page(r#"<div id="siteTable"></div>"#), Some((Vec::new(), None)), "an empty feed");
		assert_eq!(parse_feed_page("<title>Reddit - Prove your humanity</title>"), None, "not a listing page");
	}

	#[test]
	fn test_combined_feed_page() {
		let page = |path| combined_feed_page(path);
		assert_eq!(page("/r/rust%2Bgolang/new.json?&raw_json=1").as_deref(), Some("/r/rust+golang/new/?"));
		assert_eq!(
			page("/r/a+u_b-c/top.json?t=week&after=t3_x&limit=50&raw_json=1&x=1&before=").as_deref(),
			Some("/r/a+u_b-c/top/?t=week&after=t3_x&limit=50")
		);
		assert_eq!(page("/r/a+b/hot.json?limit=5").as_deref(), Some("/r/a+b/?limit=5"));
		// Single subreddits and other pages have no old listing page
		for path in [
			"/r/rust/new.json",
			"/r/popular/hot.json",
			"/r/a+b/search.json?q=x",
			"/r/a+b/about.json",
			"/r/a+b/../new.json",
			"/r/a+b?x/new.json",
			"/.json",
		] {
			assert_eq!(page(path), None, "{path}");
		}
	}

	use std::sync::{Arc, Mutex};

	const FRAGMENT: &str = r#"
<shreddit-comment thingId="t1_a" author="alice" depth="0" comment-position="0" comment-parent-positions="[]" score="12" created="2026-10-22T15:44:22.205000+0000" permalink="/r/test/comments/p/comment/a/">
  <details><summary><div slot="commentMeta"><span>alice</span><span>Edited <faceplate-timeago ts="2026-10-22T19:43:36.170Z"></faceplate-timeago></span></div></summary>
  <div id="t1_a-comment-rtjson-content" slot="comment"><div id="t1_a-post-rtjson-content"><p>Hello <a href="https://www.reddit.com/r/rust">r/rust</a></p></div></div>
  <shreddit-comment thingId="t1_b" author="bob" depth="1" parentId="t1_a" comment-position="0" comment-parent-positions="[0]" score="3" created="2026-10-22T16:00:00.000000+0000" is-mod-distinguished collapsed>
    <div id="t1_b-comment-rtjson-content" slot="comment"><div id="t1_b-post-rtjson-content"><p>Reply</p></div></div>
    <faceplate-partial src="/svc/shreddit/more-comments/test/t3_p?startingDepth=2&amp;commentParentPositions=0,0" method="post">
      <input type="hidden" name="cursor" value="c1"><faceplate-number number="7"></faceplate-number> more replies
    </faceplate-partial>
  </shreddit-comment>
  <faceplate-partial src="/svc/shreddit/more-comments/test/t3_p?startingDepth=1&amp;commentPositionOffset=1&amp;commentParentPositions=0" method="post">
    <input type="hidden" name="cursor" value="c2"><faceplate-number number="4"></faceplate-number> more replies
  </faceplate-partial>
  </details>
</shreddit-comment>
<shreddit-comment thingId="t1_c" depth="0" comment-position="1" comment-parent-positions="[]" score="1"></shreddit-comment>
<faceplate-partial src="/svc/shreddit/more-comments/test/t3_p?top-level=1&amp;comments-remaining=40&amp;commentPositionOffset=2" method="post">
  <input type="hidden" name="cursor" value="c3">
</faceplate-partial>
<faceplate-partial src="/svc/shreddit/user-hover-card/alice"></faceplate-partial>
"#;

	const MORE_REPLIES: &str = r#"
<shreddit-comment thingId="t1_d" author="dave" depth="1" comment-position="1" comment-parent-positions="[0]" score="2"></shreddit-comment>
<shreddit-comment thingId="t1_e" author="erin" depth="2" comment-position="0" comment-parent-positions="[0,1]" score="2"></shreddit-comment>
"#;

	fn tree(focus: Option<&str>) -> CommentTree {
		let mut tree = CommentTree::new("t3_p".into(), focus.map(str::to_string));
		tree.add(parse_fragment(FRAGMENT));
		tree
	}

	#[test]
	fn test_parse_fragment() {
		let fragment = parse_fragment(FRAGMENT);
		let ids: Vec<&str> = fragment.comments.iter().map(|comment| comment.id.as_str()).collect();
		assert_eq!(ids, ["t1_a", "t1_b", "t1_c"]);

		let alice = &fragment.comments[0].data;
		assert_eq!(alice["id"], "a");
		assert_eq!(alice["body"], "Hello r/rust");
		assert!(alice["body_html"]
			.as_str()
			.unwrap()
			.starts_with(r#"<div class="md"><p>Hello <a href="https://www.reddit.com/r/rust">"#));
		assert_eq!(alice["score"], 12);
		assert_eq!(alice["created_utc"], 1_792_683_862.205);
		assert_eq!(alice["edited"], 1_792_698_216.17);
		assert_eq!(alice["distinguished"], Value::Null);

		let bob = &fragment.comments[1].data;
		assert_eq!(bob["edited"], false);
		assert_eq!(bob["distinguished"], "moderator");
		assert_eq!(bob["stickied"], true);
		assert_eq!(fragment.comments[2].data["author"], "[deleted]");
		assert_eq!(fragment.comments[2].data["body"], "");

		let more: Vec<(&str, i64, Option<&str>)> = fragment
			.more
			.iter()
			.map(|more| (more.cursor.as_str(), more.count, more.parent_positions.as_deref()))
			.collect();
		assert_eq!(more, [("c1", 7, Some("0,0")), ("c2", 4, Some("0")), ("c3", 40, None)]);
	}

	#[test]
	fn test_listing_builds_tree_and_more_stubs() {
		let mut tree = tree(None);
		let batch = tree.next_batch(1);
		assert_eq!(batch[0].cursor, "c1");
		tree.skip(batch.into_iter().next().unwrap());
		tree.add(parse_fragment(MORE_REPLIES));

		let listing = tree.into_listing(0);
		let top = &listing["data"]["children"];
		assert_eq!(top.as_array().unwrap().len(), 3);
		assert_eq!(top[0]["data"]["id"], "a");
		assert_eq!(top[0]["data"]["parent_id"], "t3_p");
		assert_eq!(top[1]["data"]["id"], "c");
		assert_eq!(top[1]["data"]["replies"], "");
		assert_eq!(top[2]["kind"], "more");
		assert_eq!(top[2]["data"]["count"], 40);

		let replies = &top[0]["data"]["replies"]["data"]["children"];
		let reply_ids: Vec<&str> = replies.as_array().unwrap().iter().map(|reply| reply["data"]["id"].as_str().unwrap()).collect();
		assert_eq!(reply_ids, ["b", "d", "_"]);
		assert_eq!(replies[1]["data"]["parent_id"], "t1_a");
		assert_eq!(replies[2]["data"]["parent_id"], "t1_a");
		assert_eq!(replies[1]["data"]["replies"]["data"]["children"][0]["data"]["parent_id"], "t1_d");

		let bob_replies = &replies[0]["data"]["replies"]["data"]["children"];
		assert_eq!(bob_replies[0]["kind"], "more");
		assert_eq!(bob_replies[0]["data"]["parent_id"], "t1_b");
		assert_eq!(bob_replies[0]["data"]["count"], 7);
	}

	#[test]
	fn test_focused_thread() {
		let mut tree = tree(Some("t1_b"));
		let batch = tree.next_batch(10);
		assert_eq!(batch.iter().map(|more| more.cursor.as_str()).collect::<Vec<_>>(), ["c1"]);
		let focused = tree.into_listing(0);
		assert_eq!(focused["data"]["children"].as_array().unwrap().len(), 1);
		assert_eq!(focused["data"]["children"][0]["data"]["id"], "b");

		let with_context = self::tree(Some("t1_b")).into_listing(3);
		let top = &with_context["data"]["children"];
		assert_eq!(top.as_array().unwrap().len(), 1);
		assert_eq!(top[0]["data"]["id"], "a");
		let replies = &top[0]["data"]["replies"]["data"]["children"];
		assert_eq!(replies.as_array().unwrap().len(), 1);
		assert_eq!(replies[0]["data"]["id"], "b");
	}

	fn ids(listing: &Value) -> Vec<String> {
		listing["data"]["children"]
			.as_array()
			.unwrap()
			.iter()
			.map(|child| child["data"]["id"].as_str().unwrap().to_string())
			.collect()
	}

	fn limits(max_comments: usize, max_expansions: usize, concurrency: usize) -> CrawlLimits {
		CrawlLimits {
			max_comments,
			max_expansions,
			concurrency,
		}
	}

	/// A fetcher that records requested cursors and answers `c2` with `MORE_REPLIES`.
	fn fetcher(calls: Arc<Mutex<Vec<String>>>) -> impl Fn(String, String) -> std::future::Ready<Result<String, String>> {
		move |_path, cursor| {
			calls.lock().unwrap().push(cursor.clone());
			std::future::ready(match cursor.as_str() {
				"c2" => Ok(MORE_REPLIES.to_string()),
				_ => Err(format!("no fragment for {cursor}")),
			})
		}
	}

	#[test]
	fn test_comments_path() {
		assert_eq!(comments_path("rust", "abc", None, ""), "/svc/shreddit/comments/r/rust/t3_abc");
		assert_eq!(comments_path("rust", "abc", None, "new"), "/svc/shreddit/comments/r/rust/t3_abc?sort=NEW");
		assert_eq!(comments_path("rust", "abc", Some("def"), "q&a"), "/svc/shreddit/comments/r/rust/t3_abc/t1_def?sort=Q%26A");
	}

	#[test]
	fn test_unix_time() {
		assert_eq!(unix_time("1970-01-01T00:00:01.500Z"), Some(1.5));
		assert_eq!(unix_time("1970-01-01T00:00:01.500000+0000"), Some(1.5));
		assert_eq!(unix_time("1970-01-01T01:00:00+01:00"), Some(0.0));
		assert_eq!(unix_time("yesterday"), None);
		assert_eq!(unix_time(""), None);
	}

	#[test]
	fn test_parse_comment_edge_cases() {
		let fragment = parse_fragment(
			r#"
<shreddit-comment author="nobody" depth="0"></shreddit-comment>
<shreddit-comment thingId="t1_x" author="xavier" score="many" collapsed>
  <shreddit-comment thingId="t1_y" author="yara" is-mod-distinguished>
    <div id="t1_y-comment-rtjson-content" slot="comment"><p>Outer only</p></div>
  </shreddit-comment>
</shreddit-comment>"#,
		);
		let ids: Vec<&str> = fragment.comments.iter().map(|comment| comment.id.as_str()).collect();
		assert_eq!(ids, ["t1_x", "t1_y"], "comments without an ID are skipped");

		let xavier = &fragment.comments[0];
		assert_eq!(xavier.data["body"], "", "a missing body must not borrow the reply's body");
		assert_eq!(xavier.data["body_html"], r#"<div class="md"></div>"#);
		assert_eq!(xavier.data["score"], 0);
		assert_eq!(xavier.data["created_utc"], 0.0);
		assert_eq!(xavier.data["stickied"], false, "only moderator comments are stickied");
		assert_eq!(xavier.depth, 0);
		assert_eq!(xavier.position, "");
		assert_eq!(xavier.parent_positions, "");

		let yara = &fragment.comments[1];
		assert_eq!(yara.data["body"], "Outer only");
		assert_eq!(yara.data["body_html"], r#"<div class="md"><p>Outer only</p></div>"#);
		assert_eq!(yara.data["distinguished"], "moderator");
		assert_eq!(yara.data["stickied"], false, "uncollapsed moderator comments are not stickied");
	}

	#[test]
	fn test_parse_more_comments_edge_cases() {
		let fragment = parse_fragment(
			r#"
<faceplate-partial src="/svc/shreddit/more-comments/t/t3_p?commentParentPositions=1"></faceplate-partial>
<faceplate-partial src="https://evil.example/svc/shreddit/more-comments/t/t3_p"><input name="cursor" value="evil"></faceplate-partial>
<faceplate-partial src="/svc/shreddit/more-comments/t/t3_p?commentParentPositions=1"><input name="cursor" value="no-count"></faceplate-partial>
<faceplate-partial src="/svc/shreddit/more-comments/t/t3_p"><input name="cursor" value="c+d"><faceplate-number number="x"></faceplate-number></faceplate-partial>"#,
		);
		let more: Vec<(&str, i64, Option<&str>)> = fragment
			.more
			.iter()
			.map(|more| (more.cursor.as_str(), more.count, more.parent_positions.as_deref()))
			.collect();
		assert_eq!(more, [("no-count", 0, Some("1")), ("c+d", 0, None)]);
		assert_eq!(fragment.more[0].path, "/svc/shreddit/more-comments/t/t3_p?commentParentPositions=1");
	}

	#[test]
	fn test_add_deduplicates_and_resolves_parents() {
		let mut tree = tree(None);
		tree.add(parse_fragment(FRAGMENT));
		assert_eq!(tree.len(), 3, "comments seen in several fragments are kept once");
		assert_eq!(tree.pending.len(), 6, "placeholders are not deduplicated");

		tree.add(parse_fragment(
			r#"<shreddit-comment thingId="t1_orphan" depth="3" comment-position="0" comment-parent-positions="[9,9]"></shreddit-comment>"#,
		));
		assert_eq!(tree.parents["t1_b"], "t1_a", "parentId attribute");
		assert_eq!(tree.parents["t1_c"], "t3_p", "top-level comment");
		assert_eq!(tree.parents["t1_orphan"], "t3_p", "unknown positions fall back to the post");

		tree.add(parse_fragment(MORE_REPLIES));
		assert_eq!(tree.parents["t1_d"], "t1_a", "resolved through comment positions");
		assert_eq!(tree.parents["t1_e"], "t1_d", "resolved through nested comment positions");
	}

	#[test]
	fn test_next_batch_is_breadth_first_and_bounded() {
		let mut tree = tree(None);
		let cursors = |batch: Vec<MoreComments>| batch.into_iter().map(|more| more.cursor).collect::<Vec<_>>();
		assert_eq!(cursors(tree.next_batch(2)), ["c1", "c2"]);
		assert_eq!(cursors(tree.next_batch(2)), ["c3"]);
		assert!(tree.next_batch(2).is_empty());
	}

	#[test]
	fn test_focus_only_expands_descendants() {
		let mut tree = tree(Some("t1_a"));
		let batch: Vec<String> = tree.next_batch(10).into_iter().map(|more| more.cursor).collect();
		assert_eq!(batch, ["c1", "c2"], "the top-level placeholder is outside the thread");
		assert_eq!(tree.skipped.len(), 1);
	}

	#[test]
	fn test_focus_missing_returns_full_tree() {
		let listing = tree(Some("t1_missing")).into_listing(0);
		assert_eq!(ids(&listing), ["a", "c", "_"]);
	}

	#[test]
	fn test_context_stops_at_top_level() {
		let mut tree = tree(Some("t1_e"));
		tree.add(parse_fragment(MORE_REPLIES));

		let listing = tree.into_listing(9999);
		assert_eq!(ids(&listing), ["a"]);
		let a = &listing["data"]["children"][0]["data"];
		assert_eq!(ids(&a["replies"]), ["d"], "ancestors only keep the thread's branch");
		assert_eq!(ids(&a["replies"]["data"]["children"][0]["data"]["replies"]), ["e"]);
	}

	#[test]
	fn test_context_limits_ancestors() {
		let mut tree = tree(Some("t1_e"));
		tree.add(parse_fragment(MORE_REPLIES));
		assert_eq!(ids(&tree.into_listing(1)), ["d"]);
	}

	#[test]
	fn test_unloaded_parent_makes_comment_top_level() {
		let mut tree = CommentTree::new("t3_p".into(), None);
		tree.add(parse_fragment(
			r#"<shreddit-comment thingId="t1_x" parentId="t1_gone" depth="1"></shreddit-comment>
<shreddit-comment thingId="t1_y" parentId="t1_x" depth="2"></shreddit-comment>"#,
		));
		assert_eq!(tree.parents["t1_x"], "t3_p");
		assert_eq!(tree.parents["t1_y"], "t1_x");
	}

	#[test]
	fn test_reply_placeholder_of_unloaded_parent_stays_a_reply() {
		let mut tree = CommentTree::new("t3_p".into(), None);
		tree.add(parse_fragment(MORE_REPLIES));
		tree.add(parse_fragment(
			r#"<faceplate-partial src="/svc/shreddit/more-comments/t/t3_p?commentParentPositions=0" method="post"><input name="cursor" value="c9"></faceplate-partial>"#,
		));
		let listing = tree.into_listing(0);
		let stub = listing["data"]["children"].as_array().unwrap().last().unwrap().clone();
		assert_eq!(stub["kind"], "more");
		assert_eq!(stub["data"]["parent_id"], "t1_");
	}

	#[test]
	fn test_more_stub_shape() {
		let listing = tree(None).into_listing(0);
		let stub = &listing["data"]["children"][2];
		assert_eq!(stub["kind"], "more");
		assert_eq!(stub["data"]["count"], 40);
		assert_eq!(stub["data"]["parent_id"], "t3_p");
		assert_eq!(stub["data"]["children"], json!([]));
		assert_eq!(stub["data"]["cursor"], "c3");
		assert_eq!(stub["data"]["more_query"], "top-level=1&comments-remaining=40&commentPositionOffset=2");
	}

	#[test]
	fn test_more_path_reencodes_query() {
		assert_eq!(
			more_path("rust", "abc", "top-level=1&commentPositionOffset=2"),
			"/svc/shreddit/more-comments/rust/t3_abc?top-level=1&commentPositionOffset=2"
		);
		assert_eq!(more_path("rust", "abc", "a=b c#frag&/x"), "/svc/shreddit/more-comments/rust/t3_abc?a=b+c%23frag&%2Fx=");
	}

	#[tokio::test]
	async fn test_expand_adds_fragments_and_keeps_failures_as_stubs() {
		let calls = Arc::new(Mutex::new(Vec::new()));
		let mut tree = tree(None);
		expand(&mut tree, limits(100, 100, 10), fetcher(calls.clone())).await;

		assert_eq!(*calls.lock().unwrap(), ["c1", "c2", "c3"]);
		assert_eq!(tree.len(), 5);
		assert_eq!(tree.skipped.len(), 2, "failed expansions stay as stubs");

		let listing = tree.into_listing(0);
		assert_eq!(ids(&listing), ["a", "c", "_"]);
		assert_eq!(ids(&listing["data"]["children"][0]["data"]["replies"]), ["b", "d"]);
	}

	#[tokio::test]
	async fn test_expand_respects_expansion_budget() {
		let calls = Arc::new(Mutex::new(Vec::new()));
		let mut tree = tree(None);
		expand(&mut tree, limits(100, 2, 1), fetcher(calls.clone())).await;

		assert_eq!(*calls.lock().unwrap(), ["c1", "c2"]);
		assert_eq!(tree.pending.len(), 1, "the remaining placeholder is left unexpanded");
	}

	#[tokio::test]
	async fn test_expand_stops_at_comment_budget() {
		let calls = Arc::new(Mutex::new(Vec::new()));
		let mut tree = tree(None);
		expand(&mut tree, limits(3, 100, 10), fetcher(calls.clone())).await;

		assert!(calls.lock().unwrap().is_empty());
		assert_eq!(tree.pending.len(), 3);
	}

	#[tokio::test]
	async fn test_expand_in_focus_mode_skips_other_threads() {
		let calls = Arc::new(Mutex::new(Vec::new()));
		let mut tree = tree(Some("t1_b"));
		expand(&mut tree, limits(100, 100, 10), fetcher(calls.clone())).await;

		assert_eq!(*calls.lock().unwrap(), ["c1"]);
	}
}
