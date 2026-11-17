#![allow(clippy::cmp_owned)]
use crate::body::Body;
use crate::utils::{self, catch_random, clean_url, error, filter_posts, format_num, format_url, get_filters, param, redirect, setting, template, val, Post, Preferences};
use crate::{
	backends::json,
	server::RequestExt,
	subreddit::{can_access_quarantine, quarantine},
};
use askama::Template;
use hyper::{Request, Response};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use regex::Regex;
use rss::ChannelBuilder;
use serde_json::Value;
use std::sync::LazyLock;

// STRUCTS
struct SearchParams {
	q: String,
	/// `q` for links
	q_url: String,
	tab: &'static Tab,
	sort: String,
	t: String,
	post_types: String,
	/// The next page's `after`, for links
	after: String,
	restrict_sr: String,
}

/// A tab of the search page, like the tabs of Reddit's search.
struct Tab {
	/// Its `type` in the URL
	name: &'static str,
	label: &'static str,
	sorts: &'static [&'static str],
	post_types: &'static [&'static str],
}

static TABS: [Tab; 5] = [
	Tab {
		name: "posts",
		label: "Posts",
		sorts: &["relevance", "hot", "top", "new", "comments"],
		post_types: &["all", "text", "image", "video", "gif", "gallery", "link", "poll"],
	},
	Tab {
		name: "comments",
		label: "Comments",
		sorts: &["relevance", "top", "new"],
		post_types: &[],
	},
	Tab {
		name: "media",
		label: "Media",
		sorts: &["relevance", "hot", "top", "new"],
		post_types: &["all", "image", "video", "gif", "gallery"],
	},
	Tab {
		name: "communities",
		label: "Communities",
		sorts: &[],
		post_types: &[],
	},
	Tab {
		name: "people",
		label: "People",
		sorts: &[],
		post_types: &[],
	},
];

/// The tab for the URL's `type`, also taking the JSON API's types (`sr_user`
/// was the old subreddit list).
fn tab(typed: &str) -> &'static Tab {
	let name = match typed {
		"comment" => "comments",
		"sr" | "sr_user" => "communities",
		"user" => "people",
		typed => typed,
	};
	TABS.iter().find(|tab| tab.name == name).unwrap_or(&TABS[0])
}

// STRUCTS
/// A subreddit as a card: search results, the sidebar's community lists.
pub struct Subreddit {
	pub name: String,
	pub url: String,
	pub icon: String,
	pub description: String,
	pub subscribers: (String, String),
}

struct User {
	name: String,
	icon: String,
	karma: (String, String),
	/// Age of the account, like `6y`
	age: String,
	created: String,
}

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
	posts: Vec<Post>,
	subreddits: Vec<Subreddit>,
	users: Vec<User>,
	tabs: &'static [Tab],
	sub: String,
	params: SearchParams,
	prefs: Preferences,
	url: String,
	/// Whether the subreddit itself is filtered.
	is_filtered: bool,
	/// Whether all fetched posts are filtered (to differentiate between no posts fetched in the first place,
	/// and all fetched posts being filtered).
	all_posts_filtered: bool,
	/// Whether all posts were hidden because they are NSFW (and user has disabled show NSFW)
	all_posts_hidden_nsfw: bool,
	no_posts: bool,
}

/// Regex matched against search queries to determine if they are reddit urls.
static REDDIT_URL_MATCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://([^\./]+\.)*reddit.com/").unwrap());

// SERVICES
pub async fn find(req: Request<Body>) -> Result<Response<Body>, String> {
	// This ensures that during a search, no NSFW posts are fetched at all
	let nsfw_results = if setting(&req, "show_nsfw") == "on" && !utils::sfw_only() {
		"&include_over_18=on"
	} else {
		""
	};
	let uri_path = req.uri().path().replace("+", "%2B");
	let path = format!("{}.json?{}{}&raw_json=1", uri_path, req.uri().query().unwrap_or_default(), nsfw_results);
	let mut query = param(&path, "q").unwrap_or_default();
	query = REDDIT_URL_MATCH.replace(&query, "").to_string();

	if query.is_empty() {
		return Ok(redirect("/"));
	}

	if query.starts_with("r/") || query.starts_with("user/") {
		return Ok(redirect(&format!("/{query}")));
	}

	if query.starts_with("R/") {
		return Ok(redirect(&format!("/r{}", &query[1..])));
	}

	if query.starts_with("u/") || query.starts_with("U/") {
		return Ok(redirect(&format!("/user{}", &query[1..])));
	}

	let sub = req.param("sub").unwrap_or_default();
	let quarantined = can_access_quarantine(&req, &sub);
	// Handle random subreddits
	if let Ok(random) = catch_random(&sub, "/find").await {
		return Ok(random);
	}

	let tab = tab(&param(&path, "type").unwrap_or_default());
	let sort = param(&path, "sort")
		.filter(|sort| tab.sorts.contains(&sort.as_str()))
		.unwrap_or_else(|| "relevance".to_string());
	let post_types = param(&path, "post_types").filter(|types| tab.post_types.contains(&types.as_str())).unwrap_or_default();
	let t = param(&path, "t").filter(|_| !tab.sorts.is_empty()).unwrap_or_default();
	let restrict_sr = param(&path, "restrict_sr").unwrap_or_default();
	let after = param(&path, "after").unwrap_or_default();
	let filters = get_filters(&req);
	let url = String::from(req.uri().path_and_query().map_or("", |val| val.as_str()));

	let mut page = SearchTemplate {
		posts: Vec::new(),
		subreddits: Vec::new(),
		users: Vec::new(),
		tabs: &TABS,
		params: SearchParams {
			q: query.replace('"', "&quot;"),
			q_url: encode(&query),
			tab,
			sort,
			t,
			post_types,
			after: String::new(),
			restrict_sr,
		},
		prefs: Preferences::new(&req),
		url,
		is_filtered: false,
		all_posts_filtered: false,
		all_posts_hidden_nsfw: false,
		no_posts: false,
		sub,
	};

	if tab.name == "communities" || tab.name == "people" {
		let kind = if tab.name == "communities" { "subreddits" } else { "users" };
		let response = match json(format!("/{kind}/search.json?q={}&after={}{nsfw_results}", encode(&query), encode(&after)), false).await {
			Ok(response) => response,
			Err(msg) => return error(req, &msg).await,
		};
		let children = response["data"]["children"].as_array().map(Vec::as_slice).unwrap_or_default();
		if tab.name == "communities" {
			page.subreddits = children.iter().map(subreddit).filter(|s| !filters.contains(s.name.as_str())).collect();
		} else {
			page.users = children.iter().map(user).collect();
		}
		page.no_posts = children.is_empty();
		page.params.after = encode(response["data"]["after"].as_str().unwrap_or_default());
		return Ok(template(&page));
	}

	// If all requested subs are filtered, we don't need to fetch posts.
	if page.sub.split('+').all(|s| filters.contains(s)) {
		page.is_filtered = true;
		return Ok(template(&page));
	}

	// The first page of posts starts with the best matching subreddits, unless
	// the search is restricted to this subreddit
	if tab.name == "posts" && page.params.restrict_sr.is_empty() && after.is_empty() {
		page.subreddits = search_subreddits(&query).await;
		page.subreddits.retain(|s| !filters.contains(s.name.as_str()));
	}

	let api_type = match tab.name {
		"comments" => "comment",
		"media" => "media",
		_ => "link",
	};
	let params = &page.params;
	let posts_path = format!(
		"{uri_path}.json?q={}&type={api_type}&sort={}&t={}&post_types={}&restrict_sr={}&after={}{nsfw_results}&raw_json=1",
		params.q_url,
		params.sort,
		encode(&params.t),
		encode(&params.post_types),
		encode(&params.restrict_sr),
		encode(&after)
	);
	match Post::fetch(&posts_path, quarantined).await {
		Ok((mut posts, after)) => {
			let clean_urls = setting(&req, "clean_urls");
			if clean_urls == "on".to_owned() {
				posts.iter_mut().for_each(|post| post.media.url = clean_url(post.media.url.clone()));
			}
			let (_, all_posts_filtered) = filter_posts(&mut posts, &filters);
			page.no_posts = posts.is_empty();
			page.all_posts_hidden_nsfw = !page.no_posts && (posts.iter().all(|p| p.flags.nsfw) && setting(&req, "show_nsfw") != "on");
			page.all_posts_filtered = all_posts_filtered;
			page.posts = posts;
			page.params.after = encode(&after);
			Ok(template(&page))
		}
		Err(msg) => {
			if msg == "quarantined" || msg == "gated" {
				let sub = req.param("sub").unwrap_or_default();
				Ok(quarantine(&req, sub, &msg))
			} else {
				error(req, &msg).await
			}
		}
	}
}

fn encode(text: &str) -> String {
	utf8_percent_encode(text, NON_ALPHANUMERIC).to_string()
}

/// Feed for post search results, built from the JSON API.
/// Mirrors Reddit's feed URLs: `/search.rss?q=rust&sort=new`, `/r/rust/search.rss?q=async&restrict_sr=on`.
pub async fn rss(req: Request<Body>) -> Result<Response<Body>, String> {
	if !utils::enable_rss() {
		return Ok(error(req, "RSS is disabled on this instance.").await.unwrap_or_default());
	}

	let nsfw_results = if setting(&req, "show_nsfw") == "on" && !utils::sfw_only() {
		"&include_over_18=on"
	} else {
		""
	};
	let page_path = utils::rss_page_path(&req).replace('+', "%2B");
	let path = format!("{page_path}.json?{}{nsfw_results}&raw_json=1", req.uri().query().unwrap_or_default());

	let query = param(&path, "q").unwrap_or_default();
	if query.is_empty() {
		return error(req, "Search feeds need a query, e.g. ?q=rust").await;
	}
	// Subreddit and user search results aren't posts
	if param(&path, "type").is_some_and(|typed| typed != "link") {
		return error(req, "Only post searches have feeds").await;
	}

	let (posts, _) = Post::fetch(&path, false).await?;

	let title = match req.param("sub") {
		Some(sub) if param(&path, "restrict_sr").is_some() => format!("Search results for \"{query}\" in r/{sub}"),
		_ => format!("Search results for \"{query}\""),
	};
	let channel = ChannelBuilder::default()
		.title(title)
		.link(utils::rss_page_link(&req))
		.items(utils::build_rss_items(&posts))
		.build();

	Ok(utils::rss_response(&channel))
}

/// The three subreddits that match `q` best.
async fn search_subreddits(q: &str) -> Vec<Subreddit> {
	let subreddit_search_path = format!("/subreddits/search.json?q={}&limit=3", encode(q));

	// Send a request to the url
	json(subreddit_search_path, false).await.unwrap_or_default()["data"]["children"]
		.as_array()
		.map(ToOwned::to_owned)
		.unwrap_or_default()
		.iter()
		.map(subreddit)
		.collect::<Vec<Subreddit>>()
}

fn subreddit(subreddit: &Value) -> Subreddit {
	// Fetch subreddit icon either from the community_icon or icon_img value
	let icon = subreddit["data"]["community_icon"].as_str().map_or_else(|| val(subreddit, "icon_img"), ToString::to_string);

	Subreddit {
		name: val(subreddit, "display_name"),
		url: val(subreddit, "url"),
		icon: format_url(&icon),
		description: val(subreddit, "public_description"),
		subscribers: format_num(subreddit["data"]["subscribers"].as_f64().unwrap_or_default() as i64),
	}
}

fn user(user: &Value) -> User {
	let created = user["data"]["created_utc"].as_f64().unwrap_or_default();
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
	let days = ((now - created) / 86400.0) as i64;
	User {
		name: val(user, "name"),
		icon: format_url(&val(user, "icon_img")),
		karma: format_num(user["data"]["total_karma"].as_i64().unwrap_or_default()),
		age: match days {
			365.. => format!("{}y", days / 365),
			30.. => format!("{}mo", days / 30),
			_ => format!("{days}d"),
		},
		created: utils::time(created).1,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_tabs() {
		for (typed, name) in [
			("", "posts"),
			("link", "posts"),
			("comment", "comments"),
			("comments", "comments"),
			("media", "media"),
			("sr_user", "communities"),
			("communities", "communities"),
			("user", "people"),
			("people", "people"),
			("anything", "posts"),
		] {
			assert_eq!(tab(typed).name, name, "{typed}");
		}
	}

	fn page(typed: &str, sort: &str, restrict_sr: &str) -> SearchTemplate {
		SearchTemplate {
			posts: Vec::new(),
			subreddits: Vec::new(),
			users: Vec::new(),
			tabs: &TABS,
			params: SearchParams {
				q: "aurora borealis".to_string(),
				q_url: encode("aurora borealis"),
				tab: tab(typed),
				sort: sort.to_string(),
				t: String::new(),
				post_types: String::new(),
				after: "abc%3D".to_string(),
				restrict_sr: restrict_sr.to_string(),
			},
			prefs: Preferences::new(&Request::new(Body::from(""))),
			url: String::new(),
			is_filtered: false,
			all_posts_filtered: false,
			all_posts_hidden_nsfw: false,
			no_posts: false,
			sub: if restrict_sr.is_empty() { String::new() } else { "rust".to_string() },
		}
	}

	fn render(typed: &str, sort: &str, restrict_sr: &str) -> String {
		page(typed, sort, restrict_sr).render().unwrap()
	}

	#[test]
	fn test_tabs_are_links_with_their_own_filters() {
		let html = render("media", "top", "");
		assert!(html.contains(r#"<a href="?q=aurora%20borealis&type=media" class="selected">Media</a>"#), "{html}");
		assert!(html.contains(r#"<a href="?q=aurora%20borealis&type=people" >People</a>"#), "{html}");
		assert!(html.contains(r#"<input type="hidden" name="type" value="media">"#));
		assert!(html.contains(r#"name="post_types""#) && html.contains("GIF") && !html.contains(r#"value="poll""#));
		assert!(html.contains(r#"name="t""#) && !html.contains(r#"value="comments""#));
		assert!(html.contains(r#"<div class="media_grid search_results">"#), "{html}");
		assert!(
			html.contains(r#"<a id="show_more" href="?q=aurora%20borealis&type=media&restrict_sr=&sort=top&t=&post_types=&after=abc%3D" accesskey="N">Show more</a>"#),
			"{html}"
		);

		let html = render("comments", "new", "on");
		assert!(html.contains(r#"<div class="search_results search_results_comments">"#), "{html}");
		assert!(html.contains(r#"id="show_more""#) && !html.contains("NEXT"));
		assert!(html.contains(r#"<a href="?q=aurora%20borealis&type=posts&restrict_sr=on" >Posts</a>"#), "{html}");
		assert!(!html.contains(r#"name="post_types""#) && !html.contains(r#"name="t""#));

		let html = render("sr_user", "", "");
		assert!(html.contains(r#"class="selected">Communities</a>"#));
		assert!(!html.contains(r#"name="sort""#) && !html.contains(r#"name="post_types""#));
		assert!(html.contains(r#"<a id="show_more" href="?q=aurora%20borealis&type=communities&"#), "{html}");

		let mut people = page("people", "", "");
		people.users.push(User {
			name: "spez".to_string(),
			icon: String::new(),
			karma: ("1k".to_string(), "1000".to_string()),
			age: "6y".to_string(),
			created: String::new(),
		});
		let html = people.render().unwrap();
		assert!(html.contains(r#"<div id="search_subreddits" class="search_results">"#), "{html}");
		assert!(html.contains(r#"<a id="show_more" href="?q=aurora%20borealis&type=people&"#), "{html}");

		let mut no_more = page("posts", "", "");
		no_more.params.after.clear();
		assert!(!no_more.render().unwrap().contains("show_more"));
	}

	#[tokio::test]
	async fn test_feed_videos_carry_their_size_in_css() {
		let post = serde_json::json!({ "kind": "t3", "data": {
			"id": "abc", "title": "t", "subreddit": "test", "permalink": "/r/test/comments/abc/t/",
			"secure_media": { "reddit_video": {
				"fallback_url": "https://v.redd.it/abc/CMAF_720.mp4?source=fallback",
				"hls_url": "https://v.redd.it/abc/HLSPlaylist.m3u8",
			} },
			"preview": { "images": [{ "source": { "url": "https://external-preview.redd.it/abc.png", "width": 405, "height": 720 } }] },
		}});
		for hls in ["off", "on"] {
			let mut results = page("posts", "", "");
			results.url = "/search?q=t".to_string();
			results.prefs.use_hls = hls.to_string();
			results.posts.push(utils::parse_post(&post).await);
			let html = results.render().unwrap();
			assert!(html.contains(r#"width="405" height="720" style="--w: 405; --h: 720""#), "use_hls={hls}: {html}");
		}
	}
}
