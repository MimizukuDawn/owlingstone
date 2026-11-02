//! Owlingstone: a standalone Hearthstone Battlegrounds helper that reads the game's Power.log.

pub mod cards;
pub mod combat;
pub mod data;
pub mod errlog;
pub mod game;
pub mod net;
pub mod paths;
pub mod powerlog;
pub mod rng;
pub mod season;

/// The app's version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
