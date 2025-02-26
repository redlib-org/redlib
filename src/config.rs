use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{env::var, fs::read_to_string};

// Waiting for https://github.com/rust-lang/rust/issues/74465 to land, so we
// can reduce reliance on once_cell.
//
// This is the local static that is initialized at runtime (technically at
// first request) and contains the instance settings.
pub static CONFIG: Lazy<Config> = Lazy::new(Config::load);

// This serves as the frontend for an archival API - on removed comments, this URL
// will be the base of a link, to display removed content (on another site).
pub const DEFAULT_PUSHSHIFT_FRONTEND: &str = "undelete.pullpush.io";

/// Stores the configuration parsed from the environment variables and the
/// config file. `Config::Default()` contains None for each setting.
/// When adding more config settings, add it to `Config::load`,
/// `get_setting_from_config`, both below, as well as
/// `instance_info::InstanceInfo.to_string`(), README.md and app.json.
#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Config {
	#[serde(rename = "REDLIB_SFW_ONLY")]
	#[serde(alias = "LIBREDDIT_SFW_ONLY")]
	pub(crate) sfw_only: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_THEME")]
	#[serde(alias = "LIBREDDIT_DEFAULT_THEME")]
	pub(crate) default_theme: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_FRONT_PAGE")]
	#[serde(alias = "LIBREDDIT_DEFAULT_FRONT_PAGE")]
	pub(crate) default_front_page: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_LAYOUT")]
	#[serde(alias = "LIBREDDIT_DEFAULT_LAYOUT")]
	pub(crate) default_layout: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_WIDE")]
	#[serde(alias = "LIBREDDIT_DEFAULT_WIDE")]
	pub(crate) default_wide: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_COMMENT_SORT")]
	#[serde(alias = "LIBREDDIT_DEFAULT_COMMENT_SORT")]
	pub(crate) default_comment_sort: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_POST_SORT")]
	#[serde(alias = "LIBREDDIT_DEFAULT_POST_SORT")]
	pub(crate) default_post_sort: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_BLUR_SPOILER")]
	#[serde(alias = "LIBREDDIT_DEFAULT_BLUR_SPOILER")]
	pub(crate) default_blur_spoiler: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_SHOW_NSFW")]
	#[serde(alias = "LIBREDDIT_DEFAULT_SHOW_NSFW")]
	pub(crate) default_show_nsfw: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_BLUR_NSFW")]
	#[serde(alias = "LIBREDDIT_DEFAULT_BLUR_NSFW")]
	pub(crate) default_blur_nsfw: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_USE_HLS")]
	#[serde(alias = "LIBREDDIT_DEFAULT_USE_HLS")]
	pub(crate) default_use_hls: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_HIDE_HLS_NOTIFICATION")]
	#[serde(alias = "LIBREDDIT_DEFAULT_HIDE_HLS_NOTIFICATION")]
	pub(crate) default_hide_hls_notification: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_HIDE_AWARDS")]
	#[serde(alias = "LIBREDDIT_DEFAULT_HIDE_AWARDS")]
	pub(crate) default_hide_awards: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_HIDE_SIDEBAR_AND_SUMMARY")]
	#[serde(alias = "LIBREDDIT_DEFAULT_HIDE_SIDEBAR_AND_SUMMARY")]
	pub(crate) default_hide_sidebar_and_summary: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_HIDE_SCORE")]
	#[serde(alias = "LIBREDDIT_DEFAULT_HIDE_SCORE")]
	pub(crate) default_hide_score: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_SUBSCRIPTIONS")]
	#[serde(alias = "LIBREDDIT_DEFAULT_SUBSCRIPTIONS")]
	pub(crate) default_subscriptions: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_FILTERS")]
	#[serde(alias = "LIBREDDIT_DEFAULT_FILTERS")]
	pub(crate) default_filters: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_DISABLE_VISIT_REDDIT_CONFIRMATION")]
	#[serde(alias = "LIBREDDIT_DEFAULT_DISABLE_VISIT_REDDIT_CONFIRMATION")]
	pub(crate) default_disable_visit_reddit_confirmation: Option<String>,

	#[serde(rename = "REDLIB_BANNER")]
	#[serde(alias = "LIBREDDIT_BANNER")]
	pub(crate) banner: Option<String>,

	#[serde(rename = "REDLIB_ROBOTS_DISABLE_INDEXING")]
	#[serde(alias = "LIBREDDIT_ROBOTS_DISABLE_INDEXING")]
	pub(crate) robots_disable_indexing: Option<String>,

	#[serde(rename = "REDLIB_PUSHSHIFT_FRONTEND")]
	#[serde(alias = "LIBREDDIT_PUSHSHIFT_FRONTEND")]
	pub(crate) pushshift: Option<String>,

	#[serde(rename = "REDLIB_ENABLE_RSS")]
	pub(crate) enable_rss: Option<String>,

	#[serde(rename = "REDLIB_FULL_URL")]
	pub(crate) full_url: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS")]
	pub(crate) default_remove_default_feeds: Option<String>,
}

impl Config {
	/// Load the configuration from the environment variables and the config file.
	/// In the case that there are no environment variables set and there is no
	/// config file, this function returns a Config that contains all None values.
	pub fn load() -> Self {
		let load_config = |name: &str| {
			let new_file = read_to_string(name);
			new_file.ok().and_then(|new_file| toml::from_str::<Self>(&new_file).ok())
		};

		let config = load_config("redlib.toml").or_else(|| load_config("libreddit.toml")).unwrap_or_default();

		// This function defines the order of preference - first check for
		// environment variables with "REDLIB", then check the legacy LIBREDDIT
		// option, then check the config, then if all are `None`, return a `None`
		let parse = |key: &str| -> Option<String> {
			// Return the first non-`None` value
			// If all are `None`, return `None`
			let legacy_key = key.replace("REDLIB_", "LIBREDDIT_");
			var(key).ok().or_else(|| var(legacy_key).ok()).or_else(|| get_setting_from_config(key, &config))
		};
		Self {
			sfw_only: parse("REDLIB_SFW_ONLY"),
			default_theme: parse("REDLIB_DEFAULT_THEME"),
			default_front_page: parse("REDLIB_DEFAULT_FRONT_PAGE"),
			default_layout: parse("REDLIB_DEFAULT_LAYOUT"),
			default_post_sort: parse("REDLIB_DEFAULT_POST_SORT"),
			default_wide: parse("REDLIB_DEFAULT_WIDE"),
			default_comment_sort: parse("REDLIB_DEFAULT_COMMENT_SORT"),
			default_blur_spoiler: parse("REDLIB_DEFAULT_BLUR_SPOILER"),
			default_show_nsfw: parse("REDLIB_DEFAULT_SHOW_NSFW"),
			default_blur_nsfw: parse("REDLIB_DEFAULT_BLUR_NSFW"),
			default_use_hls: parse("REDLIB_DEFAULT_USE_HLS"),
			default_hide_hls_notification: parse("REDLIB_DEFAULT_HIDE_HLS_NOTIFICATION"),
			default_hide_awards: parse("REDLIB_DEFAULT_HIDE_AWARDS"),
			default_hide_sidebar_and_summary: parse("REDLIB_DEFAULT_HIDE_SIDEBAR_AND_SUMMARY"),
			default_hide_score: parse("REDLIB_DEFAULT_HIDE_SCORE"),
			default_subscriptions: parse("REDLIB_DEFAULT_SUBSCRIPTIONS"),
			default_filters: parse("REDLIB_DEFAULT_FILTERS"),
			default_disable_visit_reddit_confirmation: parse("REDLIB_DEFAULT_DISABLE_VISIT_REDDIT_CONFIRMATION"),
			banner: parse("REDLIB_BANNER"),
			robots_disable_indexing: parse("REDLIB_ROBOTS_DISABLE_INDEXING"),
			pushshift: parse("REDLIB_PUSHSHIFT_FRONTEND"),
			enable_rss: parse("REDLIB_ENABLE_RSS"),
			full_url: parse("REDLIB_FULL_URL"),
			default_remove_default_feeds: parse("REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS"),
		}
	}
}

fn get_setting_from_config(name: &str, config: &Config) -> Option<String> {
	match name {
		"REDLIB_SFW_ONLY" => config.sfw_only.clone(),
		"REDLIB_DEFAULT_THEME" => config.default_theme.clone(),
		"REDLIB_DEFAULT_FRONT_PAGE" => config.default_front_page.clone(),
		"REDLIB_DEFAULT_LAYOUT" => config.default_layout.clone(),
		"REDLIB_DEFAULT_COMMENT_SORT" => config.default_comment_sort.clone(),
		"REDLIB_DEFAULT_POST_SORT" => config.default_post_sort.clone(),
		"REDLIB_DEFAULT_BLUR_SPOILER" => config.default_blur_spoiler.clone(),
		"REDLIB_DEFAULT_SHOW_NSFW" => config.default_show_nsfw.clone(),
		"REDLIB_DEFAULT_BLUR_NSFW" => config.default_blur_nsfw.clone(),
		"REDLIB_DEFAULT_USE_HLS" => config.default_use_hls.clone(),
		"REDLIB_DEFAULT_HIDE_HLS_NOTIFICATION" => config.default_hide_hls_notification.clone(),
		"REDLIB_DEFAULT_WIDE" => config.default_wide.clone(),
		"REDLIB_DEFAULT_HIDE_AWARDS" => config.default_hide_awards.clone(),
		"REDLIB_DEFAULT_HIDE_SIDEBAR_AND_SUMMARY" => config.default_hide_sidebar_and_summary.clone(),
		"REDLIB_DEFAULT_HIDE_SCORE" => config.default_hide_score.clone(),
		"REDLIB_DEFAULT_SUBSCRIPTIONS" => config.default_subscriptions.clone(),
		"REDLIB_DEFAULT_FILTERS" => config.default_filters.clone(),
		"REDLIB_DEFAULT_DISABLE_VISIT_REDDIT_CONFIRMATION" => config.default_disable_visit_reddit_confirmation.clone(),
		"REDLIB_BANNER" => config.banner.clone(),
		"REDLIB_ROBOTS_DISABLE_INDEXING" => config.robots_disable_indexing.clone(),
		"REDLIB_PUSHSHIFT_FRONTEND" => config.pushshift.clone(),
		"REDLIB_ENABLE_RSS" => config.enable_rss.clone(),
		"REDLIB_FULL_URL" => config.full_url.clone(),
		"REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS" => config.default_remove_default_feeds.clone(),
		_ => None,
	}
}

/// Retrieves setting from environment variable or config file.
pub fn get_setting(name: &str) -> Option<String> {
	get_setting_from_config(name, &CONFIG)
}

#[cfg(test)]
use {sealed_test::prelude::*, std::fs::write};

#[test]
fn test_deserialize() {
	// Must handle empty input
	let result = toml::from_str::<Config>("");
	assert!(result.is_ok(), "Error: {}", result.unwrap_err());
}

#[test]
#[sealed_test(env = [("REDLIB_SFW_ONLY", "on")])]
fn test_env_var() {
	assert!(crate::utils::sfw_only())
}

#[test]
#[sealed_test]
fn test_config() {
	let config_to_write = r#"REDLIB_DEFAULT_COMMENT_SORT = "best""#;
	write("redlib.toml", config_to_write).unwrap();
	assert_eq!(get_setting("REDLIB_DEFAULT_COMMENT_SORT"), Some("best".into()));
}

#[test]
#[sealed_test]
fn test_config_legacy() {
	let config_to_write = r#"LIBREDDIT_DEFAULT_COMMENT_SORT = "best""#;
	write("libreddit.toml", config_to_write).unwrap();
	assert_eq!(get_setting("REDLIB_DEFAULT_COMMENT_SORT"), Some("best".into()));
}

#[test]
#[sealed_test(env = [("LIBREDDIT_SFW_ONLY", "on")])]
fn test_env_var_legacy() {
	assert!(crate::utils::sfw_only())
}

#[test]
#[sealed_test(env = [("REDLIB_DEFAULT_COMMENT_SORT", "top")])]
fn test_env_config_precedence() {
	let config_to_write = r#"REDLIB_DEFAULT_COMMENT_SORT = "best""#;
	write("redlib.toml", config_to_write).unwrap();
	assert_eq!(get_setting("REDLIB_DEFAULT_COMMENT_SORT"), Some("top".into()))
}

#[test]
#[sealed_test(env = [("REDLIB_DEFAULT_COMMENT_SORT", "top")])]
fn test_alt_env_config_precedence() {
	let config_to_write = r#"REDLIB_DEFAULT_COMMENT_SORT = "best""#;
	write("redlib.toml", config_to_write).unwrap();
	assert_eq!(get_setting("REDLIB_DEFAULT_COMMENT_SORT"), Some("top".into()))
}
#[test]
#[sealed_test(env = [("REDLIB_DEFAULT_SUBSCRIPTIONS", "news+bestof")])]
fn test_default_subscriptions() {
	assert_eq!(get_setting("REDLIB_DEFAULT_SUBSCRIPTIONS"), Some("news+bestof".into()));
}

#[test]
#[sealed_test(env = [("REDLIB_DEFAULT_FILTERS", "news+bestof")])]
fn test_default_filters() {
	assert_eq!(get_setting("REDLIB_DEFAULT_FILTERS"), Some("news+bestof".into()));
}

#[test]
#[sealed_test]
fn test_pushshift() {
	let config_to_write = r#"REDLIB_PUSHSHIFT_FRONTEND = "https://api.pushshift.io""#;
	write("redlib.toml", config_to_write).unwrap();
	assert!(get_setting("REDLIB_PUSHSHIFT_FRONTEND").is_some());
	assert_eq!(get_setting("REDLIB_PUSHSHIFT_FRONTEND"), Some("https://api.pushshift.io".into()));
}
