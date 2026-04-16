use crate::entity::{ItemKind, monster_spec};
use crate::protocol::Dir;
use crate::world::{Player, PlayerAction, World};
use rand::Rng;

pub const TICK_MS: u64 = 120;
pub const RESPAWN_TICKS: u32 = 25;

fn roll(rng: &mut impl Rng, max: i32) -> i32 {
    if max <= 0 { 0 } else { rng.gen_range(1..=max) }
}

fn damage_roll(rng: &mut impl Rng, atk: i32, def: i32) -> i32 {
    let a = roll(rng, atk.max(1));
    let d = rng.gen_range(0..=def.max(0));
    (a - d).max(1)
}

pub fn handle_player_move(world: &mut World, pid: u64, dir: Dir) {
    let alive = world.players.get(&pid).map(|p| p.alive).unwrap_or(false);
    if !alive {
        return;
    }
    handle_move(world, pid, dir);
}

pub fn handle_player_action(world: &mut World, pid: u64, action: PlayerAction) {
    let alive = world.players.get(&pid).map(|p| p.alive).unwrap_or(false);
    match action {
        PlayerAction::Respawn => {
            if !alive {
                if let Some(p) = world.players.get(&pid) {
                    if p.death_timer == 0 {
                        respawn(world, pid);
                    }
                }
            }
        }
        _ if !alive => {}
        other => handle_action(world, pid, other),
    }
}

/// Run monster AI, apply regen, tick death timers.
pub fn tick(world: &mut World) {
    world.tick += 1;

    // 1. Death timers and invulnerability ticks
    let ids: Vec<u64> = world.players.keys().copied().collect();
    for id in ids {
        let p = world.players.get_mut(&id).unwrap();
        if !p.alive && p.death_timer > 0 {
            p.death_timer -= 1;
        }
        if p.invuln_ticks > 0 {
            p.invuln_ticks -= 1;
            if p.invuln_ticks == 0 && p.alive {
                p.push_log("Your protective aura fades.", 8);
            }
        }
    }

    // 3. Monster AI + actions
    let monster_ids: Vec<u64> = world.monsters.keys().copied().collect();
    for mid in monster_ids {
        let Some(m) = world.monsters.get_mut(&mid) else {
            continue;
        };
        m.tick_counter += 1;
        let spec = monster_spec(m.kind);
        if m.tick_counter < spec.speed {
            continue;
        }
        m.tick_counter = 0;
        let (mx, my, depth) = (m.x, m.y, m.depth);
        // find nearest alive player on same depth
        let mut best: Option<(u64, i32, i32, i32)> = None; // (pid, px, py, dist^2)
        for p in world.players.values().filter(|p| p.alive && p.depth == depth) {
            let dx = p.x - mx;
            let dy = p.y - my;
            let dist = dx * dx + dy * dy;
            if dist <= spec.sight * spec.sight {
                if best.map_or(true, |b| dist < b.3) {
                    best = Some((p.id, p.x, p.y, dist));
                }
            }
        }
        if let Some((target_pid, tx, ty, dist_sq)) = best {
            // adjacent? attack
            if dist_sq <= 2 {
                monster_attack_player(world, mid, target_pid);
            } else {
                step_toward(world, mid, tx, ty);
            }
        } else {
            // 20% chance to drift toward nearest player anywhere on this level
            let drift = world.rng.gen_bool(0.20);
            let mut moved = false;
            if drift {
                let mut nearest: Option<(i32, i32, i32)> = None;
                for p in world.players.values().filter(|p| p.alive && p.depth == depth) {
                    let dx = p.x - mx;
                    let dy = p.y - my;
                    let d2 = dx * dx + dy * dy;
                    if nearest.map_or(true, |n| d2 < n.2) {
                        nearest = Some((p.x, p.y, d2));
                    }
                }
                if let Some((tx, ty, _)) = nearest {
                    step_toward(world, mid, tx, ty);
                    moved = true;
                }
            }
            if !moved && world.rng.gen_bool(0.30) {
                let dirs = [
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                    (1, 1),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                ];
                let (dx, dy) = dirs[world.rng.gen_range(0..dirs.len())];
                let nx = mx + dx;
                let ny = my + dy;
                if !world.blocked(depth, nx, ny, Some(mid)) {
                    let m = world.monsters.get_mut(&mid).unwrap();
                    m.x = nx;
                    m.y = ny;
                }
            }
        }
    }

    // 4. Regen: slow HP regen for players who weren't hit this tick
    let ids: Vec<u64> = world.players.keys().copied().collect();
    for id in ids {
        let p = world.players.get_mut(&id).unwrap();
        if p.alive && p.hp < p.max_hp {
            let last_dmg_tick = p.last_active_tick;
            if world.tick.saturating_sub(last_dmg_tick) > 6 && world.tick % 4 == 0 {
                p.hp = (p.hp + 1).min(p.max_hp);
            }
        }
    }
}

fn handle_action(world: &mut World, pid: u64, action: PlayerAction) {
    match action {
        PlayerAction::Pickup => try_pickup(world, pid),
        PlayerAction::Descend => try_descend(world, pid),
        PlayerAction::Ascend => try_ascend(world, pid),
        PlayerAction::Quaff => try_quaff(world, pid),
        PlayerAction::Rest => {
            // Rest: accelerated regen if no monsters are currently visible.
            let (px, py, depth) = {
                let p = world.players.get(&pid).unwrap();
                (p.x, p.y, p.depth)
            };
            let nearby = world
                .monsters
                .values()
                .any(|m| m.depth == depth && (m.x - px).abs() + (m.y - py).abs() <= 10);
            if nearby {
                if let Some(p) = world.players.get_mut(&pid) {
                    p.push_log("You sense danger nearby. You cannot rest.", 9);
                }
            } else if let Some(p) = world.players.get_mut(&pid) {
                let before = p.hp;
                p.hp = (p.hp + 4).min(p.max_hp);
                if p.hp > before {
                    p.push_log(format!("You rest. ({} -> {} HP)", before, p.hp), 14);
                } else {
                    p.push_log("You rest for a moment.", 8);
                }
            }
        }
        PlayerAction::Respawn => {
            if let Some(p) = world.players.get_mut(&pid) {
                if !p.alive && p.death_timer == 0 {
                    respawn(world, pid);
                }
            }
        }
    }
}

fn handle_move(world: &mut World, pid: u64, dir: Dir) {
    let (dx, dy) = dir.delta();
    let (px, py, depth) = {
        let p = world.players.get(&pid).unwrap();
        (p.x, p.y, p.depth)
    };
    let nx = px + dx;
    let ny = py + dy;

    // bump-attack: if monster there, attack
    if let Some(mid) = world.monster_at(depth, nx, ny) {
        player_attack_monster(world, pid, mid);
        return;
    }
    // friendly: swap positions with another player (coop-friendly)
    if let Some(other) = world.player_at(depth, nx, ny) {
        if other != pid {
            let other_name = world
                .players
                .get(&other)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let (ox, oy) = {
                let o = world.players.get_mut(&other).unwrap();
                let o_xy = (o.x, o.y);
                o.x = px;
                o.y = py;
                o_xy
            };
            if let Some(p) = world.players.get_mut(&pid) {
                p.x = ox;
                p.y = oy;
                p.push_log(format!("You swap places with {}.", other_name), 7);
            }
            return;
        }
    }
    let d = world.level(depth);
    if !d.walkable(nx, ny) {
        // Silent bump — no log spam.
        return;
    }
    if let Some(p) = world.players.get_mut(&pid) {
        p.x = nx;
        p.y = ny;
        // auto-mention items under feet
        if let Some(item_id) = world
            .items
            .values()
            .find(|i| i.depth == p.depth && i.x == nx && i.y == ny)
            .map(|i| i.id)
        {
            let item = world.items.get(&item_id).unwrap().kind;
            if let Some(p2) = world.players.get_mut(&pid) {
                p2.push_log(format!("You see here: {}. (',' to pick up)", item.name()), 7);
            }
        }
    }
}

fn player_attack_monster(world: &mut World, pid: u64, mid: u64) {
    let (atk, pname, weapon) = {
        let p = world.players.get(&pid).unwrap();
        (p.attack(), p.name.clone(), p.weapon)
    };
    let (def, mx, my, mname, mxp, depth, m_max_hp) = {
        let m = world.monsters.get(&mid).unwrap();
        let spec = monster_spec(m.kind);
        (
            spec.defense,
            m.x,
            m.y,
            spec.name.to_string(),
            spec.xp,
            m.depth,
            m.max_hp,
        )
    };
    let raw = damage_roll(&mut world.rng, atk, def);
    // 1-in-10 crit for double damage
    let crit = world.rng.gen_ratio(1, 10);
    let dmg = if crit { raw * 2 } else { raw };
    let killed = {
        let m = world.monsters.get_mut(&mid).unwrap();
        m.hp -= dmg;
        m.hp <= 0
    };
    if let Some(p) = world.players.get_mut(&pid) {
        let verb = attack_verb(weapon);
        let msg = if crit {
            format!("CRIT! You {} the {} for {} damage!", verb, mname, dmg)
        } else {
            format!("You {} the {} for {} damage.", verb, mname, dmg)
        };
        let color = if crit { 11 } else if killed { 10 } else { 15 };
        p.push_log(msg, color);
    }
    let _ = m_max_hp;
    if killed {
        world.monsters.remove(&mid);
        world.grant_xp(pid, mxp);
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log(format!("You slay the {}! (+{} xp)", mname, mxp), 10);
        }
        // chance to drop gold
        if world.rng.gen_bool(0.3) {
            let amount = world.rng.gen_range(3..=8 + depth as u32 * 2);
            let id = world.gen_id();
            world.items.insert(
                id,
                crate::world::Item {
                    id,
                    depth,
                    x: mx,
                    y: my,
                    kind: ItemKind::Gold(amount),
                },
            );
        }
        // notify nearby players
        notify_nearby(world, depth, mx, my, 15, &format!("{} killed a {}.", pname, mname), 8);
    }
}

fn monster_attack_player(world: &mut World, mid: u64, pid: u64) {
    let invuln = world
        .players
        .get(&pid)
        .map(|p| p.invuln_ticks > 0)
        .unwrap_or(false);
    if invuln {
        let (mname, mkind) = {
            let m = world.monsters.get(&mid).unwrap();
            let spec = monster_spec(m.kind);
            (spec.name.to_string(), m.kind)
        };
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log(
                format!("The {} {} at you, but you are protected!", mname, monster_attack_verb(mkind)),
                14,
            );
        }
        return;
    }
    let (matk, mname, mkind, depth, mx, my) = {
        let m = world.monsters.get(&mid).unwrap();
        let spec = monster_spec(m.kind);
        (
            spec.attack,
            spec.name.to_string(),
            m.kind,
            m.depth,
            m.x,
            m.y,
        )
    };
    let def = {
        let p = world.players.get(&pid).unwrap();
        p.defense()
    };
    let dmg = damage_roll(&mut world.rng, matk, def);
    let (died, pname) = {
        let p = world.players.get_mut(&pid).unwrap();
        p.hp -= dmg;
        p.last_active_tick = world.tick;
        p.last_damage_source = Some(mname.clone());
        let verb = monster_attack_verb(mkind);
        p.push_log(format!("The {} {} you for {} damage!", mname, verb, dmg), 9);
        (p.hp <= 0, p.name.clone())
    };
    if died {
        kill_player(world, pid, mname.clone());
        let _ = (mx, my);
        world.global_log.push((
            format!("*** {} was slain by a {}. ***", pname, mname),
            9,
        ));
    }
}

fn kill_player(world: &mut World, pid: u64, by: String) {
    let (x, y, depth, name, level) = {
        let Some(p) = world.players.get_mut(&pid) else {
            return;
        };
        p.alive = false;
        p.death_timer = RESPAWN_TICKS;
        p.hp = 0;
        p.push_log(format!("You die to the {}...", by), 9);
        (p.x, p.y, p.depth, p.name.clone(), p.level)
    };
    // drop gold on death
    let gold_drop = {
        let p = world.players.get_mut(&pid).unwrap();
        let g = p.gold;
        p.gold = 0;
        g
    };
    if gold_drop > 0 {
        let id = world.gen_id();
        world.items.insert(
            id,
            crate::world::Item {
                id,
                depth,
                x,
                y,
                kind: ItemKind::Gold(gold_drop),
            },
        );
        let p = world.players.get_mut(&pid).unwrap();
        p.push_log(
            format!("Your {} gold spills onto the floor!", gold_drop),
            11,
        );
    }
    // Place a tombstone on the death tile if it's a normal walkable tile
    // (don't overwrite stairs/altars).
    let current = world.level(depth).tile(x, y);
    if matches!(
        current,
        crate::protocol::Tile::Floor | crate::protocol::Tile::Corridor | crate::protocol::Tile::Door
    ) {
        world
            .level_mut(depth)
            .set(x, y, crate::protocol::Tile::Tombstone);
        let epitaph = format!("{} — L{} — slain by {}.", name, level, by);
        world.tombstones.insert((depth, x, y), epitaph);
    }
}

fn respawn(world: &mut World, pid: u64) {
    let d1 = world.levels.get(&1).unwrap().clone();
    // Clear any monsters that drifted into the safe room so we don't
    // die instantly on respawn.
    if let Some(r) = d1.rooms.first().cloned() {
        let in_safe = |x: i32, y: i32| -> bool {
            x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
        };
        let to_remove: Vec<u64> = world
            .monsters
            .values()
            .filter(|m| m.depth == 1 && in_safe(m.x, m.y))
            .map(|m| m.id)
            .collect();
        for id in to_remove {
            world.monsters.remove(&id);
        }
    }
    if let Some(p) = world.players.get_mut(&pid) {
        p.alive = true;
        p.hp = p.max_hp;
        p.depth = 1;
        // respawn at center of the starting room
        let (sx, sy) = if let Some(r) = d1.rooms.first() {
            r.center()
        } else {
            (1, 1)
        };
        p.x = sx;
        p.y = sy;
        p.death_timer = 0;
        p.invuln_ticks = 30; // ~3.6 seconds of grace
        p.push_log("You rise from the afterlife, a little humbler.", 14);
        p.push_log("A protective aura shimmers around you.", 14);
        // Penalty: lose half xp, keep level
        p.xp /= 2;
    }
}

fn try_pickup(world: &mut World, pid: u64) {
    let (px, py, depth) = {
        let p = world.players.get(&pid).unwrap();
        (p.x, p.y, p.depth)
    };
    // Altar handling: pray when standing on an altar.
    if world.level(depth).tile(px, py) == crate::protocol::Tile::Altar {
        pray_at_altar(world, pid);
        return;
    }
    // Tombstone: read the epitaph.
    if world.level(depth).tile(px, py) == crate::protocol::Tile::Tombstone {
        let epitaph = world
            .tombstones
            .get(&(depth, px, py))
            .cloned()
            .unwrap_or_else(|| "An unmarked grave.".to_string());
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log(format!("Grave reads: \"{}\"", epitaph), 14);
        }
        return;
    }
    let item_ids: Vec<u64> = world
        .items
        .values()
        .filter(|i| i.depth == depth && i.x == px && i.y == py)
        .map(|i| i.id)
        .collect();
    if item_ids.is_empty() {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log("There is nothing here to pick up.", 8);
        }
        return;
    }
    for iid in item_ids {
        let kind = world.items.remove(&iid).unwrap().kind;
        let p = world.players.get_mut(&pid).unwrap();
        match kind {
            ItemKind::Gold(n) => {
                p.gold += n;
                p.push_log(format!("You pick up {} gold pieces.", n), 11);
            }
            ItemKind::Potion => {
                p.potions += 1;
                p.push_log("You pocket a healing potion. Press q to drink.", 13);
            }
            ItemKind::Gem => {
                p.gems += 1;
                p.gold += 50;
                p.push_log("A sparkling gem! (+50 gold bonus)", 14);
            }
            ItemKind::Amulet => {
                p.has_amulet = true;
                p.push_log("*** You grasp the Amulet of Yendor! ***", 11);
                let name = p.name.clone();
                world.global_log.push((
                    format!(">>> {} has obtained the Amulet of Yendor! <<<", name),
                    11,
                ));
            }
            k if k.weapon_bonus().is_some() => {
                let bonus_new = k.weapon_bonus().unwrap();
                let bonus_old = p.weapon.and_then(|w| w.weapon_bonus()).unwrap_or(0);
                if bonus_new > bonus_old {
                    p.push_log(format!("You wield a {}. (atk {} -> {})", k.name(), bonus_old, bonus_new), 14);
                    p.weapon = Some(k);
                } else {
                    p.push_log(format!("You find a {} but your weapon is better.", k.name()), 8);
                    // drop it back
                    let (x, y, depth) = (p.x, p.y, p.depth);
                    let id = world.gen_id();
                    world.items.insert(
                        id,
                        crate::world::Item {
                            id,
                            depth,
                            x,
                            y,
                            kind: k,
                        },
                    );
                }
            }
            k if k.armor_bonus().is_some() => {
                let bonus_new = k.armor_bonus().unwrap();
                let bonus_old = p.armor.and_then(|a| a.armor_bonus()).unwrap_or(0);
                if bonus_new > bonus_old {
                    p.push_log(format!("You don {}. (def {} -> {})", k.name(), bonus_old, bonus_new), 14);
                    p.armor = Some(k);
                } else {
                    p.push_log(format!("You find {} but your armor is better.", k.name()), 8);
                    let (x, y, depth) = (p.x, p.y, p.depth);
                    let id = world.gen_id();
                    world.items.insert(
                        id,
                        crate::world::Item {
                            id,
                            depth,
                            x,
                            y,
                            kind: k,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn try_descend(world: &mut World, pid: u64) {
    let (px, py, depth) = {
        let p = world.players.get(&pid).unwrap();
        (p.x, p.y, p.depth)
    };
    let d = world.level(depth);
    let is_stairs = d.tile(px, py) == crate::protocol::Tile::StairsDown;
    if !is_stairs {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log("There are no stairs down here.", 8);
        }
        return;
    }
    let new_depth = depth + 1;
    world.ensure_level(new_depth);
    let nd = world.levels.get(&new_depth).unwrap();
    let spawn = if nd.stairs_up != (0, 0) {
        nd.stairs_up
    } else {
        nd.rooms.first().map(|r| r.center()).unwrap_or((1, 1))
    };
    let name = {
        let p = world.players.get_mut(&pid).unwrap();
        p.depth = new_depth;
        p.x = spawn.0;
        p.y = spawn.1;
        p.push_log(format!("You descend to level {}.", new_depth), 14);
        p.name.clone()
    };
    world
        .global_log
        .push((format!("{} descends to level {}.", name, new_depth), 7));
}

fn try_ascend(world: &mut World, pid: u64) {
    let (px, py, depth) = {
        let p = world.players.get(&pid).unwrap();
        (p.x, p.y, p.depth)
    };
    if depth == 1 {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log("You can't leave the dungeon yet.", 8);
        }
        return;
    }
    let d = world.level(depth);
    if d.tile(px, py) != crate::protocol::Tile::StairsUp {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log("There are no stairs up here.", 8);
        }
        return;
    }
    let new_depth = depth - 1;
    let nd = world.levels.get(&new_depth).unwrap();
    let spawn = nd.stairs_down;
    let name = {
        let p = world.players.get_mut(&pid).unwrap();
        p.depth = new_depth;
        p.x = spawn.0;
        p.y = spawn.1;
        p.push_log(format!("You climb up to level {}.", new_depth), 14);
        p.name.clone()
    };
    world
        .global_log
        .push((format!("{} returns to level {}.", name, new_depth), 7));
}

fn try_quaff(world: &mut World, pid: u64) {
    if let Some(p) = world.players.get_mut(&pid) {
        if p.potions == 0 {
            p.push_log("You have no potions.", 8);
            return;
        }
        p.potions -= 1;
        let heal = (p.max_hp / 2).max(8);
        let before = p.hp;
        p.hp = (p.hp + heal).min(p.max_hp);
        p.push_log(
            format!("You quaff a potion. ({} -> {} HP)", before, p.hp),
            13,
        );
    }
}

fn monster_attack_verb(kind: crate::entity::MonsterKind) -> &'static str {
    use crate::entity::MonsterKind as M;
    match kind {
        M::Rat => "bites",
        M::Bat => "swoops at",
        M::Kobold => "jabs",
        M::Goblin => "slashes",
        M::Orc => "hacks at",
        M::Zombie => "maws",
        M::Gnome => "stabs",
        M::Troll => "pummels",
        M::Ogre => "clubs",
        M::Wraith => "drains",
        M::Dragon => "breathes fire on",
        M::Lich => "hexes",
    }
}

fn attack_verb(weapon: Option<crate::entity::ItemKind>) -> &'static str {
    use crate::entity::ItemKind as IK;
    match weapon {
        Some(IK::Dagger) => "stab",
        Some(IK::ShortSword) => "slash",
        Some(IK::LongSword) => "slice",
        Some(IK::BattleAxe) => "cleave",
        Some(IK::WarHammer) => "smash",
        _ => "strike",
    }
}

fn pray_at_altar(world: &mut World, pid: u64) {
    // Each altar may be prayed at only once per visit — mark as 'used' by
    // tracking altars the player has already used.
    let (px, py, depth, name) = {
        let p = world.players.get(&pid).unwrap();
        (p.x, p.y, p.depth, p.name.clone())
    };
    if world
        .altars_used
        .get(&pid)
        .map(|s| s.contains(&(depth, px, py)))
        .unwrap_or(false)
    {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log("The gods have already heard you here today.", 8);
        }
        return;
    }
    let roll = world.rng.gen_range(0..100);
    let mut log = |world: &mut World, pid, text: String, color| {
        if let Some(p) = world.players.get_mut(&pid) {
            p.push_log(text, color);
        }
    };
    match roll {
        0..=15 => {
            log(world, pid, "The gods are displeased! Lightning strikes!".into(), 9);
            let p = world.players.get_mut(&pid).unwrap();
            let d = (p.max_hp / 4).max(3);
            p.hp = (p.hp - d).max(1);
        }
        16..=40 => {
            let p = world.players.get_mut(&pid).unwrap();
            let heal = p.max_hp / 2;
            let before = p.hp;
            p.hp = (p.hp + heal).min(p.max_hp);
            p.push_log(
                format!("A warm glow heals you. ({} -> {} HP)", before, p.hp),
                14,
            );
        }
        41..=60 => {
            let p = world.players.get_mut(&pid).unwrap();
            p.max_hp += 4;
            p.hp += 4;
            p.push_log("You feel hardier. (+4 max HP)", 10);
        }
        61..=75 => {
            let p = world.players.get_mut(&pid).unwrap();
            p.base_attack += 1;
            p.push_log("Your weapon feels lighter. (+1 attack)", 10);
        }
        76..=85 => {
            let p = world.players.get_mut(&pid).unwrap();
            p.base_defense += 1;
            p.push_log("Your skin toughens. (+1 defense)", 10);
        }
        86..=93 => {
            let p = world.players.get_mut(&pid).unwrap();
            p.potions += 2;
            p.push_log("Two healing potions appear in your pack!", 13);
        }
        _ => {
            let p = world.players.get_mut(&pid).unwrap();
            let g = 50 + world.rng.gen_range(0..50);
            p.gold += g;
            p.push_log(format!("Gold rains from the altar! (+{})", g), 11);
        }
    }
    world
        .altars_used
        .entry(pid)
        .or_default()
        .insert((depth, px, py));
    world
        .global_log
        .push((format!("{} prays at an altar...", name), 13));
}

fn step_toward(world: &mut World, mid: u64, tx: i32, ty: i32) {
    let (mx, my, depth) = {
        let m = world.monsters.get(&mid).unwrap();
        (m.x, m.y, m.depth)
    };
    let dx = (tx - mx).signum();
    let dy = (ty - my).signum();
    let candidates = [(dx, dy), (dx, 0), (0, dy), (-dy, dx), (dy, -dx)];
    for (cx, cy) in candidates {
        if cx == 0 && cy == 0 {
            continue;
        }
        let nx = mx + cx;
        let ny = my + cy;
        // allow attack if player adjacent
        if let Some(pid) = world.player_at(depth, nx, ny) {
            monster_attack_player(world, mid, pid);
            return;
        }
        if !world.blocked(depth, nx, ny, Some(mid)) {
            let m = world.monsters.get_mut(&mid).unwrap();
            m.x = nx;
            m.y = ny;
            return;
        }
    }
}

fn notify_nearby(world: &mut World, depth: u32, x: i32, y: i32, radius: i32, msg: &str, color: u8) {
    for p in world.players.values_mut().filter(|p| p.depth == depth) {
        let dx = p.x - x;
        let dy = p.y - y;
        if dx * dx + dy * dy <= radius * radius {
            p.push_log(msg, color);
        }
    }
    // keep compiler happy
    let _ = Player::new;
}
