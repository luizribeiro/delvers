use serde::{Deserialize, Serialize};

pub const MAP_W: usize = 80;
pub const MAP_H: usize = 30;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Dir {
    pub fn delta(&self) -> (i32, i32) {
        match self {
            Dir::N => (0, -1),
            Dir::S => (0, 1),
            Dir::E => (1, 0),
            Dir::W => (-1, 0),
            Dir::NE => (1, -1),
            Dir::NW => (-1, -1),
            Dir::SE => (1, 1),
            Dir::SW => (-1, 1),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Tile {
    Void = 0,
    Wall = 1,
    Floor = 2,
    Door = 3,
    Corridor = 4,
    StairsDown = 5,
    StairsUp = 6,
    Altar = 7,
    Tombstone = 8,
}

impl Tile {
    pub fn walkable(&self) -> bool {
        matches!(
            self,
            Tile::Floor
                | Tile::Door
                | Tile::Corridor
                | Tile::StairsDown
                | Tile::StairsUp
                | Tile::Altar
                | Tile::Tombstone
        )
    }
    pub fn glyph(&self) -> char {
        match self {
            Tile::Void => ' ',
            Tile::Wall => '#',
            Tile::Floor => '.',
            Tile::Door => '+',
            Tile::Corridor => '.',
            Tile::StairsDown => '>',
            Tile::StairsUp => '<',
            Tile::Altar => '_',
            Tile::Tombstone => '+',
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EntityView {
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub glyph: char,
    pub color: u8, // ANSI color index 0..15
    pub name: String,
    pub is_player: bool,
    pub is_self: bool,
    pub hp_frac: f32,      // 0..1
    #[serde(default)]
    pub bubble: Option<String>,
    #[serde(default)]
    pub invuln: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayerStats {
    pub name: String,
    pub hp: i32,
    pub max_hp: i32,
    pub attack: i32,
    pub defense: i32,
    pub level: u32,
    pub xp: u32,
    pub xp_next: u32,
    pub gold: u32,
    pub depth: u32,
    pub weapon: String,
    pub armor: String,
    pub potions: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RosterEntry {
    pub name: String,
    pub color: u8,
    pub depth: u32,
    pub level: u32,
    pub hp_frac: f32,
    pub alive: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldView {
    pub width: u16,
    pub height: u16,
    pub tiles: Vec<u8>,
    /// Per-tile visibility: 0 = unseen, 1 = remembered, 2 = visible now
    pub vis: Vec<u8>,
    pub entities: Vec<EntityView>,
    pub stats: PlayerStats,
    pub depth: u32,
    pub players_here: u32,
    pub alive: bool,
    pub sight_radius: u16,
    pub roster: Vec<RosterEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", content = "c")]
pub enum ClientMsg {
    Hello { name: String },
    Move(Dir),
    Wait,
    Pickup,
    Descend,
    Ascend,
    Quaff,
    Chat(String),
    Shout(String),
    Respawn,
    Rest,
    Quit,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", content = "c")]
pub enum ServerMsg {
    Welcome {
        player_id: u64,
        name: String,
        motd: String,
    },
    State(WorldView),
    Log {
        text: String,
        color: u8,
    },
    Chat {
        who: String,
        text: String,
        color: u8,
    },
    Death {
        by: String,
    },
    Victory {
        by: String,
    },
    Error(String),
}
