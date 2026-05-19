#![allow(dead_code)]
#![allow(clippy::cmp_owned)]

use crate::config::{self, get_setting};
//
// CRATES
//
use crate::{client::json, server::RequestExt};
use askama::Template;
use cookie::Cookie;
use hyper::{Body, Request, Response};
use libflate::deflate::{Decoder, Encoder};
use log::error;
use regex::Regex;
use revision::revisioned;
use rust_embed::RustEmbed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json_path::{JsonPath, JsonPathExt};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{Read, Write};
use std::str::FromStr;
use std::string::ToString;
use std::sync::LazyLock;
use time::{macros::format_description, Duration, OffsetDateTime};
use url::Url;

/// Write a message to stderr on debug mode. This function is a no-op on
/// release code.
#[macro_export]
macro_rules! dbg_msg {
	($x:expr) => {
		#[cfg(debug_assertions)]
		eprintln!("{}:{}: {}", file!(), line!(), $x.to_string())
	};

	($($x:expr),+) => {
		#[cfg(debug_assertions)]
		dbg_msg!(format!($($x),+))
	};
}

/// Identifies whether or not the page is a subreddit, a user page, or a post.
/// This is used by the NSFW landing template to determine the mesage to convey
/// to the user.
#[derive(PartialEq, Eq)]
pub enum ResourceType {
	Subreddit,
	User,
	Post,
}

/// Post flair with content, background color and foreground color
#[derive(Serialize)]
pub struct Flair {
	pub flair_parts: Vec<FlairPart>,
	pub text: String,
	pub background_color: String,
	pub foreground_color: String,
}

/// Part of flair, either emoji or text
#[derive(Clone, Serialize)]
pub struct FlairPart {
	pub flair_part_type: String,
	pub value: String,
}

impl FlairPart {
	pub fn parse(flair_type: &str, rich_flair: Option<&Vec<Value>>, text_flair: Option<&str>) -> Vec<Self> {
		// Parse type of flair
		match flair_type {
			// If flair contains emojis and text
			"richtext" => match rich_flair {
				Some(rich) => rich
					.iter()
					// For each part of the flair, extract text and emojis
					.map(|part| {
						let value = |name: &str| part[name].as_str().unwrap_or_default();
						Self {
							flair_part_type: value("e").to_string(),
							value: match value("e") {
								"text" => value("t").to_string(),
								"emoji" => format_url(value("u")),
								_ => String::new(),
							},
						}
					})
					.collect::<Vec<Self>>(),
				None => Vec::new(),
			},
			// If flair contains only text
			"text" => match text_flair {
				Some(text) => vec![Self {
					flair_part_type: "text".to_string(),
					value: text.to_string(),
				}],
				None => Vec::new(),
			},
			_ => Vec::new(),
		}
	}
}

#[derive(Serialize)]
pub struct Author {
	pub name: String,
	pub flair: Flair,
	pub distinguished: String,
}

#[derive(Serialize)]
pub struct Poll {
	pub poll_options: Vec<PollOption>,
	pub voting_end_timestamp: (String, String),
	pub total_vote_count: u64,
}

impl Poll {
	pub fn parse(poll_data: &Value) -> Option<Self> {
		poll_data.as_object()?;

		let total_vote_count = poll_data["total_vote_count"].as_u64()?;
		// voting_end_timestamp is in the format of milliseconds
		let voting_end_timestamp = time(poll_data["voting_end_timestamp"].as_f64()? / 1000.0);
		let poll_options = PollOption::parse(&poll_data["options"])?;

		Some(Self {
			poll_options,
			voting_end_timestamp,
			total_vote_count,
		})
	}

	pub fn most_votes(&self) -> u64 {
		self.poll_options.iter().filter_map(|o| o.vote_count).max().unwrap_or(0)
	}
}

#[derive(Serialize)]
pub struct PollOption {
	pub id: u64,
	pub text: String,
	pub vote_count: Option<u64>,
}

impl PollOption {
	pub fn parse(options: &Value) -> Option<Vec<Self>> {
		Some(
			options
				.as_array()?
				.iter()
				.filter_map(|option| {
					// For each poll option

					// we can't just use as_u64() because "id": String("...") and serde would parse it as None
					let id = option["id"].as_str()?.parse::<u64>().ok()?;
					let text = option["text"].as_str()?.to_owned();
					let vote_count = option["vote_count"].as_u64();

					// Construct PollOption items
					Some(Self { id, text, vote_count })
				})
				.collect::<Vec<Self>>(),
		)
	}
}

/// Post flags with NSFW and stickied
#[derive(Serialize)]
pub struct Flags {
	pub spoiler: bool,
	pub nsfw: bool,
	pub stickied: bool,
}

#[derive(Debug, Serialize)]
pub struct Media {
	pub url: String,
	pub alt_url: String,
	pub width: i64,
	pub height: i64,
	pub poster: String,
	pub download_name: String,
}

impl Media {
	pub async fn parse(data: &Value) -> (String, Self, Vec<GalleryMedia>) {
		let mut gallery = Vec::new();

		// Define the various known places that Reddit might put video URLs.
		let data_preview = &data["preview"]["reddit_video_preview"];
		let secure_media = &data["secure_media"]["reddit_video"];
		let crosspost_parent_media = &data["crosspost_parent_list"][0]["secure_media"]["reddit_video"];

		// If post is a video, return the video
		let (post_type, url_val, alt_url_val) = if data_preview["fallback_url"].is_string() {
			(
				if data_preview["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&data_preview["fallback_url"],
				Some(&data_preview["hls_url"]),
			)
		} else if secure_media["fallback_url"].is_string() {
			(
				if secure_media["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&secure_media["fallback_url"],
				Some(&secure_media["hls_url"]),
			)
		} else if crosspost_parent_media["fallback_url"].is_string() {
			(
				if crosspost_parent_media["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&crosspost_parent_media["fallback_url"],
				Some(&crosspost_parent_media["hls_url"]),
			)
		} else if data["post_hint"].as_str().unwrap_or("") == "image" {
			// Handle images, whether GIFs or pics
			let preview = &data["preview"]["images"][0];
			let mp4 = &preview["variants"]["mp4"];

			if mp4.is_object() {
				// Return the mp4 if the media is a gif
				("gif", &mp4["source"]["url"], None)
			} else {
				// Return the picture if the media is an image
				if data["domain"] == "i.redd.it" {
					("image", &data["url"], None)
				} else {
					("image", &preview["source"]["url"], None)
				}
			}
		} else if data["is_self"].as_bool().unwrap_or_default() {
			// If type is self, return permalink
			("self", &data["permalink"], None)
		} else if data["is_gallery"].as_bool().unwrap_or_default() {
			// If this post contains a gallery of images
			gallery = GalleryMedia::parse(&data["gallery_data"]["items"], &data["media_metadata"]);

			("gallery", &data["url"], None)
		} else if data["crosspost_parent_list"][0]["is_gallery"].as_bool().unwrap_or_default() {
			// If this post contains a gallery of images
			gallery = GalleryMedia::parse(
				&data["crosspost_parent_list"][0]["gallery_data"]["items"],
				&data["crosspost_parent_list"][0]["media_metadata"],
			);

			("gallery", &data["url"], None)
		} else if data["is_reddit_media_domain"].as_bool().unwrap_or_default() && data["domain"] == "i.redd.it" {
			// If this post contains a reddit media (image) URL.
			("image", &data["url"], None)
		} else {
			// If type can't be determined, return url
			("link", &data["url"], None)
		};

		let source = &data["preview"]["images"][0]["source"];

		let alt_url = alt_url_val.map_or(String::new(), |val| format_url(val.as_str().unwrap_or_default()));

		let download_name = if post_type == "image" || post_type == "gif" || post_type == "video" {
			let permalink_base = url_path_basename(data["permalink"].as_str().unwrap_or_default());
			let media_url_base = url_path_basename(url_val.as_str().unwrap_or_default());

			format!("redlib_{permalink_base}_{media_url_base}")
		} else {
			String::new()
		};

		(
			post_type.to_string(),
			Self {
				url: format_url(url_val.as_str().unwrap_or_default()),
				alt_url,
				// Note: in the data["is_reddit_media_domain"] path above
				// width and height will be 0.
				width: source["width"].as_i64().unwrap_or_default(),
				height: source["height"].as_i64().unwrap_or_default(),
				poster: format_url(source["url"].as_str().unwrap_or_default()),
				download_name,
			},
			gallery,
		)
	}
}

#[derive(Serialize)]
pub struct GalleryMedia {
	pub url: String,
	pub width: i64,
	pub height: i64,
	pub caption: String,
	pub outbound_url: String,
}

impl GalleryMedia {
	fn parse(items: &Value, metadata: &Value) -> Vec<Self> {
		items
			.as_array()
			.unwrap_or(&Vec::new())
			.iter()
			.map(|item| {
				// For each image in gallery
				let media_id = item["media_id"].as_str().unwrap_or_default();
				let image = &metadata[media_id]["s"];
				let image_type = &metadata[media_id]["m"];

				let url = if image_type == "image/gif" {
					image["gif"].as_str().unwrap_or_default()
				} else {
					image["u"].as_str().unwrap_or_default()
				};

				// Construct gallery items
				Self {
					url: format_url(url),
					width: image["x"].as_i64().unwrap_or_default(),
					height: image["y"].as_i64().unwrap_or_default(),
					caption: item["caption"].as_str().unwrap_or_default().to_string(),
					outbound_url: item["outbound_url"].as_str().unwrap_or_default().to_string(),
				}
			})
			.collect::<Vec<Self>>()
	}
}

/// Post containing content, metadata and media
#[derive(Serialize)]
pub struct Post {
	pub id: String,
	pub title: String,
	pub community: String,
	pub body: String,
	pub author: Author,
	pub permalink: String,
	pub link_title: String,
	pub poll: Option<Poll>,
	pub score: (String, String),
	pub upvote_ratio: i64,
	pub post_type: String,
	pub flair: Flair,
	pub flags: Flags,
	pub thumbnail: Media,
	pub media: Media,
	pub domain: String,
	pub rel_time: String,
	pub created: String,
	pub created_ts: u64,
	pub num_duplicates: u64,
	pub comments: (String, String),
	pub gallery: Vec<GalleryMedia>,
	pub awards: Awards,
	pub nsfw: bool,
	pub out_url: Option<String>,
	pub ws_url: String,
}

impl Post {
	/// Fetch posts of a user or subreddit and return a vector of posts and the "after" value
	pub async fn fetch(path: &str, quarantine: bool) -> Result<(Vec<Self>, String), String> {
		// Send a request to the url
		let res = match json(path.to_string(), quarantine).await {
			// If success, receive JSON in response
			Ok(response) => response,
			// If the Reddit API returns an error, exit this function
			Err(msg) => return Err(msg),
		};

		// Fetch the list of posts from the JSON response
		let Some(post_list) = res["data"]["children"].as_array() else {
			return Err("No posts found".to_string());
		};

		let mut posts: Vec<Self> = Vec::new();

		// For each post from posts list
		for post in post_list {
			let data = &post["data"];

			let (rel_time, created) = time(data["created_utc"].as_f64().unwrap_or_default());
			let created_ts = data["created_utc"].as_f64().unwrap_or_default().round() as u64;
			let score = data["score"].as_i64().unwrap_or_default();
			let ratio: f64 = data["upvote_ratio"].as_f64().unwrap_or(1.0) * 100.0;
			let title = val(post, "title");

			// Determine the type of media along with the media URL
			let (post_type, media, gallery) = Media::parse(data).await;
			let awards = Awards::parse(&data["all_awardings"]);

			// selftext_html is set for text posts when browsing.
			let mut body = rewrite_urls(&val(post, "selftext_html"));
			if body.is_empty() {
				body = rewrite_urls(&val(post, "body_html"));
			}

			posts.push(Self {
				id: val(post, "id"),
				title,
				community: val(post, "subreddit"),
				body,
				author: Author {
					name: val(post, "author"),
					flair: Flair {
						flair_parts: FlairPart::parse(
							data["author_flair_type"].as_str().unwrap_or_default(),
							data["author_flair_richtext"].as_array(),
							data["author_flair_text"].as_str(),
						),
						text: val(post, "link_flair_text"),
						background_color: val(post, "author_flair_background_color"),
						foreground_color: val(post, "author_flair_text_color"),
					},
					distinguished: val(post, "distinguished"),
				},
				score: if data["hide_score"].as_bool().unwrap_or_default() {
					("\u{2022}".to_string(), "Hidden".to_string())
				} else {
					format_num(score)
				},
				upvote_ratio: ratio as i64,
				post_type,
				thumbnail: Media {
					url: format_url(val(post, "thumbnail").as_str()),
					alt_url: String::new(),
					width: data["thumbnail_width"].as_i64().unwrap_or_default(),
					height: data["thumbnail_height"].as_i64().unwrap_or_default(),
					poster: String::new(),
					download_name: String::new(),
				},
				media,
				domain: val(post, "domain"),
				flair: Flair {
					flair_parts: FlairPart::parse(
						data["link_flair_type"].as_str().unwrap_or_default(),
						data["link_flair_richtext"].as_array(),
						data["link_flair_text"].as_str(),
					),
					text: val(post, "link_flair_text"),
					background_color: val(post, "link_flair_background_color"),
					foreground_color: if val(post, "link_flair_text_color") == "dark" {
						"black".to_string()
					} else {
						"white".to_string()
					},
				},
				flags: Flags {
					spoiler: data["spoiler"].as_bool().unwrap_or_default(),
					nsfw: data["over_18"].as_bool().unwrap_or_default(),
					stickied: data["stickied"].as_bool().unwrap_or_default() || data["pinned"].as_bool().unwrap_or_default(),
				},
				permalink: val(post, "permalink"),
				link_title: val(post, "link_title"),
				poll: Poll::parse(&data["poll_data"]),
				rel_time,
				created,
				created_ts,
				num_duplicates: post["data"]["num_duplicates"].as_u64().unwrap_or(0),
				comments: format_num(data["num_comments"].as_i64().unwrap_or_default()),
				gallery,
				awards,
				nsfw: post["data"]["over_18"].as_bool().unwrap_or_default(),
				ws_url: val(post, "websocket_url"),
				out_url: post["data"]["url_overridden_by_dest"].as_str().map(|a| a.to_string()),
			});
		}
		Ok((posts, res["data"]["after"].as_str().unwrap_or_default().to_string()))
	}
}

#[derive(Template)]
#[template(path = "comment.html")]
/// Comment with content, post, score and data/time that it was posted
pub struct Comment {
	pub id: String,
	pub kind: String,
	pub parent_id: String,
	pub parent_kind: String,
	pub post_link: String,
	pub post_author: String,
	pub body: String,
	pub author: Author,
	pub score: (String, String),
	pub rel_time: String,
	pub created: String,
	pub edited: (String, String),
	pub replies: Vec<Comment>,
	pub highlighted: bool,
	pub awards: Awards,
	pub collapsed: bool,
	pub is_filtered: bool,
	pub more_count: i64,
	pub prefs: Preferences,
}

#[derive(Default, Clone, Serialize)]
pub struct Award {
	pub name: String,
	pub icon_url: String,
	pub description: String,
	pub count: i64,
}

impl std::fmt::Display for Award {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{} {} {}", self.name, self.icon_url, self.description)
	}
}

#[derive(Serialize)]
pub struct Awards(pub Vec<Award>);

impl std::ops::Deref for Awards {
	type Target = Vec<Award>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl std::fmt::Display for Awards {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.iter().try_fold((), |_, award| writeln!(f, "{award}"))
	}
}

impl Awards {
	/// Convert Reddit awards JSON to Awards struct
	pub fn parse(items: &Value) -> Self {
		let parsed = items.as_array().unwrap_or(&Vec::new()).iter().fold(Vec::new(), |mut awards, item| {
			let name = item["name"].as_str().unwrap_or_default().to_string();
			let icon_url = format_url(item["resized_icons"][0]["url"].as_str().unwrap_or_default());
			let description = item["description"].as_str().unwrap_or_default().to_string();
			let count: i64 = i64::from_str(&item["count"].to_string()).unwrap_or(1);

			awards.push(Award {
				name,
				icon_url,
				description,
				count,
			});

			awards
		});

		Self(parsed)
	}
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
	pub msg: String,
	pub prefs: Preferences,
	pub url: String,
}

#[derive(Template)]
#[template(path = "info.html")]
pub struct InfoTemplate {
	pub msg: String,
	pub prefs: Preferences,
	pub url: String,
}

/// Template for NSFW landing page. The landing page is displayed when a page's
/// content is wholly NSFW, but a user has not enabled the option to view NSFW
/// posts.
#[derive(Template)]
#[template(path = "nsfwlanding.html")]
pub struct NSFWLandingTemplate {
	/// Identifier for the resource. This is either a subreddit name or a
	/// username. (In the case of the latter, set is_user to true.)
	pub res: String,

	/// Identifies whether or not the resource is a subreddit, a user page,
	/// or a post.
	pub res_type: ResourceType,

	/// User preferences.
	pub prefs: Preferences,

	/// Request URL.
	pub url: String,
}

#[derive(Default)]
/// User struct containing metadata about user
pub struct User {
	pub name: String,
	pub title: String,
	pub icon: String,
	pub karma: i64,
	pub created: String,
	pub banner: String,
	pub description: String,
	pub nsfw: bool,
}

#[derive(Default)]
/// Subreddit struct containing metadata about community
pub struct Subreddit {
	pub name: String,
	pub title: String,
	pub description: String,
	pub info: String,
	// pub moderators: Vec<String>,
	pub icon: String,
	pub members: (String, String),
	pub active: (String, String),
	pub wiki: bool,
	pub nsfw: bool,
}

/// Parser for query params, used in sorting (eg. /r/rust/?sort=hot)
#[derive(serde::Deserialize)]
pub struct Params {
	pub t: Option<String>,
	pub q: Option<String>,
	pub sort: Option<String>,
	pub after: Option<String>,
	pub before: Option<String>,
}

#[derive(Default, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[revisioned(revision = 1)]
pub struct Preferences {
	#[revision(start = 1)]
	#[serde(skip_serializing, skip_deserializing)]
	pub available_themes: Vec<String>,
	#[revision(start = 1)]
	pub theme: String,
	#[revision(start = 1)]
	pub front_page: String,
	#[revision(start = 1)]
	pub layout: String,
	#[revision(start = 1)]
	pub wide: String,
	#[revision(start = 1)]
	pub blur_spoiler: String,
	#[revision(start = 1)]
	pub show_nsfw: String,
	#[revision(start = 1)]
	pub blur_nsfw: String,
	#[revision(start = 1)]
	pub hide_hls_notification: String,
	#[revision(start = 1)]
	pub video_quality: String,
	#[revision(start = 1)]
	pub hide_sidebar_and_summary: String,
	#[revision(start = 1)]
	pub use_hls: String,
	#[revision(start = 1)]
	pub autoplay_videos: String,
	#[revision(start = 1)]
	pub fixed_navbar: String,
	#[revision(start = 1)]
	pub disable_visit_reddit_confirmation: String,
	#[revision(start = 1)]
	pub comment_sort: String,
	#[revision(start = 1)]
	pub post_sort: String,
	#[revision(start = 1)]
	#[serde(serialize_with = "serialize_vec_with_plus", deserialize_with = "deserialize_vec_with_plus")]
	pub subscriptions: Vec<String>,
	#[revision(start = 1)]
	#[serde(serialize_with = "serialize_vec_with_plus", deserialize_with = "deserialize_vec_with_plus")]
	pub filters: Vec<String>,
	#[revision(start = 1)]
	pub hide_awards: String,
	#[revision(start = 1)]
	pub hide_score: String,
	#[revision(start = 1)]
	pub remove_default_feeds: String,
}

fn serialize_vec_with_plus<S>(vec: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
	S: Serializer,
{
	serializer.serialize_str(&vec.join("+"))
}

fn deserialize_vec_with_plus<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
	D: Deserializer<'de>,
{
	let string = String::deserialize(deserializer)?;
	if string.is_empty() {
		return Ok(Vec::new());
	}
	Ok(string.split('+').map(|s| s.to_string()).collect())
}

#[derive(RustEmbed)]
#[folder = "static/themes/"]
#[include = "*.css"]
pub struct ThemeAssets;

impl Preferences {
	/// Build preferences from cookies
	pub fn new(req: &Request<Body>) -> Self {
		// Read available theme names from embedded css files.
		// Always make the default "system" theme available.
		let mut themes = vec!["system".to_string()];
		for file in ThemeAssets::iter() {
			let chunks: Vec<&str> = file.as_ref().split(".css").collect();
			themes.push(chunks[0].to_owned());
		}
		Self {
			available_themes: themes,
			theme: setting(req, "theme"),
			front_page: setting(req, "front_page"),
			layout: setting(req, "layout"),
			wide: setting(req, "wide"),
			blur_spoiler: setting(req, "blur_spoiler"),
			show_nsfw: setting(req, "show_nsfw"),
			hide_sidebar_and_summary: setting(req, "hide_sidebar_and_summary"),
			blur_nsfw: setting(req, "blur_nsfw"),
			use_hls: setting(req, "use_hls"),
			hide_hls_notification: setting(req, "hide_hls_notification"),
			video_quality: setting(req, "video_quality"),
			autoplay_videos: setting(req, "autoplay_videos"),
			fixed_navbar: setting_or_default(req, "fixed_navbar", "on".to_string()),
			disable_visit_reddit_confirmation: setting(req, "disable_visit_reddit_confirmation"),
			comment_sort: setting(req, "comment_sort"),
			post_sort: setting(req, "post_sort"),
			subscriptions: setting(req, "subscriptions").split('+').map(String::from).filter(|s| !s.is_empty()).collect(),
			filters: setting(req, "filters").split('+').map(String::from).filter(|s| !s.is_empty()).collect(),
			hide_awards: setting(req, "hide_awards"),
			hide_score: setting(req, "hide_score"),
			remove_default_feeds: setting(req, "remove_default_feeds"),
		}
	}

	pub fn to_urlencoded(&self) -> Result<String, String> {
		serde_urlencoded::to_string(self).map_err(|e| e.to_string())
	}

	pub fn to_bincode(&self) -> Result<Vec<u8>, String> {
		bincode::serialize(self).map_err(|e| e.to_string())
	}
	pub fn to_compressed_bincode(&self) -> Result<Vec<u8>, String> {
		deflate_compress(self.to_bincode()?)
	}
	pub fn to_bincode_str(&self) -> Result<String, String> {
		Ok(base2048::encode(&self.to_compressed_bincode()?))
	}
}

pub fn deflate_compress(i: Vec<u8>) -> Result<Vec<u8>, String> {
	let mut e = Encoder::new(Vec::new());
	e.write_all(&i).map_err(|e| e.to_string())?;
	e.finish().into_result().map_err(|e| e.to_string())
}

pub fn deflate_decompress(i: Vec<u8>) -> Result<Vec<u8>, String> {
	let mut decoder = Decoder::new(&i[..]);
	let mut out = Vec::new();
	decoder.read_to_end(&mut out).map_err(|e| format!("Failed to read from gzip decoder: {e}"))?;
	Ok(out)
}

/// Gets a `HashSet` of filters from the cookie in the given `Request`.
pub fn get_filters(req: &Request<Body>) -> HashSet<String> {
	setting(req, "filters").split('+').map(String::from).filter(|s| !s.is_empty()).collect::<HashSet<String>>()
}

/// Filters a `Vec<Post>` by the given `HashSet` of filters (each filter being
/// a subreddit name or a user name). If a `Post`'s subreddit or author is
/// found in the filters, it is removed.
///
/// The first value of the return tuple is the number of posts filtered. The
/// second return value is `true` if all posts were filtered.
pub fn filter_posts(posts: &mut Vec<Post>, filters: &HashSet<String>) -> (u64, bool) {
	// This is the length of the Vec<Post> prior to applying the filter.
	let lb: u64 = posts.len().try_into().unwrap_or(0);

	if posts.is_empty() {
		(0, false)
	} else {
		posts.retain(|p| !(filters.contains(&p.community) || filters.contains(&["u_", &p.author.name].concat())));

		// Get the length of the Vec<Post> after applying the filter.
		// If lb > la, then at least one post was removed.
		let la: u64 = posts.len().try_into().unwrap_or(0);

		(lb - la, posts.is_empty())
	}
}

/// Creates a [`Post`] from a provided JSON.
pub async fn parse_post(post: &Value) -> Post {
	// Grab UTC time as unix timestamp
	let (rel_time, created) = time(post["data"]["created_utc"].as_f64().unwrap_or_default());
	// Parse post score and upvote ratio
	let score = post["data"]["score"].as_i64().unwrap_or_default();
	let ratio: f64 = post["data"]["upvote_ratio"].as_f64().unwrap_or(1.0) * 100.0;

	// Determine the type of media along with the media URL
	let (post_type, media, gallery) = Media::parse(&post["data"]).await;

	let created_ts = post["data"]["created_utc"].as_f64().unwrap_or_default().round() as u64;

	let awards: Awards = Awards::parse(&post["data"]["all_awardings"]);

	let permalink = val(post, "permalink");

	let poll = Poll::parse(&post["data"]["poll_data"]);

	let body = if val(post, "removed_by_category") == "moderator" {
		format!(
			"<div class=\"md\"><p>[removed] — <a href=\"https://{}{permalink}\">view removed post</a></p></div>",
			get_setting("REDLIB_PUSHSHIFT_FRONTEND").unwrap_or_else(|| String::from(crate::config::DEFAULT_PUSHSHIFT_FRONTEND)),
		)
	} else {
		let selftext = val(post, "selftext");
		if selftext.contains("```") {
			let mut html_output = String::new();
			let parser = pulldown_cmark::Parser::new(&selftext);
			pulldown_cmark::html::push_html(&mut html_output, parser);
			rewrite_urls(&html_output)
		} else {
			rewrite_urls(&val(post, "selftext_html"))
		}
	};

	// Build a post using data parsed from Reddit post API
	Post {
		id: val(post, "id"),
		title: val(post, "title"),
		community: val(post, "subreddit"),
		body,
		author: Author {
			name: val(post, "author"),
			flair: Flair {
				flair_parts: FlairPart::parse(
					post["data"]["author_flair_type"].as_str().unwrap_or_default(),
					post["data"]["author_flair_richtext"].as_array(),
					post["data"]["author_flair_text"].as_str(),
				),
				text: val(post, "link_flair_text"),
				background_color: val(post, "author_flair_background_color"),
				foreground_color: val(post, "author_flair_text_color"),
			},
			distinguished: val(post, "distinguished"),
		},
		permalink,
		link_title: val(post, "link_title"),
		poll,
		score: format_num(score),
		upvote_ratio: ratio as i64,
		post_type,
		media,
		thumbnail: Media {
			url: format_url(val(post, "thumbnail").as_str()),
			alt_url: String::new(),
			width: post["data"]["thumbnail_width"].as_i64().unwrap_or_default(),
			height: post["data"]["thumbnail_height"].as_i64().unwrap_or_default(),
			poster: String::new(),
			download_name: String::new(),
		},
		flair: Flair {
			flair_parts: FlairPart::parse(
				post["data"]["link_flair_type"].as_str().unwrap_or_default(),
				post["data"]["link_flair_richtext"].as_array(),
				post["data"]["link_flair_text"].as_str(),
			),
			text: val(post, "link_flair_text"),
			background_color: val(post, "link_flair_background_color"),
			foreground_color: if val(post, "link_flair_text_color") == "dark" {
				"black".to_string()
			} else {
				"white".to_string()
			},
		},
		flags: Flags {
			spoiler: post["data"]["spoiler"].as_bool().unwrap_or_default(),
			nsfw: post["data"]["over_18"].as_bool().unwrap_or_default(),
			stickied: post["data"]["stickied"].as_bool().unwrap_or_default() || post["data"]["pinned"].as_bool().unwrap_or(false),
		},
		domain: val(post, "domain"),
		rel_time,
		created,
		created_ts,
		num_duplicates: post["data"]["num_duplicates"].as_u64().unwrap_or(0),
		comments: format_num(post["data"]["num_comments"].as_i64().unwrap_or_default()),
		gallery,
		awards,
		nsfw: post["data"]["over_18"].as_bool().unwrap_or_default(),
		ws_url: val(post, "websocket_url"),
		out_url: post["data"]["url_overridden_by_dest"].as_str().map(|a| a.to_string()),
	}
}

//
// FORMATTING
//

/// Grab a query parameter from a url
pub fn param(path: &str, value: &str) -> Option<String> {
	Some(
		Url::parse(format!("https://libredd.it/{path}").as_str())
			.ok()?
			.query_pairs()
			.into_owned()
			.collect::<HashMap<_, _>>()
			.get(value)?
			.clone(),
	)
}

/// Retrieve the value of a setting by name
pub fn setting(req: &Request<Body>, name: &str) -> String {
	// Parse a cookie value from request

	// If this was called with "subscriptions" and the "subscriptions" cookie has a value
	if name == "subscriptions" && req.cookie("subscriptions").is_some() {
		// Create subscriptions string
		let mut subscriptions = String::new();

		// Default subscriptions cookie
		if req.cookie("subscriptions").is_some() {
			subscriptions.push_str(req.cookie("subscriptions").unwrap().value());
		}

		// Start with first numbered subscription cookie
		let mut subscriptions_number = 1;

		// While whatever subscriptionsNUMBER cookie we're looking at has a value
		while req.cookie(&format!("subscriptions{subscriptions_number}")).is_some() {
			// Push whatever subscriptionsNUMBER cookie we're looking at into the subscriptions string
			subscriptions.push_str(req.cookie(&format!("subscriptions{subscriptions_number}")).unwrap().value());

			// Increment subscription cookie number
			subscriptions_number += 1;
		}

		// Return the subscriptions cookies as one large string
		subscriptions
	}
	// If this was called with "filters" and the "filters" cookie has a value
	else if name == "filters" && req.cookie("filters").is_some() {
		// Create filters string
		let mut filters = String::new();

		// Default filters cookie
		if req.cookie("filters").is_some() {
			filters.push_str(req.cookie("filters").unwrap().value());
		}

		// Start with first numbered filters cookie
		let mut filters_number = 1;

		// While whatever filtersNUMBER cookie we're looking at has a value
		while req.cookie(&format!("filters{filters_number}")).is_some() {
			// Push whatever filtersNUMBER cookie we're looking at into the filters string
			filters.push_str(req.cookie(&format!("filters{filters_number}")).unwrap().value());

			// Increment filters cookie number
			filters_number += 1;
		}

		// Return the filters cookies as one large string
		filters
	}
	// The above two still come to this if there was no existing value
	else {
		req
			.cookie(name)
			.unwrap_or_else(|| {
				// If there is no cookie for this setting, try receiving a default from the config
				if let Some(default) = get_setting(&format!("REDLIB_DEFAULT_{}", name.to_uppercase())) {
					Cookie::new(name, default)
				} else {
					Cookie::from(name)
				}
			})
			.value()
			.to_string()
	}
}

/// Retrieve the value of a setting by name or the default value
pub fn setting_or_default(req: &Request<Body>, name: &str, default: String) -> String {
	let value = setting(req, name);
	if value.is_empty() {
		default
	} else {
		value
	}
}

/// Detect and redirect in the event of a random subreddit
pub async fn catch_random(sub: &str, additional: &str) -> Result<Response<Body>, String> {
	if sub == "random" || sub == "randnsfw" {
		Ok(redirect(&format!(
			"/r/{}{additional}",
			json(format!("/r/{sub}/about.json?raw_json=1"), false).await?["data"]["display_name"]
				.as_str()
				.unwrap_or_default()
		)))
	} else {
		Err("No redirect needed".to_string())
	}
}

static REGEX_URL_WWW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://www\.reddit\.com/(.*)").unwrap());
static REGEX_URL_OLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://old\.reddit\.com/(.*)").unwrap());
static REGEX_URL_NP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://np\.reddit\.com/(.*)").unwrap());
static REGEX_URL_PLAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://reddit\.com/(.*)").unwrap());
static REGEX_URL_VIDEOS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://v\.redd\.it/(.*)/DASH_([0-9]{2,4}(\.mp4|$|\?source=fallback))").unwrap());
static REGEX_URL_VIDEOS_HLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://v\.redd\.it/(.+)/(HLSPlaylist\.m3u8.*)$").unwrap());
static REGEX_URL_IMAGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://i\.redd\.it/(.*)").unwrap());
static REGEX_URL_THUMBS_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://a\.thumbs\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_THUMBS_B: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://b\.thumbs\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_EMOJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://emoji\.redditmedia\.com/(.*)/(.*)").unwrap());
static REGEX_URL_PREVIEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://preview\.redd\.it/(.*)").unwrap());
static REGEX_URL_EXTERNAL_PREVIEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://external\-preview\.redd\.it/(.*)").unwrap());
static REGEX_URL_STYLES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://styles\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_STATIC_MEDIA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://www\.redditstatic\.com/(.*)").unwrap());

/// Direct urls to proxy if proxy is enabled
pub fn format_url(url: &str) -> String {
	if url.is_empty() || url == "self" || url == "default" || url == "nsfw" || url == "spoiler" {
		String::new()
	} else {
		Url::parse(url).map_or(url.to_string(), |parsed| {
			let domain = parsed.domain().unwrap_or_default();

			let capture = |regex: &Regex, format: &str, segments: i16| {
				regex.captures(url).map_or(String::new(), |caps| match segments {
					1 => [format, &caps[1]].join(""),
					2 => [format, &caps[1], "/", &caps[2]].join(""),
					_ => String::new(),
				})
			};

			macro_rules! chain {
				() => {
					{
						String::new()
					}
				};

				( $first_fn:expr, $($other_fns:expr), *) => {
					{
						let result = $first_fn;
						if result.is_empty() {
							chain!($($other_fns,)*)
						}
						else
						{
							result
						}
					}
				};
			}

			match domain {
				"www.reddit.com" => capture(&REGEX_URL_WWW, "/", 1),
				"old.reddit.com" => capture(&REGEX_URL_OLD, "/", 1),
				"np.reddit.com" => capture(&REGEX_URL_NP, "/", 1),
				"reddit.com" => capture(&REGEX_URL_PLAIN, "/", 1),
				"v.redd.it" => chain!(capture(&REGEX_URL_VIDEOS, "/vid/", 2), capture(&REGEX_URL_VIDEOS_HLS, "/hls/", 2)),
				"i.redd.it" => capture(&REGEX_URL_IMAGES, "/img/", 1),
				"a.thumbs.redditmedia.com" => capture(&REGEX_URL_THUMBS_A, "/thumb/a/", 1),
				"b.thumbs.redditmedia.com" => capture(&REGEX_URL_THUMBS_B, "/thumb/b/", 1),
				"emoji.redditmedia.com" => capture(&REGEX_URL_EMOJI, "/emoji/", 2),
				"preview.redd.it" => capture(&REGEX_URL_PREVIEW, "/preview/pre/", 1),
				"external-preview.redd.it" => capture(&REGEX_URL_EXTERNAL_PREVIEW, "/preview/external-pre/", 1),
				"styles.redditmedia.com" => capture(&REGEX_URL_STYLES, "/style/", 1),
				"www.redditstatic.com" => capture(&REGEX_URL_STATIC_MEDIA, "/static/", 1),
				_ => url.to_string(),
			}
		})
	}
}

static REGEX_BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^- (.*)$").unwrap());
static REGEX_BULLET_CONSECUTIVE_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</ul>\n<ul>").unwrap());

pub fn render_bullet_lists(input_text: &str) -> String {
	// ref: https://stackoverflow.com/a/4902622
	// First enclose each bullet with <ul> <li> tags
	let text1 = REGEX_BULLET.replace_all(input_text, "<ul><li>$1</li></ul>").to_string();
	// Then remove any consecutive </ul> <ul> tags
	REGEX_BULLET_CONSECUTIVE_LINES.replace_all(&text1, "").to_string()
}

// These are links we want to replace in-body
static REDDIT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(https|http|)://(www\.|old\.|np\.|amp\.|new\.|)(reddit\.com|redd\.it)/"#).unwrap());
static REDDIT_PREVIEW_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://(external-preview|preview|i)\.redd\.it(.*)").unwrap());
static REDDIT_EMOJI_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://(www|).redditstatic\.com/(.*)").unwrap());
static REDLIB_PREVIEW_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"/(img|preview/)(pre|external-pre)?/(.*?)>"#).unwrap());
static REDLIB_PREVIEW_TEXT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">(.*?)</a>").unwrap());

/// Rewrite Reddit links to Redlib in body of text
pub fn rewrite_urls(input_text: &str) -> String {
	let mut text1 =
		// Rewrite Reddit links to Redlib
		REDDIT_REGEX.replace_all(input_text, r#"href="/"#).to_string();

	loop {
		if REDDIT_EMOJI_REGEX.find(&text1).is_none() {
			break;
		} else {
			text1 = REDDIT_EMOJI_REGEX
				.replace_all(&text1, format_url(REDDIT_EMOJI_REGEX.find(&text1).map(|x| x.as_str()).unwrap_or_default()))
				.to_string()
		}
	}

	// Remove (html-encoded) "\" from URLs.
	text1 = text1.replace("%5C", "").replace("\\_", "_");

	// Rewrite external media previews to Redlib
	loop {
		if REDDIT_PREVIEW_REGEX.find(&text1).is_none() {
			return text1;
		} else {
			let formatted_url = format_url(REDDIT_PREVIEW_REGEX.find(&text1).map(|x| x.as_str()).unwrap_or_default());

			let image_url = REDLIB_PREVIEW_LINK_REGEX.find(&formatted_url).map_or("", |m| m.as_str());
			let mut image_caption = REDLIB_PREVIEW_TEXT_REGEX.find(&formatted_url).map_or("", |m| m.as_str());

			/* As long as image_caption isn't empty remove first and last four characters of image_text to leave us with just the text in the caption without any HTML.
			This makes it possible to enclose it in a <figcaption> later on without having stray HTML breaking it */
			if !image_caption.is_empty() {
				image_caption = &image_caption[1..image_caption.len() - 4];
			}

			// image_url contains > at the end of it, and right above this we remove image_text's front >, leaving us with just a single > between them
			let image_to_replace = format!("<p><a href=\"{image_url}{image_caption}</a></p>");

			/* We don't want to show a caption that's just the image's link, so we check if we find a Reddit preview link within the image's caption.
			If we don't find one we must have actual text, so we include a <figcaption> block that contains it.
			Otherwise we don't include the <figcaption> block as we don't need it. */
			let _image_replacement = if REDDIT_PREVIEW_REGEX.find(image_caption).is_none() {
				// Without this " would show as \" instead. "\&quot;" is how the quotes are formatted within image_text beforehand
				format!(
					"<figure><a href=\"{image_url}<img loading=\"lazy\" src=\"{image_url}</a><figcaption>{}</figcaption></figure>",
					image_caption.replace("\\&quot;", "\"")
				)
			} else {
				format!("<figure><a href=\"{image_url}<img loading=\"lazy\" src=\"{image_url}</a></figure>")
			};

			/* In order to know if we're dealing with a normal or external preview we need to take a look at the first capture group of REDDIT_PREVIEW_REGEX
			if it's preview we're dealing with something that needs /preview/pre, external-preview is /preview/external-pre, and i is /img */
			let reddit_preview_regex_capture = REDDIT_PREVIEW_REGEX.captures(&text1).unwrap().get(1).map_or("", |m| m.as_str());

			let _preview_type = match reddit_preview_regex_capture {
				"preview" => "/preview/pre",
				"external-preview" => "/preview/external-pre",
				_ => "/img",
			};

			text1 = REDDIT_PREVIEW_REGEX
				.replace(&text1, format!("{_preview_type}$2"))
				.replace(&image_to_replace, &_image_replacement)
		}
	}
}

// These links all follow a pattern of "https://reddit-econ-prod-assets-permanent.s3.amazonaws.com/asset-manager/SUBREDDIT_ID/RANDOM_FILENAME.png"
static REDDIT_EMOTE_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https://reddit-econ-prod-assets-permanent.s3.amazonaws.com/asset-manager/(.*)"#).unwrap());

// These all follow a pattern of '"emote|SUBREDDIT_IT|NUMBER"', we want the number
static REDDIT_EMOTE_ID_NUMBER_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""emote\|.*\|(.*)""#).unwrap());

pub fn rewrite_emotes(media_metadata: &Value, comment: String) -> String {
	/* Create the paths we'll use to look for our data inside the json.
	Because we don't know the name of any given emote we use a wildcard to parse them. */
	let link_path = JsonPath::parse("$[*].s.u").expect("valid JSON Path");
	let id_path = JsonPath::parse("$[*].id").expect("valid JSON Path");
	let size_path = JsonPath::parse("$[*].s.y").expect("valid JSON Path");

	// Extract all of the results from those json paths
	let link_nodes = media_metadata.json_path(&link_path);
	let id_nodes = media_metadata.json_path(&id_path);

	// Initialize our vectors
	let mut id_vec = Vec::new();
	let mut link_vec = Vec::new();

	// Add the relevant data to each of our vectors so we can access it by number later
	for current_id in id_nodes {
		id_vec.push(current_id)
	}
	for current_link in link_nodes {
		link_vec.push(current_link)
	}

	/* Set index to the length of link_vec.
	This is one larger than we'll actually be looking at, but we correct that later */
	let mut index = link_vec.len();

	// Comment needs to be in scope for when we call rewrite_urls()
	let mut comment = comment;

	/* Loop until index hits zero.
	This also prevents us from trying to do anything on an empty vector */
	while index != 0 {
		/* Subtract 1 from index to get the real index we should be looking at.
		Then continue on each subsequent loop to continue until we hit the last entry in the vector.
		This is how we get this to deal with multiple emotes in a single message and properly replace each ID with it's link */
		index -= 1;

		// Convert our current index in id_vec into a string so we can search through it with regex
		let current_id = id_vec[index].to_string();

		/* The ID number can be multiple lengths, so we capture it with regex.
		We also want to only attempt anything when we get matches to avoid panicking */
		if let Some(id_capture) = REDDIT_EMOTE_ID_NUMBER_REGEX.captures(&current_id) {
			// Format the ID to include the colons it has in the comment text
			let id = format!(":{}:", &id_capture[1]);

			// Convert current link to string to search through it with the regex
			let link = link_vec[index].to_string();

			// Make sure we only do operations when we get matches, otherwise we panic when trying to access the first match
			if let Some(link_capture) = REDDIT_EMOTE_LINK_REGEX.captures(&link) {
				/* Reddit sends a size for the image based on whether it's alone or accompanied by text.
				It's a good idea and makes everything look nicer, so we'll do the same. */
				let size = media_metadata.json_path(&size_path).first().unwrap().to_string();

				// Replace the ID we found earlier in the comment with the respective image and it's link from the regex capture
				let to_replace_with = format!(
					"<img loading=\"lazy\" src=\"/emote/{} width=\"{size}\" height=\"{size}\" style=\"vertical-align:text-bottom\">",
					&link_capture[1]
				);

				// Inside the comment replace the ID we found with the string that will embed the image
				comment = comment.replace(&id, &to_replace_with);
			}
		}
	}

	// render bullet (unordered) lists
	comment = render_bullet_lists(&comment);

	// Call rewrite_urls() to transform any other Reddit links
	rewrite_urls(&comment)
}

/// Format vote count to a string that will be displayed.
/// Append `m` and `k` for millions and thousands respectively, and
/// round to the nearest tenth.
pub fn format_num(num: i64) -> (String, String) {
	let truncated = if num >= 1_000_000 || num <= -1_000_000 {
		format!("{:.1}m", num as f64 / 1_000_000.0)
	} else if num >= 1000 || num <= -1000 {
		format!("{:.1}k", num as f64 / 1_000.0)
	} else {
		num.to_string()
	};

	(truncated, num.to_string())
}

/// Parse a relative and absolute time from a UNIX timestamp
pub fn time(created: f64) -> (String, String) {
	let time = OffsetDateTime::from_unix_timestamp(created.round() as i64).unwrap_or(OffsetDateTime::UNIX_EPOCH);
	let now = OffsetDateTime::now_utc();
	let min = time.min(now);
	let max = time.max(now);
	let time_delta = max - min;

	// If the time difference is more than a month, show full date
	let mut rel_time = if time_delta > Duration::days(30) {
		time.format(format_description!("[month repr:short] [day] '[year repr:last_two]")).unwrap_or_default()
	// Otherwise, show relative date/time
	} else if time_delta.whole_days() > 0 {
		format!("{}d", time_delta.whole_days())
	} else if time_delta.whole_hours() > 0 {
		format!("{}h", time_delta.whole_hours())
	} else {
		format!("{}m", time_delta.whole_minutes())
	};

	if time_delta <= Duration::days(30) {
		if now < time {
			rel_time += " left";
		} else {
			rel_time += " ago";
		}
	}

	(
		rel_time,
		time
			.format(format_description!("[month repr:short] [day] [year], [hour]:[minute]:[second] UTC"))
			.unwrap_or_default(),
	)
}

/// val() function used to parse JSON from Reddit APIs
pub fn val(j: &Value, k: &str) -> String {
	j["data"][k].as_str().unwrap_or_default().to_string()
}

//
// NETWORKING
//

pub fn template(t: &impl Template) -> Response<Body> {
	Response::builder()
		.status(200)
		.header("content-type", "text/html")
		.body(t.render().unwrap_or_default().into())
		.unwrap_or_default()
}

pub fn redirect(path: &str) -> Response<Body> {
	Response::builder()
		.status(302)
		.header("content-type", "text/html")
		.header("Location", path)
		.body(format!("Redirecting to <a href=\"{path}\">{path}</a>...").into())
		.unwrap_or_default()
}

/// Renders a generic error landing page.
pub async fn error(req: Request<Body>, msg: &str) -> Result<Response<Body>, String> {
	error!("Error page rendered: {}", msg.split('|').next().unwrap_or_default());
	let url = req.uri().to_string();
	let body = ErrorTemplate {
		msg: msg.to_string(),
		prefs: Preferences::new(&req),
		url,
	}
	.render()
	.unwrap_or_default();

	Ok(Response::builder().status(404).header("content-type", "text/html").body(body.into()).unwrap_or_default())
}

/// Renders a generic info landing page.
pub async fn info(req: Request<Body>, msg: &str) -> Result<Response<Body>, String> {
	let url = req.uri().to_string();
	let body = InfoTemplate {
		msg: msg.to_string(),
		prefs: Preferences::new(&req),
		url,
	}
	.render()
	.unwrap_or_default();

	Ok(Response::builder().status(200).header("content-type", "text/html").body(body.into()).unwrap_or_default())
}

/// Returns true if the config/env variable `REDLIB_SFW_ONLY` carries the
/// value `on`.
///
/// If this variable is set as such, the instance will operate in SFW-only
/// mode; all NSFW content will be filtered. Attempts to access NSFW
/// subreddits or posts or userpages for users Reddit has deemed NSFW will
/// be denied.
pub fn sfw_only() -> bool {
	match get_setting("REDLIB_SFW_ONLY") {
		Some(val) => val == "on",
		None => false,
	}
}

/// Returns true if the config/env variable REDLIB_ENABLE_RSS is set to "on".
/// If this variable is set as such, the instance will enable RSS feeds.
/// Otherwise, the instance will not provide RSS feeds.
pub fn enable_rss() -> bool {
	match get_setting("REDLIB_ENABLE_RSS") {
		Some(val) => val == "on",
		None => false,
	}
}

/// Returns true if the config/env variable `REDLIB_ROBOTS_DISABLE_INDEXING` carries the
/// value `on`.
///
/// If this variable is set as such, the instance will block all robots in robots.txt and
/// insert the noindex, nofollow meta tag on every page.
pub fn disable_indexing() -> bool {
	match get_setting("REDLIB_ROBOTS_DISABLE_INDEXING") {
		Some(val) => val == "on",
		None => false,
	}
}

/// Determines if a request should redirect to a NSFW landing gate.
pub fn should_be_nsfw_gated(req: &Request<Body>, _req_url: &str) -> bool {
	(setting(req, "show_nsfw") != "on") || sfw_only()
}

/// Renders the landing page for NSFW content when the user has not enabled
/// "show NSFW posts" in settings.
pub async fn nsfw_landing(req: Request<Body>, req_url: String) -> Result<Response<Body>, String> {
	let res_type: ResourceType;

	// Determine from the request URL if the resource is a subreddit, a user
	// page, or a post.
	let resource: String = if !req.param("name").unwrap_or_default().is_empty() {
		res_type = ResourceType::User;
		req.param("name").unwrap_or_default()
	} else if !req.param("id").unwrap_or_default().is_empty() {
		res_type = ResourceType::Post;
		req.param("id").unwrap_or_default()
	} else {
		res_type = ResourceType::Subreddit;
		req.param("sub").unwrap_or_default()
	};

	let body = NSFWLandingTemplate {
		res: resource,
		res_type,
		prefs: Preferences::new(&req),
		url: req_url,
	}
	.render()
	.unwrap_or_default();

	Ok(Response::builder().status(403).header("content-type", "text/html").body(body.into()).unwrap_or_default())
}

/// Returns the last (non-empty) segment of a path string
pub fn url_path_basename(path: &str) -> String {
	let url_result = Url::parse(format!("https://libredd.it/{path}").as_str());

	match url_result {
		Ok(mut url) => {
			url.path_segments_mut().unwrap().pop_if_empty();

			url.path_segments().unwrap().next_back().unwrap().to_string()
		}
		Err(_) => path.to_string(),
	}
}

/// Returns the URL of a post, as needed by RSS feeds
pub fn get_post_url(post: &Post) -> String {
	if let Some(out_url) = &post.out_url {
		// Handle cross post
		if out_url.starts_with("/r/") {
			format!("{}{}", config::get_setting("REDLIB_FULL_URL").unwrap_or_default(), out_url)
		} else {
			out_url.to_string()
		}
	} else {
		format!("{}{}", config::get_setting("REDLIB_FULL_URL").unwrap_or_default(), post.permalink)
	}
}

#[cfg(test)]
mod tests {
	use super::{format_num, format_url, rewrite_urls, Preferences};

	#[test]
	fn format_num_works() {
		assert_eq!(format_num(567), ("567".to_string(), "567".to_string()));
		assert_eq!(format_num(1234), ("1.2k".to_string(), "1234".to_string()));
		assert_eq!(format_num(1999), ("2.0k".to_string(), "1999".to_string()));
		assert_eq!(format_num(1001), ("1.0k".to_string(), "1001".to_string()));
		assert_eq!(format_num(1_999_999), ("2.0m".to_string(), "1999999".to_string()));
	}

	#[test]
	fn rewrite_urls_removes_backslashes_and_rewrites_url() {
		assert_eq!(
			rewrite_urls(
				"<a href=\"https://new.reddit.com/r/linux%5C_gaming/comments/x/just%5C_a%5C_test%5C/\">https://new.reddit.com/r/linux\\_gaming/comments/x/just\\_a\\_test/</a>"
			),
			"<a href=\"/r/linux_gaming/comments/x/just_a_test/\">https://new.reddit.com/r/linux_gaming/comments/x/just_a_test/</a>"
		);
		assert_eq!(
			rewrite_urls(
				"e.g. &lt;a href=\"https://www.reddit.com/r/linux%5C_gaming/comments/ql9j15/anyone%5C_else%5C_confused%5C_with%5C_linus%5C_linux%5C_issues/\"&gt;https://www.reddit.com/r/linux\\_gaming/comments/ql9j15/anyone\\_else\\_confused\\_with\\_linus\\_linux\\_issues/&lt;/a&gt;"
			),
			"e.g. &lt;a href=\"/r/linux_gaming/comments/ql9j15/anyone_else_confused_with_linus_linux_issues/\"&gt;https://www.reddit.com/r/linux_gaming/comments/ql9j15/anyone_else_confused_with_linus_linux_issues/&lt;/a&gt;"
		);
	}

	#[test]
	fn rewrite_urls_keeps_intentional_backslashes() {
		assert_eq!(
			rewrite_urls("printf \"\\npolkit.addRule(function(action, subject)"),
			"printf \"\\npolkit.addRule(function(action, subject)"
		);
	}

	#[test]
	fn test_format_url() {
		assert_eq!(format_url("https://a.thumbs.redditmedia.com/XYZ.jpg"), "/thumb/a/XYZ.jpg");
		assert_eq!(format_url("https://emoji.redditmedia.com/a/b"), "/emoji/a/b");

		assert_eq!(
			format_url("https://external-preview.redd.it/foo.jpg?auto=webp&s=bar"),
			"/preview/external-pre/foo.jpg?auto=webp&s=bar"
		);

		assert_eq!(format_url("https://i.redd.it/foobar.jpg"), "/img/foobar.jpg");
		assert_eq!(
			format_url("https://preview.redd.it/qwerty.jpg?auto=webp&s=asdf"),
			"/preview/pre/qwerty.jpg?auto=webp&s=asdf"
		);
		assert_eq!(format_url("https://v.redd.it/foo/DASH_360.mp4?source=fallback"), "/vid/foo/360.mp4");
		assert_eq!(
			format_url("https://v.redd.it/foo/HLSPlaylist.m3u8?a=bar&v=1&f=sd"),
			"/hls/foo/HLSPlaylist.m3u8?a=bar&v=1&f=sd"
		);
		assert_eq!(format_url("https://www.redditstatic.com/gold/awards/icon/icon.png"), "/static/gold/awards/icon/icon.png");
		assert_eq!(
			format_url("https://www.redditstatic.com/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif"),
			"/static/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif"
		);

		assert_eq!(format_url(""), "");
		assert_eq!(format_url("self"), "");
		assert_eq!(format_url("default"), "");
		assert_eq!(format_url("nsfw"), "");
		assert_eq!(format_url("spoiler"), "");
	}
	#[test]
	fn serialize_prefs() {
		let prefs = Preferences {
			available_themes: vec![],
			theme: "laserwave".to_owned(),
			front_page: "default".to_owned(),
			layout: "compact".to_owned(),
			wide: "on".to_owned(),
			blur_spoiler: "on".to_owned(),
			show_nsfw: "off".to_owned(),
			blur_nsfw: "on".to_owned(),
			hide_hls_notification: "off".to_owned(),
			video_quality: "best".to_owned(),
			hide_sidebar_and_summary: "off".to_owned(),
			use_hls: "on".to_owned(),
			autoplay_videos: "on".to_owned(),
			fixed_navbar: "on".to_owned(),
			disable_visit_reddit_confirmation: "on".to_owned(),
			comment_sort: "confidence".to_owned(),
			post_sort: "top".to_owned(),
			subscriptions: vec!["memes".to_owned(), "mildlyinteresting".to_owned()],
			filters: vec![],
			hide_awards: "off".to_owned(),
			hide_score: "off".to_owned(),
			remove_default_feeds: "off".to_owned(),
		};
		let urlencoded = serde_urlencoded::to_string(prefs).expect("Failed to serialize Prefs");

		assert_eq!(urlencoded, "theme=laserwave&front_page=default&layout=compact&wide=on&blur_spoiler=on&show_nsfw=off&blur_nsfw=on&hide_hls_notification=off&video_quality=best&hide_sidebar_and_summary=off&use_hls=on&autoplay_videos=on&fixed_navbar=on&disable_visit_reddit_confirmation=on&comment_sort=confidence&post_sort=top&subscriptions=memes%2Bmildlyinteresting&filters=&hide_awards=off&hide_score=off&remove_default_feeds=off");
	}
}

#[test]
fn test_rewriting_emoji() {
	let input = r#"<div class="md"><p>How can you have such hard feelings towards a license? <img src="https://www.redditstatic.com/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif" width="20" height="20" style="vertical-align:middle"> Let people use what license they want, and BSD is one of the least restrictive ones AFAIK.</p>"#;
	let output = r#"<div class="md"><p>How can you have such hard feelings towards a license? <img src="/static/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif" width="20" height="20" style="vertical-align:middle"> Let people use what license they want, and BSD is one of the least restrictive ones AFAIK.</p>"#;
	assert_eq!(rewrite_urls(input), output);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_subreddit_quarantined() {
	let subreddit = Post::fetch("/r/drugs", true).await;
	assert!(subreddit.is_ok());
	assert!(!subreddit.unwrap().0.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_nsfw_subreddit() {
	// Gonwild is a place for closed, Euclidean Geometric shapes to exchange their nth terms for karma; showing off their edges in a comfortable environment without pressure.
	// Find a good sub that is tagged NSFW but that actually isn't in case my future employers are watching (they probably are)
	// switched from randnsfw as it is no longer functional.
	let subreddit = Post::fetch("/r/gonwild", false).await;
	assert!(subreddit.is_ok());
	assert!(!subreddit.unwrap().0.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_ws() {
	let subreddit = Post::fetch("/r/popular", false).await;
	assert!(subreddit.is_ok());
	for post in subreddit.unwrap().0 {
		assert!(post.ws_url.starts_with("wss://k8s-lb.wss.redditmedia.com/link/"));
	}
}

#[test]
fn test_rewriting_image_links() {
	let input =
		r#"<p><a href="https://preview.redd.it/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc">caption 1</a></p>"#;
	let output = r#"<figure><a href="/preview/pre/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc"><img loading="lazy" src="/preview/pre/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc"></a><figcaption>caption 1</figcaption></figure>"#;
	assert_eq!(rewrite_urls(input), output);
}

#[test]
fn test_url_path_basename() {
	// without trailing slash
	assert_eq!(url_path_basename("/first/last"), "last");
	// with trailing slash
	assert_eq!(url_path_basename("/first/last/"), "last");
	// with query parameters
	assert_eq!(url_path_basename("/first/last/?some=query"), "last");
	// file path
	assert_eq!(url_path_basename("/cdn/image.jpg"), "image.jpg");
	// when a full url is passed instead of just a path
	assert_eq!(url_path_basename("https://doma.in/first/last"), "last");
	// empty path
	assert_eq!(url_path_basename("/"), "");
}

#[test]
fn test_rewriting_emotes() {
	let json_input = serde_json::from_str(r#"{"emote|t5_31hpy|2028":{"e":"Image","id":"emote|t5_31hpy|2028","m":"image/png","s":{"u":"https://reddit-econ-prod-assets-permanent.s3.amazonaws.com/asset-manager/t5_31hpy/PW6WsOaLcd.png","x":60,"y":60},"status":"valid","t":"sticker"}}"#).expect("Valid JSON");
	let comment_input = r#"<div class="comment_body "><div class="md"><p>:2028:</p></div></div>"#;
	let output = r#"<div class="comment_body "><div class="md"><p><img loading="lazy" src="/emote/t5_31hpy/PW6WsOaLcd.png" width="60" height="60" style="vertical-align:text-bottom"></p></div></div>"#;
	assert_eq!(rewrite_emotes(&json_input, comment_input.to_string()), output);
}

#[test]
fn test_rewriting_bullet_list() {
	let input = r#"<div class="md"><p>Hi, I&#39;ve bought this very same monitor and found no calibration whatsoever. I have an ICC profile that has been set up since I&#39;ve installed its driver from the LG website and it works ok. I also used <a href="http://www.lagom.nl/lcd-test/">http://www.lagom.nl/lcd-test/</a> to calibrate it. After some good tinkering I&#39;ve found the following settings + the color profile from the driver gets me past all the tests perfectly:
- Brightness 50 (still have to settle on this one, it&#39;s personal preference, it controls the backlight, not the colors)
- Contrast 70 (which for me was the default one)
- Picture mode Custom
- Super resolution + Off (it looks horrible anyway)
- Sharpness 50 (default one I think)
- Black level High (low messes up gray colors)
- DFC Off
- Response Time Middle (personal preference, <a href="https://www.blurbusters.com/">https://www.blurbusters.com/</a> show horrible overdrive with it on high)
- Freesync doesn&#39;t matter
- Black stabilizer 50
- Gamma setting on 0
- Color Temp Medium
How`s your monitor by the way? Any IPS bleed whatsoever? I either got lucky or the panel is pretty good, 0 bleed for me, just the usual IPS glow. How about the pixels? I see the pixels even at one meter away, especially on Microsoft Edge&#39;s icon for example, the blue background is just blocky, don&#39;t know why.</p>
</div>"#;
	let output = r#"<div class="md"><p>Hi, I&#39;ve bought this very same monitor and found no calibration whatsoever. I have an ICC profile that has been set up since I&#39;ve installed its driver from the LG website and it works ok. I also used <a href="http://www.lagom.nl/lcd-test/">http://www.lagom.nl/lcd-test/</a> to calibrate it. After some good tinkering I&#39;ve found the following settings + the color profile from the driver gets me past all the tests perfectly:
<ul><li>Brightness 50 (still have to settle on this one, it&#39;s personal preference, it controls the backlight, not the colors)</li><li>Contrast 70 (which for me was the default one)</li><li>Picture mode Custom</li><li>Super resolution + Off (it looks horrible anyway)</li><li>Sharpness 50 (default one I think)</li><li>Black level High (low messes up gray colors)</li><li>DFC Off</li><li>Response Time Middle (personal preference, <a href="https://www.blurbusters.com/">https://www.blurbusters.com/</a> show horrible overdrive with it on high)</li><li>Freesync doesn&#39;t matter</li><li>Black stabilizer 50</li><li>Gamma setting on 0</li><li>Color Temp Medium</li></ul>
How`s your monitor by the way? Any IPS bleed whatsoever? I either got lucky or the panel is pretty good, 0 bleed for me, just the usual IPS glow. How about the pixels? I see the pixels even at one meter away, especially on Microsoft Edge&#39;s icon for example, the blue background is just blocky, don&#39;t know why.</p>
</div>"#;

	assert_eq!(render_bullet_lists(input), output);
}

#[test]
fn test_default_prefs_serialization_loop_json() {
	let prefs = Preferences::default();
	let serialized = serde_json::to_string(&prefs).unwrap();
	let deserialized: Preferences = serde_json::from_str(&serialized).unwrap();
	assert_eq!(prefs, deserialized);
}

#[test]
fn test_default_prefs_serialization_loop_bincode() {
	let prefs = Preferences::default();
	test_round_trip(&prefs, false);
	test_round_trip(&prefs, true);
}

static KNOWN_GOOD_CONFIGS: &[&str] = &[
	"ఴӅβØØҞÉဏႢձĬ༧ȒʯऌԔӵ୮༏",
	"ਧՊΥÀÃǎƱГ۸ඣമĖฤ႙ʟาúໜϾௐɥঀĜໃહཞઠѫҲɂఙ࿔ǲઉƲӟӻĻฅΜδ໖ԜǗဖငƦơ৶Ą௩ԹʛใЛʃශаΏ",
	"ਧԩΥÀÃÎŠ౭൩ඔႠϼҭöҪƸռઇԾॐნɔາǒՍҰच௨ಖມŃЉŐདƦ๙ϩএఠȝഽйʮჯඒϰळՋ௮ສ৵ऎΦѧਹಧଟƙŃ३î༦ŌပղयƟแҜ།",
];

#[test]
fn test_known_good_configs_deserialization() {
	for config in KNOWN_GOOD_CONFIGS {
		let bytes = base2048::decode(config).unwrap();
		let decompressed = deflate_decompress(bytes).unwrap();
		assert!(bincode::deserialize::<Preferences>(&decompressed).is_ok());
	}
}

#[test]
fn test_known_good_configs_full_round_trip() {
	for config in KNOWN_GOOD_CONFIGS {
		let bytes = base2048::decode(config).unwrap();
		let decompressed = deflate_decompress(bytes).unwrap();
		let prefs: Preferences = bincode::deserialize(&decompressed).unwrap();
		test_round_trip(&prefs, false);
		test_round_trip(&prefs, true);
	}
}

fn test_round_trip(input: &Preferences, compression: bool) {
	let serialized = bincode::serialize(input).unwrap();
	let compressed = if compression { deflate_compress(serialized).unwrap() } else { serialized };
	let decompressed = if compression { deflate_decompress(compressed).unwrap() } else { compressed };
	let deserialized: Preferences = bincode::deserialize(&decompressed).unwrap();
	assert_eq!(*input, deserialized);
}
