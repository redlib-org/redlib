//! Live tests that call one backend directly instead of going through the
//! backend order, so a broken backend can't hide behind a working one.
//!
//! The session backends need `REDLIB_REDDIT_COOKIE`; without it their tests
//! pass without doing anything (CI sets it from a repository secret, which
//! pull requests from forks don't get).

use super::{cookied_json::CookiedJson, gql::Gql, json::Json, scraped::Scraped, Backend};
use crate::client::block_on_shared;
use sealed_test::prelude::*;
use serde_json::Value;

/// A post that has comments and stays up.
const POST: &str = "/r/redditdev/comments/1wubcvf.json?raw_json=1";

fn get(backend: &dyn Backend, path: &str) -> Value {
	block_on_shared(backend.fetch(path, false)).unwrap_or_else(|failure| panic!("{} {path}: {failure:?}", backend.name()))
}

/// Whether the session backends can run; says so when they can't.
fn has_session(backend: &dyn Backend) -> bool {
	let available = backend.available();
	if !available {
		eprintln!("{}: REDLIB_REDDIT_COOKIE isn't set, nothing to test", backend.name());
	}
	available
}

/// The listing's children, asserting there are some.
fn children<'a>(listing: &'a Value, what: &str) -> &'a [Value] {
	let children = listing["data"]["children"].as_array().map(Vec::as_slice).unwrap_or_default();
	assert!(!children.is_empty(), "{what}: empty listing: {listing}");
	children
}

/// Asserts the fields every post page and listing reads.
fn assert_post(post: &Value, what: &str) {
	assert_eq!(post["kind"], "t3", "{what}: {post}");
	let data = &post["data"];
	for field in ["id", "title", "author", "subreddit", "permalink"] {
		assert!(data[field].as_str().is_some_and(|v| !v.is_empty()), "{what}: post without {field}: {data}");
	}
}

fn assert_posts(listing: &Value, what: &str) {
	for post in children(listing, what) {
		assert_post(post, what);
	}
}

fn assert_thread(thread: &Value, what: &str) {
	assert_post(&children(&thread[0], what)[0], what);
	let comments = children(&thread[1], what);
	let comment = &comments.iter().find(|c| c["kind"] == "t1").unwrap_or_else(|| panic!("{what}: no comments: {thread}"))["data"];
	assert!(comment["body"].as_str().is_some_and(|b| !b.is_empty()), "{what}: comment without body: {comment}");
	assert!(comment["author"].as_str().is_some(), "{what}: comment without author: {comment}");
}

fn assert_subreddit(about: &Value, what: &str) {
	assert_eq!(about["kind"], "t5", "{what}: {about}");
	assert_eq!(about["data"]["display_name"].as_str().map(str::to_lowercase).as_deref(), Some("rust"), "{what}: {about}");
}

fn assert_user(about: &Value, what: &str) {
	assert_eq!(about["kind"], "t2", "{what}: {about}");
	assert_eq!(about["data"]["name"], "spez", "{what}: {about}");
}

/// The paths every backend must serve.
fn core_paths(backend: &dyn Backend) {
	let name = backend.name();
	assert_thread(&get(backend, POST), &format!("{name} post"));
	assert_posts(&get(backend, "/r/rust/hot.json?raw_json=1"), &format!("{name} subreddit"));
	assert_subreddit(&get(backend, "/r/rust/about.json?raw_json=1"), &format!("{name} about"));
	assert_user(&get(backend, "/user/spez/about.json?raw_json=1"), &format!("{name} user"));
}

#[test]
fn test_gql_post_comments() {
	// PostsByIds for the post, PostComments for the comments
	assert_thread(&get(&Gql, POST), "gql post");
}

#[test]
fn test_gql_posts_by_ids() {
	assert_posts(&get(&Gql, "/by_id/t3_1wubcvf.json?raw_json=1"), "gql by_id");
}

#[test]
fn test_gql_subreddit_feed() {
	assert_posts(&get(&Gql, "/r/rust/hot.json?raw_json=1"), "gql subreddit");
	assert_posts(&get(&Gql, "/r/rust/top.json?t=week&raw_json=1"), "gql subreddit top");
}

#[test]
fn test_gql_popular() {
	assert_posts(&get(&Gql, "/r/popular/hot.json?raw_json=1"), "gql popular");
}

/// Reddit refuses `/new` to the JSON API; the old listing page still has it.
#[test]
#[ignore = "www.reddit.com answers GitHub's runners with 403; run with --include-ignored"]
fn test_scraped_combined_feed() {
	// Reddit redirects to `golang+rust`
	let page = get(&Scraped, "/r/rust%2Bgolang/new.json?&raw_json=1&limit=5");
	assert_posts(&page, "scraped combined feed");
	let posts = children(&page, "scraped combined feed");
	assert!(posts.len() <= 5, "{page}");
	assert!(
		posts.iter().all(|p| ["rust", "golang"].contains(&p["data"]["subreddit"].as_str().unwrap_or_default())),
		"{page}"
	);
	let created: Vec<f64> = posts.iter().filter_map(|p| p["data"]["created_utc"].as_f64()).collect();
	assert!(created.first() > created.last(), "scraped combined feed: not newest first: {created:?}");

	// The next page has other posts
	let after = page["data"]["after"].as_str().expect("scraped combined feed: no next page");
	let next = get(&Scraped, &format!("/r/rust%2Bgolang/new.json?&raw_json=1&limit=5&after={after}"));
	assert_posts(&next, "scraped combined feed page 2");
	let first = &posts[0]["data"]["id"];
	assert!(children(&next, "scraped combined feed page 2").iter().all(|p| &p["data"]["id"] != first), "{next}");

	// Hot is the page without a sort
	assert_posts(&get(&Scraped, "/r/golang%2Brust/hot.json?&raw_json=1&limit=5"), "scraped combined feed hot");
	assert!(matches!(
		block_on_shared(Scraped.fetch("/r/rust/new.json?raw_json=1", false)),
		Err(super::Failure::Unsupported)
	));
}

/// Sessions get the old listing page too, for hosts Reddit blocks logged out.
#[test]
#[ignore = "www.reddit.com answers GitHub's runners with 403; run with --include-ignored"]
fn test_cookied_json_combined_feed() {
	if !has_session(&CookiedJson) {
		return;
	}
	let page = get(&CookiedJson, "/r/rust%2Bgolang/new.json?&raw_json=1&limit=5");
	assert_posts(&page, "cookied-json combined feed");
	assert!(page["data"]["after"].as_str().is_some_and(|after| after.starts_with("t3_")), "{page}");
}

/// `AllFeedSdui` is gone from the app; this fails once Reddit drops its hash.
#[test]
fn test_gql_all() {
	assert_posts(&get(&Gql, "/r/all/hot.json?raw_json=1"), "gql all");
}

/// Fails once Reddit closes the `r/all%2B` gap as well. A bare `r/all+`
/// gets a fixed copy of r/popular's first page that ignores `after`.
#[test]
fn test_json_all() {
	let page = get(&Json, "/r/all/hot.json?raw_json=1");
	assert_posts(&page, "json all");
	assert!(children(&page, "json all").len() >= 20, "json all: page size ignored");
	let after = page["data"]["after"].as_str().expect("json all: no after");
	let next = get(&Json, &format!("/r/all/hot.json?raw_json=1&after={after}"));
	assert_ne!(
		children(&next, "json all")[0]["data"]["id"],
		children(&page, "json all")[0]["data"]["id"],
		"json all: after ignored"
	);
}

/// Reddit refuses `/new` to every browser with the same page as a blocked one,
/// which mustn't switch away from a browser it serves.
#[test]
#[sealed_test]
fn test_json_path_block_keeps_the_emulation() {
	let _ = crate::client::CLIENT.load();
	let before = crate::client::current_emulation();
	let _ = block_on_shared(Json.fetch("/r/rust/new.json?raw_json=1", false));
	assert_eq!(crate::client::current_emulation(), before);
}

#[test]
fn test_gql_subreddit_info() {
	assert_subreddit(&get(&Gql, "/r/rust/about.json?raw_json=1"), "gql about");
}

#[test]
fn test_gql_wiki() {
	let page = get(&Gql, "/r/linux/wiki/index.json?raw_json=1");
	assert!(page["data"]["content_html"].as_str().is_some_and(|html| html.contains("<a href=")), "gql wiki: {page}");
	let missing = get(&Gql, "/r/linux/wiki/doesnotexist.json?raw_json=1");
	assert_eq!(missing["reason"], "PAGE_NOT_FOUND", "gql missing wiki page: {missing}");
}

#[test]
fn test_gql_search() {
	let page = get(&Gql, "/search.json?q=rust&raw_json=1");
	assert_posts(&page, "gql search");
	// The next page has other posts
	let after = page["data"]["after"].as_str().expect("gql search: no next page");
	let after = percent_encoding::utf8_percent_encode(after, percent_encoding::NON_ALPHANUMERIC);
	let next = get(&Gql, &format!("/search.json?q=rust&after={after}&raw_json=1"));
	assert_posts(&next, "gql search page 2");
	let first = &children(&page, "gql search")[0]["data"]["id"];
	assert!(children(&next, "gql search page 2").iter().all(|p| &p["data"]["id"] != first), "{next}");

	assert_posts(&get(&Gql, "/search.json?q=rust&sort=top&t=week&post_types=text&raw_json=1"), "gql search top text posts");
	let in_sub = get(&Gql, "/r/rust/search.json?q=async&restrict_sr=on&raw_json=1");
	assert_posts(&in_sub, "gql search in subreddit");
	assert!(children(&in_sub, "gql search in subreddit").iter().all(|p| p["data"]["subreddit"] == "rust"), "{in_sub}");
}

#[test]
fn test_gql_search_media() {
	let media = get(&Gql, "/search.json?q=aurora&type=media&raw_json=1");
	assert_posts(&media, "gql media search");
	for post in children(&media, "gql media search") {
		let data = &post["data"];
		assert!(
			data["preview"].is_object() || data["is_gallery"] == true || data["secure_media"].is_object(),
			"gql media search: no media: {data}"
		);
	}
	assert_posts(&get(&Gql, "/search.json?q=aurora&type=media&post_types=gif&raw_json=1"), "gql media search for GIFs");
}

#[test]
fn test_gql_search_comments() {
	let comments = get(&Gql, "/search.json?q=rust&type=comment&sort=new&raw_json=1");
	for comment in children(&comments, "gql comment search") {
		assert_eq!(comment["kind"], "t1", "{comment}");
		let data = &comment["data"];
		for field in ["id", "author", "body_html", "link_title", "permalink", "subreddit"] {
			assert!(data[field].as_str().is_some_and(|v| !v.is_empty()), "gql comment search: comment without {field}: {data}");
		}
	}
}

#[test]
fn test_gql_search_communities() {
	let subreddits = get(&Gql, "/subreddits/search.json?q=rust");
	for subreddit in children(&subreddits, "gql subreddit search") {
		assert_eq!(subreddit["kind"], "t5", "{subreddit}");
		assert!(subreddit["data"]["url"].as_str().is_some_and(|url| url.starts_with("/r/")), "{subreddit}");
	}
}

#[test]
fn test_gql_search_people() {
	let users = get(&Gql, "/users/search.json?q=spez");
	let users = children(&users, "gql people search");
	assert!(users.iter().all(|u| u["kind"] == "t2" && u["data"]["name"].is_string()), "{users:?}");
	assert!(users.iter().any(|u| u["data"]["total_karma"].as_i64().is_some_and(|k| k > 0)), "{users:?}");
}

#[test]
fn test_gql_user() {
	assert_user(&get(&Gql, "/user/spez/about.json?raw_json=1"), "gql user");
	assert!(
		get(&Gql, "/user/spez/about.json?raw_json=1")["data"]["total_karma"].as_i64().is_some_and(|k| k > 0),
		"gql user karma (SearchPeople)"
	);
	assert_posts(&get(&Gql, "/user/spez/submitted.json?raw_json=1"), "gql user posts");
	let comments = get(&Gql, "/user/spez/comments.json?raw_json=1");
	assert!(children(&comments, "gql user comments").iter().all(|c| c["kind"] == "t1"), "{comments}");
	children(&get(&Gql, "/user/spez/overview.json?raw_json=1"), "gql user overview");
}

#[test]
fn test_cookied_json() {
	if has_session(&CookiedJson) {
		core_paths(&CookiedJson);
	}
}

#[test]
fn test_cookied_json_all() {
	if has_session(&CookiedJson) {
		assert_posts(&get(&CookiedJson, "/r/all/hot.json?raw_json=1"), "cookied-json all");
	}
}
