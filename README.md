# ygopru
<h1>This project is under developing. anything you see may change later.</h1>

A modern async rewrite of the [YGOPro server](https://github.com/mycard/ygopro) in Rust.

ygopru reimplements the YGOPro server stack (`single_duel` and friends) from scratch in Rust, speaking the same
length-delimited binary protocol so existing YGOPro clients can connect without modification. The duelling engine
itself is still the battle-tested [ygopro-core](https://github.com/mycard/ygopro-core) (ocgcore), linked through an FFI
wrapper, while everything around it — networking, message handling, room management, replay — is pure Rust built on
`tokio`.

> [!WARNING]
> This repository is **server-side only** — it contains no client. It hosts duels and serves the YGOPro binary
> protocol; connect with any existing YGOPro client.

## Features

- **Drop-in server protocol** — byte-compatible with the original YGOPro server; any client that connects to
  `ygopro` works here.
- **Fully semantic protocol** — every byte on the wire is a typed message. The whole `ctos` / `stoc` / game-message
  protocol is expressed as Rust enums with `binrw` (de)serialization, no opaque buffers, including the masking rules
  observers rely on.
- **Join mid-game** — inner support for joining as a observer even duel already start.
- **Handler pipeline** — a tower/axum-inspired `FromRequest` / `IntoResponse` framework for processing game messages;
  works like `axum`.
- **Pluggable rooms** — a `RoomProvider` trait lets the same logic drive an in-process `SingleDuel` or bridge to an
  external `ygopro` binary as a subprocess.

## Performance

Every message is parsed from and re-serialized back to the wire as a typed struct, so compared to the original C++
server the overhead of serialization/deserialization makes performance slightly lower. The engine itself is unchanged.

## Building

### Prerequisites

- Rust toolchain (edition 2024, stable)
- A C++14 compiler (gcc/clang/MSVC) — required to build ocgcore
- Tested on:
  - linux
  - M4 mac
  - win10 with vs2022

### Steps

```bash
# fetch the ocgcore and lua submodules
git submodule update --init --recursive

# build the whole workspace (default features)
cargo build --release
```

`ygopro-core-wrapper` compiles Lua and ocgcore from the submodules as static libraries during its build script, so no
external system packages are needed.

## Usage

### Server

Run with no arguments for a quick test on a random port:

```bash
./ygopro
```

Listen on a fixed port with default rules:

```bash
./ygopro 7911
```

The full argument list (lflist, rule, mode, duel rule, LP, hand size, seeds, ...) is identical to the original
[`ygopro-server`](https://github.com/mycard/ygopro-server) — see its README for the details:

```text
./ygopro <port> <lflist> <rule> <mode> <duel_rule> <no_check_deck> <no_shuffle_deck> <start_lp> <start_hand> <draw_count> <time_limit> <replay_mode> [seed ...]
```

Example:

```bash
./ygopro 0 0 0 0 T F F 8000 5 1 180 0
```

### Toolkits

`ygopro-toolkits` bundles the everyday utilities we build while developing ygopro:

```bash
# validate one or more replay files by replaying them through the engine
./ygopro-toolkits validate-replay "replays/*.yrp" [--wait <port>] [--timeout <sec>]

# serve a replay's terminal scene to two takeover players, forever
./ygopro-toolkits tsukuyomi <replay.yrp> [--port 7911]

# logging proxy between a client and a server
./ygopro-toolkits proxy --target <server:port> [--port 8911]
```

`validate-replay` and `tsukuyomi` can run against an external `ygopro-server` binary instead of the in-process engine
via `--server-bin <path> [--server-cwd <dir>]`.

## Roadmap
- plugin system
- tag duel support
- FFI export
- srvpro-rs

## Related projects

- [ygopro](https://github.com/Fluorohydride/ygopro) — the original C++ YGOPro client
- [ygopro](https://github.com/mycard/ygopro) — the server extension this project is a port of
- [ygopro-core](https://github.com/Fluorohydride/ygopro-core) — the ocgcore engine this project links against
