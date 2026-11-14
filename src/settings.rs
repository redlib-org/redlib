#![allow(clippy::cmp_owned)]

use std::collections::HashMap;

// CRATES
use crate::body::Body;
use crate::server::ResponseExt;
use crate::subreddit::join_until_size_limit;
use crate::utils::{deflate_decompress, redirect, template, Preferences};
use askama::Template;
use cookie::Cookie;
use hyper::{Request, Response};
use time::{Duration, OffsetDateTime};
use tokio::time::timeout;
use url::form_urlencoded;

// STRUCTS
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
	prefs: Preferences,
	url: String,
	return_url: String,
}

// CONSTANTS

const PREFS: [&str; 26] = [
	"theme",
	"front_page",
	"layout",
	"wide",
	"comment_sort",
	"post_sort",
	"blur_spoiler",
	"show_nsfw",
	"blur_nsfw",
	"use_hls",
	"hide_hls_notification",
	"autoplay_videos",
	"hide_sidebar_and_summary",
	"fixed_navbar",
	"hide_awards",
	"hide_score",
	"disable_visit_reddit_confirmation",
	"video_quality",
	"remove_default_feeds",
	"geo_filter",
	"clean_urls",
	"posts_per_page",
	"max_comment_thread_depth",
	"more_replies_new_page",
	"light_theme",
	"dark_theme",
];

pub static GEO_FILTERS: [(&str, &str); 250] = [
	("GLOBAL", "🌐 Global"),
	("AX", "🇦🇽 Åland (Åland Islands)"),
	("AS", "🇦🇸 American Samoa"),
	("AD", "🇦🇩 Andorra"),
	("AO", "🇦🇴 Angola"),
	("AI", "🇦🇮 Anguilla"),
	("AQ", "🇦🇶 Antarctica"),
	("AG", "🇦🇬 Antigua and Barbuda"),
	("AR", "🇦🇷 Argentina"),
	("AW", "🇦🇼 Aruba"),
	("AU", "🇦🇺 Australia"),
	("AZ", "🇦🇿 Azərbaycan (Azerbaijan)"),
	("BS", "🇧🇸 Bahamas"),
	("BB", "🇧🇧 Barbados"),
	("BE", "🇧🇪 België (Belgium)"),
	("BZ", "🇧🇿 Belize"),
	("BJ", "🇧🇯 Bénin"),
	("BM", "🇧🇲 Bermuda"),
	("BO", "🇧🇴 Bolivia"),
	("BW", "🇧🇼 Botswana"),
	("BV", "🇧🇻 Bouvet Island"),
	("BR", "🇧🇷 Brasil (Brazil)"),
	("IO", "🇮🇴 British Indian Ocean Territory"),
	("VG", "🇻🇬 British Virgin Islands"),
	("BN", "🇧🇳 Brunei"),
	("BF", "🇧🇫 Burkina Faso"),
	("CV", "🇨🇻 Cabo Verde (Cape Verde)"),
	("CM", "🇨🇲 Cameroun (Cameroon)"),
	("CA", "🇨🇦 Canada"),
	("BQ", "🇧🇶 Caribisch Nederland (Caribbean Netherlands)"),
	("KY", "🇰🇾 Cayman Islands"),
	("CZ", "🇨🇿 Česko (Czechia)"),
	("CL", "🇨🇱 Chile"),
	("CX", "🇨🇽 Christmas Island"),
	("VA", "🇻🇦 Città del Vaticano (Vatican City)"),
	("CC", "🇨🇨 Cocos (Keeling) Islands"),
	("CO", "🇨🇴 Colombia"),
	("CG", "🇨🇬 Congo-Brazzaville (Congo)"),
	("CD", "🇨🇩 Congo-Kinshasa (DR Congo)"),
	("CK", "🇨🇰 Cook Islands"),
	("CR", "🇨🇷 Costa Rica"),
	("CI", "🇨🇮 Côte d’Ivoire"),
	("ME", "🇲🇪 Crna Gora (Montenegro)"),
	("CU", "🇨🇺 Cuba"),
	("DK", "🇩🇰 Danmark (Denmark)"),
	("DE", "🇩🇪 Deutschland (Germany)"),
	("DJ", "🇩🇯 Djibouti"),
	("DM", "🇩🇲 Dominica"),
	("EC", "🇪🇨 Ecuador"),
	("EE", "🇪🇪 Eesti (Estonia)"),
	("SV", "🇸🇻 El Salvador"),
	("ES", "🇪🇸 España (Spain)"),
	("SZ", "🇸🇿 Eswatini"),
	("FK", "🇫🇰 Falkland Islands"),
	("FJ", "🇫🇯 Fiji"),
	("FR", "🇫🇷 France"),
	("TF", "🇹🇫 French Southern Territories"),
	("FO", "🇫🇴 Føroyar (Faroe Islands)"),
	("GA", "🇬🇦 Gabon"),
	("GM", "🇬🇲 Gambia"),
	("GH", "🇬🇭 Ghana"),
	("GI", "🇬🇮 Gibraltar"),
	("GD", "🇬🇩 Grenada"),
	("GP", "🇬🇵 Guadeloupe"),
	("GU", "🇬🇺 Guam"),
	("GT", "🇬🇹 Guatemala"),
	("GG", "🇬🇬 Guernsey"),
	("GW", "🇬🇼 Guiné-Bissau (Guinea-Bissau)"),
	("GQ", "🇬🇶 Guinea Ecuatorial (Equatorial Guinea)"),
	("GN", "🇬🇳 Guinée (Guinea)"),
	("GY", "🇬🇾 Guyana"),
	("GF", "🇬🇫 Guyane française (French Guiana)"),
	("HT", "🇭🇹 Haïti"),
	("HM", "🇭🇲 Heard & McDonald Islands"),
	("HN", "🇭🇳 Honduras"),
	("HR", "🇭🇷 Hrvatska (Croatia)"),
	("ID", "🇮🇩 Indonesia"),
	("IE", "🇮🇪 Ireland"),
	("IS", "🇮🇸 Ísland (Iceland)"),
	("IM", "🇮🇲 Isle of Man"),
	("IT", "🇮🇹 Italia (Italy)"),
	("JM", "🇯🇲 Jamaica"),
	("JE", "🇯🇪 Jersey"),
	("GL", "🇬🇱 Kalaallit Nunaat (Greenland)"),
	("KE", "🇰🇪 Kenya"),
	("KI", "🇰🇮 Kiribati"),
	("CF", "🇨🇫 Ködörösêse tî Bêafrîka (Central African Republic)"),
	("CW", "🇨🇼 Kòrsou (Curaçao)"),
	("RE", "🇷🇪 La Réunion (Réunion)"),
	("LV", "🇱🇻 Latvija (Latvia)"),
	("LS", "🇱🇸 Lesotho"),
	("LR", "🇱🇷 Liberia"),
	("LI", "🇱🇮 Liechtenstein"),
	("LT", "🇱🇹 Lietuva (Lithuania)"),
	("LU", "🇱🇺 Luxembourg"),
	("MG", "🇲🇬 Madagasikara (Madagascar)"),
	("HU", "🇭🇺 Magyarország (Hungary)"),
	("MW", "🇲🇼 Malawi"),
	("MY", "🇲🇾 Malaysia"),
	("ML", "🇲🇱 Mali"),
	("MT", "🇲🇹 Malta"),
	("MH", "🇲🇭 Marshall Islands"),
	("MQ", "🇲🇶 Martinique"),
	("MU", "🇲🇺 Maurice (Mauritius)"),
	("YT", "🇾🇹 Mayotte"),
	("MX", "🇲🇽 México"),
	("FM", "🇫🇲 Micronesia"),
	("MZ", "🇲🇿 Moçambique (Mozambique)"),
	("MC", "🇲🇨 Monaco"),
	("MS", "🇲🇸 Montserrat"),
	("NA", "🇳🇦 Namibia"),
	("NR", "🇳🇷 Nauru"),
	("NL", "🇳🇱 Nederland (Netherlands)"),
	("NZ", "🇳🇿 New Zealand"),
	("NI", "🇳🇮 Nicaragua"),
	("NE", "🇳🇪 Niger"),
	("NG", "🇳🇬 Nigeria"),
	("NU", "🇳🇺 Niue"),
	("NF", "🇳🇫 Norfolk Island"),
	("NO", "🇳🇴 Norge (Norway)"),
	("MP", "🇲🇵 Northern Mariana Islands"),
	("NC", "🇳🇨 Nouvelle-Calédonie (New Caledonia)"),
	("AT", "🇦🇹 Österreich (Austria)"),
	("UZ", "🇺🇿 Oʻzbekiston (Uzbekistan)"),
	("PW", "🇵🇼 Palau"),
	("PA", "🇵🇦 Panamá"),
	("PG", "🇵🇬 Papua Niugini (Papua New Guinea)"),
	("PY", "🇵🇾 Paraguai (Paraguay)"),
	("PE", "🇵🇪 Perú"),
	("PH", "🇵🇭 Philippines"),
	("PN", "🇵🇳 Pitcairn"),
	("PL", "🇵🇱 Polska (Poland)"),
	("PF", "🇵🇫 Polynésie française (French Polynesia)"),
	("PT", "🇵🇹 Portugal"),
	("PR", "🇵🇷 Puerto Rico"),
	("DO", "🇩🇴 República Dominicana (Dominican Republic)"),
	("MD", "🇲🇩 Republica Moldova (Moldova)"),
	("RO", "🇷🇴 România"),
	("SH", "🇸🇭 Saint Helena"),
	("KN", "🇰🇳 Saint Kitts and Nevis"),
	("LC", "🇱🇨 Saint Lucia"),
	("VC", "🇻🇨 Saint Vincent and the Grenadines"),
	("BL", "🇧🇱 Saint-Barthélemy"),
	("MF", "🇲🇫 Saint-Martin"),
	("PM", "🇵🇲 Saint-Pierre-et-Miquelon (Saint Pierre and Miquelon)"),
	("WS", "🇼🇸 Samoa"),
	("SM", "🇸🇲 San Marino"),
	("ST", "🇸🇹 São Tomé e Príncipe (Sao Tome and Principe)"),
	("CH", "🇨🇭 Schweiz (Switzerland)"),
	("SN", "🇸🇳 Senegaal (Senegal)"),
	("SC", "🇸🇨 Seychelles"),
	("AL", "🇦🇱 Shqipëri (Albania)"),
	("SL", "🇸🇱 Sierra Leone"),
	("SG", "🇸🇬 Singapore"),
	("SX", "🇸🇽 Sint Maarten"),
	("SI", "🇸🇮 Slovenija (Slovenia)"),
	("SK", "🇸🇰 Slovensko (Slovakia)"),
	("SB", "🇸🇧 Solomon Islands"),
	("SO", "🇸🇴 Soomaaliya (Somalia)"),
	("ZA", "🇿🇦 South Africa"),
	("GS", "🇬🇸 South Georgia and the South Sandwich Islands"),
	("SS", "🇸🇸 South Sudan"),
	("RS", "🇷🇸 Srbija (Serbia)"),
	("SD", "🇸🇩 Sudan"),
	("FI", "🇫🇮 Suomi (Finland)"),
	("SR", "🇸🇷 Suriname"),
	("SJ", "🇸🇯 Svalbard og Jan Mayen (Svalbard and Jan Mayen)"),
	("SE", "🇸🇪 Sverige (Sweden)"),
	("TZ", "🇹🇿 Tanzania"),
	("TL", "🇹🇱 Timor-Leste"),
	("TG", "🇹🇬 Togo"),
	("TK", "🇹🇰 Tokelau"),
	("TO", "🇹🇴 Tonga"),
	("TT", "🇹🇹 Trinidad and Tobago"),
	("TR", "🇹🇷 Türkiye"),
	("TM", "🇹🇲 Türkmenistan"),
	("TC", "🇹🇨 Turks and Caicos Islands"),
	("TV", "🇹🇻 Tuvalu"),
	("RW", "🇷🇼 U Rwanda (Rwanda)"),
	("UM", "🇺🇲 U.S. Outlying Islands"),
	("VI", "🇻🇮 U.S. Virgin Islands"),
	("BI", "🇧🇮 Uburundi (Burundi)"),
	("UG", "🇺🇬 Uganda"),
	("GB", "🇬🇧 United Kingdom"),
	("US", "🇺🇸 United States"),
	("UY", "🇺🇾 Uruguay"),
	("VU", "🇻🇺 Vanuatu"),
	("VE", "🇻🇪 Venezuela"),
	("VN", "🇻🇳 Việt Nam"),
	("WF", "🇼🇫 Wallis-et-Futuna (Wallis and Futuna)"),
	("ZM", "🇿🇲 Zambia"),
	("ZW", "🇿🇼 Zimbabwe"),
	("GR", "🇬🇷 Ελλάδα (Greece)"),
	("CY", "🇨🇾 Κύπρος (Cyprus)"),
	("BY", "🇧🇾 Беларусь (Belarus)"),
	("BA", "🇧🇦 Босна и Херцеговина (Bosnia and Herzegovina)"),
	("BG", "🇧🇬 България (Bulgaria)"),
	("KZ", "🇰🇿 Казахстан (Kazakhstan)"),
	("KG", "🇰🇬 Кыргызстан (Kyrgyzstan)"),
	("MN", "🇲🇳 Монгол (Mongolia)"),
	("RU", "🇷🇺 Россия (Russia)"),
	("MK", "🇲🇰 Северна Македонија (North Macedonia)"),
	("TJ", "🇹🇯 Тоҷикистон (Tajikistan)"),
	("UA", "🇺🇦 Україна (Ukraine)"),
	("AM", "🇦🇲 Հայաստան (Armenia)"),
	("IL", "🇮🇱 ישראל (Israel)"),
	("AF", "🇦🇫 افغانستان (Afghanistan)"),
	("PS", "🇵🇸 الأراضي الفلسطينية (Palestine)"),
	("JO", "🇯🇴 الأردن (Jordan)"),
	("AE", "🇦🇪 الإمارات العربية المتحدة (United Arab Emirates)"),
	("BH", "🇧🇭 البحرين (Bahrain)"),
	("DZ", "🇩🇿 الجزائر (Algeria)"),
	("EH", "🇪🇭 الصحراء الغربية (Western Sahara)"),
	("IQ", "🇮🇶 العراق (Iraq)"),
	("KW", "🇰🇼 الكويت (Kuwait)"),
	("MA", "🇲🇦 المغرب (Morocco)"),
	("SA", "🇸🇦 المملكة العربية السعودية (Saudi Arabia)"),
	("YE", "🇾🇪 اليمن (Yemen)"),
	("IR", "🇮🇷 ایران (Iran)"),
	("TD", "🇹🇩 تشاد (Chad)"),
	("TN", "🇹🇳 تونس (Tunisia)"),
	("KM", "🇰🇲 جزر القمر (Comoros)"),
	("SY", "🇸🇾 سوريا (Syria)"),
	("OM", "🇴🇲 عُمان (Oman)"),
	("QA", "🇶🇦 قطر (Qatar)"),
	("LB", "🇱🇧 لبنان (Lebanon)"),
	("LY", "🇱🇾 ليبيا (Libya)"),
	("EG", "🇪🇬 مصر (Egypt)"),
	("MR", "🇲🇷 موريتانيا (Mauritania)"),
	("PK", "🇵🇰 پاکستان (Pakistan)"),
	("MV", "🇲🇻 ދިވެހި ރާއްޖެ (Maldives)"),
	("NP", "🇳🇵 नेपाल (Nepal)"),
	("IN", "🇮🇳 भारत (India)"),
	("BD", "🇧🇩 বাংলাদেশ (Bangladesh)"),
	("LK", "🇱🇰 ශ්‍රී ලංකාව (Sri Lanka)"),
	("TH", "🇹🇭 ไทย (Thailand)"),
	("LA", "🇱🇦 ລາວ (Laos)"),
	("BT", "🇧🇹 འབྲུག (Bhutan)"),
	("MM", "🇲🇲 မြန်မာ (Myanmar)"),
	("GE", "🇬🇪 საქართველო (Georgia)"),
	("KR", "🇰🇷 대한민국 (South Korea)"),
	("KP", "🇰🇵 조선민주주의인민공화국 (North Korea)"),
	("ET", "🇪🇹 ኢትዮጵያ (Ethiopia)"),
	("ER", "🇪🇷 ኤርትራ (Eritrea)"),
	("KH", "🇰🇭 កម្ពុជា (Cambodia)"),
	("CN", "🇨🇳 中国 (China)"),
	("TW", "🇹🇼 台灣 (Taiwan)"),
	("JP", "🇯🇵 日本 (Japan)"),
	("MO", "🇲🇴 澳門 (Macao)"),
	("HK", "🇭🇰 香港 (Hong Kong)"),
];

// FUNCTIONS

/// Retrieve cookies from request "Cookie" header
pub async fn get(req: Request<Body>) -> Result<Response<Body>, String> {
	let url = req.uri().to_string();
	let form = url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes()).collect::<HashMap<_, _>>();
	Ok(template(&SettingsTemplate {
		prefs: Preferences::new(&req),
		url,
		return_url: match form.get("return_to") {
			Some(url) => url.clone().to_string(),
			None => String::new(),
		},
	}))
}

/// Set cookies using response "Set-Cookie" header
pub async fn set(req: Request<Body>) -> Result<Response<Body>, String> {
	// Split the body into parts
	let (parts, body) = req.into_parts();

	// Grab existing cookies
	let _cookies: Vec<Cookie<'_>> = parts
		.headers
		.get_all("Cookie")
		.iter()
		.filter_map(|header| Cookie::parse(header.to_str().unwrap_or_default()).ok())
		.collect();

	// Aggregate the body...
	let body_bytes = body.collect_bytes().await.map_err(|e| e.to_string())?;

	let form = url::form_urlencoded::parse(&body_bytes).collect::<HashMap<_, _>>();

	let mut response = match form.get("return_to") {
		Some(value) => {
			if value.is_empty() {
				redirect("/settings")
			} else {
				redirect(value)
			}
		}
		None => redirect("/settings"),
	};

	for &name in &PREFS {
		match form.get(name) {
			Some(value) => response.insert_cookie(
				Cookie::build((name.to_owned(), value.clone()))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			),
			None => response.remove_cookie(name.to_string()),
		};
	}

	Ok(response)
}

fn set_cookies_method(req: Request<Body>, remove_cookies: bool) -> Response<Body> {
	// Split the body into parts
	let (parts, _) = req.into_parts();

	// Grab existing cookies
	let _cookies: Vec<Cookie<'_>> = parts
		.headers
		.get_all("Cookie")
		.iter()
		.filter_map(|header| Cookie::parse(header.to_str().unwrap_or_default()).ok())
		.collect();

	let query = parts.uri.query().unwrap_or_default().as_bytes();

	let form = url::form_urlencoded::parse(query).collect::<HashMap<_, _>>();

	let path = match form.get("redirect") {
		Some(value) => {
			let value = value.replace("%26", "&").replace("%23", "#");
			if value.starts_with('/') {
				value
			} else {
				format!("/{value}")
			}
		}
		None => "/".to_string(),
	};

	let mut response = redirect(&path);

	for name in PREFS {
		match form.get(name) {
			Some(value) => response.insert_cookie(
				Cookie::build((name.to_owned(), value.clone()))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			),
			None => {
				if remove_cookies {
					response.remove_cookie(name.to_string());
				}
			}
		};
	}

	// Get subscriptions/filters to restore from query string
	let subscriptions = form.get("subscriptions");
	let filters = form.get("filters");

	// We can't search through the cookies directly like in subreddit.rs, so instead we have to make a string out of the request's headers to search through
	let cookies_string = parts
		.headers
		.get("cookie")
		.map(|hv| hv.to_str().unwrap_or("").to_string()) // Return String
		.unwrap_or_else(String::new); // Return an empty string if None

	// If there are subscriptions to restore set them and delete any old subscriptions cookies, otherwise delete them all
	if let Some(subscriptions) = subscriptions {
		let sub_list: Vec<String> = subscriptions.split('+').map(str::to_string).collect();

		// Start at 0 to keep track of what number we need to start deleting old subscription cookies from
		let mut subscriptions_number_to_delete_from = 0;

		// Starting at 0 so we handle the subscription cookie without a number first
		for (subscriptions_number, list) in join_until_size_limit(&sub_list).into_iter().enumerate() {
			let subscriptions_cookie = if subscriptions_number == 0 {
				"subscriptions".to_string()
			} else {
				format!("subscriptions{subscriptions_number}")
			};

			response.insert_cookie(
				Cookie::build((subscriptions_cookie, list))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			);

			subscriptions_number_to_delete_from += 1;
		}

		// While subscriptionsNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("subscriptions{subscriptions_number_to_delete_from}=")) {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number_to_delete_from}"));

			// Increment subscriptions cookie number
			subscriptions_number_to_delete_from += 1;
		}
	} else if remove_cookies {
		// Remove unnumbered subscriptions cookie
		response.remove_cookie("subscriptions".to_string());

		// Starts at one to deal with the first numbered subscription cookie and onwards
		let mut subscriptions_number_to_delete_from = 1;

		// While subscriptionsNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("subscriptions{subscriptions_number_to_delete_from}=")) {
			// Remove that subscriptions cookie
			response.remove_cookie(format!("subscriptions{subscriptions_number_to_delete_from}"));

			// Increment subscriptions cookie number
			subscriptions_number_to_delete_from += 1;
		}
	}

	// If there are filters to restore set them and delete any old filters cookies, otherwise delete them all
	if let Some(filters) = filters {
		let filters_list: Vec<String> = filters.split('+').map(str::to_string).collect();

		// Start at 0 to keep track of what number we need to start deleting old subscription cookies from
		let mut filters_number_to_delete_from = 0;

		// Starting at 0 so we handle the subscription cookie without a number first
		for (filters_number, list) in join_until_size_limit(&filters_list).into_iter().enumerate() {
			let filters_cookie = if filters_number == 0 {
				"filters".to_string()
			} else {
				format!("filters{filters_number}")
			};

			response.insert_cookie(
				Cookie::build((filters_cookie, list))
					.path("/")
					.http_only(true)
					.expires(OffsetDateTime::now_utc() + Duration::weeks(52))
					.into(),
			);

			filters_number_to_delete_from += 1;
		}

		// While filtersNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("filters{filters_number_to_delete_from}=")) {
			// Remove that filters cookie
			response.remove_cookie(format!("filters{filters_number_to_delete_from}"));

			// Increment filters cookie number
			filters_number_to_delete_from += 1;
		}
	} else if remove_cookies {
		// Remove unnumbered filters cookie
		response.remove_cookie("filters".to_string());

		// Starts at one to deal with the first numbered subscription cookie and onwards
		let mut filters_number_to_delete_from = 1;

		// While filtersNUMBER= is in the string of cookies add a response removing that cookie
		while cookies_string.contains(&format!("filters{filters_number_to_delete_from}=")) {
			// Remove that sfilters cookie
			response.remove_cookie(format!("filters{filters_number_to_delete_from}"));

			// Increment filters cookie number
			filters_number_to_delete_from += 1;
		}
	}

	response
}

/// Set cookies using response "Set-Cookie" header
pub async fn restore(req: Request<Body>) -> Result<Response<Body>, String> {
	Ok(set_cookies_method(req, true))
}

pub async fn update(req: Request<Body>) -> Result<Response<Body>, String> {
	Ok(set_cookies_method(req, false))
}

pub async fn encoded_restore(req: Request<Body>) -> Result<Response<Body>, String> {
	let body = req.into_body().collect_bytes().await.map_err(|e| format!("Failed to get bytes from request body: {e}"))?;

	if body.len() > 1024 * 1024 {
		return Err("Request body too large".to_string());
	}

	let encoded_prefs = form_urlencoded::parse(&body)
		.find(|(key, _)| key == "encoded_prefs")
		.map(|(_, value)| value)
		.ok_or_else(|| "encoded_prefs parameter not found in request body".to_string())?;

	let bytes = base2048::decode(&encoded_prefs).ok_or_else(|| "Failed to decode base2048 encoded preferences".to_string())?;

	let out = timeout(std::time::Duration::from_secs(1), async { deflate_decompress(bytes) })
		.await
		.map_err(|e| format!("Failed to decompress bytes: {e}"))??;

	let mut prefs: Preferences = timeout(std::time::Duration::from_secs(1), async { Preferences::from_bincode(&out) })
		.await
		.map_err(|e| format!("Failed to deserialize preferences: {e}"))?
		.map_err(|e| format!("Failed to deserialize bytes into Preferences struct: {e}"))?;

	prefs.available_themes = vec![];

	let url = format!("/settings/restore/?{}", prefs.to_urlencoded()?);

	Ok(redirect(&url))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A raw `+` in the link would come back as a space, a raw `&` would end the address.
	#[test]
	fn test_settings_link_encodes_the_page_it_returns_to() {
		let html = SettingsTemplate {
			prefs: Preferences::new(&Request::new(Body::from(""))),
			url: "/r/a+b/new?after=x&t=day".to_string(),
			return_url: String::new(),
		}
		.render()
		.unwrap();
		assert!(html.contains(r#"href="/settings?return_to=/r/a%2Bb/new%3Fafter%3Dx%26t%3Dday""#));
	}
}
