use crate::body::Body;
use crate::{
	config::{Config, CONFIG},
	server::RequestExt,
	utils::{ErrorTemplate, Preferences},
};
use askama::Template;
use build_html::{Container, Html, HtmlContainer, Table};
use hyper::{http::Error, Request, Response};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use time::{macros::format_description, OffsetDateTime};

const SCORE_EXPLANATION: &str = "<details><summary>How the score is calculated</summary>\
<p>The score is how much Redlib currently prefers a backend: its recent success, blended with its configured position and halved for backends that use a Reddit session.</p>\
<pre><code>score = (S + 4·P) / (N + 4) / w</code></pre>\
<ul>\
<li><code>S</code>, <code>N</code>: successes and attempts. A page counts 1, a block 0, a network error ½. Results fade with a half-life of 10 minutes.</li>\
<li><code>P</code>: start value from the configured position, <code>0.5 · (n − i) / n · w</code> for place <code>i</code> (from 0) of <code>n</code>.</li>\
<li><code>w</code>: weight, 2 for session backends, otherwise 1.</li>\
</ul>\
<p>Highest score is tried first. A backend has to score 10% higher to overtake the one before it. A backend that's never tried keeps its start score.</p>\
</details>";

/// This is the local static that is initialized at runtime (technically at
/// the first request to the info endpoint) and contains the data
/// retrieved from the info endpoint.
pub static INSTANCE_INFO: LazyLock<InstanceInfo> = LazyLock::new(InstanceInfo::new);

/// Handles instance info endpoint
pub async fn instance_info(req: Request<Body>) -> Result<Response<Body>, String> {
	// This will retrieve the extension given, or create a new string - which will
	// simply become the last option, an HTML page.
	let extension = req.param("extension").unwrap_or_default();
	let response = match extension.as_str() {
		"json" => info_json(),
		"html" | "" => info_html(&req),
		_ => {
			let error = ErrorTemplate {
				msg: "Error: Invalid info extension".into(),
				detail: String::new(),
				reddit_url: String::new(),
				blocked: false,
				subreddits: Vec::new(),
				prefs: Preferences::new(&req),
				url: req.uri().to_string(),
			}
			.render()
			.unwrap();
			Response::builder().status(404).header("content-type", "text/html; charset=utf-8").body(error.into())
		}
	};
	response.map_err(|err| format!("{err}"))
}

fn info_json() -> Result<Response<Body>, Error> {
	if let Ok(body) = serde_json::to_string(&*INSTANCE_INFO) {
		Response::builder().status(200).header("content-type", "application/json").body(body.into())
	} else {
		Response::builder()
			.status(500)
			.header("content-type", "text/plain")
			.body(Body::from("Error serializing JSON"))
	}
}

fn info_html(req: &Request<Body>) -> Result<Response<Body>, Error> {
	let message = MessageTemplate {
		title: String::from("Instance information"),
		body: INSTANCE_INFO.to_table(),
		prefs: Preferences::new(req),
		url: req.uri().to_string(),
	}
	.render()
	.unwrap();
	Response::builder().status(200).header("content-type", "text/html; charset=utf8").body(Body::from(message))
}
#[derive(Serialize, Deserialize, Default)]
pub struct InstanceInfo {
	package_name: String,
	crate_version: String,
	pub git_commit: String,
	deploy_date: String,
	compile_mode: String,
	deploy_unix_ts: i64,
	config: Config,
}

impl InstanceInfo {
	pub fn new() -> Self {
		Self {
			package_name: env!("CARGO_PKG_NAME").to_string(),
			crate_version: env!("CARGO_PKG_VERSION").to_string(),
			git_commit: env!("GIT_HASH").to_string(),
			deploy_date: OffsetDateTime::now_local()
				.unwrap_or_else(|_| OffsetDateTime::now_utc())
				.format(format_description!(
					"[year]-[month]-[day] [hour]:[minute]:[second] [offset_hour sign:mandatory]:[offset_minute]"
				))
				.unwrap_or_default(),
			#[cfg(debug_assertions)]
			compile_mode: "Debug".into(),
			#[cfg(not(debug_assertions))]
			compile_mode: "Release".into(),
			deploy_unix_ts: OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()).unix_timestamp(),
			config: CONFIG.clone(),
		}
	}
	fn to_table(&self) -> String {
		let mut container = Container::default().with_attributes([("id", "instance_info")]);
		if let Some(banner) = &self.config.banner {
			container.add_header(3, "Instance banner");
			container.add_raw("<br />");
			container.add_paragraph(banner);
			container.add_raw("<br />");
		}
		let source = crate::utils::get_source_url();
		let commit = format!(
			"<a href=\"{source}/commit/{full}\" title=\"{full}\">{short}</a>",
			full = self.git_commit.trim(),
			short = crate::utils::short_git_hash()
		);
		let source_link = format!("<a href=\"{source}\">{source}</a>");
		container.add_table(
			Table::from([
				["Package name", &self.package_name],
				["Source", &source_link],
				["Git commit", &commit],
				["Deploy date", &self.deploy_date],
				["Deploy timestamp", &self.deploy_unix_ts.to_string()],
				["Compile mode", &self.compile_mode],
				["SFW only", &effective(&self.config.sfw_only, "off")],
				["Pushshift frontend", &effective(&self.config.pushshift, crate::config::DEFAULT_PUSHSHIFT_FRONTEND)],
				["RSS enabled", &effective(&self.config.enable_rss, "off")],
				["RedGifs enabled", &effective(&self.config.enable_redgifs, "on")],
				["GIPHY enabled", &effective(&self.config.enable_giphy, "on")],
				["Scrape fallback enabled", &effective(&self.config.enable_scrape_fallback, "on")],
				["GraphQL fallback enabled", &effective(&self.config.enable_gql, "on")],
				["Scrape comment prefetch", &effective(&self.config.scrape_comment_prefetch, "0")],
				["GraphQL feed requests", &effective(&self.config.gql_feed_requests, "2")],
				["Full URL", &effective(&self.config.full_url, "none")],
				["Remove default feeds", &effective(&self.config.default_remove_default_feeds, "off")],
			])
			.with_header_row(["Settings"]),
		);
		container.add_raw("<br />");
		container
			.add_table(Table::from(crate::backends::status().into_iter().map(|(kind, scores)| [kind.to_string(), scores])).with_header_row(["Backends, current order and score"]));
		container.add_raw(SCORE_EXPLANATION);
		container.add_raw("<br />");
		container.add_table(
			Table::from([
				["Hide awards", &effective(&self.config.default_hide_awards, "off")],
				["Hide score", &effective(&self.config.default_hide_score, "off")],
				["Theme", &effective(&self.config.default_theme, "system")],
				["Light theme", &effective(&self.config.default_light_theme, "default")],
				["Dark theme", &effective(&self.config.default_dark_theme, "default")],
				["Front page", &effective(&self.config.default_front_page, "default")],
				["Layout", &effective(&self.config.default_layout, "card")],
				["Wide", &effective(&self.config.default_wide, "off")],
				["Comment sort", &effective(&self.config.default_comment_sort, "confidence")],
				["Post sort", &effective(&self.config.default_post_sort, "hot")],
				["Blur Spoiler", &effective(&self.config.default_blur_spoiler, "off")],
				["Show NSFW", &effective(&self.config.default_show_nsfw, "off")],
				["Blur NSFW", &effective(&self.config.default_blur_nsfw, "off")],
				["Use HLS", &effective(&self.config.default_use_hls, "off")],
				["Hide HLS notification", &effective(&self.config.default_hide_hls_notification, "off")],
				["Subscriptions", &effective(&self.config.default_subscriptions, "none")],
				["Filters", &effective(&self.config.default_filters, "none")],
				["r/Popular region", &effective(&self.config.default_geo_filter, "GLOBAL")],
				["Clean URLs", &effective(&self.config.default_clean_urls, "off")],
				["Posts per page", &effective(&self.config.default_posts_per_page, "25")],
				["Max comment thread depth", &effective(&self.config.default_max_comment_thread_depth, "0")],
				["Always open replies in thread view", &effective(&self.config.default_more_replies_new_page, "off")],
			])
			.with_header_row(["Default preferences"]),
		);
		container.to_html_string().replace("<th>", "<th colspan=\"2\">")
	}
}
/// The configured value, or the value Redlib uses when it is unset or empty.
fn effective(value: &Option<String>, default: &str) -> String {
	match value.as_deref() {
		Some(value) if !value.is_empty() => value.to_string(),
		_ => format!("{default} (default)"),
	}
}

#[derive(Template)]
#[template(path = "message.html")]
struct MessageTemplate {
	title: String,
	body: String,
	prefs: Preferences,
	url: String,
}

#[cfg(test)]
mod tests {
	use super::effective;

	#[test]
	fn effective_shows_the_default_when_unset() {
		assert_eq!(effective(&Some("on".into()), "off"), "on");
		assert_eq!(effective(&None, "on"), "on (default)");
		assert_eq!(effective(&Some(String::new()), "none"), "none (default)");
	}
}
