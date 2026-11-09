//! Chat between PCs on the same network, without a server. No herdr or terminal code lives here,
//! so the herdr plugin and a later desktop app can share it.
//!
//! - Every instance broadcasts its name and TCP port over UDP every few seconds.
//! - Other instances on the same network hear the broadcast and remember the peer.
//! - A message is sent straight to each peer over TCP. Nothing is stored.

pub mod chat;
pub mod i18n;
pub mod net;
pub mod phone;
pub mod relay;
pub mod relay_key;
pub mod relay_rest;
pub mod store;
pub mod text;

pub use chat::{help_box, Chat, Line, Link, Me, Peer, IDLE_LIMIT};
pub use i18n::t;
pub use net::{start, udp_port, DEFAULT_UDP_PORT};
