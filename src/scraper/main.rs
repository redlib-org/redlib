use std::{collections::HashMap, fmt::Display, io::Write};

use clap::{Parser, ValueEnum};
use common_words_all::{get_top, Language, NgramSize};
use redlib::utils::Post;

#[derive(Parser)]
#[command(name = "my_cli")]
#[command(about = "A simple CLI example", long_about = None)]
struct Cli {
	#[arg(short = 's', long = "sub")]
	sub: String,

	#[arg(long = "sort")]
	sort: SortOrder,

	#[arg(short = 'f', long = "format", value_enum)]
	format: Format,
	#[arg(short = 'o', long = "output")]
	output: Option<String>,
}

#[derive(Debug, Clone, ValueEnum)]
enum SortOrder {
	Hot,
	Rising,
	New,
	Top,
	Controversial,
}

impl Display for SortOrder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SortOrder::Hot => write!(f, "hot"),
			SortOrder::Rising => write!(f, "rising"),
			SortOrder::New => write!(f, "new"),
			SortOrder::Top => write!(f, "top"),
			SortOrder::Controversial => write!(f, "controversial"),
		}
	}
}

#[derive(Debug, Clone, ValueEnum)]
enum Format {
	Json,
}

#[tokio::main]
async fn main() {
	pretty_env_logger::init();
	let cli = Cli::parse();
	let (sub, sort, format, output) = (cli.sub, cli.sort, cli.format, cli.output);
	let initial = format!("/r/{sub}/{sort}.json?&raw_json=1");
	let (posts, mut after) = Post::fetch(&initial, false).await.unwrap();
	let mut hashmap = HashMap::new();
	hashmap.extend(posts.into_iter().map(|post| (post.id.clone(), post)));
	loop {
		print!("\r");
		let path = format!("/r/{sub}/{sort}.json?sort={sort}&t=&after={after}&raw_json=1");
		let (new_posts, new_after) = Post::fetch(&path, false).await.unwrap();
		let old_len = hashmap.len();
		// convert to hashmap and extend hashmap
		let new_posts = new_posts.into_iter().map(|post| (post.id.clone(), post)).collect::<HashMap<String, Post>>();
		let len = new_posts.len();
		hashmap.extend(new_posts);
		if hashmap.len() - old_len < 3 {
			break;
		}

		let x = hashmap.len() - old_len;
		after = new_after;
		// Print number of posts fetched
		print!("Fetched {len} posts (+{x})",);
		std::io::stdout().flush().unwrap();
	}
	println!("\n\n");
	// additionally search if final count not reached

	for word in get_top(Language::English, 10_000, NgramSize::One) {
		let mut retrieved_posts_from_search = 0;
		let initial = format!("/r/{sub}/search.json?q={word}&restrict_sr=on&include_over_18=on&raw_json=1&sort={sort}");
		println!("Grabbing posts with word {word}.");
		let (posts, mut after) = Post::fetch(&initial, false).await.unwrap();
		hashmap.extend(posts.into_iter().map(|post| (post.id.clone(), post)));
		'search: loop {
			let path = format!("/r/{sub}/search.json?q={word}&restrict_sr=on&include_over_18=on&raw_json=1&sort={sort}&after={after}");
			let (new_posts, new_after) = Post::fetch(&path, false).await.unwrap();
			if new_posts.is_empty() || new_after.is_empty() {
				println!("No more posts for word {word}");
				break 'search;
			}
			retrieved_posts_from_search += new_posts.len();
			let old_len = hashmap.len();
			let new_posts = new_posts.into_iter().map(|post| (post.id.clone(), post)).collect::<HashMap<String, Post>>();
			let len = new_posts.len();
			hashmap.extend(new_posts);
			let delta = hashmap.len() - old_len;
			after = new_after;
			// Print number of posts fetched
			println!("Fetched {len} posts (+{delta})",);

			if retrieved_posts_from_search > 1000 {
				println!("Reached 1000 posts from search");
				break 'search;
			}
		}
		// Need to save incrementally. atomic save + move
		let tmp_file = output.clone().unwrap_or_else(|| format!("{sub}.json.tmp"));
		let perm_file = output.clone().unwrap_or_else(|| format!("{sub}.json"));
		write_posts(&hashmap.values().collect(), tmp_file.clone());
		// move file
		std::fs::rename(tmp_file, perm_file).unwrap();
	}

	println!("\n\n");

	println!("Size of hashmap: {}", hashmap.len());

	let posts: Vec<&Post> = hashmap.values().collect();
	match format {
		Format::Json => {
			let filename: String = output.unwrap_or_else(|| format!("{sub}.json"));
			write_posts(&posts, filename);
		}
	}
}

fn write_posts(posts: &Vec<&Post>, filename: String) {
	let json = serde_json::to_string(&posts).unwrap();
	std::fs::write(filename, json).unwrap();
}
