#![allow(clippy::cmp_owned)]

use crate::{config, utils};
// CRATES
use crate::utils::{
	catch_random, error, filter_posts, format_num, format_url, get_filters, info, nsfw_landing, param, redirect, rewrite_urls, setting, template, val, Post, Preferences,
	Subreddit,
};
use crate::{client::json, server::RequestExt, server::ResponseExt};
use askama::Template;
use cookie::Cookie;
use htmlescape::decode_html;
use hyper::{Body, Request, Response};

use chrono::DateTime;
use regex::Regex;
use std::sync::LazyLock;
use time::{Duration, OffsetDateTime};

// STRUCTS
#[derive(Template)]
#[template(path = "subreddit.html")]
struct SubredditTemplate {
	sub: Subreddit,
	posts: Vec<Post>,
	sort: (String, String),
	ends: (String, String),
	prefs: Preferences,
	url: String,
	redirect_url: String,
	/// Whether the subreddit itself is filtered.
	is_filtered: bool,
	/// Whether all fetched posts are filtered (to differentiate between no posts fetched in the first place,
	/// and all fetched posts being filtered).
	all_posts_filtered: bool,
	/// Whether all posts were hidden because they are NSFW (and user has disabled show NSFW)
	all_posts_hidden_nsfw: bool,
	no_posts: bool,
}

#[derive(Template)]
#[template(path = "wiki.html")]
struct WikiTemplate {
	sub: String,
	wiki: String,
	page: String,
	prefs: Preferences,
	url: String,
}

#[derive(Template)]
#[template(path = "wall.html")]
struct WallTemplate {
	title: String,
	sub: String,
	msg: String,
	prefs: Preferences,
	url: String,
}

static GEO_FILTER_MATCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"geo_filter=(?<region>\w+)").unwrap());

// SERVICES
pub async fn community(req: Request<Body>) -> Result<Response<Body>, String> {
	// Build Reddit API path
	let root = req.uri().path() == "/";
	let query = req.uri().query().unwrap_or_default().to_string();
	let subscribed = setting(&req, "subscriptions");
	let front_page = setting(&req, "front_page");
	let remove_default_feeds = setting(&req, "remove_default_feeds") == "on";
	let post_sort = req.cookie("post_sort").map_or_else(|| "hot".to_string(), |c| c.value().to_string());
	let sort = req.param("sort").unwrap_or_else(|| req.param("id").unwrap_or(post_sort));

	let sub_name = req.param("sub").unwrap_or(if front_page == "default" || front_page.is_empty() {
		if subscribed.is_empty() {
			"popular".to_string()
		} else {
			subscribed.clone()
		}
	} else {
		front_page.clone()
	});

	if (sub_name == "popular" || sub_name == "all") && remove_default_feeds {
		if subscribed.is_empty() {
			return info(req, "Subscribe to some subreddits! (Default feeds disabled in settings)").await;
		} else {
			// If there are subscribed subs, but we get here, then the problem is that front_page pref is set to something besides default.
			// Tell user to go to settings and change front page to default.
			return info(
				req,
				"You have subscribed to some subreddits, but your front page is not set to default. Visit settings and change front page to default.",
			)
			.await;
		}
	}

	let quarantined = can_access_quarantine(&req, &sub_name) || root;

	// Handle random subreddits
	if let Ok(random) = catch_random(&sub_name, "").await {
		return Ok(random);
	}

	if req.param("sub").is_some() && sub_name.starts_with("u_") {
		return Ok(redirect(&["/user/", &sub_name[2..]].concat()));
	}

	// Request subreddit metadata
	let sub = if !sub_name.contains('+') && sub_name != subscribed && sub_name != "popular" && sub_name != "all" {
		// Regular subreddit
		subreddit(&sub_name, quarantined).await.unwrap_or_default()
	} else if sub_name == subscribed {
		// Subscription feed
		if req.uri().path().starts_with("/r/") {
			subreddit(&sub_name, quarantined).await.unwrap_or_default()
		} else {
			Subreddit::default()
		}
	} else {
		// Multireddit, all, popular
		Subreddit {
			name: sub_name.clone(),
			..Subreddit::default()
		}
	};

	let req_url = req.uri().to_string();
	// Return landing page if this post if this is NSFW community but the user
	// has disabled the display of NSFW content or if the instance is SFW-only.
	if sub.nsfw && crate::utils::should_be_nsfw_gated(&req, &req_url) {
		return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
	}

	let mut params = String::from("&raw_json=1");
	if sub_name == "popular" {
		let geo_filter = match GEO_FILTER_MATCH.captures(&query) {
			Some(geo_filter) => geo_filter["region"].to_string(),
			None => "GLOBAL".to_owned(),
		};
		params.push_str(&format!("&geo_filter={geo_filter}"));
	}

	let path = format!("/r/{}/{sort}.json?{}{params}", sub_name.replace('+', "%2B"), req.uri().query().unwrap_or_default());
	let url = String::from(req.uri().path_and_query().map_or("", |val| val.as_str()));
	let redirect_url = url[1..].replace('?', "%3F").replace('&', "%26").replace('+', "%2B");
	let filters = get_filters(&req);

	// If all requested subs are filtered, we don't need to fetch posts.
	if sub_name.split('+').all(|s| filters.contains(s)) {
		Ok(template(&SubredditTemplate {
			sub,
			posts: Vec::new(),
			sort: (sort, param(&path, "t").unwrap_or_default()),
			ends: (param(&path, "after").unwrap_or_default(), String::new()),
			prefs: Preferences::new(&req),
			url,
			redirect_url,
			is_filtered: true,
			all_posts_filtered: false,
			all_posts_hidden_nsfw: false,
			no_posts: false,
		}))
	} else {
		match Post::fetch(&path, quarantined).await {
			Ok((mut posts, after)) => {
				let (_, all_posts_filtered) = filter_posts(&mut posts, &filters);
				let no_posts = posts.is_empty();
				let all_posts_hidden_nsfw = !no_posts && (posts.iter().all(|p| p.flags.nsfw) && setting(&req, "show_nsfw") != "on");
				if sort == "new" {
					posts.sort_by(|a, b| b.created_ts.cmp(&a.created_ts));
					posts.sort_by(|a, b| b.flags.stickied.cmp(&a.flags.stickied));
				}
				Ok(template(&SubredditTemplate {
					sub,
					posts,
					sort: (sort, param(&path, "t").unwrap_or_default()),
					ends: (param(&path, "after").unwrap_or_default(), after),
					prefs: Preferences::new(&req),
					url,
					redirect_url,
					is_filtered: false,
					all_posts_filtered,
					all_posts_hidden_nsfw,
					no_posts,
				}))
			}
			Err(msg) => match msg.as_str() {
				"quarantined" | "gated" => Ok(quarantine(&req, sub_name, &msg)),
				"private" => error(req, &format!("r/{sub_name} is a private community")).await,
				"banned" => error(req, &format!("r/{sub_name} has been banned from Reddit")).await,
				_ => error(req, &msg).await,
			},
		}
	}
}

pub fn quarantine(req: &Request<Body>, sub: String, restriction: &str) -> Response<Body> {
	let wall = WallTemplate {
		title: format!("r/{sub} is {restriction}"),
		msg: "Please click the button below to continue to this subreddit.".to_string(),
		url: req.uri().to_string(),
		sub,
		prefs: Preferences::new(req),
	};

	Response::builder()
		.status(403)
		.header("content-type", "text/html")
		.body(wall.render().unwrap_or_default().into())
		.unwrap_or_default()
}

pub async fn add_quarantine_exception(req: Request<Body>) -> Result<Response<Body>, String> {
	let subreddit = req.param("sub").ok_or("Invalid URL")?;
	let redir = param(&format!("?{}", req.uri().query().unwrap_or_default()), "redir").ok_or("Invalid URL")?;
	let mut response = redirect(&redir);
	response.insert_cookie(
		Cookie::build((&format!("allow_quaran_{}", subreddit.to_lowercase()), "true"))
			.path("/")
			.http_only(true)
			.expires(cookie::Expiration::Session)
			.into(),
	);
	Ok(response)
}

pub fn can_access_quarantine(req: &Request<Body>, sub: &str) -> bool {
	// Determine if the subreddit can be accessed
	setting(req, &format!("allow_quaran_{}", sub.to_lowercase())).parse().unwrap_or_default()
}

// Join items in chunks of 4000 bytes in length for cookies
pub fn join_until_size_limit<T: std::fmt::Display>(vec: &[T]) -> Vec<std::string::String> {
	let mut result = Vec::new();
	let mut list = String::new();
	let mut current_size = 0;

	for item in vec {
		// Size in bytes
		let item_size = item.to_string().len();
		// Use 4000 bytes to leave us some headroom because the name and options of the cookie count towards the 4096 byte cap
		if current_size + item_size > 4000 {
			// If last item add a seperator on the end of the list so it's interpreted properly in tanden with the next cookie
			list.push('+');

			// Push current list to result vector
			result.push(list);

			// Reset the list variable so we can continue with only new items
			list = String::new();
		}
		// Add separator if not the first item
		if !list.is_empty() {
			list.push('+');
		}
		// Add current item to list
		list.push_str(&item.to_string());
		current_size = list.len() + item_size;
	}
	// Make sure to push whatever the remaining subreddits are there into the result vector
	result.push(list);

	// Return resulting vector
	result
}

// Sub, filter, unfilter, or unsub by setting subscription cookie using response "Set-Cookie" header
pub async fn subscriptions_filters(req: Request<Body>) -> Result<Response<Body>, String> {
	let sub = req.param("sub").unwrap_or_default();
	let action: Vec<String> = req.uri().path().split('/').map(String::from).collect();

	// Handle random subreddits
	if sub == "random" || sub == "randnsfw" {
		if action.contains(&"filter".to_string()) || action.contains(&"unfilter".to_string()) {
			return Err("Can't filter random subreddit!".to_string());
		}
		return Err("Can't subscribe to random subreddit!".to_string());
	}

	let query = req.uri().query().unwrap_or_default().to_string();

	let preferences = Preferences::new(&req);
	let mut sub_list = preferences.subscriptions;
	let mut filters = preferences.filters;

	// Retrieve list of posts for these subreddits to extract display names

	let posts = json(format!("/r/{sub}/hot.json?raw_json=1"), true).await;
	let display_lookup: Vec<(String, &str)> = match &posts {
		Ok(posts) => posts["data"]["children"]
			.as_array()
			.map(|list| {
				list
					.iter()
					.map(|post| {
						let display_name = post["data"]["subreddit"].as_str().unwrap_or_default();
						(display_name.to_lowercase(), display_name)
					})
					.collect::<Vec<_>>()
			})
			.unwrap_or_default(),
		Err(_) => vec![],
	};

	// Find each subreddit name (separated by '+') in sub parameter
	for part in sub.split('+').filter(|x| x != &"") {
		// Retrieve display name for the subreddit
		let display;
		let part = if part.starts_with("u_") {
			part
		} else if let Some(&(_, display)) = display_lookup.iter().find(|x| x.0 == part.to_lowercase()) {
			// This is already known, doesn't require separate request
			display
		} else {
			// This subreddit display name isn't known, retrieve it
			let path: String = format!("/r/{part}/about.json?raw_json=1");
			display = json(path, true).await;
			match &display {
				Ok(display) => display["data"]["display_name"].as_str(),
				Err(_) => None,
			}
			.unwrap_or(part)
		};

		// Modify sub list based on action
		if action.contains(&"subscribe".to_string()) && !sub_list.contains(&part.to_owned()) {
			// Add each sub name to the subscribed list
			sub_list.push(part.to_owned());
			filters.retain(|s| s.to_lowercase() != part.to_lowercase());
			// Reorder sub names alphabetically
			sub_list.sort_by_key(|a| a.to_lowercase());
			filters.sort_by_key(|a| a.to_lowercase());
		} else if action.contains(&"unsubscribe".to_string()) {
			// Remove sub name from subscribed list
			sub_list.retain(|s| s.to_lowercase() != part.to_lowercase());
		} else if action.contains(&"filter".to_string()) && !filters.contains(&part.to_owned()) {
			// Add each sub name to the filtered list
			filters.push(part.to_owned());
			sub_list.retain(|s| s.to_lowercase() != part.to_lowercase());
			// Reorder sub names alphabetically
			filters.sort_by_key(|a| a.to_lowercase());
			sub_list.sort_by_key(|a| a.to_lowercase());
		} else if action.contains(&"unfilter".to_string()) {
			// Remove sub name from filtered list
			filters.retain(|s| s.to_lowercase() != part.to_lowercase());
		}
	}

	// Redirect back to subreddit
	// check for redirect parameter if unsubscribing/unfiltering from outside sidebar
	let path = if let Some(redirect_path) = param(&format!("?{query}"), "redirect") {
		format!("/{redirect_path}")
	} else {
		format!("/r/{sub}")
	};

	let mut response = redirect(&path);

	// If sub_list is empty remove all subscriptions cookies, otherwise update them and remove old ones
	if sub_list.is_empty() {
		// Remove subscriptions cookie
		response.remove_cookie("subscriptions".to_string());

		// Start with first numbered subscriptions cookie
		let mut subscriptions_number = 1;

		// While whatever subscriptionsNUMBER cookie we're looking at has a value
		while req.cookie(&format!("subscriptions{subscriptions_number}")).is_some() {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number}"));

			// Increment subscriptions cookie number
			subscriptions_number += 1;
		}
	} else {
		// Start at 0 to keep track of what number we need to start deleting old subscription cookies from
		let mut subscriptions_number_to_delete_from = 0;

		// Starting at 0 so we handle the subscription cookie without a number first
		for (subscriptions_number, list) in join_until_size_limit(&sub_list).into_iter().enumerate() {
			let subscriptions_cookie = if subscriptions_number == 0 {
				"subscriptions".to_string()
			} else {
				format!("subscriptions{subscriptions_number}")
			};

			response.insert_cookie(
				Cookie::build((subscriptions_cookie, list))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			);

			subscriptions_number_to_delete_from += 1;
		}

		// While whatever subscriptionsNUMBER cookie we're looking at has a value
		while req.cookie(&format!("subscriptions{subscriptions_number_to_delete_from}")).is_some() {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number_to_delete_from}"));

			// Increment subscriptions cookie number
			subscriptions_number_to_delete_from += 1;
		}
	}

	// If filters is empty remove all filters cookies, otherwise update them and remove old ones
	if filters.is_empty() {
		// Remove filters cookie
		response.remove_cookie("filters".to_string());

		// Start with first numbered filters cookie
		let mut filters_number = 1;

		// While whatever filtersNUMBER cookie we're looking at has a value
		while req.cookie(&format!("filters{filters_number}")).is_some() {
			// Remove that filters cookie
			response.remove_cookie(format!("filters{filters_number}"));

			// Increment filters cookie number
			filters_number += 1;
		}
	} else {
		// Start at 0 to keep track of what number we need to start deleting old filters cookies from
		let mut filters_number_to_delete_from = 0;

		for (filters_number, list) in join_until_size_limit(&filters).into_iter().enumerate() {
			let filters_cookie = if filters_number == 0 {
				"filters".to_string()
			} else {
				format!("filters{filters_number}")
			};

			response.insert_cookie(
				Cookie::build((filters_cookie, list))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			);

			filters_number_to_delete_from += 1;
		}

		// While whatever filtersNUMBER cookie we're looking at has a value
		while req.cookie(&format!("filters{filters_number_to_delete_from}")).is_some() {
			// Remove that filters cookie
			response.remove_cookie(format!("filters{filters_number_to_delete_from}"));

			// Increment filters cookie number
			filters_number_to_delete_from += 1;
		}
	}

	Ok(response)
}

pub async fn wiki(req: Request<Body>) -> Result<Response<Body>, String> {
	let sub = req.param("sub").unwrap_or_else(|| "reddit.com".to_string());
	let quarantined = can_access_quarantine(&req, &sub);
	// Handle random subreddits
	if let Ok(random) = catch_random(&sub, "/wiki").await {
		return Ok(random);
	}

	let page = req.param("page").unwrap_or_else(|| "index".to_string());
	let path: String = format!("/r/{sub}/wiki/{page}.json?raw_json=1");
	let url = req.uri().to_string();

	match json(path, quarantined).await {
		Ok(response) => Ok(template(&WikiTemplate {
			sub,
			wiki: rewrite_urls(response["data"]["content_html"].as_str().unwrap_or("<h3>Wiki not found</h3>")),
			page,
			prefs: Preferences::new(&req),
			url,
		})),
		Err(msg) => {
			if msg == "quarantined" || msg == "gated" {
				Ok(quarantine(&req, sub, &msg))
			} else {
				error(req, &msg).await
			}
		}
	}
}

pub async fn sidebar(req: Request<Body>) -> Result<Response<Body>, String> {
	let sub = req.param("sub").unwrap_or_else(|| "reddit.com".to_string());
	let quarantined = can_access_quarantine(&req, &sub);

	// Handle random subreddits
	if let Ok(random) = catch_random(&sub, "/about/sidebar").await {
		return Ok(random);
	}

	// Build the Reddit JSON API url
	let path: String = format!("/r/{sub}/about.json?raw_json=1");
	let url = req.uri().to_string();

	// Send a request to the url
	match json(path, quarantined).await {
		// If success, receive JSON in response
		Ok(response) => Ok(template(&WikiTemplate {
			wiki: rewrite_urls(&val(&response, "description_html")),
			// wiki: format!(
			// 	"{}<hr><h1>Moderators</h1><br><ul>{}</ul>",
			// 	rewrite_urls(&val(&response, "description_html"),
			// 	moderators(&sub, quarantined).await.unwrap_or(vec!["Could not fetch moderators".to_string()]).join(""),
			// ),
			sub,
			page: "Sidebar".to_string(),
			prefs: Preferences::new(&req),
			url,
		})),
		Err(msg) => {
			if msg == "quarantined" || msg == "gated" {
				Ok(quarantine(&req, sub, &msg))
			} else {
				error(req, &msg).await
			}
		}
	}
}

// pub async fn moderators(sub: &str, quarantined: bool) -> Result<Vec<String>, String> {
// 	// Retrieve and format the html for the moderators list
// 	Ok(
// 		moderators_list(sub, quarantined)
// 			.await?
// 			.iter()
// 			.map(|m| format!("<li><a style=\"color: var(--accent)\" href=\"/u/{name}\">{name}</a></li>", name = m))
// 			.collect(),
// 	)
// }

// async fn moderators_list(sub: &str, quarantined: bool) -> Result<Vec<String>, String> {
// 	// Build the moderator list URL
// 	let path: String = format!("/r/{}/about/moderators.json?raw_json=1", sub);

// 	// Retrieve response
// 	json(path, quarantined).await.map(|response| {
// 		// Traverse json tree and format into list of strings
// 		response["data"]["children"]
// 			.as_array()
// 			.unwrap_or(&Vec::new())
// 			.iter()
// 			.filter_map(|moderator| {
// 				let name = moderator["name"].as_str().unwrap_or_default();
// 				if name.is_empty() {
// 					None
// 				} else {
// 					Some(name.to_string())
// 				}
// 			})
// 			.collect::<Vec<_>>()
// 	})
// }

// SUBREDDIT
async fn subreddit(sub: &str, quarantined: bool) -> Result<Subreddit, String> {
	// Build the Reddit JSON API url
	let path: String = format!("/r/{sub}/about.json?raw_json=1");

	// Send a request to the url
	let res = json(path, quarantined).await?;

	// Metadata regarding the subreddit
	let members: i64 = res["data"]["subscribers"].as_u64().unwrap_or_default() as i64;
	let active: i64 = res["data"]["accounts_active"].as_u64().unwrap_or_default() as i64;

	// Fetch subreddit icon either from the community_icon or icon_img value
	let community_icon: &str = res["data"]["community_icon"].as_str().unwrap_or_default();
	let icon = if community_icon.is_empty() { val(&res, "icon_img") } else { community_icon.to_string() };

	Ok(Subreddit {
		name: val(&res, "display_name"),
		title: val(&res, "title"),
		description: val(&res, "public_description"),
		info: rewrite_urls(&val(&res, "description_html")),
		// moderators: moderators_list(sub, quarantined).await.unwrap_or_default(),
		icon: format_url(&icon),
		members: format_num(members),
		active: format_num(active),
		wiki: res["data"]["wiki_enabled"].as_bool().unwrap_or_default(),
		nsfw: res["data"]["over18"].as_bool().unwrap_or_default(),
	})
}

pub async fn rss(req: Request<Body>) -> Result<Response<Body>, String> {
	if config::get_setting("REDLIB_ENABLE_RSS").is_none() {
		return Ok(error(req, "RSS is disabled on this instance.").await.unwrap_or_default());
	}

	use hyper::header::CONTENT_TYPE;
	use rss::{ChannelBuilder, Item};

	// Get subreddit
	let sub = req.param("sub").unwrap_or_default();
	let post_sort = req.cookie("post_sort").map_or_else(|| "hot".to_string(), |c| c.value().to_string());
	let sort = req.param("sort").unwrap_or_else(|| req.param("id").unwrap_or(post_sort));

	// Get path
	let path = format!("/r/{sub}/{sort}.json?{}", req.uri().query().unwrap_or_default());

	// Get subreddit data
	let subreddit = subreddit(&sub, false).await?;

	// Get posts
	let (posts, _) = Post::fetch(&path, false).await?;

	// Build the RSS feed
	let channel = ChannelBuilder::default()
		.title(&subreddit.title)
		.description(&subreddit.description)
		.items(
			posts
				.into_iter()
				.map(|post| Item {
					title: Some(post.title.to_string()),
					link: Some(format_url(&utils::get_post_url(&post))),
					author: Some(post.author.name),
					content: Some(rewrite_urls(&decode_html(&post.body).unwrap())),
					pub_date: Some(DateTime::from_timestamp(post.created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
					description: Some(format!(
						"<a href='{}{}'>Comments</a>",
						config::get_setting("REDLIB_FULL_URL").unwrap_or_default(),
						post.permalink
					)),
					..Default::default()
				})
				.collect::<Vec<_>>(),
		)
		.build();

	// Serialize the feed to RSS
	let body = channel.to_string().into_bytes();

	// Create the HTTP response
	let mut res = Response::new(Body::from(body));
	res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/rss+xml"));

	Ok(res)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_subreddit() {
	let subreddit = subreddit("rust", false).await;
	assert!(subreddit.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gated_and_quarantined() {
	let quarantined = subreddit("edgy", true).await;
	assert!(quarantined.is_ok());
	let gated = subreddit("drugs", true).await;
	assert!(gated.is_ok());
}
