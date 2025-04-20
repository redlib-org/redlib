use std::{str::FromStr, time::SystemTime};

use bytes::Bytes;
use ed25519_dalek::Signature;
use futures_lite::StreamExt;
use iroh::{protocol::Router, Endpoint, NodeAddr, PublicKey};
use iroh_gossip::{
	net::{Event, Gossip, GossipEvent},
	proto::TopicId,
	ALPN as GOSSIP_ALPN,
};
use serde::{Deserialize, Serialize};

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let endpoint = Endpoint::builder().discovery_n0().bind().await?;
	let builder = Router::builder(endpoint.clone());
	let gossip = Gossip::builder().spawn(builder.endpoint().clone()).await?;
	let _router: Router = builder.accept(GOSSIP_ALPN, gossip.clone()).spawn().await?;

	let ticket_str = std::env::var("REDLIB_P2P_TICKET").expect("REDLIB_P2P_TICKET not set");
	let Ticket { topic, peers } = Ticket::from_str(&ticket_str)?;

	let ticket = {
		let me = endpoint.node_addr().await?;
		let peers = peers.iter().cloned().chain([me]).collect();
		Ticket { topic, peers }
	};
	eprintln!("> ticket to join us: {ticket}");

	let peer_ids = peers.iter().map(|p| p.node_id).collect();
	if peers.is_empty() {
		eprintln!("> waiting for peers to join us...");
	} else {
		eprintln!("> trying to connect to {} peers...", peers.len());
		// add the peer addrs from the ticket to our endpoint's addressbook so that they can be dialed
		for peer in peers.into_iter() {
			let result = endpoint.add_node_addr(peer);
			if let Err(e) = result {
				println!("> failed to add peer: {e}");
			}
		}
	};
	let (_sender, mut receiver) = gossip.subscribe_and_join(topic, peer_ids).await?.split();
	eprintln!("> connected!");
	loop {
		match receiver.try_next().await {
			Ok(Some(event)) => {
				eprintln!("received event!: {event:?}");
				if let Event::Gossip(GossipEvent::Received(msg)) = event {
					let (_from, message) = match SignedMessage::verify_and_decode(&msg.content) {
						Ok(v) => v,
						Err(e) => {
							eprintln!("> failed to verify message: {}", e);
							continue;
						}
					};
					// Log the message log
					let message_log: MessageLog = message.into();
					println!("{}", serde_json::to_string(&message_log).unwrap());
				}
			}
			Ok(None) => continue,
			Err(e) => {
				eprintln!("> failed to receive: {e}");
				continue;
			}
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct Ticket {
	topic: TopicId,
	peers: Vec<NodeAddr>,
}

impl Ticket {
	/// Deserializes from bytes.
	fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
		postcard::from_bytes(bytes).map_err(Into::into)
	}
	/// Serializes to bytes.
	pub fn to_bytes(&self) -> Vec<u8> {
		postcard::to_stdvec(self).expect("postcard::to_stdvec is infallible")
	}
}

impl FromStr for Ticket {
	type Err = anyhow::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let bytes = data_encoding::BASE32_NOPAD.decode(s.to_ascii_uppercase().as_bytes())?;
		Self::from_bytes(&bytes)
	}
}
impl std::fmt::Display for Ticket {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let mut text = data_encoding::BASE32_NOPAD.encode(&self.to_bytes()[..]);
		text.make_ascii_lowercase();
		write!(f, "{}", text)
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct SignedMessage {
	from: PublicKey,
	data: Bytes,
	signature: Signature,
}

impl SignedMessage {
	pub fn verify_and_decode(bytes: &[u8]) -> anyhow::Result<(PublicKey, MessageLog)> {
		let signed_message: Self = postcard::from_bytes(bytes)?;
		let key: PublicKey = signed_message.from;
		key.verify(&signed_message.data, &signed_message.signature)?;
		let message: MessageLog = postcard::from_bytes(&signed_message.data)?;
		Ok((signed_message.from, message))
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
	pub hostname: String,
	pub online: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageLog {
	pub timestamp: u64,
	pub message: Message,
}
impl From<Message> for MessageLog {
	fn from(message: Message) -> Self {
		let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
		Self { timestamp, message }
	}
}
