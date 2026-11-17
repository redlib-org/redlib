// Fallback for JSON API requests Reddit blocks for logged-out clients: the same
// data from Reddit's GraphQL API (gql-fed.reddit.com), which the Android app
// uses and which answers the app-only OAuth token.
//
// Each response is converted into the shape of the JSON API response it
// replaces, so the parsers and templates work unchanged.

use super::{json as reddit_json, Backend, Failure, Fetch, Kind};
use crate::client::{CLIENT, OAUTH_CLIENT};
use crate::oauth::force_refresh_token;
use crate::utils::{enable_gql, format_url, gql_feed_requests, param};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::DateTime;
use log::{debug, warn};
use regex::{Captures, Regex};
use serde_json::{json as json_value, Value};
use std::collections::HashMap;
use std::sync::LazyLock;

pub(crate) const GQL_URL: &str = "https://gql-fed.reddit.com/";

// Persisted query ID from the Android app; Reddit rejects raw query text.
// Source: github.com/Amqx/reddit-gql `src/operations.ts`, or the APK's Apollo classes.
const POST_COMMENTS_HASH: &str = "314f29c2494fd78c7844ef2c6b09acb4bef5b37ab50b4ddc01691d6d5c8433f8";
// Reddit returns at most 200 comments per request, filled breadth-first
const COMMENT_COUNT: u32 = 200;
const MAX_DEPTH: u32 = 10;
/// `more_query` of the "more comments" links built here; their `more_cursor` is a GraphQL cursor.
pub const MORE_QUERY: &str = "gql";

pub struct Gql;

impl Backend for Gql {
	fn name(&self) -> &'static str {
		"gql"
	}

	fn available(&self) -> bool {
		enable_gql()
	}

	fn supports(&self, kind: Kind) -> bool {
		kind != Kind::Duplicates
	}

	fn fetch<'a>(&'a self, path: &'a str, quarantine: bool) -> Fetch<'a> {
		Box::pin(async move {
			match json(path, quarantine).await {
				None => Err(Failure::Unsupported),
				// Quarantined subreddits stay unavailable here even after opting
				// in, so let the next backend try.
				Some(Err(reason)) if quarantine && (reason == "quarantined" || reason == "gated") => Err(Failure::Unsupported),
				Some(Err(reason)) if UNAVAILABLE_REASONS.contains(&reason.as_str()) => Err(Failure::Final(reason)),
				Some(result) => result.map_err(Failure::Invalid),
			}
		})
	}
}

/// The JSON API response for `path`, rebuilt from GraphQL. `None` if no query covers `path`.
pub async fn json(path: &str, quarantine: bool) -> Option<Result<Value, String>> {
	let route = path.split('?').next().unwrap_or_default();
	if route.contains("/comments/") {
		return Some(thread(path, quarantine).await);
	}
	let segments: Vec<&str> = route.trim_end_matches(".json").split('/').filter(|s| !s.is_empty()).collect();
	Some(match segments.as_slice() {
		["by_id", ids] => posts_by_ids(ids.split(',').collect()).await,
		["r", sub, "about"] => about(sub, quarantine).await,
		["r", sub, "wiki", page @ ..] => wiki(sub, &page.join("/")).await,
		["search"] => search(path, None, search_pane(path)).await,
		["r", sub, "search"] => search(path, Some(sub), search_pane(path)).await,
		["subreddits", "search"] => search(path, None, "communities").await,
		["users", "search"] => search(path, None, "people").await,
		["user", name, "about"] => user_about(name).await,
		["user", name, "submitted"] => user_posts(path, name, param(path, "after").as_deref()).await,
		["user", name, "comments"] => user_comments(path, name, param(path, "after").as_deref()).await,
		["user", name] | ["user", name, "overview"] => user_overview(path, name).await,
		["r", "all", rest @ ..] => all(path, sort(rest)?).await,
		["r", "popular", rest @ ..] => feed("PopularFeedElements", POPULAR_FEED_HASH, feed_variables(path, sort(rest)?), "/popular/elements").await,
		["r", sub, rest @ ..] if !sub.contains('+') && !sub.contains("%2B") => {
			let mut variables = feed_variables(path, sort(rest)?);
			variables["subredditName"] = (*sub).into();
			variables["includeSubredditInPosts"] = true.into();
			feed("SubredditFeedElements", SUBREDDIT_FEED_HASH, variables, "/postFeed/elements").await
		}
		_ => return None,
	})
}

/// Builds a `/comments/` JSON response for `path` (a `/comments/` API path, as
/// built by `post::item`). Supports the `sort` and `more_cursor` query
/// parameters and single comment threads.
async fn thread(path: &str, quarantine: bool) -> Result<Value, String> {
	let (id, comment_id) = parse_path(path).ok_or_else(|| format!("Couldn't parse post path | {path}"))?;

	let (post, comments) = futures_lite::future::zip(reddit_json(format!("/by_id/t3_{id}.json?raw_json=1"), quarantine), comments(&id, comment_id, path)).await;
	let (comments, info) = comments?;
	let mut post = post?;
	// PostsByIds drops posts their author deleted; PostComments still has the title
	if post["data"]["children"].as_array().is_some_and(Vec::is_empty) {
		if let Some(deleted) = deleted_post(&info, path) {
			post = listing(vec![deleted]);
		}
	}
	Ok(json_value!([post, listing(comments)]))
}

/// A JSON API post for a `DeletedSubredditPost`, which has no author, body, score or date.
fn deleted_post(info: &Value, path: &str) -> Option<Value> {
	if info["__typename"] != "DeletedSubredditPost" {
		return None;
	}
	let fullname = info["id"].as_str()?;
	let id = fullname.strip_prefix("t3_").unwrap_or(fullname);
	let sub = path.strip_prefix("/r/").and_then(|p| p.split('/').next()).unwrap_or_default();
	Some(json_value!({ "kind": "t3", "data": {
		"id": id,
		"name": fullname,
		"title": info["title"],
		"author": "[deleted]",
		"subreddit": sub,
		"permalink": format!("/r/{sub}/comments/{id}/"),
		"is_self": true,
		"selftext": "[deleted]",
		"selftext_html": "<div class=\"md\"><p>[deleted]</p></div>",
		"num_comments": info["commentCount"],
		"locked": info["isLocked"] == true,
		"over_18": info["isNsfw"] == true,
	}}))
}

fn parse_path(path: &str) -> Option<(String, Option<String>)> {
	let path = path.split('?').next()?.trim_end_matches(".json");
	let mut segments = path.split('/').skip_while(|s| *s != "comments").skip(1);
	let id = segments.next().filter(|s| !s.is_empty())?.to_string();
	let comment_id = segments.nth(1).filter(|s| !s.is_empty()).map(str::to_string);
	Some((id, comment_id))
}

fn listing(children: Vec<Value>) -> Value {
	json_value!({ "kind": "Listing", "data": { "children": children } })
}

/// Escaped URL attribute value, proxied through Redlib for Reddit media. Empty for `javascript:` URLs.
fn url_attr(value: Option<&str>) -> String {
	let value = value.unwrap_or_default().trim();
	if value.to_lowercase().starts_with("javascript:") {
		return String::new();
	}
	htmlescape::encode_minimal(&format_url(value))
}

/// The comments of a post, and the post as PostComments sees it (without its comments).
async fn comments(id: &str, comment_id: Option<String>, path: &str) -> Result<(Vec<Value>, Value), String> {
	let sort = param(path, "sort").filter(|s| !s.is_empty()).unwrap_or_else(|| "confidence".to_string());
	let mut variables = json_value!({
		"id": format!("t3_{id}"),
		"sortType": sort.to_uppercase(),
		"count": COMMENT_COUNT,
		"maxDepth": MAX_DEPTH,
		"includeConsistencyToken": false,
		"includePremiumAvatarTreatment": false,
	});
	if let Some(comment_id) = comment_id {
		variables["focusedCommentContext"] = json_value!({ "commentId": format!("t1_{comment_id}"), "isFocusedThreadOnly": true });
	}
	if let Some(cursor) = param(path, "more_cursor").filter(|_| param(path, "more_query").as_deref() == Some(MORE_QUERY)) {
		variables["after"] = cursor.into();
	}

	let mut response = query("PostComments", POST_COMMENTS_HASH, &variables).await?;
	let comments = comment_tree(&response, id)?;
	let mut info = response["data"]["postInfoById"].take();
	if let Some(info) = info.as_object_mut() {
		info.remove("commentForest");
	}
	Ok((comments, info))
}

fn comment_tree(response: &Value, post_id: &str) -> Result<Vec<Value>, String> {
	let trees = response["data"]["postInfoById"]["commentForest"]["trees"]
		.as_array()
		.ok_or_else(|| format!("No comments in response: {}", response["errors"]))?;
	Ok(nest(items(trees, post_id)))
}

fn items(trees: &[Value], post_id: &str) -> Vec<(u64, Value)> {
	trees
		.iter()
		.enumerate()
		.filter_map(|(i, tree)| {
			if tree["node"]["__typename"] != "DeletedComment" {
				return item(tree, post_id);
			}
			// Deleted comments have no ID, but their first reply names them as its parent
			let mut tree = tree.clone();
			if let Some(reply) = trees.get(i + 1).filter(|next| next["depth"].as_u64() == tree["depth"].as_u64().map(|depth| depth + 1)) {
				tree["node"]["id"] = reply["parentId"].clone();
			}
			item(&tree, post_id)
		})
		.collect()
}

/// Of 600 per token; JSON refreshes at 10 of 100.
const GQL_LOW_RATE_LIMIT: f32 = 60.0;

/// Runs a persisted query. Fields that time out upstream come back null with an
/// error next to the data; one retry usually fills them in.
async fn query(operation: &str, hash: &str, variables: &Value) -> Result<Value, String> {
	let body = json_value!({
		"operationName": operation,
		"variables": variables,
		"extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
	});

	let mut response = Value::Null;
	for _ in 0..2 {
		let mut builder = CLIENT
			.load_full()
			.post(GQL_URL)
			.header("X-APOLLO-OPERATION-NAME", operation)
			.header("X-APOLLO-OPERATION-ID", hash)
			.header("Accept", "multipart/mixed;deferSpec=20220824, application/graphql-response+json, application/json");
		for (key, value) in &OAUTH_CLIENT.load_full().headers_map {
			// `json()` sets its own; a duplicate Content-Type fails every query
			if !key.eq_ignore_ascii_case("content-type") {
				builder = builder.header(key, value);
			}
		}
		let res = builder.json(&body).send().await.map_err(|e| e.to_string())?;
		let status = res.status();
		let remaining = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()?.parse::<f32>().ok());
		if remaining.is_some_and(|left| left < GQL_LOW_RATE_LIMIT) {
			warn!("gql rate limit {remaining:?} is low, refreshing the token");
			tokio::spawn(force_refresh_token());
		}
		response = res.json().await.map_err(|e| format!("{operation} returned {status}: {e}"))?;

		if response["data"].is_null() {
			return Err(format!("{operation} returned {status}: {}", response["errors"]));
		}
		if response["errors"].is_null() {
			break;
		}
		debug!("{operation} returned partial data, retrying: {}", response["errors"]);
	}
	Ok(response)
}

/// Converts one entry of the comment forest to an item of the JSON API, with its depth.
fn item(tree: &Value, post_id: &str) -> Option<(u64, Value)> {
	let depth = tree["depth"].as_u64()?;
	let post_fullname = format!("t3_{post_id}");
	let parent = tree["parentId"].as_str().unwrap_or(&post_fullname).to_string();

	let more = &tree["more"];
	if !more.is_null() {
		return Some((
			depth,
			json_value!({ "kind": "more", "data": {
				"id": "",
				"count": more["count"].as_i64().unwrap_or_default(),
				"parent_id": parent,
				"cursor": more["cursor"],
				"more_query": MORE_QUERY,
			} }),
		));
	}

	let node = &tree["node"];
	let fullname = node["id"].as_str().unwrap_or_default();
	let author = author_name(&node["authorInfo"]).unwrap_or("[unknown]");
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
		(
			author,
			content["markdown"].as_str().unwrap_or_default(),
			render(&richtext["document"], &content["richtextMedia"]),
		)
	};

	let flair = &node["authorFlair"];
	let (flair_type, flair_richtext) = flair_parts(flair);

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
				"all_awardings": awards(node),
				"parent_id": parent,
				"replies": "",
			}
		}),
	))
}

/// The JSON API's `*_flair_type` and `*_flair_richtext` of a flair.
fn flair_parts(flair: &Value) -> (&'static str, Value) {
	let richtext: Value = flair["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
	let kind = if richtext.as_array().is_some_and(|r| !r.is_empty()) {
		"richtext"
	} else if flair["text"].as_str().is_some_and(|t| !t.is_empty()) {
		"text"
	} else {
		""
	};
	(kind, richtext)
}

fn awards(node: &Value) -> Vec<Value> {
	node["awardings"]
		.as_array()
		.into_iter()
		.flatten()
		.map(|a| json_value!({ "name": a["award"]["name"], "count": a["total"], "resized_icons": [{ "url": a["award"]["static_icon_16"]["url"] }] }))
		.collect()
}

const POSTS_BY_IDS_HASH: &str = "0e471495bcba33554b6c540b3cf1cae23dd5432a8b741dddc7fd6717dc2520a7";
const SUBREDDIT_FEED_HASH: &str = "97046e2a48050f10a507db354a8da60955ffccb586dcf63163bdfc6610d5cf7a";
const POPULAR_FEED_HASH: &str = "728c0f9a4d12c17b8cffe07669bb1a8770b0f2ec0c84e040485f8fad2d9c417f";
const ALL_FEED_HASH: &str = "0e9c0045dd244e934efd0ea4fefc00cb0b4d91523e2f09c8af075856cb59b937";
const SORTS: [&str; 6] = ["hot", "new", "top", "rising", "controversial", "best"];

/// The listing sort of the path segments after `/r/<sub>`, if they are one.
fn sort<'a>(rest: &[&'a str]) -> Option<&'a str> {
	match rest {
		[] => Some("hot"),
		[sort] if SORTS.contains(sort) => Some(sort),
		_ => None,
	}
}

/// Variables of a post feed query for the listing `path`.
fn feed_variables(path: &str, sort: &str) -> Value {
	let sort = sort.to_uppercase();
	let mut variables = json_value!({
		"sort": sort,
		"pageSize": param(path, "limit").and_then(|l| l.parse::<u32>().ok()).unwrap_or(25),
	});
	if let Some(range) = param(path, "t").filter(|t| !t.is_empty() && (sort == "TOP" || sort == "CONTROVERSIAL")) {
		variables["range"] = range.to_uppercase().into();
	}
	// Feed cursors are the base64 of the JSON API's `after`
	if let Some(after) = param(path, "after").filter(|a| !a.is_empty()) {
		variables["after"] = STANDARD.encode(after).into();
	}
	variables
}

/// A JSON API listing of the posts in a GraphQL connection (`edges` + `pageInfo`).
fn post_listing(connection: &Value) -> Value {
	let children: Vec<Value> = connection["edges"].as_array().into_iter().flatten().filter_map(|edge| post(&edge["node"])).collect();
	json_value!({ "kind": "Listing", "data": { "after": after(connection), "children": children } })
}

/// The JSON API's `after` of the next page of a GraphQL connection.
fn after(connection: &Value) -> Option<String> {
	if connection["pageInfo"]["hasNextPage"] == false {
		return None;
	}
	let cursor = STANDARD.decode(connection["pageInfo"]["endCursor"].as_str()?).ok()?;
	String::from_utf8(cursor).ok()
}

async fn feed(operation: &str, hash: &str, variables: Value, connection: &str) -> Result<Value, String> {
	let response = query(operation, hash, &variables).await?;
	let connection = response["data"].pointer(connection).ok_or_else(|| format!("{operation} returned no posts"))?;
	Ok(post_listing(connection))
}

async fn posts_by_ids(ids: Vec<&str>) -> Result<Value, String> {
	let variables = json_value!({ "ids": ids, "includePostContentPostHint": true, "includePostContentThumbnailEnabled": true });
	let response = query("PostsByIds", POSTS_BY_IDS_HASH, &variables).await?;
	let posts = response["data"]["postsInfoByIds"].as_array().ok_or("PostsByIds returned no posts")?;
	Ok(listing(posts.iter().filter_map(post).collect()))
}

/// r/all only returns the IDs of its posts.
async fn all(path: &str, sort: &str) -> Result<Value, String> {
	let mut variables = feed_variables(path, sort);
	variables["time"] = variables["range"].take();
	let limit = variables["pageSize"].take().as_u64().unwrap_or(25) as usize;
	if let Some(variables) = variables.as_object_mut() {
		variables.remove("range");
		variables.remove("pageSize");
	}
	let mut ids = Vec::new();
	let mut next = None;
	// The first page has ~8 posts whatever the size; later ones fill it
	for _ in 0..gql_feed_requests() {
		let response = query("AllFeedSdui", ALL_FEED_HASH, &variables).await?;
		next = take_all_page(&mut ids, &response["data"]["allV3"]["elements"], limit);
		match &next {
			Some(after) if ids.len() < limit => variables["after"] = STANDARD.encode(after).into(),
			_ => break,
		}
	}
	let mut posts = Box::pin(posts_by_ids(ids.iter().map(String::as_str).collect())).await?;
	posts["data"]["after"] = next.into();
	Ok(posts)
}

/// Adds the post IDs of an r/all page to `ids`, up to `limit`. Returns the
/// JSON API `after` of the last post taken, `None` at the end of the feed.
fn take_all_page(ids: &mut Vec<String>, elements: &Value, limit: usize) -> Option<String> {
	let page = elements["edges"]
		.as_array()
		.into_iter()
		.flatten()
		// Ads are posts too, marked by their `adPayload`
		.filter(|edge| edge["node"]["adPayload"].is_null())
		.filter_map(|edge| edge["node"]["groupId"].as_str())
		.filter(|id| id.starts_with("t3_"));
	for id in page {
		if ids.len() == limit {
			// Feed cursors are post fullnames, so the next page starts here
			return ids.last().cloned();
		}
		if !ids.iter().any(|i| i == id) {
			ids.push(id.to_string());
		}
	}
	after(elements)
}

/// Some feeds, like popular, only send `displayName`.
fn author_name(info: &Value) -> Option<&str> {
	info["name"].as_str().or_else(|| info["displayName"].as_str())
}

/// Converts a GraphQL post into a post of the JSON API.
fn post(node: &Value) -> Option<Value> {
	let node = if node["postInfo"].is_object() { &node["postInfo"] } else { node };
	if !matches!(node["__typename"].as_str(), Some("SubredditPost" | "ProfilePost")) {
		return None;
	}
	let fullname = node["id"].as_str()?;
	let content = &node["content"];
	let richtext: Value = content["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
	let (flair_type, flair_richtext) = flair_parts(&node["flair"]);
	let (author_flair_type, author_flair_richtext) = flair_parts(&node["authorFlair"]);
	let thumbnail = if node["isThumbnailEnabled"] == false { &Value::Null } else { &node["thumbnail"] };
	// PostsByIds calls it `postTitle`, the feeds `title`.
	let title = if node["title"].is_string() { &node["title"] } else { &node["postTitle"] };

	let mut data = json_value!({
		"id": fullname.strip_prefix("t3_").unwrap_or(fullname),
		"name": fullname,
		"title": title,
		"author": author_name(&node["authorInfo"]).unwrap_or("[deleted]"),
		"author_flair_type": author_flair_type,
		"author_flair_richtext": author_flair_richtext,
		"author_flair_text": node["authorFlair"]["text"],
		"author_flair_background_color": node["authorFlair"]["template"]["backgroundColor"],
		"author_flair_text_color": node["authorFlair"]["textColor"].as_str().map(str::to_lowercase),
		// Posts on a user's profile (`ProfilePost`) have a profile instead of a subreddit
		"subreddit": node["subreddit"]["name"].as_str().or(node["profile"]["name"].as_str()),
		"subreddit_name_prefixed": node["subreddit"]["prefixedName"].as_str().or(node["profile"]["redditorInfo"]["prefixedName"].as_str()),
		"permalink": node["permalink"],
		"url": node["url"],
		"domain": node["domain"],
		"is_self": node["isSelfPost"] == true,
		"selftext": content["markdown"].as_str().unwrap_or_default(),
		"selftext_html": if content.is_object() { format!("<div class=\"md\">{}</div>", render(&richtext["document"], &content["richtextMedia"])) } else { String::new() },
		"score": node["score"],
		"upvote_ratio": node["upvoteRatio"],
		"num_comments": node["commentCount"],
		"created_utc": timestamp(&node["createdAt"]).unwrap_or_default(),
		"edited": timestamp(&node["editedAt"]).map_or(Value::Bool(false), Value::from),
		"over_18": node["isNsfw"] == true,
		"spoiler": node["isSpoiler"] == true,
		"stickied": node["isStickied"] == true,
		"locked": node["isLocked"] == true,
		"archived": node["isArchived"] == true,
		"hide_score": node["isScoreHidden"] == true,
		"distinguished": node["distinguishedAs"].as_str().map(str::to_lowercase),
		"removed_by_category": node["removedByCategory"].as_str().map(str::to_lowercase),
		"link_flair_type": flair_type,
		"link_flair_richtext": flair_richtext,
		"link_flair_text": node["flair"]["text"],
		"link_flair_background_color": node["flair"]["template"]["backgroundColor"],
		"link_flair_text_color": node["flair"]["textColor"].as_str().map(str::to_lowercase),
		"thumbnail": thumbnail["url"].as_str().unwrap_or_default(),
		"thumbnail_width": thumbnail["dimensions"]["width"],
		"thumbnail_height": thumbnail["dimensions"]["height"],
		// Search results only name the type of their media
		"post_hint": node["postHint"].as_str().or(node["media"]["typeHint"].as_str().filter(|hint| *hint == "IMAGE")).map(|hint| hint.to_lowercase().replace('_', ":")),
		"all_awardings": awards(node),
		// Only ads have a call to action ("Learn More") or a disclaimer
		"is_created_from_ads_ui": node["callToAction"].is_string() || node["adSupplementaryTextRichtext"].is_string(),
	});
	if node["isSelfPost"] != true {
		data["url_overridden_by_dest"] = node["url"].clone();
	}

	let still = &node["media"]["still"]["source"];
	if still.is_object() {
		data["preview"] = json_value!({ "images": [{ "source": { "url": still["url"], "width": still["dimensions"]["width"], "height": still["dimensions"]["height"] } }] });
	}

	let streaming = &node["media"]["streaming"];
	if streaming.is_object() {
		// Reddit's copies of external videos (Streamable, ...) give an MP4 as
		// `hlsUrl` too, which must not be played as HLS.
		let hls_url = streaming["hlsUrl"].as_str().filter(|url| url.contains(".m3u8"));
		data["secure_media"] = json_value!({ "reddit_video": { "fallback_url": video_fallback_url(node, streaming), "hls_url": hls_url, "is_gif": streaming["isGif"] == true } });
	}

	if let Some(items) = node["gallery"]["items"].as_array() {
		data["is_gallery"] = true.into();
		data["gallery_data"] = json_value!({ "items": items.iter().map(|item| json_value!({ "media_id": item["media"]["id"], "caption": item["caption"], "outbound_url": item["outboundUrl"] })).collect::<Vec<_>>() });
		data["media_metadata"] = items
			.iter()
			.map(|item| {
				let m = &item["media"];
				let gif = if m["mimetype"] == "image/gif" { m["url"].clone() } else { Value::Null };
				(
					m["id"].as_str().unwrap_or_default().to_string(),
					json_value!({ "m": m["mimetype"], "s": { "u": m["url"], "gif": gif, "x": m["width"], "y": m["height"] } }),
				)
			})
			.collect::<serde_json::Map<_, _>>()
			.into();
	}

	if let Some(parent) = post(&node["crosspostRoot"]["post"]) {
		data["crosspost_parent_list"] = json_value!([parent["data"]]);
	}

	Some(json_value!({ "kind": "t3", "data": data }))
}

pub(crate) const SUBREDDIT_INFO_HASH: &str = "6b9c1679e69097e1c6df364adc11183afe6e2b6545dd1c0c1cc8f7490448c3e5";

/// Why Reddit won't show a subreddit, as the JSON API's `reason` names it.
const UNAVAILABLE_REASONS: [&str; 4] = ["private", "banned", "quarantined", "gated"];

async fn about(name: &str, quarantine: bool) -> Result<Value, String> {
	let variables = json_value!({ "subredditName": name, "filterGated": !quarantine, "includeDevvitData": false, "includeRecapFields": false, "includeWelcomePage": false });
	let response = query("SubredditInfoByName", SUBREDDIT_INFO_HASH, &variables).await?;
	about_json(&response["data"]["subredditInfoByName"], name)
}

/// The `about.json` of a `SubredditInfoByName` node, or why Reddit won't show
/// the subreddit (`private`, `banned`, `quarantined` or `gated`).
fn about_json(node: &Value, name: &str) -> Result<Value, String> {
	if node["__typename"] == "UnavailableSubreddit" {
		return Err(match node["forbiddenReason"].as_str() {
			Some(reason) => reason.to_lowercase(),
			None => format!("r/{name} is unavailable"),
		});
	}
	subreddit(node).ok_or_else(|| format!("r/{name} not found"))
}

/// Converts a GraphQL subreddit into a subreddit of the JSON API (`about.json`).
fn subreddit(node: &Value) -> Option<Value> {
	let name = node["name"].as_str()?;
	let description = &node["description"];
	let richtext: Value = description["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
	Some(json_value!({ "kind": "t5", "data": {
		"name": node["id"],
		"display_name": name,
		"display_name_prefixed": node["prefixedName"],
		"url": node["path"].as_str().map_or_else(|| format!("/r/{name}/"), str::to_string),
		"title": node["title"],
		"public_description": node["publicDescriptionText"],
		"description": description["markdown"],
		"description_html": Some(render(&richtext["document"], &Value::Null)).filter(|html| !html.is_empty()).map(|html| format!("<div class=\"md\">{html}</div>")),
		"community_icon": node["styles"]["icon"],
		"icon_img": node["styles"]["legacyIcon"]["url"],
		"subscribers": node["subscribersCount"],
		"accounts_active": node["activeCount"],
		"weekly_active_users": node["communityStats"]["weeklyActiveUsersCount"],
		"weekly_contributions": node["communityStats"]["weeklyContributionsCount"],
		"wiki_enabled": node["wikiEditMode"].as_str().is_some_and(|mode| mode != "DISABLED"),
		"over18": node["isNsfw"] == true,
	} }))
}

const SUBREDDIT_WIKI_HASH: &str = "32f9314683f033a76812f772bbba771958760286c5cd28c558071a3ff4eb231f";

async fn wiki(sub: &str, page: &str) -> Result<Value, String> {
	let variables = json_value!({
		"subredditName": sub,
		"pageName": page,
		"includePage": true,
		"includePageContent": true,
		"includeMediaFragment": true,
		"includeIsRevisable": false,
		"includeRevisionsCount": false,
		"includePageTitle": false,
	});
	let response = query("SubredditWiki", SUBREDDIT_WIKI_HASH, &variables).await?;
	wiki_json(&response["data"]["subredditInfoByName"], sub)
}

/// The JSON API's `wiki/<page>.json`; a missing page or disabled wiki is only a `reason`, as there.
fn wiki_json(node: &Value, sub: &str) -> Result<Value, String> {
	if node["__typename"] != "Subreddit" {
		return Err(format!("r/{sub} is unavailable"));
	}
	let page = &node["wiki"]["page"];
	if page["status"] != "VALID" {
		return Ok(json_value!({ "reason": page["status"] }));
	}
	let content = &page["content"];
	let richtext: Value = content["richtext"].as_str().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
	Ok(json_value!({ "kind": "wikipage", "data": {
		"content_html": format!("<div class=\"md wiki\">{}</div>", wiki_toc(&render(&richtext["document"], &content["richtextMedia"]))),
	} }))
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<h([1-6])>(.*?)</h[1-6]>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());

/// Gives the headings the ids Reddit's wiki gives them and puts its table of
/// contents in front, nested like Reddit nests it.
fn wiki_toc(html: &str) -> String {
	let mut seen: HashMap<String, usize> = HashMap::new();
	// The open lists of the table of contents, with the heading level each started at
	let mut lists: Vec<(usize, String)> = vec![(0, String::new())];
	let mut previous = 0;
	let body = HEADING.replace_all(html, |caps: &Captures| {
		let text = htmlescape::decode_html(&TAG.replace_all(&caps[2], "")).unwrap_or_default();
		if text.is_empty() {
			return caps[0].to_string();
		}
		let mut id = "wiki_".to_string();
		for byte in text.replace(' ', "_").to_lowercase().bytes() {
			match byte {
				b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' => id.push(byte as char),
				_ => id += &format!(".{byte:02X}"),
			}
		}
		let count = seen.entry(id.clone()).or_default();
		*count += 1;
		if *count > 1 {
			id += &count.to_string();
		}

		let level = caps[1].parse().unwrap_or(1);
		if previous != 0 && level > previous {
			lists.push((level, String::new()));
		} else {
			while lists.len() > 1 && lists.last().is_some_and(|(l, _)| *l > level) {
				close_toc_list(&mut lists);
			}
		}
		previous = level;
		if let Some((_, list)) = lists.last_mut() {
			*list += &format!("<li class=\"{id}\"><a href=\"#{id}\">{}</a></li>", htmlescape::encode_minimal(&text));
		}
		format!("<h{level} id=\"{id}\">{}</h{level}>", &caps[2])
	});
	if previous == 0 {
		return html.to_string();
	}
	while lists.len() > 1 {
		close_toc_list(&mut lists);
	}
	format!("<div class=\"toc\"><ul>{}</ul></div>{body}", lists[0].1)
}

fn close_toc_list(lists: &mut Vec<(usize, String)>) {
	if let Some((_, list)) = lists.pop() {
		if let Some((_, parent)) = lists.last_mut() {
			*parent += &format!("<li class=\"toc_child\"><ul>{list}</ul></li>");
		}
	}
}

const DYNAMIC_SEARCH_HASH: &str = "4acac653e5aac2ae2f08114d07ba7582e852464a57e324547c259ea3ef2ebd40";

/// The tab of the app's search a post search `path` asks for: its `type` is
/// `comment`, `media` or a post search.
fn search_pane(path: &str) -> &'static str {
	match param(path, "type").as_deref() {
		Some("comment") => "comments",
		Some("media") => "media",
		_ => "posts",
	}
}

/// Variables of the app's search (`DynamicSearch`) for the search `path`.
/// Sort, time and post types go in `filters` like the app's filter chips put
/// them; the page cursor is the JSON API's `after`.
fn search_variables(path: &str, text: String, pane: &str) -> Value {
	let nsfw = param(path, "include_over_18").is_some();
	let mut filters = vec![("safe_search", if nsfw { "off" } else { "on" }.to_string())];
	let sort = param(path, "sort").filter(|s| !s.is_empty());
	if let Some(sort) = &sort {
		filters.push(("sort", sort.to_uppercase()));
	}
	// The app clears the time range for these sorts
	if let Some(range) = param(path, "t").filter(|t| !t.is_empty() && !matches!(sort.as_deref(), Some("hot" | "new"))) {
		filters.push(("time_range", range));
	}
	if let Some(types) = param(path, "post_types").filter(|t| !t.is_empty() && t != "all") {
		filters.push(("post_types", types));
	}
	let options = [
		("page_type", "search_results".to_string()),
		("pane", pane.to_string()),
		("pref_allow_nsfw", if nsfw { "1" } else { "0" }.to_string()),
		("is_eligible_for_kwf", "0".to_string()),
		("after", param(path, "after").unwrap_or_default()),
		("treatment", "default".to_string()),
	];
	let inputs = |pairs: &[(&str, String)]| pairs.iter().map(|(key, value)| json_value!({ "key": key, "value": value })).collect::<Vec<_>>();
	// One ID per query text, so its pages share one like the app's do
	let mut hasher = std::hash::DefaultHasher::new();
	std::hash::Hash::hash(&text, &mut hasher);
	let hash = std::hash::Hasher::finish(&hasher);
	let query_id = uuid::Builder::from_random_bytes((u128::from(hash) << 64 | u128::from(hash.rotate_left(32))).to_le_bytes()).into_uuid();
	json_value!({
		"query": text,
		"queryId": query_id.to_string(),
		"filters": inputs(&filters),
		"options": inputs(&options),
		"includeSubredditUnit": true,
	})
}

/// One tab (`pane`) of the app's search as a JSON API listing: posts, comments,
/// subreddits or users.
async fn search(path: &str, sub: Option<&str>, pane: &str) -> Result<Value, String> {
	let mut text = param(path, "q").unwrap_or_default();
	if let Some(sub) = sub.filter(|_| param(path, "restrict_sr").is_some_and(|r| !r.is_empty())) {
		text = format!("{text} subreddit:{sub}");
	}
	// The media tab always searches all media types; a single type is a post search
	let pane = if pane == "media" && param(path, "post_types").is_some_and(|t| !t.is_empty() && t != "all") {
		"posts"
	} else {
		pane
	};
	let variables = search_variables(path, text, pane);
	let response = query("DynamicSearch", DYNAMIC_SEARCH_HASH, &variables).await?;
	let mut listing = search_listing(&response).ok_or_else(|| format!("DynamicSearch returned no results: {}", response["errors"]))?;
	if let Some(children) = listing["data"]["children"].as_array_mut() {
		if param(path, "include_over_18").is_none() {
			children.retain(|child| child["data"]["over_18"] != true);
		}
		if let Some(limit) = param(path, "limit").and_then(|l| l.parse().ok()) {
			children.truncate(limit);
		}
	}
	Ok(listing)
}

/// The results of a `DynamicSearch` response as a JSON API listing.
fn search_listing(response: &Value) -> Option<Value> {
	let main = response["data"]["search"]["dynamic"]["components"]["main"].as_object()?;
	let children: Vec<Value> = main["edges"]
		.as_array()
		.into_iter()
		.flatten()
		.flat_map(|edge| edge["node"]["children"].as_array().into_iter().flatten())
		.filter_map(search_result)
		.collect();
	let page = &main["pageInfo"];
	let after = if page["hasNextPage"] == true { page["endCursor"].clone() } else { Value::Null };
	Some(json_value!({ "kind": "Listing", "data": { "after": after, "children": children } }))
}

/// Converts one search result into an item of the JSON API. Ads
/// (`SearchAdPost`) and anything else that isn't a result give `None`.
fn search_result(result: &Value) -> Option<Value> {
	match result["__typename"].as_str()? {
		"SearchPost" | "SearchMediaPost" => post(&result["post"]),
		"SearchComment" => search_comment(&result["comment"]),
		"SearchCommunity" => subreddit(&result["community"]),
		"SearchAuthor" => search_user(&result["author"]),
		_ => None,
	}
}

/// A comment search result as a comment of the JSON API, with the post it is
/// on like on user pages.
fn search_comment(comment: &Value) -> Option<Value> {
	let post = &comment["postInfo"];
	let (_, mut item) = item(&json_value!({ "depth": 0, "parentId": post["id"], "node": comment }), "")?;
	let subreddit = post["subreddit"]["name"].as_str().unwrap_or_default();
	let post_id = post["id"].as_str()?.trim_start_matches("t3_");
	let id = item["data"]["id"].as_str()?.to_string();
	item["data"]["link_title"] = post["title"].clone();
	item["data"]["link_id"] = post["id"].clone();
	item["data"]["permalink"] = format!("/r/{subreddit}/comments/{post_id}/comment/{id}/").into();
	item["data"]["subreddit"] = subreddit.into();
	item["data"]["over_18"] = (post["isNsfw"] == true).into();
	Some(item)
}

/// A people search result as a user of the JSON API (`/users/search`).
fn search_user(author: &Value) -> Option<Value> {
	let profile = &author["profileInfo"];
	Some(json_value!({ "kind": "t2", "data": {
		"name": author["name"].as_str()?,
		"created_utc": timestamp(&profile["createdAt"]),
		"total_karma": author["karma"]["total"],
		"icon_img": author["icon"]["url"],
		"subreddit": { "over_18": profile["isNsfw"] == true },
	} }))
}

const PROFILE_HASH: &str = "4822704f40348a38e37626b62f7f2a4e4af88aa5ab90f271a0c539db7ab2ac2f";
const USER_POSTS_HASH: &str = "78a213c02a340db4a707d4bcd375ce34f733b75893379603acb4ec20388fafe6";
const USER_COMMENTS_HASH: &str = "e1b37f521fb40af2120946b70e7fbf6a3d924db2abaf28ff2164d4cfacca28db";
const COMMENTS_BY_IDS_HASH: &str = "51ab9f2e0eb87b6dac9aac02012ba7046049e516ec55ec442abfed0f19c6f311";

async fn user_about(name: &str) -> Result<Value, String> {
	let response = query("UserProfileDetails", PROFILE_HASH, &json_value!({ "username": name })).await?;
	user(&response["data"]["redditorInfoByName"]).ok_or_else(|| format!("u/{name} not found"))
}

/// Converts a GraphQL redditor into a user of the JSON API (`about.json`).
fn user(redditor: &Value) -> Option<Value> {
	let name = redditor["name"].as_str()?;
	let profile = &redditor["profileInfo"];
	Some(json_value!({ "kind": "t2", "data": {
		"name": name,
		"created": timestamp(&profile["createdAt"]),
		"total_karma": redditor["karma"]["total"].as_f64().map(|karma| karma.round() as i64),
		"subreddit": {
			"title": profile["title"],
			"icon_img": redditor["icon"]["url"],
			"banner_img": profile["styles"]["profileBanner"],
			"public_description": profile["publicDescriptionText"],
			"over_18": profile["isNsfw"] == true,
		},
	} }))
}

fn profile_variables(path: &str, after: Option<&str>) -> Value {
	let mut variables = feed_variables(path, &param(path, "sort").filter(|s| !s.is_empty()).unwrap_or_else(|| "new".to_string()));
	match after.filter(|a| !a.is_empty()) {
		Some(after) => variables["after"] = STANDARD.encode(after).into(),
		None => variables["after"] = Value::Null,
	}
	variables
}

async fn user_posts(path: &str, name: &str, cursor: Option<&str>) -> Result<Value, String> {
	let mut variables = profile_variables(path, cursor);
	variables["username"] = name.into();
	variables["includeSubredditInPosts"] = true.into();
	feed("UserSubmittedPosts", USER_POSTS_HASH, variables, "/postFeed/posts").await
}

/// The comments come without their content; `LiveCommentsByIds` fills it in.
async fn user_comments(path: &str, name: &str, cursor: Option<&str>) -> Result<Value, String> {
	let mut variables = profile_variables(path, cursor);
	variables["name"] = name.into();
	let response = query("SubmittedComments", USER_COMMENTS_HASH, &variables).await?;
	let connection = &response["data"]["redditorInfoByName"]["comments"];
	let ids: Vec<&str> = connection["edges"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|edge| edge["node"]["id"].as_str())
		.collect();
	let full = query(
		"LiveCommentsByIds",
		COMMENTS_BY_IDS_HASH,
		&json_value!({ "ids": ids, "includePremiumAvatarTreatment": false }),
	)
	.await?;
	let children = user_comment_items(connection, &full["data"]["commentsByIds"]);
	Ok(json_value!({ "kind": "Listing", "data": { "after": after(connection), "children": children } }))
}

/// JSON API comments of a user, from their `SubmittedComments` previews and full `LiveCommentsByIds` comments.
fn user_comment_items(previews: &Value, full: &Value) -> Vec<Value> {
	let full = full.as_array().map(Vec::as_slice).unwrap_or_default();
	previews["edges"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|edge| {
			let preview = &edge["node"];
			let comment = full.iter().find(|c| c["id"] == preview["id"])?;
			let (_, mut item) = item(&json_value!({ "depth": 0, "parentId": preview["postInfo"]["id"], "node": comment }), "")?;
			let permalink = comment["permalink"].as_str().unwrap_or_default();
			let subreddit = match permalink.split('/').nth(1) {
				Some("r") => permalink.split('/').nth(2).unwrap_or_default().to_string(),
				_ => format!("u_{}", permalink.split('/').nth(2).unwrap_or_default()),
			};
			item["data"]["link_title"] = preview["postInfo"]["title"].clone();
			item["data"]["link_id"] = preview["postInfo"]["id"].clone();
			item["data"]["permalink"] = permalink.into();
			item["data"]["subreddit"] = subreddit.into();
			Some(item)
		})
		.collect()
}

/// Posts and comments merged, newest first. The two have separate cursors, so
/// its `after` is `<post after>,<comment after>`, with `-` for a side that has no more.
async fn user_overview(path: &str, name: &str) -> Result<Value, String> {
	let after = param(path, "after").unwrap_or_default();
	let (post_after, comment_after) = after.split_once(',').unwrap_or(("", ""));
	let posts = async {
		match post_after {
			"-" => Ok(Value::Null),
			after => user_posts(path, name, Some(after)).await,
		}
	};
	let comments = async {
		match comment_after {
			"-" => Ok(Value::Null),
			after => user_comments(path, name, Some(after)).await,
		}
	};
	let (posts, comments) = futures_lite::future::zip(posts, comments).await;
	Ok(merge_overview(&posts?, &comments?, post_after, comment_after))
}

/// Merges a page of posts and a page of comments. Items older than the last one
/// of a side that has more pages wait for the next page, so the order holds across pages.
fn merge_overview(posts: &Value, comments: &Value, post_after: &str, comment_after: &str) -> Value {
	let created = |item: &Value| item["data"]["created_utc"].as_f64().unwrap_or_default();
	let sides = [(posts, post_after), (comments, comment_after)];
	let cutoff = sides
		.iter()
		.filter(|(listing, _)| listing["data"]["after"].is_string())
		.filter_map(|(listing, _)| listing["data"]["children"].as_array()?.last().map(created))
		.fold(f64::MIN, f64::max);
	let mut children = Vec::new();
	let mut next = Vec::new();
	for (listing, after) in sides {
		let page = listing["data"]["children"].as_array().map(Vec::as_slice).unwrap_or_default();
		let kept: Vec<Value> = page.iter().filter(|item| created(item) >= cutoff).cloned().collect();
		next.push(match kept.last() {
			_ if kept.len() == page.len() && !listing["data"]["after"].is_string() => "-".to_string(),
			Some(last) => last["data"]["name"].as_str().unwrap_or_default().to_string(),
			None => after.to_string(),
		});
		children.extend(kept);
	}
	children.sort_by(|a, b| created(b).total_cmp(&created(a)));
	let after = next.iter().any(|a| *a != "-").then(|| next.join(","));
	json_value!({ "kind": "Listing", "data": { "after": after, "children": children } })
}

/// The MP4 to play without HLS. The muxed MP4 or the download has sound,
/// unlike the JSON API's `fallback_url`, and also exists for external videos.
/// Without them, v.redd.it's video-only rendition that fits the video is used.
fn video_fallback_url(node: &Value, streaming: &Value) -> String {
	let media = &node["media"];
	if let Some(url) = media["packagedMedia"]["muxedMp4s"]["recommended"]["url"].as_str().or(media["download"]["url"].as_str()) {
		return url.to_string();
	}
	let dimensions = &streaming["dimensions"];
	let size = dimensions["width"].as_u64().unwrap_or_default().min(dimensions["height"].as_u64().unwrap_or_default());
	let rendition = [1080, 720, 480, 360, 240].into_iter().find(|r| *r <= size).unwrap_or(240);
	format!("{}/CMAF_{rendition}.mp4?source=fallback", node["url"].as_str().unwrap_or_default().trim_end_matches('/'))
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
		"par" => pipe_table(node, media).unwrap_or_else(|| format!("<p>{}</p>", children())),
		"text" => formatted(node),
		"br" => "<br>".to_string(),
		"hr" => "<hr>".to_string(),
		"h" => {
			let level = node["l"].as_u64().unwrap_or(1).clamp(1, 6);
			// Markdown's closing hashes (`## Title ##`) stay in the richtext
			format!("<h{level}>{}</h{level}>", children().trim_end_matches('#').trim_end())
		}
		"blockquote" => format!("<blockquote>{}</blockquote>", children()),
		"list" => {
			let tag = if node["o"] == true { "ol" } else { "ul" };
			format!("<{tag}>{}</{tag}>", children())
		}
		// Richtext puts every item in a paragraph, Reddit's HTML only items of loose lists
		"li" if node["c"].as_array().is_some_and(|c| c.len() == 1 && c[0]["e"] == "par") => format!("<li>{}</li>", render(&node["c"][0]["c"], media)),
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
		kind @ ("r/" | "u/" | "p/" | "c/") => {
			let name = htmlescape::encode_minimal(node["t"].as_str().unwrap_or_default());
			let slash = if node["l"] == true { "/" } else { "" };
			format!("<a href=\"/{kind}{name}\">{slash}{kind}{name}</a>")
		}
		"img" | "gif" => {
			let Some(item) = media.as_array().and_then(|m| m.iter().find(|m| m["id"] == node["id"])) else {
				return String::new();
			};
			let src = url_attr(item["url"].as_str());
			let caption = node["c"]
				.as_str()
				.map(|c| format!("<figcaption>{}</figcaption>", htmlescape::encode_minimal(c)))
				.unwrap_or_default();
			format!("<figure><a href=\"{src}\"><img src=\"{src}\" loading=\"lazy\"></a>{caption}</figure>")
		}
		"table" => table(node, media),
		// Unknown elements: keep their content
		_ if node["c"].is_array() => children(),
		_ => htmlescape::encode_minimal(node["t"].as_str().unwrap_or_default()),
	}
}

/// RTJSON format bits and their tags
const FORMATS: [(u64, &str); 8] = [
	(1, "strong"),
	(2, "em"),
	(4, "u"),
	(8, "del"),
	(16, "sub"),
	(32, "sup"),
	(64, "code"),
	(128, "span class=\"md-spoiler-text\""),
];

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
		tags.iter().rev().for_each(|tag| out += &format!("</{}>", tag.split(' ').next().unwrap_or(tag)));
		start = end;
	}
	out
}

static TABLE_RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\|?(\s*:?-+:?\s*\|)+(\s*:?-+:?\s*)?$").unwrap());

/// A markdown table richtext left as a paragraph of `|` lines (it does so in
/// quotes), rendered as the table Reddit's HTML has.
fn pipe_table(node: &Value, media: &Value) -> Option<String> {
	let children = node["c"].as_array()?;
	let lines: Vec<&[Value]> = children.split(|c| c["e"] == "br").collect();
	let text = |line: &[Value]| line.iter().map(|c| c["t"].as_str().unwrap_or_default()).collect::<String>().trim().to_string();
	if lines.len() < 2 || !text(lines[0]).contains('|') || !TABLE_RULE.is_match(&text(lines[1])) {
		return None;
	}
	let cells = |line: &[Value]| -> Vec<String> {
		let html = render(&Value::from(line.to_vec()), media);
		let html = html.trim();
		let html = html.strip_prefix('|').unwrap_or(html);
		let html = html.strip_suffix('|').unwrap_or(html);
		html.split('|').map(|cell| cell.trim().to_string()).collect()
	};
	let aligns: Vec<&str> = text(lines[1])
		.trim_matches('|')
		.split('|')
		.map(|rule| match (rule.trim().starts_with(':'), rule.trim().ends_with(':')) {
			(true, true) => " align=\"center\"",
			(false, true) => " align=\"right\"",
			(true, false) => " align=\"left\"",
			_ => "",
		})
		.collect();
	let row = |cells: &[String], tag: &str| -> String {
		let cells: String = cells
			.iter()
			.enumerate()
			.map(|(i, cell)| format!("<{tag}{}>{cell}</{tag}>", aligns.get(i).unwrap_or(&"")))
			.collect();
		format!("<tr>{cells}</tr>")
	};
	let head = cells(lines[0]);
	let mut body = String::new();
	for line in lines[2..].iter().filter(|line| !text(line).is_empty()) {
		let mut cells = cells(line);
		// Rows richtext joined into one line come as `a | b | | c | d`
		while cells.len() > head.len() && cells[head.len()].is_empty() {
			let rest = cells.split_off(head.len() + 1);
			cells.truncate(head.len());
			body += &row(&cells, "td");
			cells = rest;
		}
		body += &row(&cells, "td");
	}
	Some(format!("<table><thead>{}</thead><tbody>{body}</tbody></table>", row(&head, "th")))
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
	#[test]
	fn test_deleted_post() {
		let info = json_value!({ "__typename": "DeletedSubredditPost", "commentCount": 1819, "id": "t3_1xlfmka", "isLocked": false, "isNsfw": false, "title": "What exactly is \"ranch sauce\"?" });
		let post = deleted_post(&info, "/r/IWantToAskAnAmerican/comments/1xlfmka/what_exactly.json?&raw_json=1").unwrap();
		let data = &post["data"];
		assert_eq!(
			(data["id"].as_str(), data["title"].as_str(), data["author"].as_str()),
			(Some("1xlfmka"), Some("What exactly is \"ranch sauce\"?"), Some("[deleted]"))
		);
		assert_eq!(data["permalink"], "/r/IWantToAskAnAmerican/comments/1xlfmka/");
		assert_eq!(data["num_comments"], 1819);
		assert!(deleted_post(&json_value!({ "__typename": "SubredditPost", "id": "t3_x" }), "/r/a/comments/x/").is_none());
	}

	use super::*;

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
				"<ol><li>i</li></ol>",
				r#"<table><thead><tr><th align="right">H</th><th></th></tr></thead><tbody><tr><td align="right">1</td><td></td></tr></tbody></table>"#,
				"kept",
			)
		);
	}

	#[test]
	fn test_author_name() {
		assert_eq!(author_name(&json_value!({ "name": "a", "displayName": "b" })), Some("a"));
		assert_eq!(author_name(&json_value!({ "displayName": "b" })), Some("b"));
		assert_eq!(author_name(&Value::Null), None);
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
		let comments = nest(items(&trees, "p"));
		assert_eq!(comments.len(), 3);

		let a = &comments[0]["data"];
		assert_eq!(a["parent_id"], "t3_p");
		let replies = a["replies"]["data"]["children"].as_array().unwrap();
		assert_eq!(replies.len(), 2);
		assert_eq!(replies[0]["data"]["author"], "[deleted]");
		assert_eq!(replies[0]["data"]["id"], "x");
		assert_eq!(replies[0]["data"]["replies"]["data"]["children"][0]["data"]["id"], "c");
		assert_eq!(replies[1]["kind"], "more");
		assert_eq!(replies[1]["data"]["parent_id"], "t1_a");
		assert_eq!(replies[1]["data"]["count"], 5);
		assert_eq!(replies[1]["data"]["cursor"], "c1");

		assert_eq!(comments[1]["data"]["replies"], "");
		assert_eq!(comments[2]["kind"], "more");
		assert_eq!(comments[2]["data"]["parent_id"], "t3_p");
		assert_eq!(comments[2]["data"]["cursor"], "c2");
		assert_eq!(comments[2]["data"]["more_query"], MORE_QUERY);
	}

	fn request() -> hyper::Request<crate::body::Body> {
		hyper::Request::new(crate::body::Body::from(""))
	}

	#[test]
	fn test_post_comments_fixture() {
		let response: Value = serde_json::from_str(include_str!("../../tests/fixtures/gql/post_comments.json")).unwrap();
		let tree = listing(comment_tree(&response, "1xe5jpg").unwrap());
		let comments = crate::post::parse_comments(&tree, "/r/AskReddit/comments/1xe5jpg/x/", "op", "", &Default::default(), &request());

		assert_eq!(comments.len(), 2);
		assert_eq!(comments[0].author.name, "aubreynewmanoc");
		assert!(comments[0].body.contains("Red Bull Stratos"));
		assert!(!comments[0].replies.is_empty());
		let more = comments.last().unwrap();
		assert_eq!((more.kind.as_str(), more.parent_kind.as_str()), ("more", "t3"));
		assert!(more.more_href.contains("more_query=gql"));
		assert_eq!(more.more_count, 478);
	}

	#[test]
	fn test_overview_pages_keep_posts_and_comments_in_order() {
		let item = |name: &str, created: u32| json_value!({ "data": { "name": name, "created_utc": created } });
		let posts = json_value!({ "data": { "after": "t3_c", "children": [item("t3_a", 90), item("t3_b", 50), item("t3_c", 10)] } });
		let comments = json_value!({ "data": { "after": "t1_z", "children": [item("t1_x", 80), item("t1_y", 40), item("t1_z", 30)] } });

		let page = merge_overview(&posts, &comments, "", "");
		let names: Vec<_> = page["data"]["children"].as_array().unwrap().iter().map(|c| c["data"]["name"].clone()).collect();
		// t3_c is older than the last comment, so it waits for the next page
		assert_eq!(names, ["t3_a", "t1_x", "t3_b", "t1_y", "t1_z"]);
		assert_eq!(page["data"]["after"], "t3_b,t1_z");

		let last_comments = json_value!({ "data": { "after": null, "children": [item("t1_w", 5)] } });
		let page = merge_overview(&json_value!({ "data": { "after": null, "children": [item("t3_c", 10)] } }), &last_comments, "t3_b", "t1_z");
		assert_eq!(page["data"]["children"].as_array().unwrap().len(), 2);
		assert_eq!(page["data"]["after"], Value::Null);

		let page = merge_overview(&Value::Null, &comments, "-", "");
		assert_eq!(page["data"]["children"].as_array().unwrap().len(), 3);
		assert_eq!(page["data"]["after"], "-,t1_z");
	}

	fn fixture(name: &str) -> Value {
		serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/gql/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
	}

	#[tokio::test]
	async fn test_subreddit_feed_fixture() {
		let listing = post_listing(&fixture("subreddit_feed.json")["data"]["postFeed"]["elements"]);
		assert_eq!(listing["data"]["after"], "t3_1xdtpdg");
		let children = listing["data"]["children"].as_array().unwrap();
		// The recommendation unit is not a post
		assert_eq!(children.len(), 4);

		let link = crate::utils::parse_post(&children[0]).await;
		assert_eq!(
			(link.title.as_str(), link.community.as_str(), link.post_type.as_str()),
			("This Week in Rust #674", "rust", "link")
		);
		assert!(link.flags.stickied);
		assert_eq!(link.flair.flair_parts.len(), 1);
		let text = crate::utils::parse_post(&children[2]).await;
		assert_eq!(text.post_type, "self");
		assert!(text.body.starts_with("<div class=\"md\"><p>"));
		let image = crate::utils::parse_post(&children[3]).await;
		assert_eq!((image.post_type.as_str(), image.media.url.as_str()), ("image", "/img/lhwr9uf5bkxh1.png"));
	}

	#[tokio::test]
	async fn test_posts_by_ids_fixture() {
		let posts = fixture("posts_by_ids.json")["data"]["postsInfoByIds"]
			.as_array()
			.unwrap()
			.iter()
			.filter_map(post)
			.collect::<Vec<_>>();
		assert_eq!(posts.len(), 3);

		let video = crate::utils::parse_post(&posts[0]).await;
		assert_eq!(video.post_type, "video");
		assert_eq!(video.title, "Secret knowledge");
		assert_eq!(video.media.url, "/vid/apxzua5q5ixh1/CMAF/720.mp4");
		assert!(video.media.alt_url.starts_with("/hls/apxzua5q5ixh1/HLSPlaylist.m3u8"));
		assert!(video.media.needs_hls_for_audio);
		let gallery = crate::utils::parse_post(&posts[1]).await;
		assert_eq!((gallery.post_type.as_str(), gallery.gallery.len()), ("gallery", 2));
		assert_eq!(posts[2]["data"]["crosspost_parent_list"][0]["subreddit"], "gpui");
	}

	#[test]
	fn test_all_pages_fill_the_limit() {
		let page = |ids: &[&str], end: &str| {
			let mut edges: Vec<Value> = ids.iter().map(|id| json_value!({ "node": { "groupId": id, "adPayload": null } })).collect();
			edges.insert(1, json_value!({ "node": { "groupId": "t3_ad", "adPayload": {} } }));
			json_value!({ "edges": edges, "pageInfo": { "endCursor": STANDARD.encode(end) } })
		};
		let mut ids = Vec::new();
		assert_eq!(take_all_page(&mut ids, &page(&["t3_a", "t3_b", "t3_c"], "t3_c"), 4).as_deref(), Some("t3_c"));
		// The second page repeats `t3_c` and has more than needed
		assert_eq!(take_all_page(&mut ids, &page(&["t3_c", "t3_d", "t3_e"], "t3_e"), 4).as_deref(), Some("t3_d"));
		assert_eq!(ids, ["t3_a", "t3_b", "t3_c", "t3_d"]);

		let end = json_value!({ "edges": [], "pageInfo": { "hasNextPage": false } });
		assert_eq!(take_all_page(&mut ids, &end, 10), None);
	}

	#[test]
	fn test_subreddit_without_sidebar() {
		let empty = subreddit(&json_value!({ "name": "x", "description": { "richtext": "{\"document\":[]}" } })).unwrap();
		assert!(empty["data"]["description_html"].is_null());
		let doc = r#"{"document":[{"e":"par","c":[{"e":"text","t":"hi"}]}]}"#;
		let full = subreddit(&json_value!({ "name": "x", "description": { "richtext": doc } })).unwrap();
		assert_eq!(full["data"]["description_html"], "<div class=\"md\"><p>hi</p></div>");
	}

	#[test]
	fn test_render_list_items() {
		let item = |c: Value| json_value!([{ "e": "list", "c": [{ "e": "li", "c": c }] }]);
		let par = |t: &str| json_value!({ "e": "par", "c": [{ "e": "text", "t": t }] });
		assert_eq!(render(&item(json_value!([par("a")])), &Value::Null), "<ul><li>a</li></ul>");
		assert_eq!(render(&item(json_value!([par("a"), par("b")])), &Value::Null), "<ul><li><p>a</p><p>b</p></li></ul>");
	}

	#[test]
	fn test_render_heading_closing_hashes() {
		let doc = json_value!([{ "e": "h", "l": 2, "c": [{ "e": "raw", "t": "Rules ##" }] }, { "e": "h", "l": 3, "c": [{ "e": "raw", "t": "C#" }] }]);
		assert_eq!(render(&doc, &Value::Null), "<h2>Rules</h2><h3>C</h3>");
	}

	#[test]
	fn test_render_app_formats() {
		let doc = json_value!([{ "e": "par", "c": [
			{ "e": "text", "t": "us", "f": [[4, 0, 1], [128, 1, 1]] },
			{ "e": "p/", "t": "x", "l": false },
			{ "e": "c/", "t": "y", "l": true },
			{ "e": "img", "id": "i", "c": "cap <1>" },
		]}]);
		let media = json_value!([{ "id": "i", "url": "https://i.redd.it/i.png" }]);
		assert_eq!(
			render(&doc, &media),
			concat!(
				r#"<p><u>u</u><span class="md-spoiler-text">s</span><a href="/p/x">p/x</a><a href="/c/y">/c/y</a>"#,
				r#"<figure><a href="/img/i.png"><img src="/img/i.png" loading="lazy"></a><figcaption>cap &lt;1&gt;</figcaption></figure></p>"#,
			)
		);
	}

	#[test]
	fn test_render_pipe_table() {
		let line = |t: &str| json_value!({ "e": "text", "t": t });
		let br = json_value!({ "e": "br" });
		let doc = json_value!([{ "e": "par", "c": [
			line("| A | B |"), br.clone(), line("|:-|-:|"), br.clone(),
			line("| "), { "e": "link", "t": "x", "u": "https://example.com" }, line(" | 1 | | y | 2 |"), br.clone(), line("| z | 3 |"),
		]}]);
		assert_eq!(
			render(&doc, &Value::Null),
			concat!(
				r#"<table><thead><tr><th align="left">A</th><th align="right">B</th></tr></thead><tbody>"#,
				r#"<tr><td align="left"><a href="https://example.com">x</a></td><td align="right">1</td></tr>"#,
				r#"<tr><td align="left">y</td><td align="right">2</td></tr>"#,
				r#"<tr><td align="left">z</td><td align="right">3</td></tr></tbody></table>"#,
			)
		);
		let lines = json_value!([{ "e": "par", "c": [line("a | b"), br, line("c")] }]);
		assert_eq!(render(&lines, &Value::Null), "<p>a | b<br>c</p>");
	}

	#[test]
	fn test_wiki_toc() {
		let html = "<h1>Intro: A/B</h1><p>x</p><h2>Sub</h2><h3>Deep</h3><h2>Sub</h2><h1>Ünï &amp; co</h1>";
		assert_eq!(
			wiki_toc(html),
			concat!(
				r##"<div class="toc"><ul><li class="wiki_intro.3A_a.2Fb"><a href="#wiki_intro.3A_a.2Fb">Intro: A/B</a></li>"##,
				r##"<li class="toc_child"><ul><li class="wiki_sub"><a href="#wiki_sub">Sub</a></li>"##,
				r##"<li class="toc_child"><ul><li class="wiki_deep"><a href="#wiki_deep">Deep</a></li></ul></li>"##,
				r##"<li class="wiki_sub2"><a href="#wiki_sub2">Sub</a></li></ul></li>"##,
				r##"<li class="wiki_.C3.BCn.C3.AF_.26_co"><a href="#wiki_.C3.BCn.C3.AF_.26_co">Ünï &amp; co</a></li></ul></div>"##,
				r##"<h1 id="wiki_intro.3A_a.2Fb">Intro: A/B</h1><p>x</p><h2 id="wiki_sub">Sub</h2><h3 id="wiki_deep">Deep</h3>"##,
				r##"<h2 id="wiki_sub2">Sub</h2><h1 id="wiki_.C3.BCn.C3.AF_.26_co">Ünï &amp; co</h1>"##,
			)
		);
		assert_eq!(wiki_toc("<p>no headings</p>"), "<p>no headings</p>");
	}

	#[test]
	fn test_wiki_json() {
		let doc = r#"{"document":[{"e":"par","c":[{"e":"text","t":"hi"}]}]}"#;
		let page = wiki_json(
			&json_value!({ "__typename": "Subreddit", "wiki": { "page": { "status": "VALID", "content": { "richtext": doc } } } }),
			"x",
		)
		.unwrap();
		assert_eq!(page["data"]["content_html"], "<div class=\"md wiki\"><p>hi</p></div>");
		let missing = wiki_json(
			&json_value!({ "__typename": "Subreddit", "wiki": { "page": { "status": "PAGE_NOT_FOUND", "content": null } } }),
			"x",
		)
		.unwrap();
		assert_eq!(missing, json_value!({ "reason": "PAGE_NOT_FOUND" }));
		assert!(wiki_json(&json_value!({ "__typename": "UnavailableSubreddit" }), "x").is_err());
	}

	#[tokio::test]
	async fn test_packaged_media_fixture() {
		let posts = fixture("packaged_media.json")["data"]["postsInfoByIds"]
			.as_array()
			.unwrap()
			.iter()
			.filter_map(post)
			.collect::<Vec<_>>();

		// The muxed MP4 has sound, unlike v.redd.it's CMAF renditions
		let hosted = crate::utils::parse_post(&posts[0]).await;
		assert_eq!(hosted.post_type, "video");
		assert!(hosted.media.url.starts_with("/packaged/hynwcs24l1se1/muxed-high.mp4?"));
		assert!(hosted.media.alt_url.starts_with("/hls/hynwcs24l1se1/HLSPlaylist.m3u8"));
		assert!(!hosted.media.needs_hls_for_audio);

		// Reddit's copy of a Streamable clip is an MP4 behind `hlsUrl`
		let external = crate::utils::parse_post(&posts[1]).await;
		assert_eq!(external.post_type, "video");
		assert!(external.media.url.starts_with("/packaged-external/t3_1xhfggw/3p/streamable/9gsuah/video.mp4?"));
		assert_eq!(external.media.alt_url, "");
		assert!(!external.media.needs_hls_for_audio);
	}

	#[tokio::test]
	async fn test_search_fixtures() {
		let listing = |name: &str| search_listing(&fixture(name)).unwrap();
		let children = |listing: &Value| listing["data"]["children"].as_array().unwrap().clone();

		let posts = listing("dynamic_search_posts.json");
		assert!(posts["data"]["after"].as_str().is_some_and(|after| after.len() > 100));
		let posts = children(&posts);
		assert_eq!(posts.len(), 3);
		let video = crate::utils::parse_post(&posts[0]).await;
		assert_eq!(
			(video.title.as_str(), video.community.as_str(), video.post_type.as_str()),
			("Aurora Borealis seen really close", "oddlysatisfying", "video")
		);
		assert_eq!(video.author.name, "Original_Act_3481");
		assert_eq!(crate::utils::parse_post(&posts[1]).await.post_type, "self");
		assert_eq!(crate::utils::parse_post(&posts[2]).await.post_type, "image");

		let media = children(&listing("dynamic_search_media.json"));
		let mut types = Vec::new();
		for post in &media {
			types.push(crate::utils::parse_post(post).await.post_type);
		}
		assert_eq!(types, ["video", "image", "gallery"]);

		let comments = children(&listing("dynamic_search_comments.json"));
		assert_eq!(comments.len(), 2);
		let val = |key| crate::utils::val(&comments[0], key);
		assert_eq!(comments[0]["kind"], "t1");
		assert_eq!((val("author"), val("subreddit")), ("Venomspino".to_string(), "Minecraft".to_string()));
		assert_eq!(val("link_title"), "New Aurora Cape!");
		assert_eq!(val("permalink"), "/r/Minecraft/comments/1wsk2m5/comment/pcm2c6z/");
		assert!(val("body_html").contains("Aurora Borealis!?"));

		let communities = children(&listing("dynamic_search_communities.json"));
		let names: Vec<String> = communities.iter().map(|s| crate::utils::val(s, "display_name")).collect();
		assert_eq!(names, ["AuroraBorealis", "northernlights", "BreastfeedingAurora"]);
		assert_eq!(crate::utils::val(&communities[0], "url"), "/r/AuroraBorealis/");
		assert!(communities[0]["data"]["subscribers"].as_u64().is_some_and(|s| s > 20000));

		let people = children(&listing("dynamic_search_people.json"));
		let names: Vec<String> = people.iter().map(|u| crate::utils::val(u, "name")).collect();
		assert_eq!(names, ["auroraborealis", "Auroraborealissa"]);
		assert_eq!(people[0]["data"]["total_karma"], 1);
		assert!(people[0]["data"]["created_utc"].as_f64().is_some_and(|c| c > 1.2e9));
	}

	#[test]
	fn test_search_leaves_out_ads() {
		let mut response = fixture("dynamic_search_posts.json");
		let ad = json_value!({ "__typename": "SearchAdPost", "id": "ad", "post": { "__typename": "AdPost", "id": "t3_ad", "postTitle": "Buy this" } });
		response["data"]["search"]["dynamic"]["components"]["main"]["edges"][0]["node"]["children"]
			.as_array_mut()
			.unwrap()
			.insert(1, ad);
		let listing = search_listing(&response).unwrap();
		assert_eq!(listing["data"]["children"].as_array().unwrap().len(), 3);
	}

	#[test]
	fn test_search_variables() {
		let entries = |list: &Value| {
			list
				.as_array()
				.unwrap()
				.iter()
				.map(|e| format!("{}={}", e["key"].as_str().unwrap(), e["value"].as_str().unwrap()))
				.collect::<Vec<_>>()
		};
		let path = "/search.json?q=rust&sort=top&t=week&post_types=image&after=abc&include_over_18=on";
		let variables = search_variables(path, "rust".into(), "posts");
		assert_eq!(entries(&variables["filters"]), ["safe_search=off", "sort=TOP", "time_range=week", "post_types=image"]);
		assert_eq!(
			entries(&variables["options"]),
			[
				"page_type=search_results",
				"pane=posts",
				"pref_allow_nsfw=1",
				"is_eligible_for_kwf=0",
				"after=abc",
				"treatment=default"
			]
		);

		let variables_new = search_variables("/search.json?q=rust&sort=new&t=week", "rust".into(), "comments");
		assert_eq!(entries(&variables_new["filters"]), ["safe_search=on", "sort=NEW"]);
		assert!(entries(&variables_new["options"]).contains(&"after=".to_string()));
		// The pages of one query share its ID
		assert_eq!(variables["queryId"], variables_new["queryId"]);
		assert_ne!(variables["queryId"], search_variables("", "rust async".into(), "posts")["queryId"]);

		assert_eq!(search_pane("/search.json?q=rust&type=comment"), "comments");
		assert_eq!(search_pane("/r/rust/search.json?q=rust&type=media"), "media");
		assert_eq!(search_pane("/search.json?q=rust"), "posts");
	}

	#[test]
	fn test_ads_are_marked() {
		let ad = json_value!({ "__typename": "ProfilePost", "id": "t3_1xjiiv6", "callToAction": "Learn More" });
		let normal = json_value!({ "__typename": "SubredditPost", "id": "t3_1xl5x1j", "callToAction": null, "adSupplementaryTextRichtext": null });
		assert_eq!(post(&ad).unwrap()["data"]["is_created_from_ads_ui"], true);
		assert_eq!(post(&normal).unwrap()["data"]["is_created_from_ads_ui"], false);

		let response: Value = serde_json::from_str(include_str!("../../tests/fixtures/gql/post_comments.json")).unwrap();
		let tree = listing(comment_tree(&response, "1xe5jpg").unwrap());
		let mut comments = crate::post::parse_comments(&tree, "/r/AskReddit/comments/1xe5jpg/x/", "op", "", &Default::default(), &request());
		crate::post::mark_advertiser(&mut comments, "aubreynewmanoc");
		assert!(comments[0].author.advertiser);
		assert!(!comments[0].replies[0].author.advertiser);
	}

	#[test]
	fn test_profile_posts_name_the_profile() {
		let node = json_value!({
			"__typename": "ProfilePost", "id": "t3_1vgbkge", "postTitle": "Hi", "subreddit": null,
			"profile": { "name": "u_spez", "redditorInfo": { "prefixedName": "u/spez" } },
		});
		let post = post(&node).unwrap();
		assert_eq!((&post["data"]["subreddit"], &post["data"]["subreddit_name_prefixed"]), (&"u_spez".into(), &"u/spez".into()));
	}

	#[test]
	fn test_user_fixtures() {
		let about = user(&fixture("profile.json")["data"]["redditorInfoByName"]).unwrap();
		assert_eq!(about["data"]["name"], "spez");
		assert_eq!(about["data"]["total_karma"], 941258);
		assert!(about["data"]["subreddit"]["icon_img"].as_str().is_some_and(|url| url.contains("profileIcon")));
		assert!(user(&Value::Null).is_none());
		assert_eq!(about["data"]["subreddit"]["public_description"], "Reddit CEO");
		assert!(about["data"]["created"].as_f64().unwrap() > 1.1e9);

		let response = fixture("user_comments.json");
		let comments = user_comment_items(&response["data"]["redditorInfoByName"]["comments"], &response["data"]["commentsByIds"]);
		// Comments missing from `LiveCommentsByIds` are dropped
		assert_eq!(comments.len(), 1);
		let val = |key| crate::utils::val(&comments[0], key);
		assert_eq!((val("id"), val("subreddit")), ("p1wosm9".to_string(), "u_spez".to_string()));
		assert_eq!(comments[0]["kind"], "t1");
		assert_eq!(val("link_title"), "Modernizing Reddit’s infrastructure with you");
		assert!(val("body_html").contains("replicate the UI"));
		assert!(val("permalink").ends_with("/p1wosm9/"));
	}

	#[test]
	fn test_subreddit_info_fixture() {
		let about = subreddit(&fixture("subreddit_info.json")["data"]["subredditInfoByName"]).unwrap();
		let val = |key| crate::utils::val(&about, key);
		assert_eq!((val("display_name"), val("title")), ("rust".to_string(), "The Rust Programming Language".to_string()));
		assert!(val("public_description").starts_with("A place for all things related to the Rust"));
		assert!(val("community_icon").starts_with("https://styles.redditmedia.com/"));
		assert_eq!(about["data"]["subscribers"].as_u64(), Some(427823));
		let html = val("description_html");
		assert!(
			html.contains(r#"<h3>Please read <a href="https://www.rust-lang.org/policies/code-of-conduct"><strong>The Rust Community Code of Conduct</strong></a></h3><hr>"#),
			"{html}"
		);
	}

	#[test]
	fn test_subreddit_weekly_stats() {
		let node = json_value!({ "name": "news", "communityStats": { "weeklyActiveUsersCount": 4088390, "weeklyContributionsCount": 63142 } });
		let about = subreddit(&node).unwrap();
		assert_eq!(about["data"]["weekly_active_users"], 4088390);
		assert_eq!(about["data"]["weekly_contributions"], 63142);
	}

	#[test]
	fn test_unavailable_subreddits() {
		for (reason, expected) in [("PRIVATE", "private"), ("BANNED", "banned"), ("QUARANTINED", "quarantined"), ("GATED", "gated")] {
			let node = json_value!({ "__typename": "UnavailableSubreddit", "forbiddenReason": reason, "name": "x" });
			assert_eq!(about_json(&node, "x"), Err(expected.to_string()));
		}
		let node = json_value!({ "__typename": "UnavailableSubreddit", "name": "x" });
		assert_eq!(about_json(&node, "x"), Err("r/x is unavailable".to_string()));
		assert!(about_json(&fixture("subreddit_info.json")["data"]["subredditInfoByName"], "rust").is_ok());
	}
}
