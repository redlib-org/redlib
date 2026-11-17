#![allow(dead_code)]
#![allow(clippy::cmp_owned)]

use crate::body::Body;
use crate::config::{self, get_setting};
use crate::redgifs;
use crate::{backends::json, server::RequestExt};
use askama::Template;
use chrono::DateTime;
use clearurls::UrlCleaner;
use cookie::Cookie;
use hyper::{Request, Response};
use libflate::deflate::{Decoder, Encoder};
use log::error;
use regex::Regex;
use revision::{revisioned, Error};
use rss::{Channel, Enclosure, Guid, Item};
use rust_embed::RustEmbed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json_path::{JsonPath, JsonPathExt};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{Read, Write};
use std::str::FromStr;
use std::string::ToString;
use std::sync::{Arc, LazyLock};
use time::{macros::format_description, Duration, OffsetDateTime};
use url::Url;

/// Write a message to stderr on debug mode. This function is a no-op on
/// release code.
#[macro_export]
macro_rules! dbg_msg {
	// `cfg!` rather than `#[cfg]`, so release builds still see the value used
	($x:expr) => {
		if cfg!(debug_assertions) {
			eprintln!("{}:{}: {}", file!(), line!(), $x.to_string())
		}
	};

	($($x:expr),+) => {
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
	/// Posted from Reddit's ads platform, or the advertiser OP of such a post
	pub advertiser: bool,
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
	pub needs_hls_for_audio: bool,
}

impl Media {
	pub async fn parse(data: &Value) -> (String, Self, Vec<GalleryMedia>) {
		let mut gallery = Vec::new();

		// Define the various known places that Reddit might put video URLs.
		let data_preview = &data["preview"]["reddit_video_preview"];
		let secure_media = &data["secure_media"]["reddit_video"];
		let crosspost_parent_media = &data["crosspost_parent_list"][0]["secure_media"]["reddit_video"];

		// Check RedGifs FIRST before Reddit's cached fallback videos, then other video sources
		let domain = data["domain"].as_str().unwrap_or_default();
		let (post_type, url_val, video) = if enable_redgifs() && redgifs::is_redgifs_domain(domain) {
			("video", &data["url"], None)
		} else if data_preview["fallback_url"].is_string() {
			(
				if data_preview["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&data_preview["fallback_url"],
				Some(data_preview),
			)
		} else if secure_media["fallback_url"].is_string() {
			(
				if secure_media["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&secure_media["fallback_url"],
				Some(secure_media),
			)
		} else if crosspost_parent_media["fallback_url"].is_string() {
			(
				if crosspost_parent_media["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&crosspost_parent_media["fallback_url"],
				Some(crosspost_parent_media),
			)
		} else if data["post_hint"].as_str().unwrap_or("") == "rich:video" && data_preview["fallback_url"].is_string() {
			(
				if data_preview["is_gif"].as_bool().unwrap_or(false) { "gif" } else { "video" },
				&data_preview["fallback_url"],
				Some(data_preview),
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

		let alt_url = video.map_or(String::new(), |video| format_url(video["hls_url"].as_str().unwrap_or_default()));
		let url = format_url(url_val.as_str().unwrap_or_default());
		// Only the muxed MP4s from packaged media have sound without HLS
		let needs_hls_for_audio =
			post_type == "video" && !alt_url.is_empty() && !url.starts_with("/packaged") && video.is_some_and(|video| video["has_audio"].as_bool() != Some(false));

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
				url,
				alt_url,
				// Note: in the data["is_reddit_media_domain"] path above
				// width and height will be 0.
				width: source["width"].as_i64().unwrap_or_default(),
				height: source["height"].as_i64().unwrap_or_default(),
				poster: format_url(source["url"].as_str().unwrap_or_default()),
				download_name,
				needs_hls_for_audio,
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
			let mut body = rewrite_urls(&body_html(post, "selftext", "selftext_html"));
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
					advertiser: data["is_created_from_ads_ui"] == true,
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
					needs_hls_for_audio: false,
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
	/// Where a `more` link leads without JavaScript; empty if it has nowhere to go.
	pub more_href: String,
	/// The page with just the comments a `more` cursor loads, which JavaScript inserts in place; empty without a cursor.
	pub more_load: String,
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
	pub detail: String,
	pub reddit_url: String,
	/// Show instance hosts how to get past Reddit's block.
	pub blocked: bool,
	/// Subreddits of a combined feed Reddit blocked, linked one by one.
	pub subreddits: Vec<String>,
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

/// A box of the sidebar new Reddit shows, from `/r/<sub>/api/widgets`.
pub enum Widget {
	Text {
		title: String,
		html: String,
	},
	/// Rules as (title, description HTML).
	Rules {
		title: String,
		rules: Vec<(String, String)>,
	},
	/// Long lists show the first few, the rest behind "Show N more".
	Communities {
		title: String,
		communities: Vec<crate::search::Subreddit>,
		more: Vec<crate::search::Subreddit>,
	},
	Image {
		title: String,
		url: String,
		link: Option<String>,
	},
	/// A link menu or button box.
	Links {
		title: String,
		links: Vec<Link>,
	},
}

impl Widget {
	pub fn title(&self) -> &str {
		match self {
			Self::Text { title, .. } | Self::Rules { title, .. } | Self::Communities { title, .. } | Self::Image { title, .. } | Self::Links { title, .. } => title,
		}
	}

	/// Titled boxes start closed so the sidebar stays short, except the rules.
	pub fn starts_open(&self) -> bool {
		matches!(self, Self::Rules { .. })
	}
}

/// A sidebar link, or a submenu of links when `url` is empty.
pub struct Link {
	pub text: String,
	pub url: String,
	pub children: Vec<(String, String)>,
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
	/// Reddit's weekly visitors and contributions; it reports no active users
	/// anymore. Only gql has them.
	pub weekly_visitors: Option<(String, String)>,
	pub weekly_contributions: Option<(String, String)>,
	pub wiki: bool,
	pub nsfw: bool,
	/// New Reddit's sidebar; `info` is the old one.
	pub widgets: Vec<Widget>,
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

/// First byte of a revisioned settings code. Plain-bincode codes start with the
/// little-endian length of the theme name, which is never 255.
const REVISIONED_PREFS_MARKER: u8 = 0xFF;

/// Settings code layout before the revisioned format: plain bincode of the
/// original upstream fields. Frozen; never add fields here.
#[derive(Deserialize)]
struct LegacyPreferences {
	theme: String,
	front_page: String,
	layout: String,
	wide: String,
	blur_spoiler: String,
	show_nsfw: String,
	blur_nsfw: String,
	hide_hls_notification: String,
	video_quality: String,
	hide_sidebar_and_summary: String,
	use_hls: String,
	autoplay_videos: String,
	fixed_navbar: String,
	disable_visit_reddit_confirmation: String,
	comment_sort: String,
	post_sort: String,
	#[serde(deserialize_with = "deserialize_vec_with_plus")]
	subscriptions: Vec<String>,
	#[serde(deserialize_with = "deserialize_vec_with_plus")]
	filters: Vec<String>,
	hide_awards: String,
	hide_score: String,
	remove_default_feeds: String,
}

impl LegacyPreferences {
	fn decode(bytes: &[u8]) -> Result<Preferences, String> {
		let mut rest = bytes;
		let old: Self = bincode::deserialize_from(&mut rest).map_err(|e| e.to_string())?;
		// Codes exported by this fork before the revisioned format carry geo_filter next.
		let geo_filter = if rest.is_empty() {
			None
		} else {
			bincode::deserialize_from::<_, String>(&mut rest).ok()
		};
		let defaults = |f: fn(u16) -> Result<String, Error>| f(1).unwrap_or_default();
		Ok(Preferences {
			available_themes: Vec::new(),
			theme: old.theme,
			front_page: old.front_page,
			layout: old.layout,
			wide: old.wide,
			blur_spoiler: old.blur_spoiler,
			show_nsfw: old.show_nsfw,
			blur_nsfw: old.blur_nsfw,
			hide_hls_notification: old.hide_hls_notification,
			video_quality: old.video_quality,
			hide_sidebar_and_summary: old.hide_sidebar_and_summary,
			use_hls: old.use_hls,
			autoplay_videos: old.autoplay_videos,
			fixed_navbar: old.fixed_navbar,
			disable_visit_reddit_confirmation: old.disable_visit_reddit_confirmation,
			comment_sort: old.comment_sort,
			post_sort: old.post_sort,
			subscriptions: old.subscriptions,
			filters: old.filters,
			hide_awards: old.hide_awards,
			hide_score: old.hide_score,
			remove_default_feeds: old.remove_default_feeds,
			geo_filter: geo_filter.unwrap_or_else(|| defaults(Preferences::default_geo_filter)),
			clean_urls: defaults(Preferences::default_off),
			posts_per_page: defaults(Preferences::default_instance_setting),
			max_comment_thread_depth: defaults(Preferences::default_instance_setting),
			more_replies_new_page: defaults(Preferences::default_off),
			light_theme: String::new(),
			dark_theme: String::new(),
		})
	}
}

#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[revisioned(revision = 4)]
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
	#[revision(start = 2, default_fn = "default_geo_filter")]
	pub geo_filter: String,
	#[revision(start = 2, default_fn = "default_off")]
	pub clean_urls: String,
	#[revision(start = 2, default_fn = "default_instance_setting")]
	pub posts_per_page: String,
	#[revision(start = 2, default_fn = "default_instance_setting")]
	pub max_comment_thread_depth: String,
	// Revision 2 codes are already out there, so new fields can't join it.
	#[revision(start = 3, default_fn = "default_off")]
	pub more_replies_new_page: String,
	/// Themes the "system" theme switches between; empty = built-in light or dark.
	#[revision(start = 4, default_fn = "default_instance_setting")]
	pub light_theme: String,
	#[revision(start = 4, default_fn = "default_instance_setting")]
	pub dark_theme: String,
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
			geo_filter: setting_or_default(req, "geo_filter", "GLOBAL".to_string()),
			clean_urls: setting(req, "clean_urls"),
			posts_per_page: setting(req, "posts_per_page"),
			max_comment_thread_depth: setting(req, "max_comment_thread_depth"),
			more_replies_new_page: setting(req, "more_replies_new_page"),
			light_theme: setting(req, "light_theme"),
			dark_theme: setting(req, "dark_theme"),
		}
	}

	pub fn to_urlencoded(&self) -> Result<String, String> {
		serde_urlencoded::to_string(self).map_err(|e| e.to_string())
	}

	/// Encode for a settings code: marker byte + `revision` encoding, which stores the
	/// revision so fields added later decode to their `default_fn` in older codes.
	pub fn to_bincode(&self) -> Result<Vec<u8>, String> {
		let prefs = Self {
			available_themes: Vec::new(),
			..self.clone()
		};
		let mut out = vec![REVISIONED_PREFS_MARKER];
		revision::to_writer(&mut out, &prefs).map_err(|e| e.to_string())?;
		Ok(out)
	}
	/// Decode a settings code: the revisioned format, or the original plain-bincode one.
	pub fn from_bincode(bytes: &[u8]) -> Result<Self, String> {
		match bytes.split_first() {
			Some((&REVISIONED_PREFS_MARKER, rest)) => revision::from_slice(rest).map_err(|e| e.to_string()),
			_ => LegacyPreferences::decode(bytes),
		}
	}
	pub fn to_compressed_bincode(&self) -> Result<Vec<u8>, String> {
		deflate_compress(self.to_bincode()?)
	}
	pub fn to_bincode_str(&self) -> Result<String, String> {
		Ok(base2048::encode(&self.to_compressed_bincode()?))
	}
	fn default_geo_filter(_revision: u16) -> Result<String, Error> {
		Ok("GLOBAL".to_owned())
	}
	/// Empty = fall back to the instance default at read time.
	fn default_instance_setting(_revision: u16) -> Result<String, Error> {
		Ok(String::new())
	}
	fn default_off(_revision: u16) -> Result<String, Error> {
		Ok("off".to_owned())
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

	let gone = if val(post, "removed_by_category") == "moderator" {
		Some("removed")
	} else if val(post, "author") == "[deleted]" && val(post, "selftext") == "[deleted]" {
		Some("deleted")
	} else {
		None
	};
	let body = if let Some(state) = gone {
		format!(
			"<div class=\"md\"><p>[{state}] — <a href=\"https://{}{permalink}\">view {state} post</a></p></div>",
			get_setting("REDLIB_PUSHSHIFT_FRONTEND").unwrap_or_else(|| String::from(crate::config::DEFAULT_PUSHSHIFT_FRONTEND)),
		)
	} else {
		rewrite_urls(&body_html(post, "selftext", "selftext_html"))
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
			advertiser: post["data"]["is_created_from_ads_ui"] == true,
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
			needs_hls_for_audio: false,
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
static REGEX_URL_VIDEOS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://v\.redd\.it/(.*)/(DASH|CMAF)_([0-9]{2,4}(\.mp4|$|\?source=fallback))").unwrap());
static REGEX_URL_VIDEOS_HLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://v\.redd\.it/(.+)/(HLSPlaylist\.m3u8)").unwrap());
static REGEX_URL_PACKAGED_MEDIA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://packaged-media\.redd\.it/(.*)").unwrap());
static REGEX_URL_EXTERNAL_PACKAGED_MEDIA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://external-packaged-media\.redd\.it/(.*)").unwrap());
static REGEX_URL_IMAGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://i\.redd\.it/(.*)").unwrap());
static REGEX_URL_THUMBS_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://a\.thumbs\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_THUMBS_B: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://b\.thumbs\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_EMOJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://emoji\.redditmedia\.com/(.*)/(.*)").unwrap());
static REGEX_URL_PREVIEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://preview\.redd\.it/(.*)").unwrap());
static REGEX_URL_EXTERNAL_PREVIEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://external\-preview\.redd\.it/(.*)").unwrap());
static REGEX_URL_STYLES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://styles\.redditmedia\.com/(.*)").unwrap());
static REGEX_URL_STATIC_MEDIA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://www\.redditstatic\.com/(.*)").unwrap());
static REGEX_URL_REDGIFS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://(?:www\.|v\d+\.)?redgifs\.com/watch/([^?#]*)").unwrap());

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
					3 => [format, &caps[1], "/", &caps[2], "/", &caps[3]].join(""),
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
				"v.redd.it" => chain!(capture(&REGEX_URL_VIDEOS, "/vid/", 3), capture(&REGEX_URL_VIDEOS_HLS, "/hls/", 2)),
				"packaged-media.redd.it" => capture(&REGEX_URL_PACKAGED_MEDIA, "/packaged/", 1),
				"external-packaged-media.redd.it" => capture(&REGEX_URL_EXTERNAL_PACKAGED_MEDIA, "/packaged-external/", 1),
				"i.redd.it" => capture(&REGEX_URL_IMAGES, "/img/", 1),
				"a.thumbs.redditmedia.com" => capture(&REGEX_URL_THUMBS_A, "/thumb/a/", 1),
				"b.thumbs.redditmedia.com" => capture(&REGEX_URL_THUMBS_B, "/thumb/b/", 1),
				"emoji.redditmedia.com" => capture(&REGEX_URL_EMOJI, "/emoji/", 2),
				"preview.redd.it" => capture(&REGEX_URL_PREVIEW, "/preview/pre/", 1),
				"external-preview.redd.it" => capture(&REGEX_URL_EXTERNAL_PREVIEW, "/preview/external-pre/", 1),
				"styles.redditmedia.com" => capture(&REGEX_URL_STYLES, "/style/", 1),
				"www.redditstatic.com" => capture(&REGEX_URL_STATIC_MEDIA, "/static/", 1),
				"www.redgifs.com" if enable_redgifs() => capture(&REGEX_URL_REDGIFS, "/redgifs/", 1),
				"redgifs.com" if enable_redgifs() => capture(&REGEX_URL_REDGIFS, "/redgifs/", 1),
				d if enable_redgifs() && d.starts_with("v") && d.ends_with(".redgifs.com") => capture(&REGEX_URL_REDGIFS, "/redgifs/", 1),
				_ => url.to_string(),
			}
		})
	}
}

// Remove tracking query params
static URL_CLEANER: LazyLock<Arc<UrlCleaner>> = LazyLock::new(|| Arc::new(UrlCleaner::from_embedded_rules().expect("Failed to initialize UrlCleaner")));

pub fn clean_url(url: String) -> String {
	let is_external_url = match Url::parse(url.as_str()) {
		Ok(parsed_url) => parsed_url.domain().is_some(),
		_ => false,
	};
	let mut cleaned_url = url.clone();
	if is_external_url {
		cleaned_url = match URL_CLEANER.clear_single_url_str(cleaned_url.as_str()) {
			Ok(cleaned_result) => cleaned_result.as_ref().to_owned(),
			_ => cleaned_url,
		}
	}
	cleaned_url
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
static REDDIT_EMOJI_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://(?:www\.)?redditstatic\.com/[^\s"'<>]*"#).unwrap());
static REDLIB_PREVIEW_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"/(img|preview/)(pre|external-pre)?/(.*?)>"#).unwrap());
static REDLIB_PREVIEW_TEXT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">(.*?)</a>").unwrap());

/// Rewrite Reddit links to Redlib in body of text
/// The HTML of a post or comment body. Reddit's `*_html` comes from its old
/// renderer, which doesn't know fenced code blocks, so bodies with one are
/// rendered from their markdown instead.
pub fn body_html(item: &Value, markdown_key: &str, html_key: &str) -> String {
	let markdown = val(item, markdown_key);
	if markdown.contains("```") {
		markdown_html(&markdown)
	} else {
		val(item, html_key)
	}
}

/// Renders Reddit markdown, with syntax highlighting in code blocks.
pub fn markdown_html(markdown: &str) -> String {
	use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
	let mut code: Option<(String, String)> = None;
	let events = Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).filter_map(|event| match event {
		Event::Start(Tag::CodeBlock(kind)) => {
			let lang = match kind {
				CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or_default().to_string(),
				CodeBlockKind::Indented => String::new(),
			};
			code = Some((lang, String::new()));
			None
		}
		Event::Text(text) if code.is_some() => {
			if let Some((_, buf)) = &mut code {
				buf.push_str(&text);
			}
			None
		}
		Event::End(TagEnd::CodeBlock) => code.take().map(|(lang, text)| Event::Html(highlight(&lang, &text).into())),
		// Reddit shows HTML in markdown as text
		Event::Html(html) | Event::InlineHtml(html) => Some(Event::Text(html)),
		event => Some(event),
	});
	let mut html = String::from("<div class=\"md\">");
	pulldown_cmark::html::push_html(&mut html, events);
	html.push_str("</div>");
	html
}

/// Above this, a code block isn't highlighted: the regexes are too slow for it.
const HIGHLIGHT_MAX_LEN: usize = 20_000;

static SYNTAXES: LazyLock<syntect::parsing::SyntaxSet> = LazyLock::new(syntect::parsing::SyntaxSet::load_defaults_newlines);

/// A code block, its tokens in `hl-<scope>` spans when `lang` is known.
fn highlight(lang: &str, code: &str) -> String {
	use syntect::html::{ClassStyle, ClassedHTMLGenerator};
	use syntect::util::LinesWithEndings;
	let plain = || format!("<pre><code>{}</code></pre>", htmlescape::encode_minimal(code));
	let Some(syntax) = Some(lang)
		.filter(|lang| !lang.is_empty() && code.len() <= HIGHLIGHT_MAX_LEN)
		.and_then(|lang| SYNTAXES.find_syntax_by_token(lang))
	else {
		return plain();
	};
	let mut html = ClassedHTMLGenerator::new_with_class_style(syntax, &SYNTAXES, ClassStyle::SpacedPrefixed { prefix: "hl-" });
	for line in LinesWithEndings::from(code) {
		if html.parse_html_for_line_which_includes_newline(line).is_err() {
			return plain();
		}
	}
	format!("<pre><code>{}</code></pre>", html.finalize())
}

pub fn rewrite_urls(input_text: &str) -> String {
	let mut text1 =
		// Rewrite Reddit links to Redlib
		REDDIT_REGEX.replace_all(input_text, r#"href="/"#).to_string();

	text1 = REDDIT_EMOJI_REGEX
		.replace_all(&text1, |captures: &regex::Captures<'_>| format_url(&captures[0]))
		.to_string();

	// Remove (html-encoded) "\" from URLs.
	text1 = text1.replace("%5C", "").replace("\\_", "_");

	/* Remove paragraphs that only contain zero width spaces.
	Reddit ignores these in their formatting so we want to remove them so they don't mess with ours. */
	text1 = text1.replace("<p>&#8203;</p>", "").replace("<p>&#x200B;</p>", "");

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

// Match Giphy URLs in comment anchor tags, capturing the GIF ID
// Handles: giphy.com/gifs/ID, media.giphy.com/media/ID, i.giphy.com/ID
static GIPHY_EMBED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r#"(?i)<a\s+href="https?://(?:www\.)?(?:giphy\.com/(?:gifs|clips)|media\.giphy\.com/media|i\.giphy\.com)/([a-z0-9]+)[^"]*"[^>]*>[^<]*</a>"#).unwrap()
});

/// Rewrite Giphy URLs in comment body to embedded video elements
fn rewrite_giphy_links(comment: &str) -> String {
	GIPHY_EMBED_REGEX
		.replace_all(comment, |caps: &regex::Captures| {
			let id = &caps[1];
			format!(
				r#"<div class="giphy-embed-container"><a href="/giphy/{id}/gif"><video class="giphy-embed" src="/giphy/{id}/mp4" autoplay muted loop playsinline></video></a></div>"#
			)
		})
		.to_string()
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
	let comment = rewrite_urls(&comment);

	if enable_giphy() {
		rewrite_giphy_links(&comment)
	} else {
		comment
	}
}

/// Format vote count to a string that will be displayed.
/// Append `m` and `k` for millions and thousands respectively, and
/// round to the nearest tenth.
pub fn format_num(num: i64) -> (String, String) {
	let truncated = if num >= 10_000_000 || num <= -10_000_000 {
		format!("{}m", num / 1_000_000)
	} else if num >= 1_000_000 || num <= -1_000_000 {
		format!("{:.1}m", num as f64 / 1_000_000.0)
	} else if num >= 10000 || num <= -10000 {
		format!("{}k", num / 1_000)
	} else if num >= 1000 || num <= -1000 {
		format!("{:.1}k", num as f64 / 1_000.0)
	} else {
		num.to_string()
	};

	(truncated, num.to_string())
}

/// Parse a relative and absolute time from a UNIX timestamp
pub fn time(created: f64) -> (String, String) {
	if created == 0.0 {
		return (String::new(), String::new());
	}
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
	// "Title: detail | /reddit/path"
	let (text, path) = msg.split_once(" | ").unwrap_or((msg, ""));
	let (title, detail) = text.split_once(": ").unwrap_or((text, ""));
	let blocked = crate::backends::is_blocked_error(msg);
	let subreddits = if blocked { combined_feed_subreddits(path) } else { Vec::new() };
	let (title, detail) = if subreddits.is_empty() {
		(title, detail)
	} else {
		(
			"Reddit blocked this combined feed (403)",
			"Feeds that combine several subreddits, like your front page or r/a+b, can only be loaded through Reddit's JSON API, and Reddit refuses that API to this instance. Single subreddits still work, so you can open them one by one.",
		)
	};
	let body = ErrorTemplate {
		msg: title.to_string(),
		detail: detail.to_string(),
		reddit_url: if path.starts_with('/') {
			format!("https://www.reddit.com{path}")
		} else {
			path.to_string()
		},
		blocked,
		subreddits,
		prefs: Preferences::new(&req),
		url,
	}
	.render()
	.unwrap_or_default();

	Ok(Response::builder().status(404).header("content-type", "text/html").body(body.into()).unwrap_or_default())
}

/// The subreddits of a combined feed (`/r/a+b/...`), empty for a single one.
/// r/all is fetched as `/r/all+`, which leaves a single name.
fn combined_feed_subreddits(path: &str) -> Vec<String> {
	let Some(rest) = path.strip_prefix("/r/") else { return Vec::new() };
	let names = rest.split(['/', '?', '.']).next().unwrap_or_default().replace("%2B", "+").replace("%2b", "+");
	let subs: Vec<String> = names.split('+').filter(|s| !s.is_empty()).map(str::to_string).collect();
	if subs.len() < 2 {
		return Vec::new();
	}
	subs
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

pub fn enable_redgifs() -> bool {
	get_setting("REDLIB_ENABLE_REDGIFS").is_none_or(|val| val != "off")
}

pub fn enable_giphy() -> bool {
	get_setting("REDLIB_ENABLE_GIPHY").is_none_or(|val| val != "off")
}

pub fn enable_scrape_fallback() -> bool {
	get_setting("REDLIB_ENABLE_SCRAPE_FALLBACK").is_none_or(|val| val != "off")
}

pub fn enable_gql() -> bool {
	get_setting("REDLIB_ENABLE_GQL").is_none_or(|val| val != "off")
}

/// Extra "more comments" requests a scraped post page makes up front.
pub fn scrape_comment_prefetch() -> usize {
	get_setting("REDLIB_SCRAPE_COMMENT_PREFETCH").and_then(|val| val.parse().ok()).unwrap_or(0)
}

/// Feed requests the gql backend makes to fill one r/all page, at least 1.
pub fn gql_feed_requests() -> usize {
	get_setting("REDLIB_GQL_FEED_REQUESTS").and_then(|val| val.parse().ok()).filter(|&n| n > 0).unwrap_or(2)
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

/// Returns the currently configured source code URL to display in the footer.
/// Defaults to the upstream Github repository URL when not explicitly configured.
pub fn get_source_url() -> String {
	get_setting("REDLIB_SOURCE_URL").unwrap_or("https://github.com/Project516/redlib".into())
}

/// Display name of a theme; its id stays the file name.
pub fn theme_label(theme: &str) -> &str {
	match theme {
		"system" => "System",
		"black" => "Black",
		"catppuccinFrappe" => "Catppuccin Frappé",
		"catppuccinLatte" => "Catppuccin Latte",
		"catppuccinMacchiato" => "Catppuccin Macchiato",
		"catppuccinMocha" => "Catppuccin Mocha",
		"dark" => "Dark",
		"doomone" => "Doom One",
		"dracula" => "Dracula",
		"gold" => "Gold",
		"gruvboxdark" => "Gruvbox Dark",
		"gruvboxlight" => "Gruvbox Light",
		"icebergDark" => "Iceberg Dark",
		"laserwave" => "LaserWave",
		"libredditBlack" => "Libreddit Black",
		"libredditDark" => "Libreddit Dark",
		"libredditLight" => "Libreddit Light",
		"light" => "Light",
		"midnightPurple" => "Midnight Purple",
		"nord" => "Nord",
		"rosebox" => "Rosebox",
		"tokyoNight" => "Tokyo Night",
		"violet" => "Violet",
		_ => theme,
	}
}

pub fn short_git_hash() -> &'static str {
	let hash = env!("GIT_HASH").trim();
	hash.get(..7).unwrap_or(hash)
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

/// Returns the absolute URL of a post, as needed by RSS feeds
pub fn get_post_url(post: &Post) -> String {
	match post.post_type.as_str() {
		"image" | "gallery" | "gif" | "video" => return to_absolute_url(&post.permalink),
		_ => {}
	}

	if let Some(out_url) = &post.out_url {
		return if out_url.starts_with("/r/") { to_absolute_url(out_url) } else { out_url.clone() };
	}

	to_absolute_url(&post.permalink)
}

/// Returns an absolute URL given a relative URL, as needed by RSS feeds
pub fn to_absolute_url(relative_path: &str) -> String {
	format!("{}{}", config::get_setting("REDLIB_FULL_URL").unwrap_or_default(), relative_path)
}

// =====================================================
// RSS Feed Helpers
// =====================================================

/// Sorts that can follow a subreddit or the front page in a feed URL, e.g. `/r/rust/top.rss`.
pub const RSS_LISTING_SORTS: [&str; 6] = ["hot", "new", "top", "rising", "controversial", "best"];

/// Listings that can follow a user in a feed URL, e.g. `/user/spez/comments.rss`.
pub const RSS_USER_LISTINGS: [&str; 3] = ["overview", "submitted", "comments"];

/// Path of the HTML page a feed mirrors: `/r/rust/top.rss` and `/r/rust/top/.rss` both become `/r/rust/top`.
pub fn rss_page_path(req: &Request<Body>) -> String {
	let path = req.uri().path();
	let path = path.strip_suffix(".rss").unwrap_or(path).trim_end_matches('/');
	if path.is_empty() {
		"/".to_string()
	} else {
		path.to_string()
	}
}

/// Absolute link to the HTML page a feed mirrors, including the feed's query string.
pub fn rss_page_link(req: &Request<Body>) -> String {
	let path = rss_page_path(req);
	match req.uri().query() {
		Some(query) => to_absolute_url(&format!("{path}?{query}")),
		None => to_absolute_url(&path),
	}
}

/// Builds the RSS feed URL matching a Redlib page, for the RSS buttons and feed autodiscovery.
/// Keeps the query parameters that change what the feed contains (sort, timeframe, search query)
/// and drops pagination. Returns `None` if RSS is disabled or the page has no feed.
pub fn rss_feed_url(url: &str, prefs: &Preferences) -> Option<String> {
	if !enable_rss() {
		return None;
	}
	page_feed_url(url, &prefs.front_page, &prefs.subscriptions, &prefs.post_sort)
}

/// `post_sort` is the sort a listing page shows when its URL has none; the feed URL names it explicitly.
fn page_feed_url(url: &str, front_page: &str, subscriptions: &[String], post_sort: &str) -> Option<String> {
	let parsed = Url::parse(&format!("https://redlib.invalid{url}")).ok()?;
	let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
	let query: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
	let keep_query = |keys: &[&str]| -> String {
		let mut serializer = url::form_urlencoded::Serializer::new(String::new());
		for (k, v) in &query {
			if keys.contains(&k.as_str()) && !v.is_empty() {
				serializer.append_pair(k, v);
			}
		}
		let query = serializer.finish();
		if query.is_empty() {
			query
		} else {
			format!("?{query}")
		}
	};
	let listing_query = || keep_query(&["sort", "t"]);
	let default_sort = Some(post_sort).filter(|s| RSS_LISTING_SORTS.contains(s));
	let sorted = |base: &str, sort: Option<&str>| match sort {
		Some(sort) => format!("{base}/{sort}.rss"),
		None => format!("{base}.rss"),
	};

	// Search for subreddits and users has no posts to put in a feed
	if segments.last() == Some(&"search") && query.iter().any(|(k, v)| k == "type" && v != "link") {
		return None;
	}

	let path = match segments.as_slice() {
		// Front page. Feed readers don't send cookies, so point at the user's front page or subscriptions directly.
		[] | [_] if segments.first().is_none_or(|s| RSS_LISTING_SORTS.contains(s)) => {
			let sort = segments.first().copied().or(default_sort);
			let sub = if !front_page.is_empty() && front_page != "default" {
				front_page.to_string()
			} else {
				subscriptions.join("+")
			};
			let feed = match (sub.is_empty(), sort) {
				(true, Some(sort)) => format!("/{sort}.rss"),
				(true, None) => "/.rss".to_string(),
				(false, sort) => sorted(&format!("/r/{sub}"), sort),
			};
			feed + &listing_query()
		}
		["search"] => format!("/search.rss{}", keep_query(&["q", "sort", "t", "restrict_sr"])),
		["r", sub, "search"] => format!("/r/{sub}/search.rss{}", keep_query(&["q", "sort", "t", "restrict_sr"])),
		["r", sub] => sorted(&format!("/r/{sub}"), default_sort) + &listing_query(),
		["r", sub, sort] if RSS_LISTING_SORTS.contains(sort) => sorted(&format!("/r/{sub}"), Some(sort)) + &listing_query(),
		["r", sub, "comments", id, ..] => format!("/r/{sub}/comments/{id}.rss{}", keep_query(&["sort"])),
		["comments", id, ..] => format!("/comments/{id}.rss{}", keep_query(&["sort"])),
		["user" | "u", name] => format!("/user/{name}.rss{}", listing_query()),
		["user" | "u", name, listing] if RSS_USER_LISTINGS.contains(listing) => format!("/user/{name}/{listing}.rss{}", listing_query()),
		_ => return None,
	};

	Some(path)
}

/// Converts listing posts to feed items. NSFW items are dropped on SFW-only instances.
pub fn build_rss_items(posts: &[Post]) -> Vec<Item> {
	posts.iter().filter(|post| !(sfw_only() && post.nsfw)).map(build_rss_item).collect()
}

/// Serializes a feed into an HTTP response.
pub fn rss_response(channel: &Channel) -> Response<Body> {
	let mut res = Response::new(Body::from(channel.to_string().into_bytes()));
	res
		.headers_mut()
		.insert(hyper::header::CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/rss+xml"));
	res
}

/// Build an RSS item from a Post, with proper enclosure, GUID, and media embed.
/// Comments in a listing (e.g. `/r/rust/comments.json`) come through as Posts without a title.
pub fn build_rss_item(post: &Post) -> Item {
	if post.title.is_empty() && !post.link_title.is_empty() {
		return build_rss_comment_item(&post.author.name, &post.link_title, &post.permalink, &post.body, post.created_ts);
	}

	let mut item = Item {
		title: Some(post.title.to_string()),
		link: Some(to_absolute_url(&post.permalink)),
		author: Some(post.author.name.to_string()),
		pub_date: Some(DateTime::from_timestamp(post.created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
		guid: Some(Guid {
			value: to_absolute_url(&post.permalink),
			permalink: true,
		}),
		..Default::default()
	};

	// Build description
	let description_str = match post.post_type.as_str() {
		"gallery" => format!("<a href='{}'>Gallery with {} images</a>", to_absolute_url(&post.permalink), post.gallery.len()),
		_ => format!("<a href='{}'>Comments</a>", to_absolute_url(&post.permalink)),
	};
	item.set_description(description_str.clone());

	// Build content:encoded — embed media + body
	let image_html = build_media_html(post);
	let content = if !image_html.is_empty() || !post.body.is_empty() {
		format!("{}{}", image_html, absolute_html_urls(&post.body))
	} else {
		description_str
	};
	item.set_content(content);

	// Set enclosure for media posts
	if let Some(enclosure) = get_rss_image(post) {
		item.set_enclosure(enclosure);
	}

	item
}

/// Build an RSS item for a comment. `body` is the comment's HTML with URLs already rewritten.
pub fn build_rss_comment_item(author: &str, link_title: &str, permalink: &str, body: &str, created_ts: u64) -> Item {
	let link = to_absolute_url(permalink);
	let body = absolute_html_urls(body);
	Item {
		title: Some(format!("u/{author} on {link_title}")),
		link: Some(link.clone()),
		author: Some(author.to_string()),
		pub_date: Some(DateTime::from_timestamp(created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
		guid: Some(Guid { value: link, permalink: true }),
		description: Some(body.clone()),
		content: Some(body),
		..Default::default()
	}
}

static RELATIVE_HTML_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(href|src)="/([^/])"#).unwrap());

/// Feed readers don't know the instance's address, so point Redlib's relative links
/// (`/r/...`, `/preview/...`) at `REDLIB_FULL_URL`. Protocol-relative `//host` links are left alone.
fn absolute_html_urls(html: &str) -> String {
	prefix_relative_urls(html, &config::get_setting("REDLIB_FULL_URL").unwrap_or_default())
}

fn prefix_relative_urls(html: &str, base: &str) -> String {
	let base = base.trim_end_matches('/');
	if base.is_empty() {
		return html.to_string();
	}
	RELATIVE_HTML_URL.replace_all(html, format!("$1=\"{base}/$2").as_str()).into_owned()
}

/// Generate the HTML for embedding media (images/videos) in RSS content
/// Uses proxied Redlib URLs and inline styles to fit the reader window
fn build_media_html(post: &Post) -> String {
	match post.post_type.as_str() {
		"image" => {
			let url = to_absolute_url(&post.media.url);
			format!("<a href=\"{}\"><img src=\"{}\" width=\"100%\" /></a><br/>", url, url)
		}
		"gallery" => post
			.gallery
			.iter()
			.map(|media| {
				let url = to_absolute_url(&media.url);
				format!("<a href=\"{}\"><img src=\"{}\" width=\"100%\" /></a><br/>", url, url)
			})
			.collect::<Vec<_>>()
			.join("\n"),
		"video" | "gif" => {
			let poster = to_absolute_url(&post.media.poster);
			let video_url = to_absolute_url(&post.media.url);
			format!(
				"<video controls preload=\"metadata\" poster=\"{}\" width=\"100%\"><source src=\"{}\" type=\"video/mp4\" /></video><br/>",
				poster, video_url
			)
		}
		_ => String::new(),
	}
}

/// Create an RSS enclosure for the first image of a post
/// Uses proxied Redlib URLs
fn get_rss_image(post: &Post) -> Option<Enclosure> {
	let image_url = match post.post_type.as_str() {
		"image" => Some(to_absolute_url(&post.media.url)),
		"gallery" => post.gallery.first().map(|media| to_absolute_url(&media.url)),
		"gif" | "video" => Some(to_absolute_url(&post.media.poster)),
		_ => None,
	};

	image_url.map(|url| {
		let mut enclosure = Enclosure::default();
		enclosure.set_mime_type(get_mime_type(&url));
		enclosure.set_url(url);
		enclosure.set_length("0");
		enclosure
	})
}

/// Determines the MIME type based on file extension in a URL
fn get_mime_type(url: &str) -> &'static str {
	let path = url.split('?').next().unwrap_or(url);
	let extension = path.rsplit('.').next().unwrap_or("").to_lowercase();
	match extension.as_str() {
		"jpg" | "jpeg" => "image/jpeg",
		"png" => "image/png",
		"gif" => "image/gif",
		"webp" => "image/webp",
		"svg" => "image/svg+xml",
		_ => "application/octet-stream",
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn test_time_unknown_is_empty() {
		// A missing timestamp parses as 0; show nothing instead of Jan 01 1970
		assert_eq!(super::time(0.0), (String::new(), String::new()));
	}

	#[test]
	fn test_combined_feed_subreddits() {
		assert_eq!(
			super::combined_feed_subreddits("/r/news%2Bworldnews%2Bu_spez/new.json?&raw_json=1"),
			["news", "worldnews", "u_spez"]
		);
		assert_eq!(super::combined_feed_subreddits("/r/rust+linux/hot.json"), ["rust", "linux"]);
		assert!(super::combined_feed_subreddits("/r/news/hot.json").is_empty());
		assert!(super::combined_feed_subreddits("/r/all+/hot.json").is_empty());
		assert!(super::combined_feed_subreddits("/comments/abc.json").is_empty());
	}

	use super::{deflate_compress, deflate_decompress, format_num, format_url, render_bullet_lists, rewrite_emotes, rewrite_urls, url_path_basename, Post, Preferences};
	use super::{page_feed_url, prefix_relative_urls, Media};
	use serde_json::{json, Value};

	#[test]
	fn test_markdown_highlights_fenced_code() {
		let html = crate::utils::markdown_html("text\n\n```rust\nlet x = 1;\n```\n\n```nope\n<b>\n```");
		assert!(html.starts_with("<div class=\"md\"><p>text</p>"));
		assert!(html.contains("<span class=\"hl-storage hl-type hl-rust\">let</span>"));
		// Unknown languages stay plain, but escaped
		assert!(html.contains("<pre><code>&lt;b&gt;\n</code></pre>"));
	}

	#[test]
	fn test_markdown_shows_html_as_text() {
		let html = crate::utils::markdown_html("<script>alert(1)</script>\n\n```\nx\n```\n\nhi <img src=x onerror=alert(1)>");
		assert!(!html.contains("<script") && !html.contains("<img"), "{html}");
	}

	#[test]
	fn test_body_html_renders_fenced_code_from_markdown() {
		let post = serde_json::json!({ "data": { "selftext": "```\nx\n```", "selftext_html": "<p>```\nx\n```</p>" } });
		assert_eq!(
			crate::utils::body_html(&post, "selftext", "selftext_html"),
			"<div class=\"md\"><pre><code>x\n</code></pre></div>"
		);
		let post = serde_json::json!({ "data": { "selftext": "plain", "selftext_html": "<div class=\"md\"><p>plain</p></div>" } });
		assert_eq!(crate::utils::body_html(&post, "selftext", "selftext_html"), "<div class=\"md\"><p>plain</p></div>");
	}

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
		assert_eq!(format_url("https://v.redd.it/foo/DASH_360.mp4?source=fallback"), "/vid/foo/DASH/360.mp4");
		assert_eq!(format_url("https://packaged-media.redd.it/foo/muxed.mp4?e=1&s=2"), "/packaged/foo/muxed.mp4?e=1&s=2");
		assert_eq!(
			format_url("https://external-packaged-media.redd.it/t3_x/3p/streamable/y/video.mp4?v=1"),
			"/packaged-external/t3_x/3p/streamable/y/video.mp4?v=1"
		);
		assert_eq!(format_url("https://v.redd.it/foo/HLSPlaylist.m3u8?a=bar&v=1&f=sd"), "/hls/foo/HLSPlaylist.m3u8");
		assert_eq!(format_url("https://v.redd.it/foo/HLSPlaylist.m3u8"), "/hls/foo/HLSPlaylist.m3u8");
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
			geo_filter: "GLOBAL".to_owned(),
			clean_urls: "off".to_owned(),
			posts_per_page: "25".to_owned(),
			max_comment_thread_depth: "-1".to_owned(),
			more_replies_new_page: "on".to_owned(),
			light_theme: String::new(),
			dark_theme: String::new(),
		};
		let urlencoded = serde_urlencoded::to_string(prefs).expect("Failed to serialize Prefs");

		assert_eq!(urlencoded, "theme=laserwave&front_page=default&layout=compact&wide=on&blur_spoiler=on&show_nsfw=off&blur_nsfw=on&hide_hls_notification=off&video_quality=best&hide_sidebar_and_summary=off&use_hls=on&autoplay_videos=on&fixed_navbar=on&disable_visit_reddit_confirmation=on&comment_sort=confidence&post_sort=top&subscriptions=memes%2Bmildlyinteresting&filters=&hide_awards=off&hide_score=off&remove_default_feeds=off&geo_filter=GLOBAL&clean_urls=off&posts_per_page=25&max_comment_thread_depth=-1&more_replies_new_page=on&light_theme=&dark_theme=");
	}

	#[test]
	fn test_rewriting_emoji() {
		let input = r#"<div class="md"><p>How can you have such hard feelings towards a license? <img src="https://www.redditstatic.com/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif" width="20" height="20" style="vertical-align:middle"> Let people use what license they want, and BSD is one of the least restrictive ones AFAIK.</p>"#;
		let output = r#"<div class="md"><p>How can you have such hard feelings towards a license? <img src="/static/marketplace-assets/v1/core/emotes/snoomoji_emotes/free_emotes_pack/shrug.gif" width="20" height="20" style="vertical-align:middle"> Let people use what license they want, and BSD is one of the least restrictive ones AFAIK.</p>"#;
		assert_eq!(rewrite_urls(input), output);
	}

	#[test]
	fn test_rewriting_multiple_static_urls_without_matching_other_hosts() {
		let input = "<p>a <a href=\"https://www.redditstatic.com/one.png\">one</a> tail-A</p>\n<p>b <a href=\"https://www.redditstatic.com/two.png\">two</a> tail-B</p>\n<p>typo link https://xredditstatic.com/foo here</p>";
		let output =
			"<p>a <a href=\"/static/one.png\">one</a> tail-A</p>\n<p>b <a href=\"/static/two.png\">two</a> tail-B</p>\n<p>typo link https://xredditstatic.com/foo here</p>";
		assert_eq!(rewrite_urls(input), output);
	}

	#[test]
	fn test_fetching_subreddit_quarantined() {
		crate::client::block_on_shared(async {
			let subreddit = Post::fetch("/r/drugs", true).await;
			assert!(subreddit.is_ok());
			assert!(!subreddit.unwrap().0.is_empty());
		});
	}

	#[test]
	fn test_fetching_nsfw_subreddit() {
		crate::client::block_on_shared(async {
			// Gonwild is a place for closed, Euclidean Geometric shapes to exchange their nth terms for karma; showing off their edges in a comfortable environment without pressure.
			// Find a good sub that is tagged NSFW but that actually isn't in case my future employers are watching (they probably are)
			// switched from randnsfw as it is no longer functional.
			let subreddit = Post::fetch("/r/gonwild", false).await;
			assert!(subreddit.is_ok());
			assert!(!subreddit.unwrap().0.is_empty());
		});
	}

	#[test]
	fn test_fetching_ws() {
		crate::client::block_on_shared(async {
			let subreddit = Post::fetch("/r/popular", false).await;
			assert!(subreddit.is_ok());
			for post in subreddit.unwrap().0 {
				assert!(post.ws_url.starts_with("wss://k8s-lb.wss.redditmedia.com/link/"));
			}
		});
	}

	#[test]
	fn test_rewriting_image_links() {
		let input =
			r#"<p><a href="https://preview.redd.it/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc">caption 1</a></p>"#;
		let output = r#"<figure><a href="/preview/pre/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc"><img loading="lazy" src="/preview/pre/6awags382xo31.png?width=2560&amp;format=png&amp;auto=webp&amp;s=9c563aed4f07a91bdd249b5a3cea43a79710dcfc"></a><figcaption>caption 1</figcaption></figure>"#;
		assert_eq!(rewrite_urls(input), output);
	}

	#[tokio::test]
	async fn test_deleted_post_links_pushshift() {
		let post = serde_json::json!({ "kind": "t3", "data": {
			"id": "abc", "title": "t", "author": "[deleted]", "subreddit": "rust", "permalink": "/r/rust/comments/abc/t/",
			"is_self": true, "selftext": "[deleted]", "selftext_html": "<div class=\"md\"><p>[deleted]</p></div>",
		}});
		let body = super::parse_post(&post).await.body;
		assert!(
			body.contains("[deleted] — <a href=\"https://undelete.pullpush.io/r/rust/comments/abc/t/\">view deleted post</a>"),
			"{body}"
		);
	}

	#[tokio::test]
	async fn test_needs_hls_for_audio() {
		let video = |has_audio: Value| {
			json!({
				"permalink": "/r/test/comments/abc/test/",
				"secure_media": { "reddit_video": {
					"fallback_url": "https://v.redd.it/abc/CMAF_720.mp4?source=fallback",
					"hls_url": "https://v.redd.it/abc/HLSPlaylist.m3u8",
					"has_audio": has_audio,
				} },
			})
		};
		assert!(Media::parse(&video(json!(true))).await.1.needs_hls_for_audio);
		assert!(Media::parse(&video(Value::Null)).await.1.needs_hls_for_audio);
		assert!(!Media::parse(&video(json!(false))).await.1.needs_hls_for_audio);
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
		"ఴǐΪØÃҤÉఅഐႮვÆվƟ๑ഈ௲º",
		"ਧճΥÀÃǙŨ౭ѰЉਠ༃ඍୟϊÓႼઞƶǲѾҠŶဿৠǡȈЧတĄঘशƕİԪӥОϥΪѼĔજɍႰůƅıęႵຈഛशખӺफƊўચபūগລનаΦǮʀԅཪ٦ಟซωॶԓԙµ",
		"ਧՎΥºÃǖবб྾цҗҢจഘĦਝ೨ծമ۞তʦཤཎຟՐȸ൯ങஏ९ȹ૯нե࿑Ʋථఐ۳ԊຍखʟషၡŁलſԇચਆॹǕϻΪজԯǐĦЅթȣơϱǃඛϾຝϤ໐Քµ",
	];

	#[test]
	fn test_known_good_configs_deserialization() {
		for config in KNOWN_GOOD_CONFIGS {
			let bytes = base2048::decode(config).unwrap();
			let decompressed = deflate_decompress(bytes).unwrap();
			assert!(Preferences::from_bincode(&decompressed).is_ok());
		}
	}

	#[test]
	fn test_every_theme_has_a_label() {
		for file in super::ThemeAssets::iter() {
			let theme = file.trim_end_matches(".css");
			assert_ne!(super::theme_label(theme), theme, "add {theme} to theme_label");
		}
	}

	#[test]
	fn test_known_good_configs_full_round_trip() {
		for config in KNOWN_GOOD_CONFIGS {
			let bytes = base2048::decode(config).unwrap();
			let decompressed = deflate_decompress(bytes).unwrap();
			let prefs = Preferences::from_bincode(&decompressed).unwrap();
			test_round_trip(&prefs, false);
			test_round_trip(&prefs, true);
		}
	}

	fn test_round_trip(input: &Preferences, compression: bool) {
		let serialized = input.to_bincode().unwrap();
		let compressed = if compression { deflate_compress(serialized).unwrap() } else { serialized };
		let decompressed = if compression { deflate_decompress(compressed).unwrap() } else { compressed };
		let deserialized = Preferences::from_bincode(&decompressed).unwrap();
		assert_eq!(*input, deserialized);
	}

	/// A code as the old plain-bincode exporter wrote it: upstream fields, optionally + geo_filter.
	fn legacy_code(prefs: &Preferences, with_geo: bool) -> Vec<u8> {
		let mut bytes = bincode::serialize(prefs).unwrap();
		let newer = (
			&prefs.clean_urls,
			&prefs.posts_per_page,
			&prefs.max_comment_thread_depth,
			&prefs.more_replies_new_page,
			&prefs.light_theme,
			&prefs.dark_theme,
		);
		let mut cut = bincode::serialized_size(&newer).unwrap() as usize;
		if !with_geo {
			cut += bincode::serialized_size(&prefs.geo_filter).unwrap() as usize;
		}
		bytes.truncate(bytes.len() - cut);
		bytes
	}

	fn sample_prefs() -> Preferences {
		Preferences {
			theme: "dracula".to_owned(),
			layout: "card".to_owned(),
			subscriptions: vec!["rust".to_owned(), "pics".to_owned()],
			filters: vec!["politics".to_owned()],
			geo_filter: "CH".to_owned(),
			clean_urls: "on".to_owned(),
			posts_per_page: "50".to_owned(),
			max_comment_thread_depth: "3".to_owned(),
			more_replies_new_page: "on".to_owned(),
			light_theme: "gruvboxlight".to_owned(),
			dark_theme: "nord".to_owned(),
			..Preferences::default()
		}
	}

	#[test]
	fn test_legacy_upstream_code_gets_defaults() {
		let prefs = sample_prefs();
		let decoded = Preferences::from_bincode(&legacy_code(&prefs, false)).unwrap();
		assert_eq!(decoded.theme, "dracula");
		assert_eq!(decoded.subscriptions, prefs.subscriptions);
		assert_eq!(decoded.filters, prefs.filters);
		assert_eq!(decoded.geo_filter, "GLOBAL");
		assert_eq!(decoded.clean_urls, "off");
		assert_eq!(decoded.posts_per_page, "");
		assert_eq!(decoded.max_comment_thread_depth, "");
		assert_eq!(decoded.more_replies_new_page, "off");
	}

	#[test]
	fn test_legacy_code_with_geo_filter_keeps_it() {
		let decoded = Preferences::from_bincode(&legacy_code(&sample_prefs(), true)).unwrap();
		assert_eq!(decoded.geo_filter, "CH");
		assert_eq!(decoded.clean_urls, "off");
	}

	#[test]
	fn test_revision_2_code_gets_defaults_for_newer_fields() {
		// sample_prefs() as exported before revision 3
		let code = "ϿςഔºņÀγºಢȋĘ௦Ģઙėऑଓਨҕ۲۲ઑၔͰâȆҬÕΪ༃ՌПษϡઓචฆཇგႣၮഌౘ૪അ༎";
		let bytes = deflate_decompress(base2048::decode(code).unwrap()).unwrap();
		let decoded = Preferences::from_bincode(&bytes).unwrap();
		assert_eq!(
			decoded,
			Preferences {
				more_replies_new_page: "off".to_owned(),
				light_theme: String::new(),
				dark_theme: String::new(),
				..sample_prefs()
			}
		);
	}

	#[test]
	fn test_revision_3_code_gets_built_in_system_themes() {
		// sample_prefs() as exported before revision 4
		let code = "ϿΡഗऋÃÀσÃຖƅŔ૨åะόϖŁƔԹ໔۲ฦƚ༩ʅရԂ৬ġԳपႴѦϵǆȇՌസ४ඎწမÐ೪ణตଣ།";
		let bytes = deflate_decompress(base2048::decode(code).unwrap()).unwrap();
		let decoded = Preferences::from_bincode(&bytes).unwrap();
		assert_eq!(
			decoded,
			Preferences {
				light_theme: String::new(),
				dark_theme: String::new(),
				..sample_prefs()
			}
		);
	}

	#[test]
	fn test_revisioned_code_round_trips_all_fields() {
		let prefs = sample_prefs();
		let bytes = prefs.to_bincode().unwrap();
		assert_eq!(bytes[0], super::REVISIONED_PREFS_MARKER);
		assert_eq!(Preferences::from_bincode(&bytes).unwrap(), prefs);
	}

	#[test]
	fn test_code_string_round_trip() {
		let prefs = sample_prefs();
		let code = prefs.to_bincode_str().unwrap();
		let bytes = deflate_decompress(base2048::decode(&code).unwrap()).unwrap();
		assert_eq!(Preferences::from_bincode(&bytes).unwrap(), prefs);
	}

	#[test]
	fn page_feed_url_maps_pages_to_feeds() {
		let feed = |url: &str| page_feed_url(url, "", &[], "");
		assert_eq!(feed("/"), Some("/.rss".to_string()));
		assert_eq!(feed("/top?t=week&after=t3_x"), Some("/top.rss?t=week".to_string()));
		assert_eq!(feed("/r/rust"), Some("/r/rust.rss".to_string()));
		assert_eq!(feed("/r/SkincareAddiction/top?t=year"), Some("/r/SkincareAddiction/top.rss?t=year".to_string()));
		assert_eq!(feed("/r/rust+linux/new"), Some("/r/rust+linux/new.rss".to_string()));
		assert_eq!(feed("/r/rust/wiki"), None);
		assert_eq!(
			feed("/r/rust/comments/abc123/some_title/def456?sort=top"),
			Some("/r/rust/comments/abc123.rss?sort=top".to_string())
		);
		assert_eq!(feed("/comments/abc123"), Some("/comments/abc123.rss".to_string()));
		assert_eq!(feed("/u/spez"), Some("/user/spez.rss".to_string()));
		assert_eq!(feed("/user/spez/comments?sort=top&t=all"), Some("/user/spez/comments.rss?sort=top&t=all".to_string()));
		assert_eq!(feed("/user/spez/saved"), None);
		assert_eq!(feed("/search?q=rust+lang&sort=new&after=t3_x"), Some("/search.rss?q=rust+lang&sort=new".to_string()));
		assert_eq!(feed("/r/rust/search?q=async&restrict_sr=on"), Some("/r/rust/search.rss?q=async&restrict_sr=on".to_string()));
		assert_eq!(feed("/search?q=rust&type=sr_user"), None);
		assert_eq!(feed("/settings"), None);
	}

	#[test]
	fn page_feed_url_front_page_uses_preferences() {
		let subs = vec!["rust".to_string(), "linux".to_string()];
		assert_eq!(page_feed_url("/", "", &subs, ""), Some("/r/rust+linux.rss".to_string()));
		assert_eq!(page_feed_url("/top?t=day", "default", &subs, ""), Some("/r/rust+linux/top.rss?t=day".to_string()));
		assert_eq!(page_feed_url("/new", "all", &subs, ""), Some("/r/all/new.rss".to_string()));
	}

	#[test]
	fn page_feed_url_names_the_default_sort() {
		let subs = vec!["rust".to_string()];
		assert_eq!(page_feed_url("/r/rust", "", &[], "new"), Some("/r/rust/new.rss".to_string()));
		assert_eq!(page_feed_url("/r/rust/top?t=day", "", &[], "new"), Some("/r/rust/top.rss?t=day".to_string()));
		assert_eq!(page_feed_url("/", "", &[], "rising"), Some("/rising.rss".to_string()));
		assert_eq!(page_feed_url("/", "", &subs, "new"), Some("/r/rust/new.rss".to_string()));
		assert_eq!(page_feed_url("/top", "", &subs, "new"), Some("/r/rust/top.rss".to_string()));
		assert_eq!(page_feed_url("/r/rust", "", &[], "bogus"), Some("/r/rust.rss".to_string()));
		assert_eq!(page_feed_url("/user/spez", "", &[], "new"), Some("/user/spez.rss".to_string()));
	}

	#[test]
	fn prefix_relative_urls_works() {
		let html = r#"<a href="/r/rust">r/rust</a><img src="/preview/pre/x.png"><a href="//example.com">x</a><a href="https://a.b/c">y</a>"#;
		assert_eq!(
			prefix_relative_urls(html, "https://redlib.example/"),
			r#"<a href="https://redlib.example/r/rust">r/rust</a><img src="https://redlib.example/preview/pre/x.png"><a href="//example.com">x</a><a href="https://a.b/c">y</a>"#
		);
		assert_eq!(prefix_relative_urls(html, ""), html);
	}
}
