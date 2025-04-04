#![allow(clippy::cmp_owned)]

// CRATES
use crate::client::json;
use crate::config::get_setting;
use crate::server::RequestExt;
use crate::subreddit::{can_access_quarantine, quarantine};
use crate::utils::{cookie_jar_from_oldreq, error, get_filters, nsfw_landing, param, parse_post, setting, setting_from_cookiejar, template, Comment, Post, Preferences};
use hyper::{Body, Request, Response};

use axum::RequestExt as AxumRequestExt;
use axum_extra::extract::cookie::CookieJar;
use once_cell::sync::Lazy;
use regex::Regex;
use rinja::Template;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use unwrap_infallible::UnwrapInfallible;

// STRUCTS
#[derive(Template)]
#[template(path = "post.html")]
struct PostTemplate {
	comments: Vec<Comment>,
	post: Post,
	sort: String,
	prefs: Preferences,
	single_thread: bool,
	url: String,
	url_without_query: String,
	comment_query: String,
}

static COMMENT_SEARCH_CAPTURE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\?q=(.*)&type=comment").unwrap());

pub async fn item(req: Request<Body>) -> Result<Response<Body>, String> {
	// Build Reddit API path
	let mut path: String = format!("{}.json?{}&raw_json=1", req.uri().path(), req.uri().query().unwrap_or_default());
	let sub = req.param("sub").unwrap_or_default();
	let quarantined = can_access_quarantine(&req, &sub);
	let url = req.uri().to_string();

	// Set sort to sort query parameter
	let sort = param(&path, "sort").unwrap_or_else(|| {
		// Grab default comment sort method from Cookies
		let default_sort = setting(&req, "comment_sort");

		// If there's no sort query but there's a default sort, set sort to default_sort
		if default_sort.is_empty() {
			String::new()
		} else {
			path = format!("{}.json?{}&sort={}&raw_json=1", req.uri().path(), req.uri().query().unwrap_or_default(), default_sort);
			default_sort
		}
	});

	// Log the post ID being fetched in debug mode
	#[cfg(debug_assertions)]
	req.param("id").unwrap_or_default();

	let single_thread = req.param("comment_id").is_some();
	let highlighted_comment = &req.param("comment_id").unwrap_or_default();

	// Send a request to the url, receive JSON in response
	match json(path, quarantined).await {
		// Otherwise, grab the JSON output from the request
		Ok(response) => {
			// Parse the JSON into Post and Comment structs
			let post = parse_post(&response[0]["data"]["children"][0]).await;

			let req_url = req.uri().to_string();
			// Return landing page if this post if this Reddit deems this post
			// NSFW, but we have also disabled the display of NSFW content
			// or if the instance is SFW-only.
			if post.nsfw && crate::utils::should_be_nsfw_gated(&req, &req_url) {
				return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
			}

			let query_body = match COMMENT_SEARCH_CAPTURE.captures(&url) {
				Some(captures) => captures.get(1).unwrap().as_str().replace("%20", " ").replace('+', " "),
				None => String::new(),
			};

			let query_string = format!("q={query_body}&type=comment");
			let form = url::form_urlencoded::parse(query_string.as_bytes()).collect::<HashMap<_, _>>();
			let query = form.get("q").unwrap().clone().to_string();

			let comments = match query.as_str() {
				"" => parse_comments(
					&response[1],
					&post.permalink,
					&post.author.name,
					highlighted_comment,
					&get_filters(&req),
					&cookie_jar_from_oldreq(&req),
				),
				_ => query_comments(
					&response[1],
					&post.permalink,
					&post.author.name,
					highlighted_comment,
					&get_filters(&req),
					&query,
					&cookie_jar_from_oldreq(&req),
				),
			};

			// Use the Post and Comment structs to generate a website to show users
			Ok(template(&PostTemplate {
				comments,
				post,
				url_without_query: url.clone().trim_end_matches(&format!("?q={query}&type=comment")).to_string(),
				sort,
				prefs: Preferences::new(&req),
				single_thread,
				url: req_url,
				comment_query: query,
			}))
		}
		// If the Reddit API returns an error, exit and send error page to user
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

// COMMENTS

/// A Vec of all comments defined in a json response
fn parse_comments(json: &serde_json::Value, post_link: &str, post_author: &str, highlighted_comment: &str, filters: &HashSet<String>, cookies: &CookieJar) -> Vec<Comment> {
	let comments = json["data"]["children"].as_array();
	if let Some(comments) = comments {
		comments
			.into_iter()
			.map(|comment| {
				let data = &comment["data"];
				let replies: Vec<Comment> = if data["replies"].is_object() {
					parse_comments(&data["replies"], post_link, post_author, highlighted_comment, filters, cookies)
				} else {
					Vec::new()
				};
				Comment::build(&comment, data, replies, post_link, post_author, highlighted_comment, filters, cookies)
			})
			.collect()
	} else {
		Vec::new()
	}
}

/// like parse_comments, but filters comment body by query parameter.
fn query_comments(
	json: &serde_json::Value,
	post_link: &str,
	post_author: &str,
	highlighted_comment: &str,
	filters: &HashSet<String>,
	query: &str,
	cookies: &CookieJar,
) -> Vec<Comment> {
	let query_lc = query.to_lowercase();
	parse_comments(json, post_link, post_author, highlighted_comment, filters, cookies)
		.into_iter()
		.filter(|c| c.body.to_lowercase().contains(&query_lc))
		.collect()
}
