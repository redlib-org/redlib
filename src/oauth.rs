use std::{collections::HashMap, sync::atomic::Ordering, time::Duration};

use crate::{
	client::{OAUTH_CLIENT, OAUTH_IS_ROLLING_OVER, OAUTH_RATELIMIT_REMAINING},
	oauth_resources::ANDROID_APP_VERSION_LIST,
};
use base64::{engine::general_purpose, Engine as _};
use hyper::{client, Body, Method, Request};
use log::{error, info, trace};

use serde_json::json;
use tokio::time::{error::Elapsed, timeout};

static REDDIT_ANDROID_OAUTH_CLIENT_ID: &str = "ohXpoqrZYub1kg";

static AUTH_ENDPOINT: &str = "https://www.reddit.com";

// Spoofed client for Android devices
#[derive(Debug, Clone, Default)]
pub struct Oauth {
	pub(crate) initial_headers: HashMap<String, String>,
	pub(crate) headers_map: HashMap<String, String>,
	pub(crate) token: String,
	expires_in: u64,
	device: Device,
}

impl Oauth {
	/// Create a new OAuth client
	pub(crate) async fn new() -> Self {
		// Call new_internal until it succeeds
		loop {
			let attempt = Self::new_with_timeout().await;
			match attempt {
				Ok(Some(oauth)) => {
					info!("[✅] Successfully created OAuth client");
					return oauth;
				}
				Ok(None) => {
					error!("Failed to create OAuth client. Retrying in 5 seconds...");
					continue;
				}
				Err(duration) => {
					error!("Failed to create OAuth client in {duration:?}. Retrying in 5 seconds...");
				}
			}
		}
	}

	async fn new_with_timeout() -> Result<Option<Self>, Elapsed> {
		let mut oauth = Self::default();
		timeout(Duration::from_secs(5), oauth.login()).await.map(|result| result.map(|_| oauth))
	}

	pub(crate) fn default() -> Self {
		// Generate a device to spoof
		let device = Device::new();
		let headers_map = device.headers.clone();
		let initial_headers = device.initial_headers.clone();
		// For now, just insert headers - no token request
		Self {
			headers_map,
			initial_headers,
			token: String::new(),
			expires_in: 0,
			device,
		}
	}
	async fn login(&mut self) -> Option<()> {
		// Construct URL for OAuth token
		let url = format!("{AUTH_ENDPOINT}/auth/v2/oauth/access-token/loid");
		let mut builder = Request::builder().method(Method::POST).uri(&url);

		// Add headers from spoofed client
		for (key, value) in &self.initial_headers {
			builder = builder.header(key, value);
		}
		// Set up HTTP Basic Auth - basically just the const OAuth ID's with no password,
		// Base64-encoded. https://en.wikipedia.org/wiki/Basic_access_authentication
		// This could be constant, but I don't think it's worth it. OAuth ID's can change
		// over time and we want to be flexible.
		let auth = general_purpose::STANDARD.encode(format!("{}:", self.device.oauth_id));
		builder = builder.header("Authorization", format!("Basic {auth}"));

		// Set JSON body. I couldn't tell you what this means. But that's what the client sends
		let json = json!({
				"scopes": ["*","email"]
		});
		let body = Body::from(json.to_string());

		// Build request
		let request = builder.body(body).unwrap();

		trace!("Sending token request...");

		// Send request
		let client: client::Client<_, Body> = {
			let https = hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().https_only().enable_http1().build();
			client::Client::builder().build(https)
		};
		let resp = client.request(request).await.ok()?;

		trace!("Received response with status {} and length {:?}", resp.status(), resp.headers().get("content-length"));

		// Parse headers - loid header _should_ be saved sent on subsequent token refreshes.
		// Technically it's not needed, but it's easy for Reddit API to check for this.
		// It's some kind of header that uniquely identifies the device.
		// Not worried about the privacy implications, since this is randomly changed
		// and really only as privacy-concerning as the OAuth token itself.
		if let Some(header) = resp.headers().get("x-reddit-loid") {
			self.headers_map.insert("x-reddit-loid".to_owned(), header.to_str().ok()?.to_string());
		}

		// Same with x-reddit-session
		if let Some(header) = resp.headers().get("x-reddit-session") {
			self.headers_map.insert("x-reddit-session".to_owned(), header.to_str().ok()?.to_string());
		}

		trace!("Serializing response...");

		// Serialize response
		let body_bytes = hyper::body::to_bytes(resp.into_body()).await.ok()?;
		let json: serde_json::Value = serde_json::from_slice(&body_bytes).ok()?;

		trace!("Accessing relevant fields...");

		// Save token and expiry
		self.token = json.get("access_token")?.as_str()?.to_string();
		self.expires_in = json.get("expires_in")?.as_u64()?;
		self.headers_map.insert("Authorization".to_owned(), format!("Bearer {}", self.token));

		info!("[✅] Success - Retrieved token \"{}...\", expires in {}", &self.token[..32], self.expires_in);

		Some(())
	}
}

pub async fn token_daemon() {
	// Monitor for refreshing token
	loop {
		// Get expiry time - be sure to not hold the read lock
		let expires_in = { OAUTH_CLIENT.load_full().expires_in };

		// sleep for the expiry time minus 2 minutes
		let duration = Duration::from_secs(expires_in - 120);

		info!("[⏳] Waiting for {duration:?} seconds before refreshing OAuth token...");

		tokio::time::sleep(duration).await;

		info!("[⌛] {duration:?} Elapsed! Refreshing OAuth token...");

		// Refresh token - in its own scope
		{
			force_refresh_token().await;
		}
	}
}

pub async fn force_refresh_token() {
	if OAUTH_IS_ROLLING_OVER.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
		trace!("Skipping refresh token roll over, already in progress");
		return;
	}

	trace!("Rolling over refresh token. Current rate limit: {}", OAUTH_RATELIMIT_REMAINING.load(Ordering::SeqCst));
	let new_client = Oauth::new().await;
	OAUTH_CLIENT.swap(new_client.into());
	OAUTH_RATELIMIT_REMAINING.store(99, Ordering::SeqCst);
	OAUTH_IS_ROLLING_OVER.store(false, Ordering::SeqCst);
}

#[derive(Debug, Clone, Default)]
struct Device {
	oauth_id: String,
	initial_headers: HashMap<String, String>,
	headers: HashMap<String, String>,
}

impl Device {
	fn android() -> Self {
		// Generate uuid
		let uuid = uuid::Uuid::new_v4().to_string();

		// Generate random user-agent
		let android_app_version = choose(ANDROID_APP_VERSION_LIST).to_string();
		let android_version = fastrand::u8(9..=14);

		let android_user_agent = format!("Reddit/{android_app_version}/Android {android_version}");

		// Android device headers
		let headers = HashMap::from([
			("Client-Vendor-Id".into(), uuid.clone()),
			("X-Reddit-Device-Id".into(), uuid.clone()),
			("User-Agent".into(), android_user_agent),
		]);

		info!("[🔄] Spoofing Android client with headers: {headers:?}, uuid: \"{uuid}\", and OAuth ID \"{REDDIT_ANDROID_OAUTH_CLIENT_ID}\"");

		Self {
			oauth_id: REDDIT_ANDROID_OAUTH_CLIENT_ID.to_string(),
			headers: headers.clone(),
			initial_headers: headers,
		}
	}
	fn new() -> Self {
		// See https://github.com/redlib-org/redlib/issues/8
		Self::android()
	}
}

fn choose<T: Copy>(list: &[T]) -> T {
	*fastrand::choose_multiple(list.iter(), 1)[0]
}

#[tokio::test(flavor = "multi_thread")]
async fn test_oauth_client() {
	assert!(!OAUTH_CLIENT.load_full().token.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_oauth_client_refresh() {
	force_refresh_token().await;
}
#[tokio::test(flavor = "multi_thread")]
async fn test_oauth_token_exists() {
	assert!(!OAUTH_CLIENT.load_full().token.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_oauth_headers_len() {
	assert!(OAUTH_CLIENT.load_full().headers_map.len() >= 3);
}

#[test]
fn test_creating_device() {
	Device::new();
}
