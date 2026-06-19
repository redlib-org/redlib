use hyper::{Body, Response};
use serde_json::Value;
use wreq::{Method, Request};
use wreq::header::HeaderValue;
use std::sync::LazyLock;

use crate::client::{proxy, CLIENT};

// RedGifs token cache: (token, expiry_timestamp)
static REDGIFS_TOKEN: LazyLock<std::sync::Mutex<(String, i64)>> = LazyLock::new(|| std::sync::Mutex::new((String::new(), 0)));

pub fn is_redgifs_domain(domain: &str) -> bool {
	domain == "redgifs.com" || domain == "www.redgifs.com" || domain.ends_with(".redgifs.com")
}

/// Handles both video IDs (redirects) and actual video files (proxies)
pub async fn handler(req: hyper::Request<Body>) -> Result<Response<Body>, String> {
	let path = req.uri().path();

	if path.ends_with(".mp4") {
		return proxy(req, &format!("https://media.redgifs.com/{}", path)).await;
	}

	match fetch_video_url(&format!("https://www.redgifs.com/watch/{}", path)).await.ok() {
		Some(video_url) => {
			let filename = video_url.strip_prefix("https://media.redgifs.com/").unwrap_or(&video_url);
			Ok(Response::builder()
				.status(302)
				.header("Location", format!("/redgifs/{}", filename))
				.body(Body::empty())
				.unwrap_or_default())
		}
		None => Ok(Response::builder().status(404).body("RedGifs video not found".into()).unwrap_or_default()),
	}
}

async fn fetch_video_url(redgifs_url: &str) -> Result<String, String> {
	let video_id = redgifs_url
		.split('/')
		.last()
		.and_then(|s| s.split('?').next())
		.ok_or("Invalid RedGifs URL")?;

	let token = get_token().await?;
	let api_url = format!("https://api.redgifs.com/v2/gifs/{}?views=yes", video_id);

	let req = create_request(&api_url, Some(&token));
	let res = CLIENT.execute(req).await.map_err(|e| e.to_string())?;
	let body_bytes = hyper::body::to_bytes(res.into()).await.map_err(|e| e.to_string())?;
	let json: Value = serde_json::from_slice(&body_bytes).map_err(|e| e.to_string())?;

	// Prefer HD, fallback to SD
	let hd_url = json["gif"]["urls"]["hd"].as_str();
	let sd_url = json["gif"]["urls"]["sd"].as_str();

	hd_url
		.or(sd_url)
		.map(String::from)
		.ok_or_else(|| "No video URL in RedGifs response".to_string())
}

async fn get_token() -> Result<String, String> {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_err(|_| "Time error")?
		.as_secs() as i64;

	// Return cached token if still valid (without holding lock across await)
	{
		let cache = REDGIFS_TOKEN.lock().map_err(|_| "Lock error")?;
		if !cache.0.is_empty() && now < cache.1 {
			return Ok(cache.0.clone());
		}
	}

	let req = create_request("https://api.redgifs.com/v2/auth/temporary", None);
	let res = CLIENT.execute(req).await.expect("Ouch");
	let body_bytes = res.bytes().await.expect("Ouch");
	let json: Value = serde_json::from_slice(&body_bytes).map_err(|e| e.to_string())?;
	let token = json["token"].as_str().map(String::from).ok_or_else(|| "No token in RedGifs response".to_string())?;

	let mut cache = REDGIFS_TOKEN.lock().map_err(|_| "Lock error")?;
	cache.0 = token.clone();
	cache.1 = now + 86000; // 24h - 400s buffer
	Ok(token)
}

fn create_request(url: &str, token: Option<&str>) -> Request {
	let real_url = url.into();
	let real_token = token.clone();
	let mut builder = wreq::Request::new(Method::GET, wreq::Uri::from_static(real_url));
	let mut builder_header = builder.headers_mut();
	builder_header.insert("user-agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"));
	builder_header.insert("referer", HeaderValue::from_static("https://www.redgifs.com/"));
	builder_header.insert("origin", HeaderValue::from_static("https://www.redgifs.com"));
	builder_header.insert("content-type", HeaderValue::from_static("application/json"));
	
	if let Some(t) = real_token {
		builder_header.insert("Authorization", HeaderValue::from_static(t.clone()));
	}
	
	return builder;
}
