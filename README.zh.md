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
> 本项目仍在开发中。之后的内容随时可能变动。

用 Rust 对 [YGOPro 服务器](https://github.com/mycard/ygopro) 的现代化异步重写。

ygopru 从头用 Rust 重新实现了整个 YGOPro 服务器栈，并沿用相同的协议，因此现有的 YGOPro 客户端无需改动即可连接。对局引擎本身仍是久经考验的 [ygopro-core](https://github.com/mycard/ygopro-core)（ocgcore），通过一个 FFI 包装层链接；而它周围的一切 —— 网络、消息处理、房间管理、回放 —— 均由 Rust 重写。

> [!WARNING]
> 本仓库**只有服务端**——它不包含客户端。
> 它负责启动服务端 YGOPro 并执行其二进制协议；请用任意已有的 YGOPro 客户端连接。
> 如果你需要一个 GUI 客户端，请看 [ygopro3](https://github.com/jwyxym/YGOPro3)。

## 特性

- **即插即用的服务端协议** —— 与原始 YGOPro 服务器字节兼容；任何能连 `ygopro` 的客户端都能在这里工作。
- **完全语义化的协议** —— 线上每个字节都是强类型消息。整个 `ctos` / `stoc` / `game_message` 协议都用带 `binrw`（反）序列化的 Rust 结构体表达，你可以清楚的获知每一个bit的用途。
- **灵活可拓展的插件系统** —— 一个灵活且可扩展的插件系统，方便对 ygopro 的功能进行扩展、组装和拆卸。
- **中途加入** —— 支持在对局进行中加入作为观察者，即使对局已经开始（插件，默认关闭）。

## 性能

由于每条消息都会进行序列化/反序列化并进行记录，因此与原始 C++ 服务器相比，此服务的性能略有降低。

## 构建

### 前置条件

- Rust 工具链（edition 2024，stable）
- 一个 C++14 编译器（gcc/clang/MSVC）—— 构建 ocgcore 需要
- 已在以下环境测试：
  - linux
  - M4 mac
  - win10 with vs2022

### 编译

```bash
# 拉取 ocgcore 和 lua 子模块
git submodule update --init --recursive

# 构建整个 workspace
cargo build --release
```

请注意 `ygopro-core-wrapper` 会从子模块编译 `Lua` 和 `ocgcore`。

## 使用

### 服务器

启动前，请确保以下文件位于当前目录：
- `cards.cdb`，可从 [ygopro-database](https://github.com/mycard/ygopro-database) 获取。
- `script` 文件夹，可从 [ygopro-scripts](https://github.com/Fluorohydride/ygopro-scripts) 下载。
- `lflist.conf`。如果你不需要禁限表，可以跳过此文件。
- 不需要 `system.conf` 和 `strings.conf`。

不带参数运行，启动一个随机端口的一次性服务器：

```bash
./ygopro
```

用固定端口和默认规则运行一次性服务器：

```bash
./ygopro 7911
```

完整参数列表（lflist、rule、mode、duel rule、LP、手牌数、种子等）与原始
[`ygopro-server`](https://github.com/mycard/ygopro/tree/server) 完全一致——详见其 README：

```text
./ygopro <port> <lflist> <rule> <mode> <duel_rule> <no_check_deck> <no_shuffle_deck> <start_lp> <start_hand> <draw_count> <time_limit> <replay_mode> [seed ...]
```

示例：

```bash
# 用完整参数运行一次性服务器：随机端口，使用第一个禁限表，允许O/T/DIY所有卡，单人对局模式，
# 使用新大师规则2020，正常检查卡组和洗牌，起手 5 张手牌，8000 LP，每回合抽 1 张，限时 180s。
./ygopro 0 0 0 0 F F F 8000 5 1 180 0
```

你也可以脱离命令行、用代码来驱动一场对局。用 `HostInfo` 和 `Configuration` 构建一个 `DuelHost`，
给它一个客户端流并轮询它的服务端流——这正是 `./ygopro` 二进制内部所做的事：

```rust,no_run
use ygopro::host::DuelHost;
use ygopro::Configuration;
use ygopro_data::message::HostInfo;

let mut configuration = Configuration::default();

// `soumatou` 插件允许玩家对局中途作为观察者加入。
configuration.enable_plugin("ygopro::plugin::soumatou");

// `preload_script` 会向它启动的每场对局注入一个额外的规则脚本。
configuration.enable_plugin_with_configuration(
    "ygopro::plugin::preload_script",
    preload_script::Configuration {
        preloaded_scripts: vec!["./script/my_fantastic_rule.lua".to_string()],
    },
);

let mut duel_host = DuelHost::new(HostInfo::default(), configuration);

// 在固定端口启动一次性服务器
ygopro::cli::start_local_server(7911, duel_host).await;
```

如果你需要一个多局（常驻）服务器，请看 `srvpru` 项目。（正在进行中。）

你也可以替换 `srvpro` 下的 `ygopro` 二进制文件。

### 用 Docker 运行

Docker Hub 上发布了预构建镜像 `iami/ygopru`。

```bash
# 拉取并运行镜像
docker run -p 7911:7911 \
    -v "$PWD/cards.cdb:/ygopro/cards.cdb" \
    -v "$PWD/script:/ygopro/script" \
    iami/ygopru
```

你也可以用同样方式传入完整的 CLI 参数列表，例如 `docker run -p 23333:23333 iami/ygopru 23333 0 0 0 F F F 8000 5 1 180 0`。

### 工具包

`ygopro-toolkits` 汇集了我们在开发 ygopro 时常用的日常工具。使用 `--help` 查看详情。

```bash
# 通过引擎重放一个或多个回放文件来校验它们
./ygopro-toolkits validate-replay "replays/*.yrp" [--wait <port>] [--timeout <sec>]

# 将一个回放的决胜场面提供给两名接管玩家，永远运行
./ygopro-toolkits tsukuyomi <replay.yrp> [--port 7911]

# 在客户端与服务器之间的日志代理
./ygopro-toolkits proxy --target <server:port> [--port 8911]
```

`validate-replay` 和 `tsukuyomi` 可以通过 `--server-bin <path> [--server-cwd <dir>]` 使用外部的 `ygopro-server` 二进制运行，而不是进程内引擎。你需要一个自己编译的原生`ygopro`的`server`分支，且其开局不会洗牌。

## 相关项目

- [ygopro](https://github.com/Fluorohydride/ygopro) —— 原始 C++ YGOPro 客户端
- [ygopro](https://github.com/mycard/ygopro) —— 本项目所基于的服务器扩展
- [ygopro-core](https://github.com/Fluorohydride/ygopro-core) —— 本项目链接的 ocgcore 对局引擎
