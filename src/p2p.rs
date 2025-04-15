use iroh::{protocol::Router, Endpoint};
use iroh_gossip::{net::Gossip, ALPN as GOSSIP_ALPN};

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let endpoint = Endpoint::builder().discovery_n0().bind().await?;
	println!("[P2P] Endpoint node ID: {}", endpoint.node_id());
	let builder = Router::builder(endpoint);
	let gossip = Gossip::builder().spawn(builder.endpoint().clone()).await?;
	let _router: Router = builder.accept(GOSSIP_ALPN, gossip).spawn().await?;
	Ok(())
}
