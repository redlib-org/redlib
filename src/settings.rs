#![allow(clippy::cmp_owned)]

use std::collections::HashMap;

// CRATES
use crate::server::ResponseExt;
use crate::subreddit::join_until_size_limit;
use crate::utils::{redirect, template, Preferences};
use cookie::Cookie;
use futures_lite::StreamExt;
use hyper::{Body, Request, Response};
use rinja::Template;
use time::{Duration, OffsetDateTime};

// STRUCTS
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
	prefs: Preferences,
	url: String,
}

// CONSTANTS

const PREFS: [&str; 18] = [
	"theme",
	"front_page",
	"layout",
	"wide",
	"comment_sort",
	"post_sort",
	"blur_spoiler",
	"show_nsfw",
	"blur_nsfw",
	"use_hls",
	"hide_hls_notification",
	"autoplay_videos",
	"hide_sidebar_and_summary",
	"fixed_navbar",
	"hide_awards",
	"hide_score",
	"disable_visit_reddit_confirmation",
	"video_quality",
];

// FUNCTIONS

// Retrieve cookies from request "Cookie" header
pub async fn get(req: Request<Body>) -> Result<Response<Body>, String> {
	let url = req.uri().to_string();
	Ok(template(&SettingsTemplate {
		prefs: Preferences::new(&req),
		url,
	}))
}

// Set cookies using response "Set-Cookie" header
pub async fn set(req: Request<Body>) -> Result<Response<Body>, String> {
	// Split the body into parts
	let (parts, mut body) = req.into_parts();

	// Grab existing cookies
	let _cookies: Vec<Cookie<'_>> = parts
		.headers
		.get_all("Cookie")
		.iter()
		.filter_map(|header| Cookie::parse(header.to_str().unwrap_or_default()).ok())
		.collect();

	// Aggregate the body...
	// let whole_body = hyper::body::aggregate(req).await.map_err(|e| e.to_string())?;
	let body_bytes = body
		.try_fold(Vec::new(), |mut data, chunk| {
			data.extend_from_slice(&chunk);
			Ok(data)
		})
		.await
		.map_err(|e| e.to_string())?;

	let form = url::form_urlencoded::parse(&body_bytes).collect::<HashMap<_, _>>();

	let mut response = redirect("/settings");

	for &name in &PREFS {
		match form.get(name) {
			Some(value) => response.insert_cookie(
				Cookie::build((name.to_owned(), value.clone()))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			),
			None => response.remove_cookie(name.to_string()),
		};
	}

	Ok(response)
}

fn set_cookies_method(req: Request<Body>, remove_cookies: bool) -> Response<Body> {
	// Split the body into parts
	let (parts, _) = req.into_parts();

	// Grab existing cookies
	let _cookies: Vec<Cookie<'_>> = parts
		.headers
		.get_all("Cookie")
		.iter()
		.filter_map(|header| Cookie::parse(header.to_str().unwrap_or_default()).ok())
		.collect();

	let query = parts.uri.query().unwrap_or_default().as_bytes();

	let form = url::form_urlencoded::parse(query).collect::<HashMap<_, _>>();

	let path = match form.get("redirect") {
		Some(value) => format!("/{}", value.replace("%26", "&").replace("%23", "#")),
		None => "/".to_string(),
	};

	let mut response = redirect(&path);

	for name in PREFS {
		match form.get(name) {
			Some(value) => response.insert_cookie(
				Cookie::build((name.to_owned(), value.clone()))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			),
			None => {
				if remove_cookies {
					response.remove_cookie(name.to_string());
				}
			}
		};
	}

	// Get subscriptions/filters to restore from query string
	let subscriptions = form.get("subscriptions");
	let filters = form.get("filters");

	// We can't search through the cookies directly like in subreddit.rs, so instead we have to make a string out of the request's headers to search through
	let cookies_string = parts
		.headers
		.get("cookie")
		.map(|hv| hv.to_str().unwrap_or("").to_string()) // Return String
		.unwrap_or_else(String::new); // Return an empty string if None

	// If there are subscriptions to restore set them and delete any old subscriptions cookies, otherwise delete them all
	if subscriptions.is_some() {
		let sub_list: Vec<String> = subscriptions.expect("Subscriptions").split('+').map(str::to_string).collect();

		// Start at 0 to keep track of what number we need to start deleting old subscription cookies from
		let mut subscriptions_number_to_delete_from = 0;

		// Starting at 0 so we handle the subscription cookie without a number first
		for (subscriptions_number, list) in join_until_size_limit(&sub_list).into_iter().enumerate() {
			let subscriptions_cookie = if subscriptions_number == 0 {
				"subscriptions".to_string()
			} else {
				format!("subscriptions{}", subscriptions_number)
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

		// While subscriptionsNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("subscriptions{subscriptions_number_to_delete_from}=")) {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number_to_delete_from}"));

			// Increment subscriptions cookie number
			subscriptions_number_to_delete_from += 1;
		}
	} else {
		// Remove unnumbered subscriptions cookie
		response.remove_cookie("subscriptions".to_string());

		// Starts at one to deal with the first numbered subscription cookie and onwards
		let mut subscriptions_number_to_delete_from = 1;

		// While subscriptionsNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("subscriptions{subscriptions_number_to_delete_from}=")) {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number_to_delete_from}"));

			// Increment subscriptions cookie number
			subscriptions_number_to_delete_from += 1;
		}
	}

	// If there are filters to restore set them and delete any old filters cookies, otherwise delete them all
	if filters.is_some() {
		let filters_list: Vec<String> = filters.expect("Filters").split('+').map(str::to_string).collect();

		// Start at 0 to keep track of what number we need to start deleting old subscription cookies from
		let mut filters_number_to_delete_from = 0;

		// Starting at 0 so we handle the subscription cookie without a number first
		for (filters_number, list) in join_until_size_limit(&filters_list).into_iter().enumerate() {
			let filters_cookie = if filters_number == 0 {
				"filters".to_string()
			} else {
				format!("filters{}", filters_number)
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

		// While filtersNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("filters{filters_number_to_delete_from}=")) {
			// Remove that filters cookie
			response.remove_cookie(format!("filters{filters_number_to_delete_from}"));

			// Increment filters cookie number
			filters_number_to_delete_from += 1;
		}
	} else {
		// Remove unnumbered filters cookie
		response.remove_cookie("filters".to_string());

		// Starts at one to deal with the first numbered subscription cookie and onwards
		let mut filters_number_to_delete_from = 1;

		// While filtersNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("filters{filters_number_to_delete_from}=")) {
			// Remove that sfilters cookie
			response.remove_cookie(format!("filters{filters_number_to_delete_from}"));

			// Increment filters cookie number
			filters_number_to_delete_from += 1;
		}
	}

	response
}

// Set cookies using response "Set-Cookie" header
pub async fn restore(req: Request<Body>) -> Result<Response<Body>, String> {
	Ok(set_cookies_method(req, true))
}

pub async fn update(req: Request<Body>) -> Result<Response<Body>, String> {
	Ok(set_cookies_method(req, false))
}
