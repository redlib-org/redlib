//! Reorders each kind's backends at runtime by how well they have been doing.
//!
//! Every backend keeps a success rate per kind, weighted towards recent results
//! (half-life [`HALF_LIFE`]), and backends are tried by success rate per
//! [`Backend::cost`]. The configured order seeds each rate as [`PRIOR_WEIGHT`]
//! made-up results, so without data the order is the configured one. Real
//! results fade back to that seed, so a backend that failed gets tried again
//! once its failures are old.

use super::Backend;
use std::time::{Duration, Instant};

const HALF_LIFE: Duration = Duration::from_secs(600);
const PRIOR_WEIGHT: f64 = 4.0;
/// How much better a backend has to score to move ahead of the one before it,
/// so two backends with about the same score don't swap on every request.
const MARGIN: f64 = 1.1;

/// One request's result for the backend that served it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
	/// The page, or Reddit's real answer (not found, private, …).
	Success,
	/// Blocked, rate limited, challenged, or an unusable response.
	Failure,
	/// Timeouts and connection errors, which usually aren't the backend's fault.
	NetworkError,
}

impl Outcome {
	/// (successes, results) this outcome adds.
	fn counts(self) -> (f64, f64) {
		match self {
			Self::Success => (1.0, 1.0),
			Self::Failure => (0.0, 1.0),
			Self::NetworkError => (0.0, 0.5),
		}
	}
}

struct Stat {
	prior: f64,
	successes: f64,
	results: f64,
	updated: Instant,
}

impl Stat {
	fn decay(&self, now: Instant) -> f64 {
		0.5_f64.powf(now.saturating_duration_since(self.updated).as_secs_f64() / HALF_LIFE.as_secs_f64())
	}

	fn rate(&self, now: Instant) -> f64 {
		let decay = self.decay(now);
		(self.successes * decay + self.prior * PRIOR_WEIGHT) / (self.results * decay + PRIOR_WEIGHT)
	}

	fn record(&mut self, outcome: Outcome, now: Instant) {
		let decay = self.decay(now);
		let (successes, results) = outcome.counts();
		self.successes = self.successes * decay + successes;
		self.results = self.results * decay + results;
		self.updated = now;
	}
}

/// The backends of one kind of request, with their scores and current order.
pub struct Ranking {
	/// The configured order.
	backends: Vec<&'static dyn Backend>,
	stats: Vec<Stat>,
	/// Indexes into `backends`, best first.
	order: Vec<usize>,
}

impl Ranking {
	/// Starts every backend with a score that falls with its configured
	/// position. Scores stay at most 0.5 so a backend costing 2 still has a
	/// seed rate of at most 1.
	pub fn new(backends: Vec<&'static dyn Backend>, now: Instant) -> Self {
		let n = backends.len() as f64;
		let stats = backends
			.iter()
			.enumerate()
			.map(|(i, backend)| Stat {
				prior: (0.5 * (n - i as f64) / n * backend.cost()).min(1.0),
				successes: 0.0,
				results: 0.0,
				updated: now,
			})
			.collect();
		let order = (0..backends.len()).collect();
		Self { backends, stats, order }
	}

	pub fn configured(&self) -> &[&'static dyn Backend] {
		&self.backends
	}

	fn score(&self, i: usize, now: Instant) -> f64 {
		self.stats[i].rate(now) / self.backends[i].cost()
	}

	/// The backends to try, best first.
	pub fn order(&mut self, now: Instant) -> Vec<&'static dyn Backend> {
		for i in 1..self.order.len() {
			let mut j = i;
			while j > 0 && self.score(self.order[j], now) > self.score(self.order[j - 1], now) * MARGIN {
				self.order.swap(j, j - 1);
				j -= 1;
			}
		}
		self.order.iter().map(|&i| self.backends[i]).collect()
	}

	pub fn record(&mut self, backend: &str, outcome: Outcome, now: Instant) {
		if let Some(i) = self.backends.iter().position(|known| known.name() == backend) {
			self.stats[i].record(outcome, now);
		}
	}

	/// Each backend's name and score, in the current order.
	pub fn scores(&self, now: Instant) -> Vec<(&'static str, f64)> {
		self.order.iter().map(|&i| (self.backends[i].name(), self.score(i, now))).collect()
	}
}

#[cfg(test)]
mod tests {
	use super::super::Fetch;
	use super::*;

	struct Fake(&'static str, f64);

	impl Backend for Fake {
		fn name(&self) -> &'static str {
			self.0
		}

		fn cost(&self) -> f64 {
			self.1
		}

		fn fetch<'a>(&'a self, _path: &'a str, _quarantine: bool) -> Fetch<'a> {
			unreachable!()
		}
	}

	static GQL: Fake = Fake("gql", 1.0);
	static COOKIED: Fake = Fake("cookied-json", 2.0);
	static SCRAPED: Fake = Fake("scraped", 1.0);
	static JSON: Fake = Fake("json", 1.0);

	fn names(order: Vec<&dyn Backend>) -> Vec<&'static str> {
		order.iter().map(|backend| backend.name()).collect()
	}

	fn comments(now: Instant) -> Ranking {
		Ranking::new(vec![&GQL, &COOKIED, &SCRAPED, &JSON], now)
	}

	#[test]
	fn test_without_results_the_configured_order_stands() {
		let now = Instant::now();
		let mut ranking = comments(now);
		assert_eq!(names(ranking.order(now)), ["gql", "cookied-json", "scraped", "json"]);
		assert_eq!(names(ranking.order(now + HALF_LIFE * 10)), ["gql", "cookied-json", "scraped", "json"]);
	}

	#[test]
	fn test_one_failure_moves_nothing() {
		let now = Instant::now();
		let mut ranking = comments(now);
		ranking.record("gql", Outcome::Failure, now);
		assert_eq!(names(ranking.order(now)), ["gql", "cookied-json", "scraped", "json"]);
	}

	#[test]
	fn test_failing_backend_drops_and_comes_back_once_its_failures_are_old() {
		let now = Instant::now();
		let mut ranking = comments(now);
		for _ in 0..3 {
			ranking.record("gql", Outcome::Failure, now);
		}
		assert_eq!(names(ranking.order(now))[0], "cookied-json");
		assert_eq!(names(ranking.order(now + HALF_LIFE * 6))[0], "gql");
	}

	#[test]
	fn test_working_backend_moves_ahead_of_a_failing_one() {
		let now = Instant::now();
		let mut ranking = comments(now);
		for _ in 0..5 {
			ranking.record("gql", Outcome::Failure, now);
			ranking.record("cookied-json", Outcome::Failure, now);
			ranking.record("scraped", Outcome::Success, now);
		}
		assert_eq!(names(ranking.order(now))[0], "scraped");
	}

	#[test]
	fn test_cheaper_backend_wins_at_the_same_success_rate() {
		let now = Instant::now();
		let mut ranking = Ranking::new(vec![&COOKIED, &JSON], now);
		for _ in 0..20 {
			ranking.record("cookied-json", Outcome::Success, now);
			ranking.record("json", Outcome::Success, now);
		}
		assert_eq!(names(ranking.order(now)), ["json", "cookied-json"]);
	}

	#[test]
	fn test_close_scores_keep_their_order() {
		let now = Instant::now();
		let mut ranking = Ranking::new(vec![&GQL, &JSON], now);
		for _ in 0..20 {
			ranking.record("gql", Outcome::Success, now);
			ranking.record("json", Outcome::Success, now);
		}
		ranking.record("gql", Outcome::Failure, now);
		assert_eq!(names(ranking.order(now)), ["gql", "json"]);
	}

	#[test]
	fn test_network_errors_count_less_than_failures() {
		let now = Instant::now();
		let mut failed = Ranking::new(vec![&GQL], now);
		let mut timed_out = Ranking::new(vec![&GQL], now);
		failed.record("gql", Outcome::Failure, now);
		timed_out.record("gql", Outcome::NetworkError, now);
		assert!(timed_out.scores(now)[0].1 > failed.scores(now)[0].1);
	}
}
