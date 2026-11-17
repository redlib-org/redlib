use serde::{Deserialize, Serialize};
use std::{env::var, fs::read_to_string, sync::LazyLock};

/// This is the local static that is initialized at runtime (technically at
/// first request) and contains the instance settings.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::load);

/// This serves as the frontend for an archival API - on removed comments, this URL
/// will be the base of a link, to display removed content (on another site).
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

	#[serde(rename = "REDLIB_ENABLE_REDGIFS")]
	pub(crate) enable_redgifs: Option<String>,

	#[serde(rename = "REDLIB_ENABLE_GIPHY")]
	pub(crate) enable_giphy: Option<String>,

	#[serde(rename = "REDLIB_ENABLE_SCRAPE_FALLBACK")]
	pub(crate) enable_scrape_fallback: Option<String>,

	#[serde(rename = "REDLIB_ENABLE_GQL")]
	pub(crate) enable_gql: Option<String>,

	#[serde(rename = "REDLIB_SCRAPE_COMMENT_PREFETCH")]
	pub(crate) scrape_comment_prefetch: Option<String>,

	#[serde(rename = "REDLIB_GQL_FEED_REQUESTS")]
	pub(crate) gql_feed_requests: Option<String>,

	#[serde(rename = "REDLIB_FULL_URL")]
	pub(crate) full_url: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS")]
	pub(crate) default_remove_default_feeds: Option<String>,

	#[serde(rename = "REDLIB_SOURCE_URL")]
	pub(crate) source_url: Option<String>,
	#[serde(rename = "REDLIB_DEFAULT_GEO_FILTER")]
	pub(crate) default_geo_filter: Option<String>,
	#[serde(rename = "REDLIB_DEFAULT_CLEAN_URLS")]
	pub(crate) default_clean_urls: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_POSTS_PER_PAGE")]
	pub(crate) default_posts_per_page: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_MAX_COMMENT_THREAD_DEPTH")]
	pub(crate) default_max_comment_thread_depth: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_MORE_REPLIES_NEW_PAGE")]
	pub(crate) default_more_replies_new_page: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_LIGHT_THEME")]
	pub(crate) default_light_theme: Option<String>,

	#[serde(rename = "REDLIB_DEFAULT_DARK_THEME")]
	pub(crate) default_dark_theme: Option<String>,

	/// A logged-in Reddit web session, sent as the `Cookie` header on requests
	/// Reddit refuses to serve to logged-out clients.
	///
	/// This is a credential, so it must never be serialized: `/info.json`
	/// serializes the whole config.
	#[serde(rename = "REDLIB_REDDIT_COOKIE")]
	#[serde(skip_serializing)]
	pub(crate) reddit_cookie: Option<String>,

	/// File with more sessions, one per line, reread while running. The path
	/// says nothing useful to visitors, so it isn't serialized either.
	#[serde(rename = "REDLIB_REDDIT_COOKIE_FILE")]
	#[serde(skip_serializing)]
	pub(crate) reddit_cookie_file: Option<String>,

	#[serde(rename = "REDLIB_STALE_CACHE_MB")]
	pub(crate) stale_cache_mb: Option<String>,

	#[serde(rename = "REDLIB_STALE_CACHE_MAX_AGE")]
	pub(crate) stale_cache_max_age: Option<String>,

	/// Listener settings; `main` reads them, `/info` doesn't show them.
	#[serde(rename = "REDLIB_LISTEN_PORT")]
	#[serde(skip_serializing)]
	pub(crate) port: Option<String>,

	#[serde(rename = "REDLIB_LISTEN_ADDRESS")]
	#[serde(skip_serializing)]
	pub(crate) address: Option<String>,

	#[serde(rename = "REDLIB_HSTS")]
	#[serde(skip_serializing)]
	pub(crate) hsts: Option<String>,

	#[serde(rename = "REDLIB_CACERT")]
	#[serde(skip_serializing)]
	pub(crate) cacert: Option<String>,

	#[serde(rename = "REDLIB_IPV4_ONLY")]
	#[serde(skip_serializing)]
	pub(crate) ipv4_only: Option<String>,

	#[serde(rename = "REDLIB_IPV6_ONLY")]
	#[serde(skip_serializing)]
	pub(crate) ipv6_only: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS")]
	pub(crate) endpoints: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_POST")]
	pub(crate) endpoints_post: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_COMMENTS")]
	pub(crate) endpoints_comments: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_LISTING")]
	pub(crate) endpoints_listing: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_ALL")]
	pub(crate) endpoints_all: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_SUBREDDIT")]
	pub(crate) endpoints_subreddit: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_USER")]
	pub(crate) endpoints_user: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_SEARCH")]
	pub(crate) endpoints_search: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_WIKI")]
	pub(crate) endpoints_wiki: Option<String>,

	#[serde(rename = "REDLIB_ENDPOINTS_DUPLICATES")]
	pub(crate) endpoints_duplicates: Option<String>,
}

impl Config {
	/// Parses a config file. Every setting is a string, so numbers are read as
	/// text and booleans as `on`/`off`.
	fn parse_file(file: &str) -> Result<Self, toml::de::Error> {
		let table: toml::Table = file.parse()?;
		let table = table
			.into_iter()
			.map(|(key, value)| {
				let value = match value {
					toml::Value::String(s) => s,
					toml::Value::Boolean(b) => if b { "on" } else { "off" }.to_string(),
					other => other.to_string(),
				};
				(key, toml::Value::String(value))
			})
			.collect();
		toml::Value::Table(table).try_into()
	}

	/// Load the configuration from the environment variables and the config file.
	/// In the case that there are no environment variables set and there is no
	/// config file, this function returns a Config that contains all None values.
	pub fn load() -> Self {
		let load_config = |name: &str| {
			let file = read_to_string(name).ok()?;
			Self::parse_file(&file).map_err(|e| log::error!("Ignoring {name}, it isn't valid: {e}")).ok()
		};

		let config = load_config("redlib.toml").or_else(|| load_config("libreddit.toml")).unwrap_or_default();

		// This function defines the order of preference - first check for
		// environment variables with "REDLIB", then check the legacy LIBREDDIT
		// option, then check the config, then if all are `None`, return a `None`
		let parse = |key: &str| -> Option<String> {
			// Return the first non-`None` value
			// If all are `None`, return `None`
			let legacy_key = key.replace("REDLIB_", "LIBREDDIT_");
			var(key)
				.ok()
				.or_else(|| var(legacy_key).ok())
				.or_else(|| get_setting_from_config(key, &config))
				.map(|value| normalize_case(key, value))
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
			enable_redgifs: parse("REDLIB_ENABLE_REDGIFS"),
			enable_giphy: parse("REDLIB_ENABLE_GIPHY"),
			enable_scrape_fallback: parse("REDLIB_ENABLE_SCRAPE_FALLBACK"),
			enable_gql: parse("REDLIB_ENABLE_GQL"),
			scrape_comment_prefetch: parse("REDLIB_SCRAPE_COMMENT_PREFETCH"),
			gql_feed_requests: parse("REDLIB_GQL_FEED_REQUESTS"),
			full_url: parse("REDLIB_FULL_URL"),
			default_remove_default_feeds: parse("REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS"),
			source_url: parse("REDLIB_SOURCE_URL"),
			default_geo_filter: parse("REDLIB_DEFAULT_GEO_FILTER"),
			default_clean_urls: parse("REDLIB_DEFAULT_CLEAN_URLS"),
			default_posts_per_page: parse("REDLIB_DEFAULT_POSTS_PER_PAGE"),
			default_max_comment_thread_depth: parse("REDLIB_DEFAULT_MAX_COMMENT_THREAD_DEPTH"),
			default_more_replies_new_page: parse("REDLIB_DEFAULT_MORE_REPLIES_NEW_PAGE"),
			default_light_theme: parse("REDLIB_DEFAULT_LIGHT_THEME"),
			default_dark_theme: parse("REDLIB_DEFAULT_DARK_THEME"),
			reddit_cookie: parse("REDLIB_REDDIT_COOKIE"),
			reddit_cookie_file: parse("REDLIB_REDDIT_COOKIE_FILE"),
			stale_cache_mb: parse("REDLIB_STALE_CACHE_MB"),
			stale_cache_max_age: parse("REDLIB_STALE_CACHE_MAX_AGE"),
			port: parse("REDLIB_LISTEN_PORT"),
			address: parse("REDLIB_LISTEN_ADDRESS"),
			hsts: parse("REDLIB_HSTS"),
			cacert: parse("REDLIB_CACERT"),
			ipv4_only: parse("REDLIB_IPV4_ONLY"),
			ipv6_only: parse("REDLIB_IPV6_ONLY"),
			endpoints: parse("REDLIB_ENDPOINTS"),
			endpoints_post: parse("REDLIB_ENDPOINTS_POST"),
			endpoints_comments: parse("REDLIB_ENDPOINTS_COMMENTS"),
			endpoints_listing: parse("REDLIB_ENDPOINTS_LISTING"),
			endpoints_all: parse("REDLIB_ENDPOINTS_ALL"),
			endpoints_subreddit: parse("REDLIB_ENDPOINTS_SUBREDDIT"),
			endpoints_user: parse("REDLIB_ENDPOINTS_USER"),
			endpoints_search: parse("REDLIB_ENDPOINTS_SEARCH"),
			endpoints_wiki: parse("REDLIB_ENDPOINTS_WIKI"),
			endpoints_duplicates: parse("REDLIB_ENDPOINTS_DUPLICATES"),
		}
	}
}

/// Switches, choices and backend lists are compared in lowercase and geo
/// filters in uppercase, so `ON` or `Card` would otherwise silently fall back.
/// So is the Pushshift frontend's host. Free text, names, URLs and sessions
/// stay as written.
fn normalize_case(key: &str, value: String) -> String {
	match key {
		"REDLIB_DEFAULT_GEO_FILTER" => value.to_ascii_uppercase(),
		"REDLIB_DEFAULT_SUBSCRIPTIONS" | "REDLIB_DEFAULT_FILTERS" => value,
		"REDLIB_PUSHSHIFT_FRONTEND" => match value.split_once('/') {
			Some((host, path)) => format!("{}/{path}", host.to_ascii_lowercase()),
			None => value.to_ascii_lowercase(),
		},
		"REDLIB_SFW_ONLY" | "REDLIB_ROBOTS_DISABLE_INDEXING" | "REDLIB_IPV4_ONLY" | "REDLIB_IPV6_ONLY" | "REDLIB_HSTS" => value.to_ascii_lowercase(),
		_ if key.starts_with("REDLIB_DEFAULT_") || key.starts_with("REDLIB_ENABLE_") || key.starts_with("REDLIB_ENDPOINTS") => value.to_ascii_lowercase(),
		_ => value,
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
		"REDLIB_ENABLE_REDGIFS" => config.enable_redgifs.clone(),
		"REDLIB_ENABLE_GIPHY" => config.enable_giphy.clone(),
		"REDLIB_ENABLE_SCRAPE_FALLBACK" => config.enable_scrape_fallback.clone(),
		"REDLIB_ENABLE_GQL" => config.enable_gql.clone(),
		"REDLIB_SCRAPE_COMMENT_PREFETCH" => config.scrape_comment_prefetch.clone(),
		"REDLIB_GQL_FEED_REQUESTS" => config.gql_feed_requests.clone(),
		"REDLIB_FULL_URL" => config.full_url.clone(),
		"REDLIB_DEFAULT_REMOVE_DEFAULT_FEEDS" => config.default_remove_default_feeds.clone(),
		"REDLIB_SOURCE_URL" => config.source_url.clone(),
		"REDLIB_DEFAULT_GEO_FILTER" => config.default_geo_filter.clone(),
		"REDLIB_DEFAULT_CLEAN_URLS" => config.default_clean_urls.clone(),
		"REDLIB_DEFAULT_POSTS_PER_PAGE" => config.default_posts_per_page.clone(),
		"REDLIB_DEFAULT_MAX_COMMENT_THREAD_DEPTH" => config.default_max_comment_thread_depth.clone(),
		"REDLIB_DEFAULT_MORE_REPLIES_NEW_PAGE" => config.default_more_replies_new_page.clone(),
		"REDLIB_DEFAULT_LIGHT_THEME" => config.default_light_theme.clone(),
		"REDLIB_DEFAULT_DARK_THEME" => config.default_dark_theme.clone(),
		"REDLIB_REDDIT_COOKIE" => config.reddit_cookie.clone(),
		"REDLIB_REDDIT_COOKIE_FILE" => config.reddit_cookie_file.clone(),
		"REDLIB_STALE_CACHE_MB" => config.stale_cache_mb.clone(),
		"REDLIB_STALE_CACHE_MAX_AGE" => config.stale_cache_max_age.clone(),
		"REDLIB_LISTEN_PORT" => config.port.clone(),
		"REDLIB_LISTEN_ADDRESS" => config.address.clone(),
		"REDLIB_HSTS" => config.hsts.clone(),
		"REDLIB_CACERT" => config.cacert.clone(),
		"REDLIB_IPV4_ONLY" => config.ipv4_only.clone(),
		"REDLIB_IPV6_ONLY" => config.ipv6_only.clone(),
		"REDLIB_ENDPOINTS" => config.endpoints.clone(),
		"REDLIB_ENDPOINTS_POST" => config.endpoints_post.clone(),
		"REDLIB_ENDPOINTS_COMMENTS" => config.endpoints_comments.clone(),
		"REDLIB_ENDPOINTS_LISTING" => config.endpoints_listing.clone(),
		"REDLIB_ENDPOINTS_ALL" => config.endpoints_all.clone(),
		"REDLIB_ENDPOINTS_SUBREDDIT" => config.endpoints_subreddit.clone(),
		"REDLIB_ENDPOINTS_USER" => config.endpoints_user.clone(),
		"REDLIB_ENDPOINTS_SEARCH" => config.endpoints_search.clone(),
		"REDLIB_ENDPOINTS_WIKI" => config.endpoints_wiki.clone(),
		"REDLIB_ENDPOINTS_DUPLICATES" => config.endpoints_duplicates.clone(),
		_ => None,
	}
}

/// Retrieves setting from environment variable or config file.
pub fn get_setting(name: &str) -> Option<String> {
	get_setting_from_config(name, &CONFIG)
}

#[cfg(test)]
mod tests {
	use super::*;
	use {sealed_test::prelude::*, std::fs::write};

	#[test]
	fn test_deserialize() {
		// Must handle empty input
		let result = toml::from_str::<Config>("");
		assert!(result.is_ok(), "Error: {}", result.unwrap_err());
	}

	/// Every `#[serde(<kind> = "...")]` name in this file, so new settings are covered automatically.
	fn serde_names(kind: &str) -> Vec<String> {
		let marker = format!("#[serde({kind} = \"");
		include_str!("config.rs")
			.lines()
			.filter_map(|line| line.trim().strip_prefix(marker.as_str())?.strip_suffix("\")]").map(str::to_string))
			.collect()
	}

	fn parsed(file: &str) -> Config {
		Config::parse_file(file).unwrap_or_else(|e| panic!("{file:?} didn't parse: {e}"))
	}

	#[test]
	fn test_every_setting_reads_every_value_type() {
		let keys = serde_names("rename");
		assert!(keys.len() > 30, "found only {} settings", keys.len());
		for key in &keys {
			for (value, expected) in [
				("\"text\"", "text"),
				("\"\"", ""),
				("7", "7"),
				("-1", "-1"),
				("1.5", "1.5"),
				("true", "on"),
				("false", "off"),
			] {
				let config = parsed(&format!("{key} = {value}"));
				assert_eq!(get_setting_from_config(key, &config).as_deref(), Some(expected), "{key} = {value}");
			}
		}
	}

	#[test]
	fn test_every_legacy_name_reads_into_its_setting() {
		let aliases = serde_names("alias");
		assert!(!aliases.is_empty());
		for alias in &aliases {
			let config = parsed(&format!("{alias} = 3"));
			let key = alias.replace("LIBREDDIT_", "REDLIB_");
			assert_eq!(get_setting_from_config(&key, &config).as_deref(), Some("3"), "{alias}");
		}
	}

	#[test]
	fn test_full_config_with_mixed_value_types() {
		let keys = serde_names("rename");
		let file: String = keys
			.iter()
			.enumerate()
			.map(|(i, key)| match i % 3 {
				0 => format!("{key} = \"v{i}\"\n"),
				1 => format!("{key} = {i}\n"),
				_ => format!("{key} = {}\n", i % 2 == 0),
			})
			.collect();
		let config = parsed(&file);
		for (i, key) in keys.iter().enumerate() {
			let expected = match i % 3 {
				0 => format!("v{i}"),
				1 => i.to_string(),
				_ => if i % 2 == 0 { "on" } else { "off" }.to_string(),
			};
			assert_eq!(get_setting_from_config(key, &config), Some(expected), "{key}");
		}
	}

	#[test]
	fn test_file_layout_variations() {
		for file in [
			"",
			"# only a comment\n",
			"\n\n   \n",
			"REDLIB_DEFAULT_THEME = \"dark\" # trailing comment\r\nREDLIB_DEFAULT_WIDE = true\r\n",
			"REDLIB_DEFAULT_THEME='dark'\nREDLIB_DEFAULT_WIDE=true",
			"\"REDLIB_DEFAULT_THEME\" = \"\"\"dark\"\"\"\nREDLIB_DEFAULT_WIDE = true\n",
			"SOME_UNKNOWN_SETTING = 1\nREDLIB_DEFAULT_THEME = \"dark\"\nREDLIB_DEFAULT_WIDE = true\n",
			"REDLIB_DEFAULT_THEME = \"dark\"\nREDLIB_DEFAULT_WIDE = true\nREDLIB_UNUSED = [1, \"a\"]\nREDLIB_ALSO_UNUSED = { a = 1 }\n",
		] {
			let config = parsed(file);
			if file.contains("REDLIB_DEFAULT_THEME") {
				assert_eq!(get_setting_from_config("REDLIB_DEFAULT_THEME", &config).as_deref(), Some("dark"), "{file:?}");
				assert_eq!(get_setting_from_config("REDLIB_DEFAULT_WIDE", &config).as_deref(), Some("on"), "{file:?}");
			}
		}
	}

	#[test]
	fn test_invalid_files_are_errors() {
		for file in [
			"REDLIB_DEFAULT_THEME = dark",
			"REDLIB_DEFAULT_THEME = \"dark",
			"REDLIB_DEFAULT_THEME",
			"A = 1\nA = 2",
			"[section\n",
		] {
			assert!(Config::parse_file(file).is_err(), "{file:?} parsed");
		}
	}

	#[test]
	#[sealed_test]
	fn test_invalid_file_falls_back_to_defaults() {
		write("redlib.toml", "REDLIB_DEFAULT_THEME = dark").unwrap();
		assert_eq!(get_setting("REDLIB_DEFAULT_THEME"), None);
	}

	#[test]
	#[sealed_test]
	fn test_listener_settings_from_file() {
		write("redlib.toml", "REDLIB_LISTEN_PORT = 8085\nREDLIB_LISTEN_ADDRESS = \"127.0.0.1\"\nREDLIB_IPV4_ONLY = true").unwrap();
		assert_eq!(get_setting("REDLIB_LISTEN_PORT"), Some("8085".into()));
		assert_eq!(get_setting("REDLIB_LISTEN_ADDRESS"), Some("127.0.0.1".into()));
		assert_eq!(get_setting("REDLIB_IPV4_ONLY"), Some("on".into()));
		assert!(!toml::to_string(&*CONFIG).unwrap().contains("8085"), "listener settings must not show in /info");
	}

	// Kubernetes sets `<SERVICE>_PORT=tcp://<ip>:<port>` for a Service named `redlib`.
	#[test]
	#[sealed_test(env = [("REDLIB_PORT", "tcp://10.0.0.1:8080"), ("REDLIB_SERVICE_PORT", "8080")])]
	fn test_kubernetes_service_variables_are_not_listener_settings() {
		assert_eq!(get_setting("REDLIB_LISTEN_PORT"), None);
		assert_eq!(get_setting("REDLIB_LISTEN_ADDRESS"), None);
	}

	#[test]
	#[sealed_test]
	fn test_config_numbers_and_booleans() {
		let config_to_write = "REDLIB_SCRAPE_COMMENT_PREFETCH = 0\nREDLIB_ENABLE_RSS = false\nREDLIB_DEFAULT_SUBSCRIPTIONS = \"rust\"";
		write("redlib.toml", config_to_write).unwrap();
		assert_eq!(get_setting("REDLIB_SCRAPE_COMMENT_PREFETCH"), Some("0".into()));
		assert_eq!(get_setting("REDLIB_ENABLE_RSS"), Some("off".into()));
		assert_eq!(get_setting("REDLIB_DEFAULT_SUBSCRIPTIONS"), Some("rust".into()));
	}

	#[test]
	#[sealed_test(env = [("REDLIB_SFW_ONLY", "on")])]
	fn test_env_var() {
		assert!(crate::utils::sfw_only())
	}

	#[test]
	#[sealed_test(env = [
		("REDLIB_ENABLE_RSS", "ON"),
		("REDLIB_DEFAULT_LAYOUT", "Card"),
		("REDLIB_ENDPOINTS_LISTING", "Gql,JSON"),
		("REDLIB_DEFAULT_GEO_FILTER", "global"),
		("REDLIB_BANNER", "Hello World"),
		("REDLIB_DEFAULT_SUBSCRIPTIONS", "Rust+Linux"),
		("REDLIB_FULL_URL", "https://Example.com/Redlib"),
		("REDLIB_PUSHSHIFT_FRONTEND", "Undelete.Example.com/Reddit"),
	])]
	fn test_options_in_any_case() {
		write("redlib.toml", "REDLIB_SFW_ONLY = \"Off\"\nREDLIB_ROBOTS_DISABLE_INDEXING = \"On\"").unwrap();
		assert!(crate::utils::enable_rss());
		assert!(crate::utils::disable_indexing());
		assert_eq!(get_setting("REDLIB_SFW_ONLY"), Some("off".into()));
		assert_eq!(get_setting("REDLIB_DEFAULT_LAYOUT"), Some("card".into()));
		assert_eq!(get_setting("REDLIB_ENDPOINTS_LISTING"), Some("gql,json".into()));
		assert_eq!(get_setting("REDLIB_DEFAULT_GEO_FILTER"), Some("GLOBAL".into()));
		// free text, names and URLs stay as written
		assert_eq!(get_setting("REDLIB_BANNER"), Some("Hello World".into()));
		assert_eq!(get_setting("REDLIB_DEFAULT_SUBSCRIPTIONS"), Some("Rust+Linux".into()));
		assert_eq!(get_setting("REDLIB_FULL_URL"), Some("https://Example.com/Redlib".into()));
		// only the host of the Pushshift frontend, its path may care
		assert_eq!(get_setting("REDLIB_PUSHSHIFT_FRONTEND"), Some("undelete.example.com/Reddit".into()));
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
	#[sealed_test(env = [("REDLIB_SOURCE_URL", "https://example.com")])]
	fn test_source_url() {
		assert_eq!(get_setting("REDLIB_SOURCE_URL"), Some("https://example.com".into()));
	}

	#[test]
	#[sealed_test(env = [("REDLIB_GQL_FEED_REQUESTS", "3")])]
	fn test_gql_feed_requests() {
		assert_eq!(crate::utils::gql_feed_requests(), 3);
	}

	#[test]
	#[sealed_test(env = [("REDLIB_GQL_FEED_REQUESTS", "0")])]
	fn test_gql_feed_requests_falls_back_to_the_default() {
		assert_eq!(crate::utils::gql_feed_requests(), 2);
	}

	#[test]
	#[sealed_test]
	fn test_pushshift() {
		let config_to_write = r#"REDLIB_PUSHSHIFT_FRONTEND = "https://api.pushshift.io""#;
		write("redlib.toml", config_to_write).unwrap();
		assert!(get_setting("REDLIB_PUSHSHIFT_FRONTEND").is_some());
		assert_eq!(get_setting("REDLIB_PUSHSHIFT_FRONTEND"), Some("https://api.pushshift.io".into()));
	}

	#[test]
	#[sealed_test(env = [("REDLIB_REDDIT_COOKIE", "reddit_session=secret-cookie-value")])]
	fn test_reddit_cookie_is_never_serialized() {
		let config = Config::load();
		assert_eq!(config.reddit_cookie.as_deref(), Some("reddit_session=secret-cookie-value"));

		// `/info.json` serializes the whole config, so the session must not
		// show up there.
		assert!(!serde_json::to_string(&config).unwrap().contains("secret-cookie-value"));
	}

	#[test]
	#[sealed_test]
	fn test_reddit_cookie_from_config_file() {
		// CI sets one for the live tests, and the environment wins over the file
		std::env::remove_var("REDLIB_REDDIT_COOKIE");
		let config_to_write = r#"REDLIB_REDDIT_COOKIE = "reddit_session=from-file""#;
		write("redlib.toml", config_to_write).unwrap();
		assert_eq!(get_setting("REDLIB_REDDIT_COOKIE"), Some("reddit_session=from-file".into()));
	}
}
