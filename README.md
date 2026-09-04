<div align="center">
    <h1>ygopru</h1>
    <p>
        <img alt="Rust" src="https://img.shields.io/badge/rustc-1.96.0-blue?logo=rust&logoColor=white">
        <img alt="Lua" src="https://img.shields.io/badge/lua-5.5.1-purple?logo=lua&logoColor=white">
    </p>
    <p><a href="README.md">English</a> · <a href="README.zh.md">中文</a></p>
</div>

---

> [!IMPORTANT]
> This project is under development. Anything you see may change later.

A modern async rewrite of the [YGOPro server](https://github.com/mycard/ygopro) in Rust.

ygopru reimplements the YGOPro server stack from scratch in Rust, speaking the same protocol 
so existing YGOPro clients can connect without modification. The duelling engine
itself is still the battle-tested [ygopro-core](https://github.com/mycard/ygopro-core) (ocgcore), linked through an FFI
wrapper, while everything around it — networking, message handling, room management, replay — is pure Rust.

> [!WARNING]
> This repository is **server-side only** — it contains no client.
> It hosts duels and serves the YGOPro binary protocol; connect with any existing YGOPro client.
> If you need a GUI client, please check [ygopro3](https://github.com/jwyxym/YGOPro3).

## Features

- **Drop-in server protocol** — byte-compatible with the original YGOPro server; any client that connects to
  `ygopro` works here.
- **Fully semantic protocol** — every byte on the wire is a typed message. The whole `ctos` / `stoc` / game-message
  protocol is expressed as Rust enums with `binrw` (de)serialization, no opaque buffers, including the masking rules
  observers rely on.
- **Pluggable plugin system** — a flexible and extensible plugin system that makes it easy to extend, assemble, and
  tear down ygopro functionality.
- **Join mid-game** — supports joining mid-duel as an observer, even if the duel has already started (Plugin, default disabled).

## Performance

Every message is parsed from and re-serialized back to the wire as a typed struct, so compared to the original C++
server the serialization/deserialization overhead makes performance slightly lower.

## Building

### Prerequisites

- Rust toolchain (edition 2024, stable)
- A C++14 compiler (gcc/clang/MSVC) — required to build ocgcore
- Tested on:
  - linux
  - M4 mac
  - win10 with vs2022

### Compiling

```bash
# fetch the ocgcore and lua submodules
git submodule update --init --recursive

# build the whole workspace (default features)
cargo build --release
```

Please note `ygopro-core-wrapper` compiles `Lua` and `ocgcore` from the submodules.

## Usage

### Server

Before start, please make sure following files are put in current directory:
- `cards.cdb`, you can find one from [ygopro-database](https://github.com/mycard/ygopro-database).
- `script` folder, you can download from [ygopro-scripts](https://github.com/Fluorohydride/ygopro-scripts)
- `lflist.conf`. This file can be skipped if you don't need a ban/limit list.
- `system.conf` and `strings.conf` are not needed.

Run with no arguments for a one-shot server on a random port:

```bash
./ygopro
```

Run a one-shot server for a fixed port with default rules:

```bash
./ygopro 7911
```

The full argument list (lflist, rule, mode, duel rule, LP, hand size, seeds, ...) is identical to the original
[`ygopro-server`](https://github.com/mycard/ygopro/tree/server) — see its README for the details:

```text
./ygopro <port> <lflist> <rule> <mode> <duel_rule> <no_check_deck> <no_shuffle_deck> <start_lp> <start_hand> <draw_count> <time_limit> <replay_mode> [seed ...]
```

Example:

```bash
# Run a one-shot server with full arguments: random port, use first lflist, allow all cards, Single Mode,
# New master rule 2020, check deck, shuffle deck, start with 8000 LP and 5 hands, draw 1 card each turn, limit 180s.
./ygopro 0 0 0 0 F F F 8000 5 1 180 0
```

You can also drive a duel from code instead of the CLI. Build a `DuelHost` from a `HostInfo` and a `Configuration`,
then give it a client stream and poll its server stream — that is all the `./ygopro` binary does internally:

```rust,no_run
use ygopro::host::DuelHost;
use ygopro::Configuration;
use ygopro_data::message::HostInfo;

let mut configuration = Configuration::default();

// The `soumatou` plugin lets a player join mid-duel as an observer.
configuration.enable_plugin("ygopro::plugin::soumatou");

// `preload_script` injects an extra rule script into every duel it starts.
configuration.enable_plugin_with_configuration(
    "ygopro::plugin::preload_script",
    preload_script::Configuration {
        preloaded_scripts: vec!["./script/my_fantastic_rule.lua".to_string()],
    },
);

let mut duel_host = DuelHost::new(HostInfo::default(), configuration);

// start the one-shot server on a fixed port
ygopro::cli::start_local_server(7911, duel_host).await;
```

If you need a multiple-game (always-on) server, check the `srvpru` project instead. (It's a work in progress.)

You can also replace the `ygopro` binary file under `srvpro`.

### Run with Docker

A prebuilt image is published on Docker Hub as `iami/ygopru`.

```bash
# pull the image and run it
docker run -p 7911:7911 \
    -v "$PWD/cards.cdb:/ygopro/cards.cdb" \
    -v "$PWD/script:/ygopro/script" \
    iami/ygopru
```

You can also pass the full CLI argument list the same way, e.g. `docker run -p 23333:23333 iami/ygopru 23333 0 0 0 F F F 8000 5 1 180 0`.

### Toolkits

`ygopro-toolkits` bundles the everyday utilities we build while developing ygopro. Use `--help` to check details.

```bash
# validate one or more replay files by replaying them through the engine
./ygopro-toolkits validate-replay "replays/*.yrp" [--wait <port>] [--timeout <sec>]

# serve a replay's terminal scene to two takeover players, forever
./ygopro-toolkits tsukuyomi <replay.yrp> [--port 7911]

# logging proxy between a client and a server
./ygopro-toolkits proxy --target <server:port> [--port 8911]
```

`validate-replay` and `tsukuyomi` can run against an external `ygopro-server` binary instead of the in-process engine
via `--server-bin <path> [--server-cwd <dir>]`. You need a self-compiling server which disable the init shuffle.

## Related projects

- [ygopro](https://github.com/Fluorohydride/ygopro) — the original C++ YGOPro client
- [ygopro](https://github.com/mycard/ygopro) — the server extension this project is based on
- [ygopro-core](https://github.com/Fluorohydride/ygopro-core) — the ocgcore duelling engine this project links against
