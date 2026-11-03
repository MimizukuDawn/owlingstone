//! Owlingstone: a standalone Hearthstone Battlegrounds helper that reads the game's Power.log.

pub mod advice;
pub mod background;
pub mod cards;
pub mod combat;
pub mod comps;
pub mod knowledge;
pub mod rules;
pub mod data;
pub mod errlog;
pub mod game;
pub mod history;
pub mod hslogs;
pub mod net;
pub mod paths;
pub mod pool;
pub mod planner;
pub mod powerlog;
pub mod rng;
pub mod season;
pub mod settings;
pub mod share;
pub mod snapshot;
pub mod trinkets;
pub mod update;
pub mod watcher;

/// The app's version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
