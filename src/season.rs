//! Season facts from data/season.yaml: tribes per lobby and tribes rotated out of the pool.

use std::sync::Arc;

use serde::Deserialize;

use crate::data;

#[derive(Debug, Clone, Deserialize)]
pub struct Season {
    #[serde(default = "five")]
    pub lobby_tribes: usize,
    #[serde(default)]
    pub rotated_out: Vec<String>,
}

fn five() -> usize {
    5
}

impl Default for Season {
    fn default() -> Self {
        Season { lobby_tribes: 5, rotated_out: vec!["NAGA".into()] }
    }
}

/// The season facts (reloaded when data/season.yaml changes).
pub fn get() -> Arc<Season> {
    data::season()
}
