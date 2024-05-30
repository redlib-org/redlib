use cached::proc_macro::cached;
use futures_lite::future::block_on;
use futures_lite::{future::Boxed, FutureExt};
use hyper::client::HttpConnector;
use hyper::{body, body::Buf, client, header, Body, Client, Method, Request, Response, Uri};
use hyper_rustls::HttpsConnector;
use libflate::gzip;
use log::error;
use once_cell::sync::Lazy;
use percent_encoding::{percent_encode, CONTROLS};
use serde_json::Value;

use std::{io, result::Result};
use tokio::sync::RwLock;

use crate::dbg_msg;
use crate::oauth::{force_refresh_token, token_daemon, Oauth};
use crate::server::RequestExt;
use crate::utils::format_url;

const REDDIT_URL_BASE: &str = "https://oauth.reddit.com";

pub static CLIENT: Lazy<Client<HttpsConnector<HttpConnector>>> = Lazy::new(|| {
	let https = hyper_rustls::HttpsConnectorBuilder::new()
		.with_native_roots()
		.expect("No native root certificates found")
		.https_only()
		.enable_http1()
		.build();
	client::Client::builder().build(https)
});

pub static OAUTH_CLIENT: Lazy<RwLock<Oauth>> = Lazy::new(|| {
	let client = block_on(Oauth::new());
	tokio::spawn(token_daemon());
	RwLock::new(client)
});

/// Gets the canonical path for a resource on Reddit. This is accomplished by
/// making a `HEAD` request to Reddit at the path given in `path`.
///
/// This function returns `Ok(Some(path))`, where `path`'s value is identical
/// to that of the value of the argument `path`, if Reddit responds to our
/// `HEAD` request with a 2xx-family HTTP code. It will also return an
/// `Ok(Some(String))` if Reddit responds to our `HEAD` request with a
/// `Location` header in the response, and the HTTP code is in the 3xx-family;
/// the `String` will contain the path as reported in `Location`. The return
/// value is `Ok(None)` if Reddit responded with a 3xx, but did not provide a
/// `Location` header. An `Err(String)` is returned if Reddit responds with a
/// 429, or if we were unable to decode the value in the `Location` header.
#[cached(size = 1024, time = 600, result = true)]
pub async fn canonical_path(path: String) -> Result<Option<String>, String> {
	let res = reddit_head(path.clone(), true).await?;
	let status = res.status().as_u16();

	match status {
		429 => Err("Too many requests.".to_string()),

		// If Reddit responds with a 2xx, then the path is already canonical.
		200..=299 => Ok(Some(path)),

		// If Reddit responds with a 301, then the path is redirected.
		301 => match res.headers().get(header::LOCATION) {
			Some(val) => {
				let Ok(original) = val.to_str() else {
					return Err("Unable to decode Location header.".to_string());
				};
				// We need to strip the .json suffix from the original path.
				// In addition, we want to remove share parameters.
				// Cut it off here instead of letting it propagate all the way
				// to main.rs
				let stripped_uri = original.strip_suffix(".json").unwrap_or(original).split('?').next().unwrap_or_default();

				// The reason why we now have to format_url, is because the new OAuth
				// endpoints seem to return full paths, instead of relative paths.
				// So we need to strip the .json suffix from the original path, and
				// also remove all Reddit domain parts with format_url.
				// Otherwise, it will literally redirect to Reddit.com.
				let uri = format_url(stripped_uri);
				Ok(Some(uri))
			}
			None => Ok(None),
		},

		// If Reddit responds with anything other than 3xx (except for the 2xx and 301
		// as above), return a None.
		300..=399 => Ok(None),

		_ => Ok(
			res
				.headers()
				.get(header::LOCATION)
				.map(|val| percent_encode(val.as_bytes(), CONTROLS).to_string().trim_start_matches(REDDIT_URL_BASE).to_string()),
		),
	}
}

pub async fn proxy(req: Request<Body>, format: &str) -> Result<Response<Body>, String> {
	let mut url = format!("{format}?{}", req.uri().query().unwrap_or_default());

	// For each parameter in request
	for (name, value) in &req.params() {
		// Fill the parameter value in the url
		url = url.replace(&format!("{{{name}}}"), value);
	}

	stream(&url, &req).await
}

async fn stream(url: &str, req: &Request<Body>) -> Result<Response<Body>, String> {
	// First parameter is target URL (mandatory).
	let parsed_uri = url.parse::<Uri>().map_err(|_| "Couldn't parse URL".to_string())?;

	// Build the hyper client from the HTTPS connector.
	let client: Client<_, Body> = CLIENT.clone();

	let mut builder = Request::get(parsed_uri);

	// Copy useful headers from original request
	for &key in &["Range", "If-Modified-Since", "Cache-Control"] {
		if let Some(value) = req.headers().get(key) {
			builder = builder.header(key, value);
		}
	}

	let stream_request = builder.body(Body::empty()).map_err(|_| "Couldn't build empty body in stream".to_string())?;

	client
		.request(stream_request)
		.await
		.map(|mut res| {
			let mut rm = |key: &str| res.headers_mut().remove(key);

			rm("access-control-expose-headers");
			rm("server");
			rm("vary");
			rm("etag");
			rm("x-cdn");
			rm("x-cdn-client-region");
			rm("x-cdn-name");
			rm("x-cdn-server-region");
			rm("x-reddit-cdn");
			rm("x-reddit-video-features");
			rm("Nel");
			rm("Report-To");

			res
		})
		.map_err(|e| e.to_string())
}

/// Makes a GET request to Reddit at `path`. By default, this will honor HTTP
/// 3xx codes Reddit returns and will automatically redirect.
fn reddit_get(path: String, quarantine: bool) -> Boxed<Result<Response<Body>, String>> {
	request(&Method::GET, path, true, quarantine)
}

/// Makes a HEAD request to Reddit at `path`. This will not follow redirects.
fn reddit_head(path: String, quarantine: bool) -> Boxed<Result<Response<Body>, String>> {
	request(&Method::HEAD, path, false, quarantine)
}

/// Makes a request to Reddit. If `redirect` is `true`, `request_with_redirect`
/// will recurse on the URL that Reddit provides in the Location HTTP header
/// in its response.
fn request(method: &'static Method, path: String, redirect: bool, quarantine: bool) -> Boxed<Result<Response<Body>, String>> {
	// Build Reddit URL from path.
	let url = format!("{REDDIT_URL_BASE}{path}");

	// Construct the hyper client from the HTTPS connector.
	let client: Client<_, Body> = CLIENT.clone();

	let (token, vendor_id, device_id, mut user_agent, loid) = {
		let client = block_on(OAUTH_CLIENT.read());
		(
			client.token.clone(),
			client.headers_map.get("Client-Vendor-Id").cloned().unwrap_or_default(),
			client.headers_map.get("X-Reddit-Device-Id").cloned().unwrap_or_default(),
			client.headers_map.get("User-Agent").cloned().unwrap_or_default(),
			client.headers_map.get("x-reddit-loid").cloned().unwrap_or_default(),
		)
	};

	// Replace "Android" with a tricky word.
	// Issues: #78/#115, #116
	// If you include the word "Android", you will get a number of different errors
	// I guess they don't expect mobile traffic on the endpoints we use
	// Scrawled on wall for next poor soul: Run the test suite.
	user_agent = user_agent.replace("Android", "Andr\u{200B}oid");

	// Build request to Reddit. When making a GET, request gzip compression.
	// (Reddit doesn't do brotli yet.)
	let builder = Request::builder()
		.method(method)
		.uri(&url)
		.header("User-Agent", user_agent)
		.header("Client-Vendor-Id", vendor_id)
		.header("X-Reddit-Device-Id", device_id)
		.header("x-reddit-loid", loid)
		.header("Host", "oauth.reddit.com")
		.header("Authorization", &format!("Bearer {token}"))
		.header("Accept-Encoding", if method == Method::GET { "gzip" } else { "identity" })
		.header("Accept-Language", "en-US,en;q=0.5")
		.header("Connection", "keep-alive")
		.header(
			"Cookie",
			if quarantine {
				"_options=%7B%22pref_quarantine_optin%22%3A%20true%2C%20%22pref_gated_sr_optin%22%3A%20true%7D"
			} else {
				""
			},
		)
		.body(Body::empty());

	async move {
		match builder {
			Ok(req) => match client.request(req).await {
				Ok(mut response) => {
					// Reddit may respond with a 3xx. Decide whether or not to
					// redirect based on caller params.
					if response.status().is_redirection() {
						if !redirect {
							return Ok(response);
						};

						return request(
							method,
							response
								.headers()
								.get(header::LOCATION)
								.map(|val| {
									// We need to make adjustments to the URI
									// we get back from Reddit. Namely, we
									// must:
									//
									//     1. Remove the authority (e.g.
									//     https://www.reddit.com) that may be
									//     present, so that we recurse on the
									//     path (and query parameters) as
									//     required.
									//
									//     2. Percent-encode the path.
									let new_path = percent_encode(val.as_bytes(), CONTROLS).to_string().trim_start_matches(REDDIT_URL_BASE).to_string();
									format!("{new_path}{}raw_json=1", if new_path.contains('?') { "&" } else { "?" })
								})
								.unwrap_or_default()
								.to_string(),
							true,
							quarantine,
						)
						.await;
					};

					match response.headers().get(header::CONTENT_ENCODING) {
						// Content not compressed.
						None => Ok(response),

						// Content encoded (hopefully with gzip).
						Some(hdr) => {
							match hdr.to_str() {
								Ok(val) => match val {
									"gzip" => {}
									"identity" => return Ok(response),
									_ => return Err("Reddit response was encoded with an unsupported compressor".to_string()),
								},
								Err(_) => return Err("Reddit response was invalid".to_string()),
							}

							// We get here if the body is gzip-compressed.

							// The body must be something that implements
							// std::io::Read, hence the conversion to
							// bytes::buf::Buf and then transformation into a
							// Reader.
							let mut decompressed: Vec<u8>;
							{
								let mut aggregated_body = match body::aggregate(response.body_mut()).await {
									Ok(b) => b.reader(),
									Err(e) => return Err(e.to_string()),
								};

								let mut decoder = match gzip::Decoder::new(&mut aggregated_body) {
									Ok(decoder) => decoder,
									Err(e) => return Err(e.to_string()),
								};

								decompressed = Vec::<u8>::new();
								if let Err(e) = io::copy(&mut decoder, &mut decompressed) {
									return Err(e.to_string());
								};
							}

							response.headers_mut().remove(header::CONTENT_ENCODING);
							response.headers_mut().insert(header::CONTENT_LENGTH, decompressed.len().into());
							*(response.body_mut()) = Body::from(decompressed);

							Ok(response)
						}
					}
				}
				Err(e) => {
					dbg_msg!("{} {}: {}", method, path, e);

					Err(e.to_string())
				}
			},
			Err(_) => Err("Post url contains non-ASCII characters".to_string()),
		}
	}
	.boxed()
}

// Make a request to a Reddit API and parse the JSON response
#[cached(size = 100, time = 30, result = true)]
pub async fn json(path: String, quarantine: bool) -> Result<Value, String> {
	// Closure to quickly build errors
	let err = |msg: &str, e: String| -> Result<Value, String> {
		// eprintln!("{} - {}: {}", url, msg, e);
		Err(format!("{msg}: {e}"))
	};

	// Fetch the url...
	match reddit_get(path.clone(), quarantine).await {
		Ok(response) => {
			let status = response.status();

			// asynchronously aggregate the chunks of the body
			match hyper::body::aggregate(response).await {
				Ok(body) => {
					// Parse the response from Reddit as JSON
					match serde_json::from_reader(body.reader()) {
						Ok(value) => {
							let json: Value = value;
							// If Reddit returned an error
							if json["error"].is_i64() {
								// OAuth token has expired; http status 401
								if json["message"] == "Unauthorized" {
									error!("Forcing a token refresh");
									let () = force_refresh_token().await;
									return Err("OAuth token has expired. Please refresh the page!".to_string());
								}
								Err(format!("Reddit error {} \"{}\": {}", json["error"], json["reason"], json["message"]))
							} else {
								Ok(json)
							}
						}
						Err(e) => {
							error!("Got an invalid response from reddit {e}. Status code: {status}");
							if status.is_server_error() {
								Err("Reddit is having issues, check if there's an outage".to_string())
							} else {
								err("Failed to parse page JSON data", e.to_string())
							}
						}
					}
				}
				Err(e) => err("Failed receiving body from Reddit", e.to_string()),
			}
		}
		Err(e) => err("Couldn't send request to Reddit", e),
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn test_localization_popular() {
	let val = json("/r/popular/hot.json?&raw_json=1&geo_filter=GLOBAL".to_string(), false).await.unwrap();
	assert_eq!("GLOBAL", val["data"]["geo_filter"].as_str().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_obfuscated_share_link() {
	let share_link = "/r/rust/s/kPgq8WNHRK".into();
	// Correct link without share parameters
	let canonical_link = "/r/rust/comments/18t5968/why_use_tuple_struct_over_standard_struct/kfbqlbc".into();
	assert_eq!(canonical_path(share_link).await, Ok(Some(canonical_link)));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_share_link_strip_json() {
	let link = "/17krzvz".into();
	let canonical_link = "/r/nfl/comments/17krzvz/rapoport_sources_former_no_2_overall_pick/".into();
	assert_eq!(canonical_path(link).await, Ok(Some(canonical_link)));
}
