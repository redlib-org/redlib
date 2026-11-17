#![allow(clippy::cmp_owned)]

use crate::body::Body;
use crate::utils::{
	build_rss_items, catch_random, clean_url, enable_rss, error, filter_posts, format_num, format_url, get_filters, info, nsfw_landing, param, redirect, rewrite_urls,
	rss_page_link, rss_page_path, rss_response, setting, setting_or_default, should_be_nsfw_gated, template, val, Link, Post, Preferences, Subreddit, Widget, RSS_LISTING_SORTS,
};
use crate::{backends::json, server::RequestExt, server::ResponseExt};
use askama::Template;
use cookie::Cookie;
use hyper::{Request, Response};
use rss::ChannelBuilder;
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

// SERVICES
pub async fn community(req: Request<Body>) -> Result<Response<Body>, String> {
	let prefs = Preferences::new(&req);
	// Build Reddit API path
	let root = req.uri().path() == "/";
	let subscribed = setting(&req, "subscriptions");
	let front_page = setting(&req, "front_page");
	let remove_default_feeds = setting(&req, "remove_default_feeds") == "on";
	let post_sort = setting_or_default(&req, "post_sort", "hot".to_string());
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
		// Regular subreddit, with new Reddit's sidebar fetched alongside
		let (sub, widgets) = tokio::join!(subreddit(&sub_name, quarantined), widgets(&sub_name, quarantined));
		Subreddit {
			widgets,
			..sub.unwrap_or_default()
		}
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
	if sub.nsfw && should_be_nsfw_gated(&req, &req_url) {
		return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
	}

	let mut params = format!("&raw_json=1{}", default_timeframe(&sort, req.uri().query().unwrap_or_default()));
	if sub_name == "popular" {
		params.push_str(&format!("&geo_filter={}", prefs.geo_filter));
	}

	let posts_per_page: u32 = setting(&req, "posts_per_page").parse().unwrap_or(25).clamp(1, 100);
	if posts_per_page != 25 {
		params.push_str(&format!("&limit={}", posts_per_page));
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
			prefs,
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
				let clean_urls = setting(&req, "clean_urls");
				if clean_urls == "on".to_owned() {
					posts.iter_mut().for_each(|post| post.media.url = clean_url(post.media.url.clone()));
				}
				let (_, all_posts_filtered) = filter_posts(&mut posts, &filters);
				let no_posts = posts.is_empty();
				let all_posts_hidden_nsfw = !no_posts && (posts.iter().all(|p| p.flags.nsfw) && setting(&req, "show_nsfw") != "on");
				if sort == "new" {
					posts.sort_by_key(|post| std::cmp::Reverse(post.created_ts));
					posts.sort_by_key(|post| std::cmp::Reverse(post.flags.stickied));
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

fn wiki_html(response: &serde_json::Value, page: &str) -> String {
	let missing = if page == "index" {
		"<h3>This subreddit has no wiki pages</h3>"
	} else {
		"<h3>Wiki page not found</h3>"
	};
	rewrite_urls(response["data"]["content_html"].as_str().unwrap_or(missing))
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
			wiki: wiki_html(&response, &page),
			sub,
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

/// Community lists longer than this fold the rest away.
const FOLD_COMMUNITIES_AFTER: usize = 5;

/// New Reddit's sidebar, empty when Reddit doesn't send one.
async fn widgets(sub: &str, quarantined: bool) -> Vec<Widget> {
	json(format!("/r/{sub}/api/widgets?raw_json=1"), quarantined)
		.await
		.map(|res| parse_widgets(&res))
		.unwrap_or_default()
}

/// The sidebar widgets in the order Reddit lays them out. Kinds Redlib can't
/// show (calendars, custom CSS, post flair) are left out.
fn parse_widgets(res: &serde_json::Value) -> Vec<Widget> {
	let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
	let html = |v: &serde_json::Value| rewrite_urls(v.as_str().unwrap_or_default());
	let items = &res["items"];
	let order = res["layout"]["topbar"]["order"]
		.as_array()
		.into_iter()
		.chain(res["layout"]["sidebar"]["order"].as_array())
		.flatten();
	order
		.filter_map(|id| {
			let widget = &items[id.as_str()?];
			let title = text(&widget["shortName"]);
			let data = widget["data"].as_array().map(Vec::as_slice).unwrap_or_default();
			let widget = match widget["kind"].as_str()? {
				"textarea" => Widget::Text {
					title,
					html: html(&widget["textHtml"]),
				},
				"subreddit-rules" => Widget::Rules {
					title,
					rules: data
						.iter()
						.map(|rule| {
							let description = html(&rule["descriptionHtml"]);
							(text(&rule["shortName"]), if has_text(&description) { description } else { String::new() })
						})
						.collect(),
				},
				"community-list" => {
					let mut communities: Vec<crate::search::Subreddit> = data
						.iter()
						.map(|community| {
							let icon = [&community["communityIcon"], &community["iconUrl"]]
								.into_iter()
								.find_map(|v| v.as_str().filter(|s| !s.is_empty()))
								.unwrap_or_default();
							let name = text(&community["name"]);
							crate::search::Subreddit {
								url: format!("/r/{name}"),
								name,
								icon: format_url(icon),
								description: String::new(),
								subscribers: format_num(community["subscribers"].as_f64().unwrap_or_default() as i64),
							}
						})
						.collect();
					// folding away a single row saves nothing
					let more = if communities.len() > FOLD_COMMUNITIES_AFTER + 1 {
						communities.split_off(FOLD_COMMUNITIES_AFTER)
					} else {
						Vec::new()
					};
					Widget::Communities { title, communities, more }
				}
				"image" => {
					let image = data.last()?;
					Widget::Image {
						title,
						url: format_url(image["url"].as_str()?),
						link: image["linkUrl"].as_str().filter(|s| !s.is_empty()).map(format_url),
					}
				}
				"menu" | "button" => {
					// buttons keep theirs under `buttons`, menus under `data`
					let entries = widget["buttons"].as_array().map(Vec::as_slice).unwrap_or(data);
					let link = |v: &serde_json::Value| Some((text(&v["text"]), format_url(v["url"].as_str().filter(|u| !u.is_empty())?)));
					let links: Vec<_> = entries
						.iter()
						.filter_map(|entry| {
							let children: Vec<_> = entry["children"].as_array().into_iter().flatten().filter_map(link).collect();
							let url = entry["url"].as_str().filter(|u| !u.is_empty()).map(format_url).unwrap_or_default();
							(!url.is_empty() || !children.is_empty()).then(|| Link {
								text: text(&entry["text"]),
								url,
								children,
							})
						})
						.collect();
					Widget::Links { title, links }
				}
				_ => return None,
			};
			Some(widget)
		})
		.filter(|widget| !matches!(widget, Widget::Links { links, .. } if links.is_empty()))
		.collect()
}

/// Whether `html` shows any text, so `<div class="md"></div>` counts as empty.
fn has_text(html: &str) -> bool {
	let mut in_tag = false;
	html.chars().any(|c| match c {
		'<' => {
			in_tag = true;
			false
		}
		'>' => {
			in_tag = false;
			false
		}
		_ => !in_tag && !c.is_whitespace(),
	})
}

// SUBREDDIT
async fn subreddit(sub: &str, quarantined: bool) -> Result<Subreddit, String> {
	// Build the Reddit JSON API url
	let path: String = format!("/r/{sub}/about.json?raw_json=1");

	// Send a request to the url
	let res = json(path, quarantined).await?;
	Ok(subreddit_from(&res))
}

/// A subreddit from its `about.json`.
fn subreddit_from(res: &serde_json::Value) -> Subreddit {
	// Reddit sends big counts as floats, e.g. `31604051.0` for r/news
	let members = res["data"]["subscribers"].as_f64().unwrap_or_default() as i64;
	let count = |key| res["data"][key].as_f64().map(|n| format_num(n as i64));

	// Fetch subreddit icon either from the community_icon or icon_img value
	let community_icon: &str = res["data"]["community_icon"].as_str().unwrap_or_default();
	let icon = if community_icon.is_empty() { val(res, "icon_img") } else { community_icon.to_string() };

	Subreddit {
		name: val(res, "display_name"),
		title: val(res, "title"),
		description: val(res, "public_description"),
		info: rewrite_urls(&val(res, "description_html")),
		// moderators: moderators_list(sub, quarantined).await.unwrap_or_default(),
		icon: format_url(&icon),
		members: format_num(members),
		weekly_visitors: count("weekly_active_users"),
		weekly_contributions: count("weekly_contributions"),
		wiki: res["data"]["wiki_enabled"].as_bool().unwrap_or_default(),
		nsfw: res["data"]["over18"].as_bool().unwrap_or_default(),
		widgets: Vec::new(),
	}
}

/// Feed for a subreddit, multireddit or the front page, built from the JSON API.
/// Mirrors Reddit's feed URLs: `/r/rust.rss`, `/r/rust/top.rss?t=year`, `/r/rust/comments.rss`, `/top.rss`.
pub async fn rss(req: Request<Body>) -> Result<Response<Body>, String> {
	if !enable_rss() {
		return Ok(error(req, "RSS is disabled on this instance.").await.unwrap_or_default());
	}

	// Front page feeds have no subreddit: the sort is the whole path (`/top.rss`)
	let (sub, sort) = match req.param("sub") {
		Some(sub) => (sub, req.param("sort")),
		None => {
			let front_page = setting(&req, "front_page");
			let subscribed = setting(&req, "subscriptions");
			let sub = if !front_page.is_empty() && front_page != "default" {
				front_page
			} else if !subscribed.is_empty() {
				subscribed
			} else {
				"popular".to_string()
			};
			let sort = rss_page_path(&req).trim_start_matches('/').to_string();
			(sub, Some(sort).filter(|s| !s.is_empty()))
		}
	};
	let sort = sort.unwrap_or_else(|| Some(setting(&req, "post_sort")).filter(|s| !s.is_empty()).unwrap_or_else(|| "hot".to_string()));
	if sort != "comments" && !RSS_LISTING_SORTS.contains(&sort.as_str()) {
		return error(req, "Unknown feed sort").await;
	}

	// Only real subreddits have metadata; multireddits, r/all and r/popular don't
	let subreddit = if sub.contains('+') || sub == "all" || sub == "popular" {
		Subreddit {
			name: sub.clone(),
			title: format!("r/{sub}"),
			..Subreddit::default()
		}
	} else {
		subreddit(&sub, false).await?
	};
	if subreddit.nsfw && crate::utils::sfw_only() {
		return error(req, "NSFW content is disabled on this instance.").await;
	}

	let mut params = format!("&raw_json=1{}", default_timeframe(&sort, req.uri().query().unwrap_or_default()));
	if sub == "popular" {
		params.push_str(&format!("&geo_filter={}", Preferences::new(&req).geo_filter));
	}
	let path = format!("/r/{}/{sort}.json?{}{params}", sub.replace('+', "%2B"), req.uri().query().unwrap_or_default());
	let (posts, _) = Post::fetch(&path, false).await?;

	let title = match sort.as_str() {
		"comments" => format!("Comments in {}", subreddit.title),
		_ => subreddit.title.clone(),
	};
	let channel = ChannelBuilder::default()
		.title(title)
		.description(&subreddit.description)
		.link(rss_page_link(&req))
		.items(build_rss_items(&posts))
		.build();

	Ok(rss_response(&channel))
}

/// Reddit's API reads a missing `t` as all time, while reddit.com and the timeframe menu show today.
fn default_timeframe(sort: &str, query: &str) -> &'static str {
	if matches!(sort, "top" | "controversial") && param(&format!("?{query}"), "t").unwrap_or_default().is_empty() {
		"&t=day"
	} else {
		""
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn test_top_defaults_to_today() {
		assert_eq!(default_timeframe("top", ""), "&t=day");
		assert_eq!(default_timeframe("controversial", "t=&after=t3_x"), "&t=day");
		assert_eq!(default_timeframe("top", "after=t3_x"), "&t=day");
		assert_eq!(default_timeframe("top", "t=week"), "");
		assert_eq!(default_timeframe("hot", "t="), "");
	}

	use super::*;

	#[test]
	fn test_wiki_html() {
		let missing = serde_json::json!({ "reason": "PAGE_NOT_CREATED" });
		assert_eq!(wiki_html(&missing, "index"), "<h3>This subreddit has no wiki pages</h3>");
		assert_eq!(wiki_html(&missing, "rules"), "<h3>Wiki page not found</h3>");
		let page = serde_json::json!({ "data": { "content_html": "<p>hi</p>" } });
		assert_eq!(wiki_html(&page, "index"), "<p>hi</p>");
	}

	#[test]
	fn test_member_count_sent_as_float() {
		let about = serde_json::json!({ "data": { "display_name": "news", "subscribers": 31604051.0 } });
		assert_eq!(subreddit_from(&about).members, format_num(31604051));
		let about = serde_json::json!({ "data": { "display_name": "rust", "subscribers": 429352 } });
		assert_eq!(subreddit_from(&about).members, format_num(429352));
	}

	#[test]
	fn test_weekly_stats_only_when_reddit_sends_them() {
		let about = serde_json::json!({ "data": { "weekly_active_users": 4088390, "weekly_contributions": 63142.0 } });
		let sub = subreddit_from(&about);
		assert_eq!((sub.weekly_visitors, sub.weekly_contributions), (Some(format_num(4088390)), Some(format_num(63142))));
		let sub = subreddit_from(&serde_json::json!({ "data": { "accounts_active": 0 } }));
		assert_eq!((sub.weekly_visitors, sub.weekly_contributions), (None, None));
	}

	/// Reddit's `/r/<sub>/api/widgets` answer, saved in `tests/fixtures`.
	fn widgets_fixture(sub: &str) -> serde_json::Value {
		serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/widgets_{sub}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
	}

	#[test]
	fn test_widgets_r_news() {
		let widgets = parse_widgets(&widgets_fixture("news"));
		let kinds: Vec<&str> = widgets
			.iter()
			.map(|w| match w {
				Widget::Text { .. } => "text",
				Widget::Rules { .. } => "rules",
				Widget::Communities { .. } => "communities",
				Widget::Image { .. } => "image",
				Widget::Links { .. } => "links",
			})
			.collect();
		// the menu from the top bar first, then the sidebar in Reddit's order
		assert_eq!(kinds, ["links", "rules", "communities", "text", "text"]);
		let Widget::Rules { rules, .. } = &widgets[1] else { unreachable!() };
		assert_eq!(rules[0].0, "Submissions must be articles about a specific news event");
		let Widget::Communities { communities, more, .. } = &widgets[2] else { unreachable!() };
		// 10 related communities: 5 shown, 5 behind "Show 5 more"
		assert_eq!((communities.len(), more.len()), (5, 5));
		assert!(widgets[1].starts_open() && !widgets[2].starts_open());
		assert_eq!(communities[0].name, "worldnews");
		assert_eq!(communities[0].subscribers, format_num(47691337));
		assert_eq!(communities[0].url, "/r/worldnews");
		assert!(communities[0].icon.starts_with("/style/"), "{}", communities[0].icon);
		let Widget::Links { links, .. } = &widgets[0] else { unreachable!() };
		assert_eq!((links[0].text.as_str(), links[0].url.as_str()), ("Rules", "/r/news/wiki/rules"));
		assert!(parse_widgets(&serde_json::json!({})).is_empty());
	}

	#[test]
	fn test_widgets_r_worldnews_menus_and_buttons() {
		let widgets = parse_widgets(&widgets_fixture("worldnews"));
		// the region menu: entries with only sub-links
		let Widget::Links { links, .. } = &widgets[0] else { panic!("menu first") };
		let africa = links.iter().find(|l| l.text == "Africa").expect("Africa submenu");
		assert!(africa.url.is_empty());
		assert!(
			africa.children.contains(&("r/SouthAfrica".to_string(), "/r/SouthAfrica".to_string())),
			"{:?}",
			africa.children
		);
		// the "Info" button box
		let info = widgets.iter().find_map(|w| match w {
			Widget::Links { title, links } if title == "Info" => Some(links),
			_ => None,
		});
		let info = info.expect("Info buttons");
		assert_eq!(info[0].text, "Rules: Submissions");
		assert_eq!(info[0].url, "/r/worldnews/wiki/rules#wiki_disallowed_submissions");
		// a rule without a description has nothing to expand
		let Widget::Rules { rules, .. } = widgets.iter().find(|w| matches!(w, Widget::Rules { .. })).unwrap() else {
			unreachable!()
		};
		let (name, description) = rules.last().unwrap();
		assert!(name.starts_with("No chatGPT bots"), "{name}");
		assert!(description.is_empty());
		assert!(!rules[0].1.is_empty());
		assert!(!has_text("<!-- SC_OFF --><div class=\"md\">\n</div><!-- SC_ON -->"));
	}

	#[test]
	fn test_fetching_subreddit() {
		crate::client::block_on_shared(async {
			let subreddit = subreddit("rust", false).await;
			assert!(subreddit.is_ok());
		});
	}

	#[test]
	fn test_gated_and_quarantined() {
		crate::client::block_on_shared(async {
			let quarantined = subreddit("edgy", true).await;
			assert!(quarantined.is_ok());
			let gated = subreddit("drugs", true).await;
			assert!(gated.is_ok());
		});
	}
}
