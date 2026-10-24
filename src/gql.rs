//! Post comments through Reddit's Android GraphQL API. The anonymous JSON
//! comments endpoint returns HTML with HTTP 403, including encoded paths.
//!
//! Adapted from polak14's GraphQL implementation (AGPL-3.0-only):
//! https://github.com/polak14/redlib/commit/a63460b4ac29c27401e3e2aaab34b68dca02991e
//! The RTJSON renderer and comment conversion retain that implementation's design.

use crate::client::{json, CLIENT, OAUTH_CLIENT};
use crate::utils::{format_url, param};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use cached::proc_macro::cached;
use chrono::DateTime;
use serde_json::{json as json_value, Value};
use std::time::Duration;

const GQL_URL: &str = "https://gql-fed.reddit.com/";
// Android PostComments persisted operation; raw GraphQL queries are rejected.
// https://github.com/Amqx/reddit-gql/blob/main/src/operations.ts
const POST_COMMENTS_HASH: &str = "314f29c2494fd78c7844ef2c6b09acb4bef5b37ab50b4ddc01691d6d5c8433f8";
const COMMENT_COUNT: u32 = 200;
const MAX_DEPTH: u32 = 10;

/// Preserve the existing two-Listing response consumed by post::item. Fetch
/// metadata first, so missing/private/quarantined posts retain their API errors.
#[cached(size = 100, time = 30, result = true)]
pub async fn thread(path: String, quarantine: bool) -> Result<Value, String> {
	let (id, comment_id) = parse_path(&path).ok_or("Invalid post path")?;
	let post = json(format!("/by_id/t3_{id}.json?raw_json=1"), quarantine).await?;
	let metadata = post["data"]["children"].as_array().and_then(|p| p.first()).ok_or("Post not found")?;
	let sort = param(&path, "sort")
		.filter(|s| !s.is_empty())
		.or_else(|| metadata["data"]["suggested_sort"].as_str().filter(|s| sort_type(s).is_some()).map(str::to_owned))
		.unwrap_or_else(|| "confidence".to_string());
	let variables = variables(&id, comment_id.as_deref(), &path, &sort)?;
	let body = json_value!({
	 "operationName": "PostComments", "variables": variables,
	 "extensions": { "persistedQuery": { "version": 1, "sha256Hash": POST_COMMENTS_HASH } }
	});
	let response = query(&body).await?;
	let mut comments = comment_listing(&response, &id)?;
	if let Some(comment_id) = comment_id {
		let context = param(&path, "context").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
		if context > 0 && param(&path, "cursor").is_none() {
			let children = comments["data"]["children"].as_array().ok_or("Invalid comments listing")?;
			let (mut depth, mut focused) = focused_branch(children, &comment_id).ok_or("Comment not found")?;
			while depth > context {
				focused = focused["data"]["replies"]["data"]["children"][0].take();
				depth -= 1;
			}
			comments = listing(vec![focused]);
		}
	}
	comments["data"]["sort"] = if sort == "best" { "confidence".into() } else { sort.into() };
	Ok(json_value!([post, comments]))
}

fn sort_type(sort: &str) -> Option<&'static str> {
	Some(match sort {
		"confidence" | "best" => "CONFIDENCE",
		"top" => "TOP",
		"new" => "NEW",
		"controversial" => "CONTROVERSIAL",
		"old" => "OLD",
		"qa" => "QA",
		_ => return None,
	})
}

fn variables(id: &str, comment_id: Option<&str>, path: &str, sort: &str) -> Result<Value, String> {
	let sort = sort_type(sort).ok_or("Unsupported comment sort")?;
	let mut variables = json_value!({
	 "id": format!("t3_{id}"), "sortType": sort, "count": COMMENT_COUNT,
	 "maxDepth": MAX_DEPTH, "includeConsistencyToken": false, "includePremiumAvatarTreatment": false
	});
	if let Some(comment_id) = comment_id {
		// With context=..., Reddit also includes ancestors of the focused comment.
		let context = param(path, "context").and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
		variables["focusedCommentContext"] = json_value!({
		 "commentId": format!("t1_{comment_id}"), "isFocusedThreadOnly": context == 0
		});
	}
	if let Some(cursor) = param(path, "cursor") {
		if cursor.len() > 8192 {
			return Err("Invalid comments cursor".into());
		}
		let cursor = URL_SAFE_NO_PAD
			.decode(cursor)
			.ok()
			.and_then(|c| String::from_utf8(c).ok())
			.ok_or("Invalid comments cursor")?;
		variables["after"] = cursor.into();
	}
	Ok(variables)
}

async fn query(body: &Value) -> Result<Value, String> {
	for attempt in 0..2 {
		let mut builder = CLIENT.post(GQL_URL).timeout(Duration::from_secs(20));
		for (key, value) in &OAUTH_CLIENT.load_full().headers_map {
			if !key.eq_ignore_ascii_case("content-type") {
				builder = builder.header(key, value);
			}
		}
		let response = builder.json(body).send().await.map_err(|e| format!("Reddit comments request failed: {e}"))?;
		let status = response.status();
		if !status.is_success() {
			return Err(format!("Reddit comments request returned {status}"));
		}
		let response: Value = response.json().await.map_err(|_| "Reddit returned an invalid comments response")?;
		// Upstream sometimes times out individual fields while returning HTTP 200.
		if response["errors"].as_array().is_some_and(|errors| !errors.is_empty()) {
			if attempt == 0 {
				continue;
			}
			return Err(format!("Reddit comments API error: {}", response["errors"]));
		}
		return Ok(response);
	}
	unreachable!()
}

fn comment_listing(response: &Value, post_id: &str) -> Result<Value, String> {
	let trees = response["data"]["postInfoById"]["commentForest"]["trees"]
		.as_array()
		.ok_or("Reddit returned no comment forest")?;
	let after = trees
		.iter()
		.find(|t| is_next_page(t))
		.and_then(|t| t["more"]["cursor"].as_str())
		.map(|cursor| URL_SAFE_NO_PAD.encode(cursor));
	let children = nest(trees.iter().filter(|t| !is_next_page(t)).filter_map(|t| item(t, post_id)).collect());
	let mut result = listing(children);
	result["data"]["after"] = after.into();
	Ok(result)
}

fn is_next_page(tree: &Value) -> bool {
	tree["depth"] == 0 && tree["parentId"].is_null() && !tree["more"].is_null()
}

/// Retain the ancestor chain of a focused comment without unrelated siblings.
fn focused_branch(comments: &[Value], id: &str) -> Option<(usize, Value)> {
	for comment in comments {
		if comment["data"]["id"] == id {
			return Some((0, comment.clone()));
		}
		if let Some(replies) = comment["data"]["replies"]["data"]["children"].as_array() {
			if let Some((depth, child)) = focused_branch(replies, id) {
				let mut parent = comment.clone();
				parent["data"]["replies"] = listing(vec![child]);
				return Some((depth + 1, parent));
			}
		}
	}
	None
}

fn listing(children: Vec<Value>) -> Value {
	json_value!({ "kind": "Listing", "data": { "children": children } })
}

fn parse_path(path: &str) -> Option<(String, Option<String>)> {
	let base = path.split('?').next()?.strip_suffix(".json")?;
	let tail = if let Some(tail) = base.strip_prefix("/comments/") {
		tail
	} else {
		let (sub, rest) = base.strip_prefix("/r/")?.split_once('/')?;
		if sub.is_empty() {
			return None;
		}
		rest.strip_prefix("comments/")?
	};
	let parts: Vec<_> = tail.trim_end_matches('/').split('/').collect();
	if parts.len() > 3 || !valid_id(parts[0]) {
		return None;
	}
	let comment = parts.get(2).copied().filter(|s| !s.is_empty());
	if comment.is_some_and(|s| !valid_id(s)) {
		return None;
	}
	Some((parts[0].into(), comment.map(str::to_owned)))
}

fn valid_id(id: &str) -> bool {
	!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Only HTTP(S) and local links may appear in rendered comment attributes.
fn url_attr(value: Option<&str>) -> String {
	let value = value.unwrap_or_default().trim();
	if value.chars().any(char::is_control) {
		return String::new();
	}
	let local = value.starts_with('/') && !value.starts_with("//") && !value.contains('\\');
	let web = url::Url::parse(value).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
	if !local && !web {
		return String::new();
	}
	htmlescape::encode_minimal(&format_url(value))
}

/// Converts one entry of the comment forest to an item of the JSON API, with its depth.
fn item(tree: &Value, post_id: &str) -> Option<(u64, Value)> {
	let depth = tree["depth"].as_u64()?;
	let post_fullname = format!("t3_{post_id}");
	let parent = tree["parentId"].as_str().filter(|p| p.starts_with("t1_")).unwrap_or(&post_fullname).to_string();

	let more = &tree["more"];
	if !more.is_null() {
		let count = more["count"].as_i64().unwrap_or_default();
		let cursor = more["cursor"].as_str().map(|c| URL_SAFE_NO_PAD.encode(c)).unwrap_or_default();
		return Some((depth, json_value!({ "kind": "more", "data": { "id": cursor, "count": count, "parent_id": parent } })));
	}

	let node = &tree["node"];
	let fullname = node["id"].as_str().unwrap_or_default();
	if node["__typename"] != "DeletedComment" && !fullname.strip_prefix("t1_").is_some_and(valid_id) {
		return None;
	}
	let author = node["authorInfo"]["name"].as_str().unwrap_or("[unknown]");
	let content = &node["content"];

	// The JSON API's representation of deleted and removed comments, which post.rs looks for
	let (author, body, body_html) = if node["__typename"] == "DeletedComment" {
		("[deleted]", "[deleted]", "<p>[deleted]</p>".to_string())
	} else if node["isAdminTakedown"] == true {
		(author, "[ Removed by Reddit ]", String::new())
	} else if node["isRemoved"] == true {
		("[deleted]", "[removed]", String::new())
	} else {
		let richtext: Value = content["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
		let html = if richtext["document"].is_array() {
			render(&richtext["document"], &content["richtextMedia"])
		} else {
			format!("<p>{}</p>", htmlescape::encode_minimal(content["markdown"].as_str().unwrap_or_default()))
		};
		(author, content["markdown"].as_str().unwrap_or_default(), html)
	};

	let flair = &node["authorFlair"];
	let flair_richtext: Value = flair["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
	let flair_type = if flair_richtext.as_array().is_some_and(|r| !r.is_empty()) {
		"richtext"
	} else if flair["text"].as_str().is_some_and(|t| !t.is_empty()) {
		"text"
	} else {
		""
	};

	let awards: Vec<Value> = node["awardings"]
		.as_array()
		.into_iter()
		.flatten()
		.map(|a| json_value!({ "name": a["award"]["name"], "count": a["total"], "resized_icons": [{ "url": a["award"]["static_icon_16"]["url"] }] }))
		.collect();

	Some((
		depth,
		json_value!({
			"kind": "t1",
			"data": {
				"id": fullname.strip_prefix("t1_").unwrap_or(fullname),
				"name": fullname,
				"author": author,
				"body": body,
				"body_html": format!("<div class=\"md\">{body_html}</div>"),
				"created_utc": timestamp(&node["createdAt"]).unwrap_or_default(),
				"edited": timestamp(&node["editedAt"]).map_or(Value::Bool(false), Value::from),
				"score": node["score"].as_i64().unwrap_or_default(),
				"score_hidden": node["isScoreHidden"] == true,
				"stickied": node["isStickied"] == true,
				"distinguished": node["distinguishedAs"].as_str().map(str::to_lowercase),
				"author_flair_type": flair_type,
				"author_flair_richtext": flair_richtext,
				"author_flair_text": flair["text"],
				"author_flair_background_color": flair["template"]["backgroundColor"],
				"author_flair_text_color": flair["textColor"].as_str().map(str::to_lowercase),
				"all_awardings": awards,
				"parent_id": parent,
				"replies": "",
			}
		}),
	))
}

fn timestamp(value: &Value) -> Option<f64> {
	DateTime::parse_from_str(value.as_str()?, "%Y-%m-%dT%H:%M:%S%.f%z").ok().map(|t| t.timestamp() as f64)
}

/// Builds the reply tree from items in depth-first order. Goes by depth rather
/// than parent IDs, as deleted comments have no ID.
fn nest(items: Vec<(u64, Value)>) -> Vec<Value> {
	let mut top = Vec::new();
	let mut open: Vec<(u64, Value)> = Vec::new();
	for (depth, item) in items {
		while open.last().is_some_and(|(d, _)| *d >= depth) {
			close(&mut open, &mut top);
		}
		open.push((depth, item));
	}
	while !open.is_empty() {
		close(&mut open, &mut top);
	}
	top
}

/// Attaches the innermost open item to its parent, or to the top level.
fn close(open: &mut Vec<(u64, Value)>, top: &mut Vec<Value>) {
	let Some((_, item)) = open.pop() else { return };
	let Some((_, parent)) = open.last_mut() else {
		top.push(item);
		return;
	};
	let replies = &mut parent["data"]["replies"];
	if !replies.is_object() {
		*replies = listing(Vec::new());
	}
	if let Some(children) = replies["data"]["children"].as_array_mut() {
		children.push(item);
	}
}

/// Renders richtext (RTJSON) nodes as the HTML of the JSON API's `body_html`.
fn render(nodes: &Value, media: &Value) -> String {
	nodes.as_array().into_iter().flatten().map(|node| element(node, media)).collect()
}

fn element(node: &Value, media: &Value) -> String {
	let children = || render(&node["c"], media);
	match node["e"].as_str().unwrap_or_default() {
		// A paragraph of only images is a figure, like the JSON API's
		"par" if node["c"].as_array().is_some_and(|c| c.iter().all(|c| matches!(c["e"].as_str(), Some("img" | "gif")))) => children(),
		"par" => format!("<p>{}</p>", children()),
		"text" => formatted(node),
		"br" => "<br>".to_string(),
		"hr" => "<hr>".to_string(),
		"h" => {
			let level = node["l"].as_u64().unwrap_or(1).clamp(1, 6);
			format!("<h{level}>{}</h{level}>", children())
		}
		"blockquote" => format!("<blockquote>{}</blockquote>", children()),
		"list" => {
			let tag = if node["o"] == true { "ol" } else { "ul" };
			format!("<{tag}>{}</{tag}>", children())
		}
		"li" => format!("<li>{}</li>", children()),
		"code" => {
			let lines: Vec<&str> = node["c"].as_array().into_iter().flatten().map(|line| line["t"].as_str().unwrap_or_default()).collect();
			format!("<pre><code>{}</code></pre>", htmlescape::encode_minimal(&lines.join("\n")))
		}
		"spoilertext" => format!("<span class=\"md-spoiler-text\">{}</span>", children()),
		"link" => {
			let title = node["a"].as_str().map(|a| format!(" title=\"{}\"", htmlescape::encode_minimal(a))).unwrap_or_default();
			format!("<a href=\"{}\"{title}>{}</a>", url_attr(node["u"].as_str()), formatted(node))
		}
		kind @ ("r/" | "u/") => {
			let name = htmlescape::encode_minimal(node["t"].as_str().unwrap_or_default());
			let slash = if node["l"] == true { "/" } else { "" };
			format!("<a href=\"/{kind}{name}\">{slash}{kind}{name}</a>")
		}
		"img" | "gif" => {
			let Some(item) = media.as_array().and_then(|m| m.iter().find(|m| m["id"] == node["id"])) else {
				return String::new();
			};
			let src = url_attr(item["url"].as_str());
			format!("<figure><a href=\"{src}\"><img src=\"{src}\" loading=\"lazy\"></a></figure>")
		}
		"table" => table(node, media),
		// Unknown elements: keep their content
		_ if node["c"].is_array() => children(),
		_ => htmlescape::encode_minimal(node["t"].as_str().unwrap_or_default()),
	}
}

/// RTJSON format bits and their tags
const FORMATS: [(u64, &str); 6] = [(1, "strong"), (2, "em"), (8, "del"), (16, "sub"), (32, "sup"), (64, "code")];

/// Text with format ranges (`[bits, start, length]` in characters) as HTML.
fn formatted(node: &Value) -> String {
	let text: Vec<char> = node["t"].as_str().unwrap_or_default().chars().collect();
	let mut bits = vec![0u64; text.len()];
	for range in node["f"].as_array().into_iter().flatten() {
		let (Some(format), Some(start), Some(len)) = (range[0].as_u64(), range[1].as_u64(), range[2].as_u64()) else {
			continue;
		};
		for b in bits.iter_mut().skip(start as usize).take(len as usize) {
			*b |= format;
		}
	}

	let mut out = String::new();
	let mut start = 0;
	while start < text.len() {
		let end = (start..text.len()).find(|&i| bits[i] != bits[start]).unwrap_or(text.len());
		let tags: Vec<&str> = FORMATS.iter().filter(|(bit, _)| bits[start] & bit != 0).map(|(_, tag)| *tag).collect();
		tags.iter().for_each(|tag| out += &format!("<{tag}>"));
		out += &htmlescape::encode_minimal(&text[start..end].iter().collect::<String>());
		tags.iter().rev().for_each(|tag| out += &format!("</{tag}>"));
		start = end;
	}
	out
}

fn table(node: &Value, media: &Value) -> String {
	let align = |header: Option<&Value>| match header.and_then(|h| h["a"].as_str()) {
		Some("L") => " align=\"left\"",
		Some("C") => " align=\"center\"",
		Some("R") => " align=\"right\"",
		_ => "",
	};
	let headers = node["h"].as_array().map(Vec::as_slice).unwrap_or_default();

	let mut out = "<table><thead><tr>".to_string();
	for header in headers {
		out += &format!("<th{}>{}</th>", align(Some(header)), render(&header["c"], media));
	}
	out += "</tr></thead><tbody>";
	for row in node["c"].as_array().into_iter().flatten() {
		out += "<tr>";
		for (i, cell) in row.as_array().into_iter().flatten().enumerate() {
			out += &format!("<td{}>{}</td>", align(headers.get(i)), render(&cell["c"], media));
		}
		out += "</tr>";
	}
	out + "</tbody></table>"
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn post_paths_do_not_confuse_titles_or_queries_with_ids() {
		for path in [
			"/r/unixporn/comments/1xdp2ww/title/.json?sort=new",
			"/comments/1xdp2ww.json",
			"/r/comments/comments/1xdp2ww/.json",
		] {
			assert_eq!(parse_path(path), Some(("1xdp2ww".into(), None)));
		}
		assert_eq!(
			parse_path("/r/unixporn/comments/1xdp2ww/title/abc/.json?context=3"),
			Some(("1xdp2ww".into(), Some("abc".into())))
		);
		for path in [
			"/r/rust/hot.json",
			"/user/a/comments.json",
			"/r//comments/abc.json",
			"/comments/.json",
			"/comments/a/title/b/extra.json",
			"/comments/a%2fb.json",
		] {
			assert_eq!(parse_path(path), None, "{path}");
		}
	}

	#[test]
	fn sorts_focus_and_opaque_cursor_are_preserved() {
		let cursor = URL_SAFE_NO_PAD.encode("opaque:+/= cursor");
		let path = format!("/comments/abc/title/def.json?context=3&cursor={cursor}");
		for (sort, expected) in [
			("confidence", "CONFIDENCE"),
			("best", "CONFIDENCE"),
			("top", "TOP"),
			("new", "NEW"),
			("controversial", "CONTROVERSIAL"),
			("old", "OLD"),
			("qa", "QA"),
		] {
			let vars = variables("abc", Some("def"), &path, sort).unwrap();
			assert_eq!(vars["id"], "t3_abc");
			assert_eq!(vars["sortType"], expected);
			assert_eq!(vars["after"], "opaque:+/= cursor");
			assert_eq!(vars["focusedCommentContext"], json_value!({"commentId": "t1_def", "isFocusedThreadOnly": false}));
		}
		assert_eq!(variables("abc", Some("def"), "?", "top").unwrap()["focusedCommentContext"]["isFocusedThreadOnly"], true);
		assert!(variables("abc", None, "?cursor=%FF", "top").is_err());
		assert!(variables("abc", None, "?", "invalid").is_err());
	}

	#[test]
	fn listing_distinguishes_empty_comments_from_upstream_failure() {
		assert!(comment_listing(&json_value!({"errors": [{"message":"blocked"}]}), "p").is_err());
		assert!(comment_listing(&json_value!({"data":{"postInfoById":null}}), "p").is_err());
		let listing = comment_listing(&json_value!({"data":{"postInfoById":{"commentForest":{"trees":[]}}}}), "p").unwrap();
		assert_eq!(listing["data"]["children"], json_value!([]));
		assert!(listing["data"]["after"].is_null());
	}

	#[test]
	fn pagination_is_separate_from_reply_placeholders() {
		let response = json_value!({"data":{"postInfoById":{"commentForest":{"trees":[
			{"depth":0,"node":{"id":"t1_a","__typename":"Comment"}},
			{"depth":1,"parentId":"t1_a","more":{"cursor":"nested","count":7}},
			{"depth":0,"more":{"cursor":"opaque:+/= cursor","count":30}}
		]}}}});
		let listing = comment_listing(&response, "p").unwrap();
		assert_eq!(listing["data"]["children"].as_array().unwrap().len(), 1);
		assert_eq!(listing["data"]["children"][0]["data"]["replies"]["data"]["children"][0]["data"]["parent_id"], "t1_a");
		assert_eq!(listing["data"]["children"][0]["data"]["replies"]["data"]["children"][0]["data"]["count"], 7);
		let cursor = listing["data"]["after"].as_str().unwrap();
		assert_eq!(variables("p", None, &format!("?cursor={cursor}"), "new").unwrap()["after"], "opaque:+/= cursor");
		assert_eq!(
			listing["data"]["children"][0]["data"]["replies"]["data"]["children"][0]["data"]["id"],
			URL_SAFE_NO_PAD.encode("nested")
		);
	}

	#[test]
	fn focused_context_retains_ancestors_and_replies_without_siblings() {
		let response = json_value!({"data":{"postInfoById":{"commentForest":{"trees":[
			{"depth":0,"node":{"id":"t1_a","__typename":"Comment"}},
			{"depth":1,"parentId":"t1_a","node":{"id":"t1_b","__typename":"Comment"}},
			{"depth":2,"parentId":"t1_b","node":{"id":"t1_c","__typename":"Comment"}},
			{"depth":1,"parentId":"t1_a","node":{"id":"t1_sibling","__typename":"Comment"}},
			{"depth":0,"node":{"id":"t1_other","__typename":"Comment"}}
		]}}}});
		let listing = comment_listing(&response, "p").unwrap();
		let children = listing["data"]["children"].as_array().unwrap();
		let (depth, branch) = focused_branch(children, "b").unwrap();
		assert_eq!(depth, 1);
		assert_eq!(branch["data"]["id"], "a");
		let replies = branch["data"]["replies"]["data"]["children"].as_array().unwrap();
		assert_eq!(replies.len(), 1);
		assert_eq!(replies[0]["data"]["id"], "b");
		assert_eq!(replies[0]["data"]["replies"]["data"]["children"][0]["data"]["id"], "c");
		assert!(focused_branch(children, "missing").is_none());
	}

	#[test]
	fn comments_keep_metadata_and_plain_text_fallback_is_escaped() {
		let tree = json_value!({"depth":0,"node":{
			"id":"t1_a","__typename":"Comment","authorInfo":{"name":"alice"},
			"content":{"markdown":"<script>x</script> & text"},
			"createdAt":"2026-10-20T17:00:53.706000+0000", "editedAt":"2026-10-20T17:01:53.000000+0000",
			"score":18,"isStickied":true,"isScoreHidden":true,"distinguishedAs":"MODERATOR",
			"authorFlair":{"text":"flair","textColor":"DARK","template":{"backgroundColor":"#fff"}}
		}});
		let (_, comment) = item(&tree, "p").unwrap();
		let data = &comment["data"];
		assert_eq!(data["id"], "a");
		assert_eq!(data["parent_id"], "t3_p");
		assert_eq!(data["body_html"], "<div class=\"md\"><p>&lt;script&gt;x&lt;/script&gt; &amp; text</p></div>");
		assert_eq!(data["created_utc"], 1792515653.0);
		assert_eq!(data["edited"], 1792515713.0);
		assert_eq!(data["score"], 18);
		assert_eq!(data["distinguished"], "moderator");
		assert_eq!(data["stickied"], true);
		assert_eq!(data["score_hidden"], true);
		assert_eq!(data["author_flair_text"], "flair");
		let mut removed = tree;
		removed["node"]["isRemoved"] = true.into();
		assert_eq!(item(&removed, "p").unwrap().1["data"]["body"], "[removed]");
	}

	#[test]
	fn unsafe_urls_never_reach_html_attributes() {
		for url in [
			"javascript:alert(1)",
			"JaVaScRiPt:alert(1)",
			"java\nscript:alert(1)",
			"data:text/html,<script>x</script>",
			"//evil.test/x",
			"/\\evil.test",
			"vbscript:msgbox(1)",
		] {
			assert!(url_attr(Some(url)).is_empty(), "{url}");
		}
		assert_eq!(url_attr(Some("/r/rust?a=1&b=2")), "/r/rust?a=1&amp;b=2");
	}

	#[test]
	fn test_render() {
		let doc = json_value!([
			{ "e": "par", "c": [
				{ "e": "text", "t": "a <b> ünï bold", "f": [[1, 10, 4], [3, 12, 2]] },
				{ "e": "link", "t": "x", "u": "javascript:alert(1)", "a": "t\"" },
				{ "e": "r/", "t": "rust", "l": true },
				{ "e": "spoilertext", "c": [{ "e": "text", "t": "s" }] },
			]},
			{ "e": "par", "c": [{ "e": "gif", "id": "giphy|g" }, { "e": "img", "id": "missing" }] },
			{ "e": "code", "c": [{ "e": "raw", "t": "<x>" }, { "e": "raw", "t": "y" }] },
			{ "e": "list", "o": true, "c": [{ "e": "li", "c": [{ "e": "par", "c": [{ "e": "text", "t": "i" }] }] }] },
			{ "e": "table", "h": [{ "a": "R", "c": [{ "e": "text", "t": "H" }] }, { "c": [] }], "c": [[{ "c": [{ "e": "text", "t": "1" }] }, { "c": [] }]] },
			{ "e": "future", "c": [{ "e": "text", "t": "kept" }] },
		]);
		let media = json_value!([{ "id": "giphy|g", "url": "https://external-preview.redd.it/g.gif?s=1&a=2" }]);
		assert_eq!(
			render(&doc, &media),
			concat!(
				r#"<p>a &lt;b&gt; ünï <strong>bo</strong><strong><em>ld</em></strong><a href="" title="t&quot;">x</a><a href="/r/rust">/r/rust</a><span class="md-spoiler-text">s</span></p>"#,
				r#"<figure><a href="/preview/external-pre/g.gif?s=1&amp;a=2"><img src="/preview/external-pre/g.gif?s=1&amp;a=2" loading="lazy"></a></figure>"#,
				"<pre><code>&lt;x&gt;\ny</code></pre>",
				"<ol><li><p>i</p></li></ol>",
				r#"<table><thead><tr><th align="right">H</th><th></th></tr></thead><tbody><tr><td align="right">1</td><td></td></tr></tbody></table>"#,
				"kept",
			)
		);
	}

	#[test]
	fn test_tree() {
		let comment = |id: &str, depth: u64, parent: Option<&str>| json_value!({ "depth": depth, "parentId": parent, "node": { "__typename": "Comment", "id": id, "authorInfo": { "name": "a" } } });
		let trees = [
			comment("t1_a", 0, None),
			// Deleted comments have no ID; their replies nest by depth
			json_value!({ "depth": 1, "parentId": "t1_a", "node": { "__typename": "DeletedComment" } }),
			comment("t1_c", 2, Some("t1_x")),
			json_value!({ "depth": 1, "parentId": "t1_a", "more": { "count": 5, "cursor": "c1" } }),
			comment("t1_d", 0, None),
			json_value!({ "depth": 0, "parentId": null, "more": { "count": 90, "cursor": "c2" } }),
		];
		let comments = nest(trees.iter().filter_map(|t| item(t, "p")).collect());
		assert_eq!(comments.len(), 3);

		let a = &comments[0]["data"];
		assert_eq!(a["parent_id"], "t3_p");
		let replies = a["replies"]["data"]["children"].as_array().unwrap();
		assert_eq!(replies.len(), 2);
		assert_eq!(replies[0]["data"]["author"], "[deleted]");
		assert_eq!(replies[0]["data"]["replies"]["data"]["children"][0]["data"]["id"], "c");
		assert_eq!(replies[1]["kind"], "more");
		assert_eq!(replies[1]["data"]["parent_id"], "t1_a");
		assert_eq!(replies[1]["data"]["count"], 5);

		assert_eq!(comments[1]["data"]["replies"], "");
		assert_eq!(comments[2]["kind"], "more");
	}
}
