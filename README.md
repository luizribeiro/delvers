# delvers

[![CI](https://github.com/luizribeiro/delvers/actions/workflows/ci.yml/badge.svg)](https://github.com/luizribeiro/delvers/actions/workflows/ci.yml)

A cooperative multi-player roguelike in your terminal. Built with
[ratatui](https://ratatui.rs/) and a custom Unix-socket protocol, so every
player is an instance of the same binary talking to a shared world server
that auto-spawns on first launch.

```
┌ Dungeon ───────────────────────────┐┌ Stats ─────────────┐
│           #########                ││████ HP 28/30 ██████│
│           #.@alice..#              ││   L2 XP 12/40     │
│           #..@......<bob>          ││                   │
│           #........·······+#       ││Name   alice       │
│           #....$...#######          ││Depth  2  Here 2  │
│           ##########               ││Atk 5   Def 1     │
└────────────────────────────────────┘└──────────────────┘
```

## Quick start

```sh
cargo run --release -- --name alice
```

The first client creates the server (a detached background process bound to
a Unix socket under `$XDG_RUNTIME_DIR`). Every subsequent client just
connects — open as many terminals as you like:

```sh
cargo run --release -- --name bob
cargo run --release -- --name carol
```

All players share one dungeon, see each other move in real-time, and
chat over the wire.

## Keybinds

| Key | Action |
| --- | --- |
| `hjkl` / arrows | Move (bump into a monster to attack) |
| `yubn` | Diagonal movement |
| `.` | Wait a turn |
| `,` | Pick up / pray at altar / read tombstone |
| `>` `<` | Descend / ascend stairs |
| `q` | Quaff a healing potion |
| `r` | Rest in place (blocked if monsters near) |
| `t` | Global chat — everyone hears you |
| `s` | Shout — only players on your current level hear |
| `Tab` | Toggle floating player name labels |
| `?` | Help |
| `Q` | Quit |

## What's in the dungeon

- **10 levels** of procedurally-generated rooms and corridors. First room
  on every level is a safe starting area.
- **12 monster types** with per-depth spawn tables and flavor verbs — a
  bat *swoops*, a dragon *breathes fire*, a wraith *drains*.
- **Items**: gold piles, healing potions, 5 tiers of weapons and 3 tiers
  of armor, sparkling gems, and the **Amulet of Yendor** on level 10.
- **Altars** let you pray for random divine favor (or wrath).
- **Tombstones** stay where players die, with an epitaph you can read
  by stepping on them and pressing `,`.
- **Field of view** with symmetric shadowcasting — remembered tiles
  render dim; unseen tiles are blank.
- **Damage floaters**, crit highlights, chat bubbles above players,
  HP/XP gauges, and a leaderboard-sorted roster.

## Win condition

Descend to level 10, grab the Amulet of Yendor, and climb back to
level 1 alive. A full-screen victory banner calls out the champion.

## Architecture

```
src/
  main.rs        entry + server/client bootstrap (auto-spawn)
  protocol.rs    JSON-lines message types shared by both sides
  server.rs      unix socket listener, per-client tasks, tick loop
  world.rs       world state, FOV, roster, scoring
  game.rs        combat, AI, actions
  dungeon.rs     room + corridor generation
  entity.rs      monster and item definitions
  client.rs      ratatui UI, rendering, input handling
```

Messages are newline-delimited JSON. Player actions (move, pickup,
quaff, chat) are processed on receipt for snappy input; monster AI runs
on a 120 ms tick.

## Running as a shell replacement

The binary is self-contained, so you can set it as a login shell on a
server (`chsh -s /usr/local/bin/delvers`) to let SSH users drop
straight into the dungeon. That part isn't wired up yet — but the
single-binary, auto-spawn design is built to make it easy.

## Development

```sh
cargo build          # dev build with incremental speed
cargo build --release
```

Clean up a stuck server socket:

```sh
rm -f "$XDG_RUNTIME_DIR/delvers.sock"
```

Server logs go to `/tmp/delvers.log` by default (override with
`DELVERS_LOG`).
