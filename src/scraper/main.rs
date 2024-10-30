use std::{fmt::Display, io::Write};

use clap::{Parser, ValueEnum};
use redlib::utils::Post;

#[derive(Parser)]
#[command(name = "my_cli")]
#[command(about = "A simple CLI example", long_about = None)]
struct Cli {
	#[arg(short = 's', long = "sub")]
	sub: String,

	#[arg(short = 'c', long = "count")]
	count: usize,

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
	let cli = Cli::parse();
	let (sub, final_count, sort, format, output) = (cli.sub, cli.count, cli.sort, cli.format, cli.output);
	let initial = format!("/r/{sub}/{sort}.json?&raw_json=1");
	let (mut posts, mut after) = Post::fetch(&initial, false).await.unwrap();
	while posts.len() < final_count {
        print!("\r");
		let path = format!("/r/{sub}/{sort}.json?sort={sort}&t=&after={after}&raw_json=1");
		let (new_posts, new_after) = Post::fetch(&path, false).await.unwrap();
		posts.extend(new_posts);
		after = new_after;
		// Print number of posts fetched
		print!("Fetched {} posts", posts.len());
        std::io::stdout().flush().unwrap();
	}

	match format {
		Format::Json => {
			let filename: String = output.unwrap_or_else(|| format!("{sub}.json"));
			let json = serde_json::to_string(&posts).unwrap();
			std::fs::write(filename, json).unwrap();
		}
	}
}
