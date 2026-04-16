use crate::protocol::{MAP_H, MAP_W, Tile};
use rand::{Rng, SeedableRng, rngs::StdRng};

#[derive(Clone, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x <= o.x + o.w + 1
            && self.x + self.w + 1 >= o.x
            && self.y <= o.y + o.h + 1
            && self.y + self.h + 1 >= o.y
    }
}

#[derive(Clone, Debug)]
pub struct Dungeon {
    pub w: usize,
    pub h: usize,
    pub tiles: Vec<Tile>,
    pub rooms: Vec<Rect>,
    pub stairs_down: (i32, i32),
    pub stairs_up: (i32, i32),
}

impl Dungeon {
    pub fn generate(depth: u32, seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let w = MAP_W;
        let h = MAP_H;
        let mut tiles = vec![Tile::Void; w * h];

        let mut rooms: Vec<Rect> = Vec::new();
        // Slightly more rooms on deeper levels.
        let max_rooms = 9 + (depth as usize).min(6);
        let min_size = 4;
        let max_size = 9;

        for _ in 0..max_rooms * 3 {
            if rooms.len() >= max_rooms {
                break;
            }
            let rw = rng.gen_range(min_size..=max_size);
            let rh = rng.gen_range(min_size..=max_size.min(7));
            let rx = rng.gen_range(1..(w as i32 - rw - 1));
            let ry = rng.gen_range(1..(h as i32 - rh - 1));
            let new_room = Rect {
                x: rx,
                y: ry,
                w: rw,
                h: rh,
            };
            if rooms.iter().any(|r| r.intersects(&new_room)) {
                continue;
            }
            carve_room(&mut tiles, w, &new_room);
            if let Some(prev) = rooms.last() {
                let (px, py) = prev.center();
                let (nx, ny) = new_room.center();
                if rng.gen_bool(0.5) {
                    carve_h_tunnel(&mut tiles, w, px, nx, py);
                    carve_v_tunnel(&mut tiles, w, py, ny, nx);
                } else {
                    carve_v_tunnel(&mut tiles, w, py, ny, px);
                    carve_h_tunnel(&mut tiles, w, px, nx, ny);
                }
            }
            rooms.push(new_room);
        }

        // Surround floors with walls where they touch void.
        for y in 0..h {
            for x in 0..w {
                if tiles[y * w + x] == Tile::Void {
                    // check neighbors
                    let mut touches = false;
                    'outer: for dy in -1..=1i32 {
                        for dx in -1..=1i32 {
                            let nx = x as i32 + dx;
                            let ny = y as i32 + dy;
                            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                                continue;
                            }
                            let t = tiles[ny as usize * w + nx as usize];
                            if matches!(
                                t,
                                Tile::Floor | Tile::Corridor | Tile::Door | Tile::StairsDown | Tile::StairsUp
                            ) {
                                touches = true;
                                break 'outer;
                            }
                        }
                    }
                    if touches {
                        tiles[y * w + x] = Tile::Wall;
                    }
                }
            }
        }

        // Occasional doors where corridor meets room walls.
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if tiles[y * w + x] == Tile::Corridor {
                    let n = tiles[(y - 1) * w + x];
                    let s = tiles[(y + 1) * w + x];
                    let e = tiles[y * w + x + 1];
                    let we = tiles[y * w + x - 1];
                    let walls = [n, s, e, we].iter().filter(|t| **t == Tile::Wall).count();
                    if walls >= 2 && rng.gen_bool(0.15) {
                        tiles[y * w + x] = Tile::Door;
                    }
                }
            }
        }

        let stairs_down = if let Some(r) = rooms.last() {
            r.center()
        } else {
            ((w / 2) as i32, (h / 2) as i32)
        };
        let stairs_up = if let Some(r) = rooms.first() {
            r.center()
        } else {
            ((w / 2) as i32, (h / 2) as i32)
        };
        tiles[stairs_down.1 as usize * w + stairs_down.0 as usize] = Tile::StairsDown;
        if depth > 1 {
            tiles[stairs_up.1 as usize * w + stairs_up.0 as usize] = Tile::StairsUp;
        }

        Dungeon {
            w,
            h,
            tiles,
            rooms,
            stairs_down,
            stairs_up,
        }
    }

    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return Tile::Void;
        }
        self.tiles[y as usize * self.w + x as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, t: Tile) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        self.tiles[y as usize * self.w + x as usize] = t;
    }

    pub fn walkable(&self, x: i32, y: i32) -> bool {
        self.tile(x, y).walkable()
    }

    pub fn random_floor(&self, rng: &mut impl Rng) -> (i32, i32) {
        loop {
            if self.rooms.is_empty() {
                return (1, 1);
            }
            let r = &self.rooms[rng.gen_range(0..self.rooms.len())];
            let x = rng.gen_range(r.x..r.x + r.w);
            let y = rng.gen_range(r.y..r.y + r.h);
            if self.walkable(x, y) {
                return (x, y);
            }
        }
    }
}

fn carve_room(tiles: &mut [Tile], w: usize, r: &Rect) {
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            tiles[y as usize * w + x as usize] = Tile::Floor;
        }
    }
}

fn carve_h_tunnel(tiles: &mut [Tile], w: usize, x1: i32, x2: i32, y: i32) {
    let (a, b) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
    for x in a..=b {
        let idx = y as usize * w + x as usize;
        if tiles[idx] == Tile::Void {
            tiles[idx] = Tile::Corridor;
        }
    }
}

fn carve_v_tunnel(tiles: &mut [Tile], w: usize, y1: i32, y2: i32, x: i32) {
    let (a, b) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
    for y in a..=b {
        let idx = y as usize * w + x as usize;
        if tiles[idx] == Tile::Void {
            tiles[idx] = Tile::Corridor;
        }
    }
}
