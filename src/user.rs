#![allow(clippy::cmp_owned)]

// CRATES
use crate::client::json;
use crate::config::CONFIG;
use crate::server::RequestExt;
use crate::utils::{error, filter_posts, format_url, get_filters, nsfw_landing, nsfw_landingx, param, rewrite_urls, setting, template, Post, Preferences, ResourceType, User};
use crate::{config, utils};
use askama::Template;
use axum::extract::{OriginalUri, Path, Query, RawQuery};
use axum::response::{Html, IntoResponse};
use chrono::DateTime;
use htmlescape::decode_html;
use http_api_problem::{ApiError, StatusCode};
use hyper::{Body, Request, Response};
use serde::Deserialize;
use serde_inline_default::serde_inline_default;
use std::collections::HashMap;
use std::sync::Arc;
use time::{macros::format_description, OffsetDateTime};

// STRUCTS
#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
	user: User,
	posts: Vec<Post>,
	sort: (String, String),
	ends: (String, String),
	/// "overview", "comments", or "submitted"
	listing: String, // TODO turn into an enum
	prefs: Arc<Preferences>,
	url: String,
	redirect_url: String,
	/// Whether the user themself is filtered.
	is_filtered: bool,
	/// Whether all fetched posts are filtered (to differentiate between no posts fetched in the first place,
	/// and all fetched posts being filtered).
	all_posts_filtered: bool,
	/// Whether all posts were hidden because they are NSFW (and user has disabled show NSFW)
	all_posts_hidden_nsfw: bool,
	no_posts: bool,
}

// FUNCTIONS
pub async fn profile(req: Request<Body>) -> Result<Response<Body>, String> {
	let listing = req.param("listing").unwrap_or_else(|| "overview".to_string());

	// Build the Reddit JSON API path
	let path = format!(
		"/user/{}/{listing}.json?{}&raw_json=1",
		req.param("name").unwrap_or_else(|| "reddit".to_string()),
		req.uri().query().unwrap_or_default(),
	);
	let url = String::from(req.uri().path_and_query().map_or("", |val| val.as_str()));
	let redirect_url = url[1..].replace('?', "%3F").replace('&', "%26");

	// Retrieve other variables from Redlib request
	let sort = param(&path, "sort").unwrap_or_default();
	let username = req.param("name").unwrap_or_default();

	// Retrieve info from user about page.
	let user = user(&username).await.unwrap_or_default();

	let req_url = req.uri().to_string();
	// Return landing page if this post if this Reddit deems this user NSFW,
	// but we have also disabled the display of NSFW content or if the instance
	// is SFW-only.
	if user.nsfw && crate::utils::should_be_nsfw_gated(&req, &req_url) {
		return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
	}

	let filters = get_filters(&req);
	if filters.contains(&["u_", &username].concat()) {
		Ok(template(&UserTemplate {
			user,
			posts: Vec::new(),
			sort: (sort, param(&path, "t").unwrap_or_default()),
			ends: (param(&path, "after").unwrap_or_default(), String::new()),
			listing,
			prefs: Arc::new(Preferences::new(&req)),
			url,
			redirect_url,
			is_filtered: true,
			all_posts_filtered: false,
			all_posts_hidden_nsfw: false,
			no_posts: false,
		}))
	} else {
		// Request user posts/comments from Reddit
		match Post::fetch(&path, false).await {
			Ok((mut posts, after)) => {
				let (_, all_posts_filtered) = filter_posts(&mut posts, &filters);
				let no_posts = posts.is_empty();
				let all_posts_hidden_nsfw = !no_posts && (posts.iter().all(|p| p.flags.nsfw) && setting(&req, "show_nsfw") != "on");
				Ok(template(&UserTemplate {
					user,
					posts,
					sort: (sort, param(&path, "t").unwrap_or_default()),
					ends: (param(&path, "after").unwrap_or_default(), after),
					listing,
					prefs: Arc::new(Preferences::new(&req)),
					url,
					redirect_url,
					is_filtered: false,
					all_posts_filtered,
					all_posts_hidden_nsfw,
					no_posts,
				}))
			}
			// If there is an error show error page
			Err(msg) => error(req, &msg).await,
		}
	}
}

#[serde_inline_default]
#[derive(Deserialize)]
pub struct ProfilePathParameters {
	pub name: String,
	#[serde_inline_default("overview".into())]
	pub listing: String, // TODO convert to enum
}
pub async fn profilex(
	Path(parameters): Path<ProfilePathParameters>,
	RawQuery(raw_query): RawQuery,
	query: Query<HashMap<String, String>>,
	prefs: Preferences,
	original_uri: OriginalUri,
) -> Result<Html<String>, ApiError> {
	let prefs = Arc::new(prefs);
	let url: String = format!("/user/{}/{}.json?{}&raw_json=1", parameters.name, parameters.listing, raw_query.unwrap_or_default());

	let path_and_query_url = original_uri
		.path_and_query()
		.expect("We had path parameters, therefore we should have a path at least")
		.as_str();
	// As the above should never be empty, the below code (designed to remove leading "/") should never panic.
	// Using a url_encoding crate is potentially expensive, so use this for now (which has worked fine for many years since LibReddit).
	// TODO: if the query contains url-encoded "?" or "&", they will probably be decoded too early.
	// TODO: This should proabably be redesigned to avoid this unsafe slice. `user.html` needs to be refactored.
	let redirect_url = path_and_query_url[1..].replace('?', "%3F").replace('&', "%26");
	let user = user(&parameters.name).await.unwrap_or_default();
	let sort = query.get("sort").cloned().unwrap_or_default();

	if user.nsfw && crate::utils::should_be_nsfw_gatedx(&prefs, &query) {
		return nsfw_landingx(prefs, parameters.name, ResourceType::User, original_uri.to_string()).await;
	}

	//RANDOM STUFF BELOW !!!

	if prefs.filters.contains(&format!("u_{}", parameters.name)) {
		Ok(Html(
			UserTemplate {
				user,
				posts: Vec::new(), // Return empty vector because user is filtered
				sort: (sort, query.get("t").cloned().unwrap_or_default()),
				ends: (query.get("after").cloned().unwrap_or_default(), String::new()),
				listing: parameters.listing,
				prefs, // We can move prefs. Otherwise stick to prefs.clone()
				url: path_and_query_url.to_string(),
				redirect_url,
				is_filtered: true,
				all_posts_filtered: false,
				all_posts_hidden_nsfw: false,
				no_posts: false,
			}
			.render()
			.unwrap(),
		)) //FIXME: is this unwrap safe?
	} else {
		// Request user posts/comments from Reddit
		match Post::fetch(&url, false).await {
			Ok((mut posts, after)) => {
				let (_, all_posts_filtered) = filter_posts(&mut posts, &prefs.get_filters_hashset());
				let no_posts = posts.is_empty();
				//TODO: Currently the actual nsfw filtering happens in the user template. It's better to bring that functionality here.
				let all_posts_hidden_nsfw = !no_posts && prefs.show_nsfw != "on" && posts.iter().all(|p| p.flags.nsfw);
				Ok(Html(
					UserTemplate {
						user,
						posts,
						sort: (sort, query.get("t").cloned().unwrap_or_default()),
						ends: (query.get("after").cloned().unwrap_or_default(), after),
						listing: parameters.listing,
						prefs,
						url: path_and_query_url.to_string(),
						redirect_url,
						is_filtered: false,
						all_posts_filtered,
						all_posts_hidden_nsfw,
						no_posts,
					}
					.render()
					.unwrap(),
				)) //FIXME: is this unwrap safe?
			}
			// If there is an error show error page
			Err(msg) => Err(
				ApiError::builder(http_api_problem::StatusCode::INTERNAL_SERVER_ERROR)
					.title("Temporary error while error messages are migrated") //FIXME
					.message(msg)
					.finish(),
			),
		}
	}
}

// TODO: Why not just use the Reddit RSS Api? This way is very roundabout and I would avoid it if possible.
pub async fn rssx(Path(params): Path<ProfilePathParameters>, RawQuery(raw_query): RawQuery) -> Result<impl IntoResponse, ApiError> {
	if CONFIG.enable_rss.is_none() {
		// I'm a teapot, yay!
		return Err(
			ApiError::builder(http_api_problem::StatusCode::IM_A_TEAPOT)
				.title("RSS is disabled on this instance")
				.message("The RSS feed is disabled on this instance.")
				.finish(),
		);
	}

	use rss::{ChannelBuilder, Item};
	let path = format!("/user/{}/{}.json?{}&raw_json=1", &params.name, &params.listing, &raw_query.unwrap_or_default());

	//FIXME: This unwrap is in original code. Should be fixed once this function is reimplemented.
	let user = user(&params.name).await.unwrap_or_default();
	let (posts, _) = Post::fetch(&path, false)
		.await
		.map_err(|e| ApiError::builder(StatusCode::BAD_GATEWAY).title("Could Not Fetch Posts").message(e))?;

	let channel = ChannelBuilder::default()
		.title(params.name)
		.description(user.description)
		.items(
			posts
				.into_iter()
				.map(|post| Item {
					title: Some(post.title.to_string()),
					link: Some(format_url(&utils::get_post_url(&post))),
					author: Some(post.author.name),
					pub_date: Some(DateTime::from_timestamp(post.created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
					content: Some(rewrite_urls(&decode_html(&post.body).unwrap())),
					..Default::default()
				})
				.collect::<Vec<_>>(),
		)
		.build();

	// set content-type
	Ok(([(axum::http::header::CONTENT_TYPE, "application/rss+xml")], channel.to_string()))
}

// USER
// TODO: reimplement this helper function
async fn user(name: &str) -> Result<User, String> {
	// Build the Reddit JSON API path
	let path: String = format!("/user/{name}/about.json?raw_json=1");

	// Send a request to the url
	json(path, false).await.map(|res| {
		// Grab creation date as unix timestamp
		let created_unix = res["data"]["created"].as_f64().unwrap_or(0.0).round() as i64;
		let created = OffsetDateTime::from_unix_timestamp(created_unix).unwrap_or(OffsetDateTime::UNIX_EPOCH);

		// Closure used to parse JSON from Reddit APIs
		let about = |item| res["data"]["subreddit"][item].as_str().unwrap_or_default().to_string();

		// Parse the JSON output into a User struct
		User {
			name: res["data"]["name"].as_str().unwrap_or(name).to_owned(),
			title: about("title"),
			icon: format_url(&about("icon_img")),
			karma: res["data"]["total_karma"].as_i64().unwrap_or(0),
			created: created.format(format_description!("[month repr:short] [day] '[year repr:last_two]")).unwrap_or_default(),
			banner: about("banner_img"),
			description: about("public_description"),
			nsfw: res["data"]["subreddit"]["over_18"].as_bool().unwrap_or_default(),
		}
	})
}

#[inline]
pub async fn user_deleted_error() -> ApiError {
	ApiError::builder(http_api_problem::StatusCode::NOT_FOUND)
		.title("User has deleted their account")
		.message("The user you are looking for does not exist or has been deleted.")
		.finish()
}

pub async fn rss(req: Request<Body>) -> Result<Response<Body>, String> {
	if config::get_setting("REDLIB_ENABLE_RSS").is_none() {
		return Ok(error(req, "RSS is disabled on this instance.").await.unwrap_or_default());
	}
	use crate::utils::rewrite_urls;
	use hyper::header::CONTENT_TYPE;
	use rss::{ChannelBuilder, Item};

	// Get user
	let user_str = req.param("name").unwrap_or_default();

	let listing = req.param("listing").unwrap_or_else(|| "overview".to_string());

	// Get path
	let path = format!("/user/{user_str}/{listing}.json?{}&raw_json=1", req.uri().query().unwrap_or_default(),);

	// Get user
	let user_obj = user(&user_str).await.unwrap_or_default();

	// Get posts
	let (posts, _) = Post::fetch(&path, false).await?;

	// Build the RSS feed
	let channel = ChannelBuilder::default()
		.title(user_str)
		.description(user_obj.description)
		.items(
			posts
				.into_iter()
				.map(|post| Item {
					title: Some(post.title.to_string()),
					link: Some(format_url(&utils::get_post_url(&post))),
					author: Some(post.author.name),
					pub_date: Some(DateTime::from_timestamp(post.created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
					content: Some(rewrite_urls(&decode_html(&post.body).unwrap())),
					..Default::default()
				})
				.collect::<Vec<_>>(),
		)
		.build();

	// Serialize the feed to RSS
	let body = channel.to_string().into_bytes();

	// Create the HTTP response
	let mut res = Response::new(Body::from(body));
	res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/rss+xml"));

	Ok(res)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_user() {
	let user = user("spez").await;
	assert!(user.is_ok());
	assert!(user.unwrap().karma > 100);
}
