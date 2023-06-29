use std::collections::HashMap;

use serde::{Serialize,  Deserialize};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub name: String,
    pub players: Vec<Player>,
}

#[derive(Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Player {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Match {
    pub map: Map,
    pub team_won: Team,
    pub team_lost: Team,
    pub won_score: u32,
    pub lost_score: u32,
    pub agents: HashMap<Player, Agent>,
}

#[derive(Debug, Eq, Hash, PartialOrd, Ord, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Agent {
    pub name: String,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Map {
    pub name: String,
}