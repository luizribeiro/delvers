use crate::dungeon::Dungeon;
use crate::entity::{ItemKind, MonsterKind, all_monsters, monster_spec};
use crate::protocol::{Dir, EntityView, PlayerStats, Tile, WorldView};
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::collections::HashMap;

pub const DEFAULT_CHAR_COLORS: [u8; 6] = [14, 11, 10, 13, 9, 12];

#[derive(Clone, Debug)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub color: u8,
    pub depth: u32,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub max_hp: i32,
    pub base_attack: i32,
    pub base_defense: i32,
    pub weapon: Option<ItemKind>,
    pub armor: Option<ItemKind>,
    pub potions: u32,
    pub gold: u32,
    pub gems: u32,
    pub xp: u32,
    pub level: u32,
    pub has_amulet: bool,
    pub alive: bool,
    pub death_timer: u32,
    pub connected: bool,
    pub pending_move: Option<Dir>,
    pub pending_action: Option<PlayerAction>,
    pub log: Vec<(String, u8)>, // color-coded
    pub last_damage_source: Option<String>,
    pub last_active_tick: u64,
}

#[derive(Clone, Debug)]
pub enum PlayerAction {
    Pickup,
    Descend,
    Ascend,
    Quaff,
    Respawn,
    Wait,
}

impl Player {
    pub fn new(id: u64, name: String, color: u8, start: (i32, i32)) -> Self {
        Player {
            id,
            name,
            color,
            depth: 1,
            x: start.0,
            y: start.1,
            hp: 30,
            max_hp: 30,
            base_attack: 3,
            base_defense: 0,
            weapon: Some(ItemKind::Dagger),
            armor: None,
            potions: 2,
            gold: 0,
            gems: 0,
            xp: 0,
            level: 1,
            has_amulet: false,
            alive: true,
            death_timer: 0,
            connected: true,
            pending_move: None,
            pending_action: None,
            log: Vec::new(),
            last_damage_source: None,
            last_active_tick: 0,
        }
    }

    pub fn xp_next(&self) -> u32 {
        10 * self.level * self.level
    }

    pub fn attack(&self) -> i32 {
        self.base_attack + self.weapon.and_then(|w| w.weapon_bonus()).unwrap_or(0)
    }

    pub fn defense(&self) -> i32 {
        self.base_defense + self.armor.and_then(|a| a.armor_bonus()).unwrap_or(0)
    }

    pub fn push_log(&mut self, text: impl Into<String>, color: u8) {
        self.log.push((text.into(), color));
        if self.log.len() > 200 {
            self.log.drain(0..100);
        }
    }

    pub fn stats(&self) -> PlayerStats {
        PlayerStats {
            name: self.name.clone(),
            hp: self.hp,
            max_hp: self.max_hp,
            attack: self.attack(),
            defense: self.defense(),
            level: self.level,
            xp: self.xp,
            xp_next: self.xp_next(),
            gold: self.gold,
            depth: self.depth,
            weapon: self
                .weapon
                .map(|w| w.name())
                .unwrap_or_else(|| "bare hands".into()),
            armor: self
                .armor
                .map(|a| a.name())
                .unwrap_or_else(|| "none".into()),
            potions: self.potions,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Monster {
    pub id: u64,
    pub kind: MonsterKind,
    pub depth: u32,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub max_hp: i32,
    pub tick_counter: u32,
    pub target: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub depth: u32,
    pub x: i32,
    pub y: i32,
    pub kind: ItemKind,
}

pub struct World {
    pub levels: HashMap<u32, Dungeon>,
    pub players: HashMap<u64, Player>,
    pub monsters: HashMap<u64, Monster>,
    pub items: HashMap<u64, Item>,
    pub next_id: u64,
    pub tick: u64,
    pub rng: StdRng,
    pub chat_log: Vec<(String, String, u8)>, // (who, text, color)
    pub global_log: Vec<(String, u8)>,
    pub base_seed: u64,
}

impl World {
    pub fn new() -> Self {
        let seed: u64 = rand::thread_rng().r#gen();
        let mut w = World {
            levels: HashMap::new(),
            players: HashMap::new(),
            monsters: HashMap::new(),
            items: HashMap::new(),
            next_id: 1,
            tick: 0,
            rng: StdRng::seed_from_u64(seed),
            chat_log: Vec::new(),
            global_log: Vec::new(),
            base_seed: seed,
        };
        w.ensure_level(1);
        w
    }

    pub fn gen_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn ensure_level(&mut self, depth: u32) {
        if self.levels.contains_key(&depth) {
            return;
        }
        let d = Dungeon::generate(depth, self.base_seed.wrapping_add(depth as u64 * 9973));
        self.levels.insert(depth, d);
        self.populate_level(depth);
    }

    fn populate_level(&mut self, depth: u32) {
        // Use a level-specific RNG so population is deterministic per seed+depth.
        let mut rng = StdRng::seed_from_u64(self.base_seed.wrapping_add(depth as u64 * 1_000_003));
        let dungeon = self.levels.get(&depth).unwrap().clone();
        // First room on every level is a "safe room" — no spawns there.
        let safe_room = dungeon.rooms.first().cloned();
        let in_safe_room = |x: i32, y: i32| -> bool {
            if let Some(r) = &safe_room {
                x >= r.x - 1 && x <= r.x + r.w && y >= r.y - 1 && y <= r.y + r.h
            } else {
                false
            }
        };
        // Monster count scales with depth
        let mcount = 5 + depth as usize * 2;
        let pool: Vec<MonsterKind> = all_monsters()
            .iter()
            .filter(|k| monster_spec(**k).min_depth <= depth)
            .copied()
            .collect();
        if !pool.is_empty() {
            for _ in 0..mcount {
                // Weighted selection
                let weights: Vec<u32> = pool.iter().map(|k| monster_spec(*k).rarity).collect();
                let total: u32 = weights.iter().sum();
                let mut pick = rng.gen_range(0..total);
                let mut chosen = pool[0];
                for (i, w) in weights.iter().enumerate() {
                    if pick < *w {
                        chosen = pool[i];
                        break;
                    }
                    pick -= *w;
                }
                let mut x;
                let mut y;
                let mut tries = 0;
                loop {
                    let (tx, ty) = dungeon.random_floor(&mut rng);
                    x = tx;
                    y = ty;
                    if !in_safe_room(x, y) || tries > 20 {
                        break;
                    }
                    tries += 1;
                }
                if in_safe_room(x, y) {
                    continue;
                }
                let spec = monster_spec(chosen);
                let id = self.gen_id();
                self.monsters.insert(
                    id,
                    Monster {
                        id,
                        kind: chosen,
                        depth,
                        x,
                        y,
                        hp: spec.hp,
                        max_hp: spec.hp,
                        tick_counter: 0,
                        target: None,
                    },
                );
            }
        }

        // Items — gold piles, potions, weapons, armor.
        let icount = 4 + depth as usize;
        for _ in 0..icount {
            let roll = rng.gen_range(0..100);
            let kind = if roll < 45 {
                ItemKind::Gold(rng.gen_range(5..=25 + depth as u32 * 5))
            } else if roll < 60 {
                ItemKind::Potion
            } else if roll < 75 {
                // weapon pool grows with depth
                let wpool: &[ItemKind] = if depth <= 2 {
                    &[ItemKind::Dagger, ItemKind::ShortSword]
                } else if depth <= 5 {
                    &[ItemKind::ShortSword, ItemKind::LongSword, ItemKind::BattleAxe]
                } else {
                    &[ItemKind::LongSword, ItemKind::BattleAxe, ItemKind::WarHammer]
                };
                *wpool.choose(&mut rng).unwrap()
            } else if roll < 88 {
                let apool: &[ItemKind] = if depth <= 2 {
                    &[ItemKind::LeatherArmor]
                } else if depth <= 5 {
                    &[ItemKind::LeatherArmor, ItemKind::ChainMail]
                } else {
                    &[ItemKind::ChainMail, ItemKind::PlateMail]
                };
                *apool.choose(&mut rng).unwrap()
            } else {
                ItemKind::Gem
            };
            let (x, y) = dungeon.random_floor(&mut rng);
            let id = self.gen_id();
            self.items.insert(
                id,
                Item {
                    id,
                    depth,
                    x,
                    y,
                    kind,
                },
            );
        }

        // Amulet of Yendor on depth 10.
        if depth == 10 {
            let (x, y) = dungeon.random_floor(&mut rng);
            let id = self.gen_id();
            self.items.insert(
                id,
                Item {
                    id,
                    depth,
                    x,
                    y,
                    kind: ItemKind::Amulet,
                },
            );
        }
    }

    pub fn spawn_player(&mut self, name: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let color = DEFAULT_CHAR_COLORS[(id as usize - 1) % DEFAULT_CHAR_COLORS.len()];
        let d = self.levels.get(&1).unwrap().clone();
        // Find an unoccupied tile in the first (safe) room.
        let room = d.rooms.first().cloned();
        let (sx, sy) = if let Some(r) = room {
            let mut pick = (r.center().0, r.center().1);
            'found: for dy in 0..r.h {
                for dx in 0..r.w {
                    let x = r.x + dx;
                    let y = r.y + dy;
                    if d.walkable(x, y) && self.player_at(1, x, y).is_none() {
                        pick = (x, y);
                        break 'found;
                    }
                }
            }
            pick
        } else {
            (1, 1)
        };
        let mut p = Player::new(id, name, color, (sx, sy));
        p.last_active_tick = self.tick;
        self.players.insert(id, p);
        id
    }

    pub fn remove_player(&mut self, id: u64) {
        self.players.remove(&id);
    }

    pub fn level(&self, depth: u32) -> &Dungeon {
        self.levels.get(&depth).unwrap()
    }

    pub fn level_mut(&mut self, depth: u32) -> &mut Dungeon {
        self.levels.get_mut(&depth).unwrap()
    }

    pub fn blocked(&self, depth: u32, x: i32, y: i32, ignore_id: Option<u64>) -> bool {
        let d = self.level(depth);
        if !d.walkable(x, y) {
            return true;
        }
        for p in self.players.values() {
            if Some(p.id) == ignore_id {
                continue;
            }
            if p.alive && p.depth == depth && p.x == x && p.y == y {
                return true;
            }
        }
        for m in self.monsters.values() {
            if Some(m.id) == ignore_id {
                continue;
            }
            if m.depth == depth && m.x == x && m.y == y {
                return true;
            }
        }
        false
    }

    pub fn monster_at(&self, depth: u32, x: i32, y: i32) -> Option<u64> {
        self.monsters
            .values()
            .find(|m| m.depth == depth && m.x == x && m.y == y)
            .map(|m| m.id)
    }

    pub fn player_at(&self, depth: u32, x: i32, y: i32) -> Option<u64> {
        self.players
            .values()
            .find(|p| p.alive && p.depth == depth && p.x == x && p.y == y)
            .map(|p| p.id)
    }

    pub fn item_at(&self, depth: u32, x: i32, y: i32) -> Option<u64> {
        self.items
            .values()
            .find(|i| i.depth == depth && i.x == x && i.y == y)
            .map(|i| i.id)
    }

    pub fn build_view_for(&self, player_id: u64) -> Option<WorldView> {
        let p = self.players.get(&player_id)?;
        let d = self.level(p.depth);
        let tiles: Vec<u8> = d
            .tiles
            .iter()
            .map(|t| match t {
                Tile::Void => 0,
                Tile::Wall => 1,
                Tile::Floor => 2,
                Tile::Door => 3,
                Tile::Corridor => 4,
                Tile::StairsDown => 5,
                Tile::StairsUp => 6,
            })
            .collect();

        let mut entities: Vec<EntityView> = Vec::new();

        // Items first (drawn under entities)
        for it in self.items.values().filter(|i| i.depth == p.depth) {
            entities.push(EntityView {
                id: it.id,
                x: it.x,
                y: it.y,
                glyph: it.kind.glyph(),
                color: it.kind.color(),
                name: it.kind.name(),
                is_player: false,
                is_self: false,
                hp_frac: 1.0,
            });
        }

        // Monsters
        for m in self.monsters.values().filter(|m| m.depth == p.depth) {
            let s = monster_spec(m.kind);
            entities.push(EntityView {
                id: m.id,
                x: m.x,
                y: m.y,
                glyph: s.glyph,
                color: s.color,
                name: s.name.into(),
                is_player: false,
                is_self: false,
                hp_frac: m.hp as f32 / m.max_hp.max(1) as f32,
            });
        }

        // Other players
        for other in self.players.values().filter(|o| o.depth == p.depth && o.alive) {
            entities.push(EntityView {
                id: other.id,
                x: other.x,
                y: other.y,
                glyph: '@',
                color: other.color,
                name: other.name.clone(),
                is_player: true,
                is_self: other.id == player_id,
                hp_frac: other.hp as f32 / other.max_hp.max(1) as f32,
            });
        }

        let players_here = self
            .players
            .values()
            .filter(|pl| pl.depth == p.depth && pl.alive)
            .count() as u32;

        Some(WorldView {
            width: d.w as u16,
            height: d.h as u16,
            tiles,
            entities,
            stats: p.stats(),
            depth: p.depth,
            players_here,
            alive: p.alive,
        })
    }

    pub fn grant_xp(&mut self, player_id: u64, xp: u32) {
        if let Some(p) = self.players.get_mut(&player_id) {
            p.xp += xp;
            while p.xp >= p.xp_next() {
                p.xp -= p.xp_next();
                p.level += 1;
                p.max_hp += 6;
                p.hp = p.max_hp;
                p.base_attack += 1;
                if p.level % 2 == 0 {
                    p.base_defense += 1;
                }
                p.push_log(format!("*** Welcome to level {}! ***", p.level), 14);
                self.global_log
                    .push((format!("{} reached level {}!", p.name, p.level), 14));
            }
        }
    }
}
