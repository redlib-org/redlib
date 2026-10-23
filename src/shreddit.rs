//! Comment trees from Reddit's server-rendered web frontend ("shreddit").
//!
//! Reddit's edge blocks the OAuth comments API for anonymous clients, but
//! reddit.com still serves logged-out visitors HTML fragments of comment
//! trees. This module crawls those fragments and converts them into the API's
//! `Listing` JSON, so comment parsing and rendering stay unchanged.

use crate::client::shreddit_fragment;
use cached::proc_macro::cached;
use chrono::DateTime;
use log::warn;
use scraper::{ElementRef, Html, Selector};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::LazyLock;
use tokio::task::JoinSet;

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

const CRAWL_LIMITS: CrawlLimits = CrawlLimits {
	max_comments: 200,
	max_expansions: 30,
	concurrency: 10,
};

const MORE_COMMENTS_PATH_PREFIX: &str = "/svc/shreddit/more-comments/";

static COMMENT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("shreddit-comment").unwrap());
static COMMENT_BODY: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"[slot="comment"]"#).unwrap());
static COMMENT_META: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"[slot="commentMeta"]"#).unwrap());
static SPAN: LazyLock<Selector> = LazyLock::new(|| Selector::parse("span").unwrap());
static TIMEAGO: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-timeago[ts]").unwrap());
static MORE_COMMENTS: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-partial[src]").unwrap());
static CURSOR: LazyLock<Selector> = LazyLock::new(|| Selector::parse(r#"input[name="cursor"]"#).unwrap());
static COUNT: LazyLock<Selector> = LazyLock::new(|| Selector::parse("faceplate-number[number]").unwrap());

/// Fetch the comment tree of a post as a Reddit API comment `Listing`.
///
/// When `comment_id` is set, only that comment's thread is returned, preceded
/// by up to `context` ancestors, mirroring the API's comment permalinks.
#[cached(size = 100, time = 30, result = true)]
pub async fn comments(subreddit: String, post_id: String, comment_id: Option<String>, sort: String, context: Option<String>) -> Result<Value, String> {
	let html = shreddit_fragment(comments_path(&subreddit, &post_id, comment_id.as_deref(), &sort), None).await?;

	let mut tree = CommentTree::new(format!("t3_{post_id}"), comment_id.map(|id| format!("t1_{id}")));
	tree.add(parse_fragment(&html));
	expand(&mut tree, CRAWL_LIMITS, |path, cursor| shreddit_fragment(path, Some(cursor))).await;

	let context = context.and_then(|context| context.parse().ok()).unwrap_or(0);
	Ok(tree.into_listing(context))
}

fn comments_path(subreddit: &str, post_id: &str, comment_id: Option<&str>, sort: &str) -> String {
	let mut path = format!("/svc/shreddit/comments/r/{subreddit}/t3_{post_id}");
	if let Some(comment_id) = comment_id {
		path.push_str(&format!("/t1_{comment_id}"));
	}
	if !sort.is_empty() {
		path.push_str(&format!("?{}", url::form_urlencoded::Serializer::new(String::new()).append_pair("sort", sort).finish()));
	}
	path
}

/// Expand "more comments" placeholders breadth-first, keeping up to
/// `limits.concurrency` requests in flight. Placeholders that fail or exceed
/// `limits` stay in the tree as stubs.
async fn expand<F, Fut>(tree: &mut CommentTree, limits: CrawlLimits, fetch: F)
where
	F: Fn(String, String) -> Fut,
	Fut: Future<Output = Result<String, String>> + Send + 'static,
{
	let mut in_flight = JoinSet::new();
	let mut expansions = 0;
	loop {
		while in_flight.len() < limits.concurrency && expansions < limits.max_expansions && tree.len() < limits.max_comments {
			let Some(more) = tree.next_wanted() else { break };
			let request = fetch(more.path.clone(), more.cursor.clone());
			in_flight.spawn(async move { (more, request.await) });
			expansions += 1;
		}

		match in_flight.join_next().await {
			None => break,
			Some(Ok((_, Ok(html)))) => tree.add(parse_fragment(&html)),
			Some(Ok((more, Err(e)))) => {
				warn!("Couldn't expand more comments: {e}");
				tree.skip(more);
			}
			Some(Err(e)) => warn!("Couldn't expand more comments: {e}"),
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
	More(i64),
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
			let parent = comment
				.parent_id
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

	/// Take the next placeholder worth expanding, breadth-first.
	fn next_wanted(&mut self) -> Option<MoreComments> {
		while let Some(more) = self.pending.pop_front() {
			if self.wants(&more) {
				return Some(more);
			}
			self.skipped.push(more);
		}
		None
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
			self.children.entry(parent).or_default().push(Child::More(more.count));
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
			Child::More(count) => json!({
				"kind": "more",
				"data": { "count": count, "parent_id": parent, "id": "_", "children": [] }
			}),
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
		let more = tree.next_wanted().unwrap();
		assert_eq!(more.cursor, "c1");
		tree.skip(more);
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
		assert_eq!(wanted_cursors(&mut tree), ["c1"]);
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

	fn wanted_cursors(tree: &mut CommentTree) -> Vec<String> {
		std::iter::from_fn(|| tree.next_wanted()).map(|more| more.cursor).collect()
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
		assert_eq!(comments_path("rust", "abc", None, "new"), "/svc/shreddit/comments/r/rust/t3_abc?sort=new");
		assert_eq!(comments_path("rust", "abc", Some("def"), "q&a"), "/svc/shreddit/comments/r/rust/t3_abc/t1_def?sort=q%26a");
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
	fn test_next_wanted_is_breadth_first() {
		let mut tree = tree(None);
		assert_eq!(wanted_cursors(&mut tree), ["c1", "c2", "c3"]);
		assert!(tree.next_wanted().is_none());
	}

	#[test]
	fn test_focus_only_expands_descendants() {
		let mut tree = tree(Some("t1_a"));
		assert_eq!(wanted_cursors(&mut tree), ["c1", "c2"], "the top-level placeholder is outside the thread");
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
	fn test_lineage_survives_parent_cycles() {
		let mut tree = CommentTree::new("t3_p".into(), Some("t1_z".into()));
		tree.add(parse_fragment(
			r#"<shreddit-comment thingId="t1_x" parentId="t1_y" depth="1"></shreddit-comment>
<shreddit-comment thingId="t1_y" parentId="t1_x" depth="1"></shreddit-comment>"#,
		));
		let x = "t1_x".to_string();
		assert_eq!(tree.lineage(&x).count(), 3);
	}

	#[test]
	fn test_more_stub_shape() {
		let listing = tree(None).into_listing(0);
		let stub = &listing["data"]["children"][2];
		assert_eq!(stub["kind"], "more");
		assert_eq!(stub["data"]["count"], 40);
		assert_eq!(stub["data"]["parent_id"], "t3_p");
		assert_eq!(stub["data"]["children"], json!([]));
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

	#[tokio::test]
	async fn test_expand_does_not_wait_for_slow_requests() {
		let (release_c1, c1_released) = tokio::sync::oneshot::channel::<()>();
		let release_c1 = Mutex::new(Some(release_c1));
		let c1_released = Mutex::new(Some(c1_released));
		let fetch = move |_path, cursor: String| -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
			if cursor == "c1" {
				let released = c1_released.lock().unwrap().take().unwrap();
				return Box::pin(async move { released.await.map(|()| String::new()).map_err(|e| e.to_string()) });
			}
			if cursor == "c3" {
				release_c1.lock().unwrap().take().unwrap().send(()).unwrap();
			}
			Box::pin(std::future::ready(Ok(String::new())))
		};

		let mut tree = tree(None);
		let crawl = expand(&mut tree, limits(100, 100, 2), fetch);
		tokio::time::timeout(std::time::Duration::from_secs(5), crawl)
			.await
			.expect("c3 must start while c1 is still in flight");
	}
}
