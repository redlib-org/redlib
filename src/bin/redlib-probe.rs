//! Low-rate, opt-in anonymous thread diagnostic.
//!
//! Usage:
//! `cargo run --bin redlib-probe -- [thread-path ...]`
//!
//! Output is JSON Lines and intentionally contains no request headers, tokens,
//! cookies, response bodies, or Reddit tracking identifiers.

use redlib::client::json_uncached;
use redlib::thread::fetch_uncached;
use serde_json::json;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_THREADS: &[&str] = &[
	"/r/northernireland/comments/1xau6bp/teachers_and_youth_workers_find_out_how_you_can/",
	"/r/mildlyinfuriating/comments/1xcm2ou/the_guy_next_to_me_claimed_both_armrests_and/",
	"/r/politics/comments/1x9xx9e/",
];

fn timestamp() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[tokio::main]
async fn main() {
	let paths: Vec<String> = std::env::args().skip(1).collect();
	let mut threads: Vec<String> = if paths.is_empty() {
		DEFAULT_THREADS.iter().map(|path| (*path).to_owned()).collect()
	} else {
		paths
	};
	// One listing control plus at most 299 thread probes.
	threads.truncate(299);

	let started = Instant::now();
	let listing_path = format!("/r/reddit/hot.json?raw_json=1&probe={}", timestamp());
	let listing = json_uncached(listing_path.clone(), false).await;
	let listing_succeeded = listing.is_ok();
	let listing_children = listing.as_ref().ok().and_then(|value| value["data"]["children"].as_array()).map_or(0, Vec::len);
	let listing_error = listing.as_ref().err().cloned();
	println!(
		"{}",
		json!({
			"time_unix_utc": timestamp(),
			"category": "listing-control",
			"logical_path": listing_path,
			"transport": "https://oauth.reddit.com/r/reddit/hot.json",
			"status": if listing_succeeded { "success" } else { "failure" },
			"latency_ms": started.elapsed().as_millis(),
			"validated_listing_children": listing_children,
			"error": listing_error,
		})
	);
	if !listing_succeeded {
		std::process::exit(2);
	}

	let mut consecutive_failures = 0_u8;
	for path in threads {
		tokio::time::sleep(Duration::from_secs(2)).await;
		let logical_path = format!("{}.json?sort=confidence&raw_json=1", path.trim_end_matches('/'));
		let started = Instant::now();
		let result = fetch_uncached(logical_path.clone(), false).await;
		let rate_limited = matches!(&result, Err(redlib::thread::ThreadError::RateLimited(_)));
		if result.is_ok() {
			consecutive_failures = 0;
		} else {
			consecutive_failures += 1;
		}
		println!(
			"{}",
			json!({
				"time_unix_utc": timestamp(),
				"category": "thread",
				"logical_path": logical_path,
				"transport": "https://oauth.reddit.com/by_id + https://www.reddit.com/svc/shreddit/comments",
				"status": if result.is_ok() { "success" } else { "failure" },
				"content_type": "text/html (comments), application/json (post metadata)",
				"redirects_followed": 0,
				"latency_ms": started.elapsed().as_millis(),
				"post_id": result.as_ref().ok().map(|data| data.post_id()),
				"validated_comments": result.as_ref().ok().map_or(0, |data| data.rendered_comment_count()),
				"continuations": result.as_ref().ok().map_or(0, |data| data.continuation_count()),
				"error": result.err().map(|error| error.to_string()),
			})
		);
		// Do not send another request before a Retry-After interval. This probe
		// has no need to retry, so stopping is the safest way to honor it.
		if rate_limited || consecutive_failures >= 3 {
			break;
		}
	}
}
