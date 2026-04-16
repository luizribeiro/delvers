use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonsterKind {
    Rat,
    Bat,
    Kobold,
    Goblin,
    Orc,
    Zombie,
    Gnome,
    Troll,
    Ogre,
    Wraith,
    Dragon,
    Lich,
}

#[derive(Clone, Debug)]
pub struct MonsterSpec {
    pub glyph: char,
    pub color: u8,
    pub name: &'static str,
    pub hp: i32,
    pub attack: i32,
    pub defense: i32,
    pub speed: u32, // moves per tick (1 = every tick, 2 = every other)
    pub xp: u32,
    pub sight: i32,
    pub min_depth: u32,
    pub rarity: u32, // higher = more common
}

pub fn monster_spec(kind: MonsterKind) -> MonsterSpec {
    match kind {
        MonsterKind::Rat => MonsterSpec {
            glyph: 'r',
            color: 3,
            name: "rat",
            hp: 4,
            attack: 2,
            defense: 0,
            speed: 2,
            xp: 2,
            sight: 6,
            min_depth: 1,
            rarity: 10,
        },
        MonsterKind::Bat => MonsterSpec {
            glyph: 'b',
            color: 13,
            name: "bat",
            hp: 3,
            attack: 2,
            defense: 1,
            speed: 1,
            xp: 3,
            sight: 8,
            min_depth: 1,
            rarity: 8,
        },
        MonsterKind::Kobold => MonsterSpec {
            glyph: 'k',
            color: 11,
            name: "kobold",
            hp: 6,
            attack: 3,
            defense: 1,
            speed: 2,
            xp: 4,
            sight: 7,
            min_depth: 1,
            rarity: 9,
        },
        MonsterKind::Goblin => MonsterSpec {
            glyph: 'g',
            color: 2,
            name: "goblin",
            hp: 9,
            attack: 4,
            defense: 1,
            speed: 2,
            xp: 6,
            sight: 8,
            min_depth: 2,
            rarity: 9,
        },
        MonsterKind::Orc => MonsterSpec {
            glyph: 'o',
            color: 2,
            name: "orc",
            hp: 14,
            attack: 5,
            defense: 2,
            speed: 2,
            xp: 10,
            sight: 8,
            min_depth: 3,
            rarity: 7,
        },
        MonsterKind::Zombie => MonsterSpec {
            glyph: 'Z',
            color: 8,
            name: "zombie",
            hp: 18,
            attack: 4,
            defense: 0,
            speed: 3,
            xp: 8,
            sight: 6,
            min_depth: 3,
            rarity: 6,
        },
        MonsterKind::Gnome => MonsterSpec {
            glyph: 'G',
            color: 14,
            name: "gnome",
            hp: 12,
            attack: 4,
            defense: 2,
            speed: 2,
            xp: 8,
            sight: 9,
            min_depth: 3,
            rarity: 5,
        },
        MonsterKind::Troll => MonsterSpec {
            glyph: 'T',
            color: 10,
            name: "troll",
            hp: 28,
            attack: 8,
            defense: 3,
            speed: 2,
            xp: 22,
            sight: 7,
            min_depth: 5,
            rarity: 5,
        },
        MonsterKind::Ogre => MonsterSpec {
            glyph: 'O',
            color: 3,
            name: "ogre",
            hp: 40,
            attack: 10,
            defense: 4,
            speed: 3,
            xp: 35,
            sight: 7,
            min_depth: 6,
            rarity: 4,
        },
        MonsterKind::Wraith => MonsterSpec {
            glyph: 'W',
            color: 5,
            name: "wraith",
            hp: 22,
            attack: 7,
            defense: 2,
            speed: 1,
            xp: 30,
            sight: 10,
            min_depth: 6,
            rarity: 4,
        },
        MonsterKind::Dragon => MonsterSpec {
            glyph: 'D',
            color: 9,
            name: "dragon",
            hp: 75,
            attack: 14,
            defense: 6,
            speed: 2,
            xp: 120,
            sight: 12,
            min_depth: 8,
            rarity: 2,
        },
        MonsterKind::Lich => MonsterSpec {
            glyph: 'L',
            color: 5,
            name: "lich",
            hp: 60,
            attack: 12,
            defense: 5,
            speed: 2,
            xp: 100,
            sight: 10,
            min_depth: 9,
            rarity: 2,
        },
    }
}

pub fn all_monsters() -> &'static [MonsterKind] {
    &[
        MonsterKind::Rat,
        MonsterKind::Bat,
        MonsterKind::Kobold,
        MonsterKind::Goblin,
        MonsterKind::Orc,
        MonsterKind::Zombie,
        MonsterKind::Gnome,
        MonsterKind::Troll,
        MonsterKind::Ogre,
        MonsterKind::Wraith,
        MonsterKind::Dragon,
        MonsterKind::Lich,
    ]
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Gold(u32),
    Potion,
    Dagger,
    ShortSword,
    LongSword,
    BattleAxe,
    WarHammer,
    LeatherArmor,
    ChainMail,
    PlateMail,
    Gem,
    Amulet,
}

impl ItemKind {
    pub fn glyph(&self) -> char {
        match self {
            ItemKind::Gold(_) => '$',
            ItemKind::Potion => '!',
            ItemKind::Dagger
            | ItemKind::ShortSword
            | ItemKind::LongSword
            | ItemKind::BattleAxe
            | ItemKind::WarHammer => ')',
            ItemKind::LeatherArmor | ItemKind::ChainMail | ItemKind::PlateMail => '[',
            ItemKind::Gem => '*',
            ItemKind::Amulet => '"',
        }
    }
    pub fn color(&self) -> u8 {
        match self {
            ItemKind::Gold(_) => 11,
            ItemKind::Potion => 13,
            ItemKind::Dagger | ItemKind::ShortSword => 7,
            ItemKind::LongSword => 15,
            ItemKind::BattleAxe | ItemKind::WarHammer => 7,
            ItemKind::LeatherArmor => 3,
            ItemKind::ChainMail => 7,
            ItemKind::PlateMail => 15,
            ItemKind::Gem => 14,
            ItemKind::Amulet => 13,
        }
    }
    pub fn name(&self) -> String {
        match self {
            ItemKind::Gold(n) => format!("{} gold", n),
            ItemKind::Potion => "healing potion".into(),
            ItemKind::Dagger => "dagger".into(),
            ItemKind::ShortSword => "short sword".into(),
            ItemKind::LongSword => "long sword".into(),
            ItemKind::BattleAxe => "battle axe".into(),
            ItemKind::WarHammer => "war hammer".into(),
            ItemKind::LeatherArmor => "leather armor".into(),
            ItemKind::ChainMail => "chain mail".into(),
            ItemKind::PlateMail => "plate mail".into(),
            ItemKind::Gem => "sparkling gem".into(),
            ItemKind::Amulet => "amulet of yendor".into(),
        }
    }
    pub fn weapon_bonus(&self) -> Option<i32> {
        match self {
            ItemKind::Dagger => Some(1),
            ItemKind::ShortSword => Some(2),
            ItemKind::LongSword => Some(4),
            ItemKind::BattleAxe => Some(6),
            ItemKind::WarHammer => Some(8),
            _ => None,
        }
    }
    pub fn armor_bonus(&self) -> Option<i32> {
        match self {
            ItemKind::LeatherArmor => Some(1),
            ItemKind::ChainMail => Some(3),
            ItemKind::PlateMail => Some(5),
            _ => None,
        }
    }
}
