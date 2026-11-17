use crate::{
	client::{CLIENT, OAUTH_CLIENT, OAUTH_IS_ROLLING_OVER, OAUTH_RATELIMIT_REMAINING},
	oauth_resources::ANDROID_APP_VERSION_LIST,
};
use base64::{engine::general_purpose, Engine as _};
use log::{error, info, trace, warn};
use serde_json::json;
use std::{collections::HashMap, sync::atomic::Ordering, time::Duration};
use tegen::tegen::TextGenerator;
use tokio::time::{error::Elapsed, timeout};

const REDDIT_ANDROID_OAUTH_CLIENT_ID: &str = "ohXpoqrZYub1kg";

/// `MANUFACTURER;MODEL`, as the app sends it in `Device-Name`
const ANDROID_DEVICE_LIST: &[&str] = &[
	"samsung;SM-S921B",
	"samsung;SM-S928B",
	"samsung;SM-A556B",
	"Google;Pixel 8",
	"Google;Pixel 9 Pro",
	"Google;Pixel 7a",
	"OnePlus;CPH2581",
	"Xiaomi;23127PN0CG",
];

const AUTH_ENDPOINT: &str = "https://www.reddit.com";

const OAUTH_TIMEOUT: Duration = Duration::from_secs(5);

// Response from OAuth backend authentication
#[derive(Debug, Clone)]
pub struct OauthResponse {
	pub token: String,
	pub expires_in: u64,
	pub additional_headers: HashMap<String, String>,
}

// Trait for OAuth backend implementations
trait OauthBackend: Send + Sync {
	fn authenticate(&mut self) -> impl std::future::Future<Output = Result<OauthResponse, AuthError>> + Send;
	fn user_agent(&self) -> &str;
	fn get_headers(&self) -> HashMap<String, String>;
}

// OAuth backend implementations
#[derive(Debug, Clone)]
pub(crate) enum OauthBackendImpl {
	MobileSpoof(MobileSpoofAuth),
	GenericWeb(GenericWebAuth),
}

impl OauthBackend for OauthBackendImpl {
	async fn authenticate(&mut self) -> Result<OauthResponse, AuthError> {
		match self {
			OauthBackendImpl::MobileSpoof(backend) => backend.authenticate().await,
			OauthBackendImpl::GenericWeb(backend) => backend.authenticate().await,
		}
	}

	fn user_agent(&self) -> &str {
		match self {
			OauthBackendImpl::MobileSpoof(backend) => backend.user_agent(),
			OauthBackendImpl::GenericWeb(backend) => backend.user_agent(),
		}
	}

	fn get_headers(&self) -> HashMap<String, String> {
		match self {
			OauthBackendImpl::MobileSpoof(backend) => backend.get_headers(),
			OauthBackendImpl::GenericWeb(backend) => backend.get_headers(),
		}
	}
}

// Spoofed client for Android devices
#[derive(Debug, Clone)]
pub struct Oauth {
	pub(crate) headers_map: HashMap<String, String>,
	expires_in: u64,
	pub(crate) backend: OauthBackendImpl,
}

impl Oauth {
	/// Create a new OAuth client
	pub(crate) async fn new() -> Self {
		// Try MobileSpoofAuth first, then fall back to GenericWebAuth
		let mut failure_count = 0;
		let mut backend = OauthBackendImpl::MobileSpoof(MobileSpoofAuth::new());

		loop {
			let attempt = Self::new_with_timeout_with_backend(backend.clone()).await;
			match attempt {
				Ok(Ok(oauth)) => {
					info!("[✅] Successfully created OAuth client");
					return oauth;
				}
				Ok(Err(e)) => {
					error!(
						"[⛔] Failed to create OAuth client: {}. Retrying in 5 seconds...",
						match e {
							AuthError::Wreq(error) => error.to_string(),
							AuthError::SerdeDeserialize(error) => error.to_string(),
							AuthError::Field((value, error)) => format!("{error}\n{value}"),
						}
					);
				}
				Err(_) => {
					error!("[⛔] Failed to create OAuth client before timeout. Retrying in 5 seconds...");
				}
			}

			failure_count += 1;

			// Switch to GenericWeb after 5 failures with MobileSpoof
			if matches!(backend, OauthBackendImpl::MobileSpoof(_)) && failure_count >= 5 {
				warn!("[🔄] MobileSpoofAuth failed 5 times. Falling back to GenericWebAuth...");
				backend = OauthBackendImpl::GenericWeb(GenericWebAuth::new());
			}

			// Crash after 10 total failures
			if failure_count >= 10 {
				error!("[⛔] Failed to create OAuth client (mobile + generic)");
				std::process::exit(1);
			}

			tokio::time::sleep(OAUTH_TIMEOUT).await;
		}
	}

	async fn new_with_timeout_with_backend(mut backend: OauthBackendImpl) -> Result<Result<Self, AuthError>, Elapsed> {
		timeout(OAUTH_TIMEOUT, async move {
			let response = backend.authenticate().await?;

			// Build headers_map from backend headers + Authorization header
			let mut headers_map = backend.get_headers();
			headers_map.insert("Authorization".to_owned(), format!("Bearer {}", response.token));
			headers_map.extend(response.additional_headers);

			Ok(Self {
				headers_map,
				expires_in: response.expires_in,
				backend,
			})
		})
		.await
	}

	pub fn user_agent(&self) -> &str {
		self.backend.user_agent()
	}
}

#[derive(Debug)]
enum AuthError {
	Wreq(wreq::Error),
	SerdeDeserialize(serde_json::Error),
	Field((serde_json::Value, &'static str)),
}

impl From<wreq::Error> for AuthError {
	fn from(err: wreq::Error) -> Self {
		AuthError::Wreq(err)
	}
}

impl From<serde_json::Error> for AuthError {
	fn from(err: serde_json::Error) -> Self {
		AuthError::SerdeDeserialize(err)
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
	user_agent: String,
}

// MobileSpoofAuth backend - spoofs an Android mobile device
#[derive(Debug, Clone)]
pub struct MobileSpoofAuth {
	device: Device,
	additional_headers: HashMap<String, String>,
}

impl MobileSpoofAuth {
	fn new() -> Self {
		Self {
			device: Device::new(),
			additional_headers: HashMap::new(),
		}
	}
}

#[derive(Clone, Copy)]
pub(crate) enum Grant<'a> {
	Loid,
	Session(&'a str),
}

impl MobileSpoofAuth {
	async fn authenticate_with(&mut self, client: &wreq::Client, grant: Grant<'_>) -> Result<OauthResponse, AuthError> {
		// Construct URL for OAuth token
		let url = match grant {
			Grant::Loid => format!("{AUTH_ENDPOINT}/auth/v2/oauth/access-token/loid"),
			Grant::Session(_) => format!("{AUTH_ENDPOINT}/auth/v2/oauth/access-token/session"),
		};
		let mut builder = client.post(&url);
		if let Grant::Session(cookie) = grant {
			builder = builder.header("Cookie", cookie);
		}

		// Add headers from spoofed client
		for (key, value) in &self.device.initial_headers {
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
				"scopes": ["*", "email", "pii", "adsread", "adsedit"]
		});

		trace!("Sending token request to {url}...");

		// Send request
		let resp = builder.json(&json).send().await?;

		trace!("Received response with status {} and length {:?}", resp.status(), resp.headers().get("content-length"));
		// may hold a new session cookie
		if let Grant::Loid = grant {
			trace!("OAuth headers: {:#?}", resp.headers());
		}

		// Parse headers - loid header _should_ be saved sent on subsequent token refreshes.
		// Technically it's not needed, but it's easy for Reddit API to check for this.
		// It's some kind of header that uniquely identifies the device.
		// Not worried about the privacy implications, since this is randomly changed
		// and really only as privacy-concerning as the OAuth token itself.
		if let Some(header) = resp.headers().get("x-reddit-loid") {
			let header_val: &wreq::header::HeaderValue = header;
			self.additional_headers.insert("x-reddit-loid".to_owned(), header_val.to_str().unwrap().to_string());
		}

		// Same with x-reddit-session
		if let Some(header) = resp.headers().get("x-reddit-session") {
			let header_val: &wreq::header::HeaderValue = header;
			self.additional_headers.insert("x-reddit-session".to_owned(), header_val.to_str().unwrap().to_string());
		}

		trace!("Serializing response...");

		// Serialize response
		let json: serde_json::Value = resp.json().await?;

		trace!("Accessing relevant fields...");

		// Save token and expiry
		let token = json
			.get("access_token")
			.ok_or_else(|| AuthError::Field((json.clone(), "access_token")))?
			.as_str()
			.ok_or_else(|| AuthError::Field((json.clone(), "access_token: as_str")))?
			.to_string();
		let expires_in = json
			.get("expires_in")
			.ok_or_else(|| AuthError::Field((json.clone(), "expires_in")))?
			.as_u64()
			.ok_or_else(|| AuthError::Field((json.clone(), "expires_in: as_u64")))?;

		if let Grant::Loid = grant {
			info!("[✅] Success - Retrieved token \"{}...\", expires in {}", &token[..32], expires_in);
		}

		Ok(OauthResponse {
			token,
			expires_in,
			additional_headers: self.additional_headers.clone(),
		})
	}
}

/// The error never contains the token or the cookie.
pub(crate) async fn session_token(cookie: &str) -> Result<(String, u64), String> {
	let response = MobileSpoofAuth::new()
		.authenticate_with(&CLIENT.load_full(), Grant::Session(cookie))
		.await
		.map_err(|e| match e {
			AuthError::Wreq(error) => error.to_string(),
			AuthError::SerdeDeserialize(error) => error.to_string(),
			AuthError::Field((_, field)) => format!("no {field} in Reddit's answer"),
		})?;
	Ok((response.token, response.expires_in))
}

/// Headers of a fresh anonymous token minted with `client`.
#[cfg(test)]
pub(crate) async fn android_headers(client: &wreq::Client) -> Result<HashMap<String, String>, String> {
	let mut backend = MobileSpoofAuth::new();
	let response = backend.authenticate_with(client, Grant::Loid).await.map_err(|e| format!("{e:?}"))?;
	let mut headers = backend.get_headers();
	headers.insert("Authorization".to_owned(), format!("Bearer {}", response.token));
	Ok(headers)
}

impl OauthBackend for MobileSpoofAuth {
	async fn authenticate(&mut self) -> Result<OauthResponse, AuthError> {
		self.authenticate_with(&CLIENT.load_full(), Grant::Loid).await
	}

	fn user_agent(&self) -> &str {
		&self.device.user_agent
	}

	fn get_headers(&self) -> HashMap<String, String> {
		let mut headers = self.device.headers.clone();
		headers.extend(self.additional_headers.clone());
		headers
	}
}

// GenericWebAuth backend - the anonymous token Reddit's website gets. The
// website has no token request: www.reddit.com answers a fresh visitor with a
// JS challenge page, and the answer to it sets the token as the `token_v2`
// cookie.
#[derive(Debug, Clone)]
pub struct GenericWebAuth {
	user_agent: String,
	additional_headers: HashMap<String, String>,
}

impl GenericWebAuth {
	fn new() -> Self {
		info!("[🔄] Using GenericWebAuth");

		Self {
			user_agent: fake_user_agent::get_rua().to_owned(),
			additional_headers: HashMap::new(),
		}
	}
}

/// The `solution` and `jsc_token` of Reddit's JS challenge page. Its script
/// only doubles a string: `(async e=>e+e)("<challenge>")`.
fn challenge_answer(page: &str) -> Option<(String, String)> {
	let challenge = page.split("e=>e+e)(\"").nth(1)?.split('"').next()?;
	let token = page.split("name=\"jsc_token\" value=\"").nth(1)?.split('"').next()?;
	Some((format!("{challenge}{challenge}"), token.to_string()))
}

/// The `name=value` of every cookie a response sets.
fn set_cookies(resp: &wreq::Response) -> Vec<String> {
	resp
		.headers()
		.get_all("set-cookie")
		.iter()
		.filter_map(|value| value.to_str().ok()?.split(';').next().map(str::to_string))
		.collect()
}

/// Seconds a `token_v2` JWT is valid for (`exp - iat`).
fn token_lifetime(token: &str) -> Option<u64> {
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
	let claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').nth(1)?).ok()?).ok()?;
	Some((claims["exp"].as_f64()? - claims["iat"].as_f64()?) as u64)
}

impl OauthBackend for GenericWebAuth {
	async fn authenticate(&mut self) -> Result<OauthResponse, AuthError> {
		let client = CLIENT.load_full();
		let field = |what: &'static str| AuthError::Field((serde_json::Value::Null, what));

		// Reddit sometimes answers with a reCAPTCHA page instead, for a few
		// seconds at a time. Oauth::new tries again after OAUTH_TIMEOUT.
		trace!("Loading the GenericWebAuth challenge page...");
		let resp = client.get("https://www.reddit.com/").header("Accept", "text/html").send().await?;
		let cookies = set_cookies(&resp);
		let status = resp.status();
		let page = resp.text().await?;
		// Without the challenge, name what came instead (e.g. "Reddit - Prove your humanity")
		let title = || format!("{status} {}", page.split("<title>").nth(1).and_then(|t| t.split('<').next()).unwrap_or_default().trim());
		let (solution, jsc_token) = challenge_answer(&page).ok_or_else(|| AuthError::Field((title().into(), "JS challenge on www.reddit.com")))?;

		trace!("Answering the GenericWebAuth challenge...");
		let url = format!("https://www.reddit.com/?solution={solution}&js_challenge=1&jsc_token={jsc_token}&jsc_orig_r=");
		let resp = client
			.get(url)
			.header("Accept", "text/html")
			.header("Referer", "https://www.reddit.com/")
			.header("Cookie", cookies.join("; "))
			.send()
			.await?;
		let token = set_cookies(&resp)
			.into_iter()
			.find_map(|cookie| cookie.strip_prefix("token_v2=").map(str::to_string))
			.ok_or_else(|| field("token_v2 cookie"))?;
		let expires_in = token_lifetime(&token).ok_or_else(|| field("token_v2 expiry"))?;

		info!(
			"[✅] GenericWebAuth success - Retrieved token \"{}...\", expires in {}",
			&token[..32.min(token.len())],
			expires_in
		);

		// Insert a few necessary headers
		self.additional_headers.insert("Origin".to_owned(), "https://www.reddit.com".to_owned());
		self.additional_headers.insert("User-Agent".to_owned(), self.user_agent.to_owned());

		Ok(OauthResponse {
			token,
			expires_in,
			additional_headers: self.additional_headers.clone(),
		})
	}

	fn user_agent(&self) -> &str {
		&self.user_agent
	}

	fn get_headers(&self) -> HashMap<String, String> {
		self.additional_headers.clone()
	}
}

impl Device {
	fn android() -> Self {
		// Generate uuid
		let uuid = uuid::Uuid::new_v4().to_string();

		// Generate random user-agent
		let android_app_version = choose(ANDROID_APP_VERSION_LIST).to_string();
		let android_version = fastrand::u8(12..=16);

		let android_user_agent = format!("Reddit/{android_app_version}/Android {android_version}");

		let qos = fastrand::u32(1000..=100_000);
		let qos: f32 = qos as f32 / 1000.0;
		let qos = format!("down-rate-mbps={qos:.3}");

		let codecs = TextGenerator::new().generate("available-codecs=video/avc, video/hevc{, video/x-vnd.on2.vp9|}");

		// Android device headers
		let headers: HashMap<String, String> = HashMap::from([
			("User-Agent".into(), android_user_agent.clone()),
			("x-reddit-retry".into(), "attempt=0, max=3".into()),
			("x-reddit-compression".into(), "1".into()),
			("x-reddit-qos".into(), qos),
			("x-reddit-media-codecs".into(), codecs),
			("Content-Type".into(), "application/json; charset=UTF-8".into()),
			("client-vendor-id".into(), uuid.clone()),
			("X-Reddit-Device-Id".into(), uuid.clone()),
			("X-Dev-Ad-Id".into(), uuid::Uuid::new_v4().to_string()),
			("Device-Name".into(), choose(ANDROID_DEVICE_LIST).to_string()),
			("Accept-Language".into(), "en-US".into()),
			("X-Reddit-Http-QoS".into(), "GOOD".into()),
		]);

		info!("[🔄] Spoofing Android client with headers: {headers:?}, uuid: \"{uuid}\", and OAuth ID \"{REDDIT_ANDROID_OAUTH_CLIENT_ID}\"");

		Self {
			oauth_id: REDDIT_ANDROID_OAUTH_CLIENT_ID.to_string(),
			headers: headers.clone(),
			initial_headers: headers,
			user_agent: android_user_agent,
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_mobile_spoof_backend() {
		crate::client::block_on_shared(async {
			// Test MobileSpoofAuth backend specifically
			let mut backend = MobileSpoofAuth::new();
			let response = backend.authenticate().await;
			assert!(response.is_ok(), "{response:?}");
			let response = response.unwrap();
			assert!(!response.token.is_empty());
			assert!(response.expires_in > 0);
			assert!(!backend.user_agent().is_empty());
			assert!(!backend.get_headers().is_empty());
		});
	}

	#[test]
	#[ignore = "www.reddit.com answers GitHub's runners with 403; run with --include-ignored"]
	fn test_generic_web_backend() {
		crate::client::block_on_shared(async {
			// Test GenericWebAuth backend specifically, retrying like Oauth::new
			let mut backend = GenericWebAuth::new();
			let mut response = backend.authenticate().await;
			for _ in 1..5 {
				if response.is_ok() {
					break;
				}
				tokio::time::sleep(OAUTH_TIMEOUT).await;
				response = backend.authenticate().await;
			}
			assert!(response.is_ok(), "{response:?}");
			let response = response.unwrap();
			assert!(!response.token.is_empty());
			assert!(response.expires_in > 0);
			assert!(!backend.user_agent().is_empty());

			// The token works on the API like the app's
			let about = CLIENT
				.load_full()
				.get("https://oauth.reddit.com/r/rust/about.json?raw_json=1")
				.header("Authorization", format!("Bearer {}", response.token))
				.header("User-Agent", backend.user_agent())
				.send()
				.await
				.unwrap();
			assert_eq!(about.status(), 200);
		});
	}

	#[test]
	fn test_challenge_answer() {
		let page =
			r#"<script>e.elements.namedItem("solution").value=n,await (async e=>e+e)("dca4e9c023c34e03");</script><input type="hidden" name="jsc_token" value="2824be10"/>"#;
		assert_eq!(challenge_answer(page), Some(("dca4e9c023c34e03dca4e9c023c34e03".to_string(), "2824be10".to_string())));
		assert_eq!(challenge_answer("<html></html>"), None);
	}

	#[test]
	fn test_oauth_client() {
		crate::client::block_on_shared(async {
			// Integration test - tests the overall Oauth client
			assert!(OAUTH_CLIENT.load_full().headers_map.contains_key("Authorization"));
		});
	}

	#[test]
	fn test_oauth_client_refresh() {
		crate::client::block_on_shared(async {
			force_refresh_token().await;
		});
	}

	#[test]
	fn test_oauth_token_exists() {
		crate::client::block_on_shared(async {
			let client = OAUTH_CLIENT.load_full();
			let auth_header = client.headers_map.get("Authorization").unwrap();
			assert!(auth_header.starts_with("Bearer "));
		});
	}

	#[test]
	fn test_oauth_headers_len() {
		crate::client::block_on_shared(async {
			assert!(OAUTH_CLIENT.load_full().headers_map.len() >= 3);
		});
	}

	#[test]
	fn test_creating_device() {
		Device::new();
	}

	#[test]
	fn test_creating_backends() {
		// Test that both backends can be created
		MobileSpoofAuth::new();
		GenericWebAuth::new();
	}
}
