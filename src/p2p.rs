use std::{
	str::FromStr,
	sync::atomic::{AtomicBool, Ordering},
};

use bytes::Bytes;
use dashmap::DashMap;
use ed25519_dalek::Signature;
use futures_lite::StreamExt;
use iroh::{protocol::Router, Endpoint, NodeAddr, PublicKey, SecretKey};
use iroh_gossip::{
	net::{Event, Gossip, GossipEvent, GossipReceiver, GossipSender},
	proto::TopicId,
	ALPN as GOSSIP_ALPN,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::{task, time::sleep};

use crate::config;

pub static DASHMAP: Lazy<DashMap<String, bool>> = Lazy::new(DashMap::new);
pub static ONLINE: Lazy<AtomicBool> = Lazy::new(AtomicBool::default);

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let endpoint = Endpoint::builder().discovery_n0().bind().await?;
	println!("[P2P] Endpoint node ID: {}", endpoint.node_id());
	let builder = Router::builder(endpoint.clone());
	let gossip = Gossip::builder().spawn(builder.endpoint().clone()).await?;
	let _router: Router = builder.accept(GOSSIP_ALPN, gossip.clone()).spawn().await?;

	// there are two ways to run the p2p chat
	// 1. "bootstrap" mode - this requires REDLIB_P2P_BOOTSTRAP=true and REDLIB_P2P_TOPIC set to the topic ID you want to use
	//    in this mode, the node will create the topic and start a chat room
	// 2. "join" mode - this requires REDLIB_P2P_BOOTSTRAP=false (or unset) and REDLIB_P2P_TICKET set to a ticket.
	//    in this mode, the node will join the existing chat room with the given topic ID, and connect to the peers listed
	//    in the ticket
	let (topic, peers) = if std::env::var("REDLIB_P2P_BOOTSTRAP").unwrap_or_default() == "true" {
		let topic = std::env::var("REDLIB_P2P_TOPIC").map(|s| TopicId::from_str(&s).unwrap()).unwrap();
		println!("> opening chat room for topic {topic}");
		(topic, vec![])
	} else {
		let ticket_str = std::env::var("REDLIB_P2P_TICKET").expect("REDLIB_P2P_TICKET not set");
		let Ticket { topic, peers } = Ticket::from_str(&ticket_str)?;
		println!("> joining chat room for topic {topic}");
		(topic, peers)
	};

	let ticket = {
		let me = endpoint.node_addr().await?;
		let peers = peers.iter().cloned().chain([me]).collect();
		Ticket { topic, peers }
	};
	println!("> ticket to join us: {ticket}");

	let peer_ids = peers.iter().map(|p| p.node_id).collect();
	if peers.is_empty() {
		println!("> waiting for peers to join us...");
	} else {
		println!("> trying to connect to {} peers...", peers.len());
		// add the peer addrs from the ticket to our endpoint's addressbook so that they can be dialed
		for peer in peers.into_iter() {
			endpoint.add_node_addr(peer)?;
		}
	};
	let (sender, receiver) = gossip.subscribe_and_join(topic, peer_ids).await?.split();
	println!("> connected!");

	let secret_key = endpoint.secret_key().clone();

	let message = Message {
		hostname: config::get_setting("REDLIB_FULL_URL").unwrap_or_default(),
		online: true,
	};
	let encoded_message = SignedMessage::sign_and_encode(&secret_key, &message)?;
	sender.broadcast(encoded_message).await?;

	task::spawn(subscribe_loop(receiver));

	task::spawn(sender_loop(sender, secret_key));

	Ok(())
}

async fn subscribe_loop(mut receiver: GossipReceiver) {
	while let Ok(Some(event)) = receiver.try_next().await {
		if let Event::Gossip(GossipEvent::Received(msg)) = event {
			let (_from, message) = match SignedMessage::verify_and_decode(&msg.content) {
				Ok(v) => v,
				Err(e) => {
					println!("> failed to verify message: {}", e);
					break;
				}
			};
			// Update dashmap with message's hostname and alive status
			DASHMAP.insert(message.hostname.clone(), message.online);
		}
	}
}

async fn sender_loop(sender: GossipSender, secret_key: SecretKey) {
	loop {
		let message = Message {
			hostname: config::get_setting("REDLIB_FULL_URL").unwrap_or_default(),
			online: ONLINE.load(Ordering::SeqCst),
		};
		let encoded_message = SignedMessage::sign_and_encode(&secret_key, &message).unwrap();
		let _ = sender.broadcast(encoded_message).await;

		sleep(std::time::Duration::from_secs(10)).await;
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
	pub fn verify_and_decode(bytes: &[u8]) -> anyhow::Result<(PublicKey, Message)> {
		let signed_message: Self = postcard::from_bytes(bytes)?;
		let key: PublicKey = signed_message.from;
		key.verify(&signed_message.data, &signed_message.signature)?;
		let message: Message = postcard::from_bytes(&signed_message.data)?;
		Ok((signed_message.from, message))
	}

	pub fn sign_and_encode(secret_key: &SecretKey, message: &Message) -> anyhow::Result<Bytes> {
		let data: Bytes = postcard::to_stdvec(&message)?.into();
		let signature = secret_key.sign(&data);
		let from: PublicKey = secret_key.public();
		let signed_message = Self { from, data, signature };
		let encoded = postcard::to_stdvec(&signed_message)?;
		Ok(encoded.into())
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
	hostname: String,
	online: bool,
}
