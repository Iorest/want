# want

[![ci](https://github.com/Iorest/want/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Iorest/want/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/Iorest/want?label=release)](https://github.com/Iorest/want/releases)

**Wolfram|Alpha Network Tool**：用 Rust 写的 W|A 命令行工具，同时能作为 MCP server 跑，并自带一个 agent skill。

[English](./README.md)

专治大模型最容易出错的那部分。Wolfram|Alpha 解析问题、在整理过的数据上做计算，不是凭记忆
复述；每个答案都带抓取时间，agent 可以引用这个数字，而不是猜一个。

借鉴自 [`wolfram-cli`](https://github.com/ConnorGray/wolfram-cli)。它封装的是本地
WolframKernel（走 WSTP），`want` 直接调 Wolfram|Alpha 的 HTTP API，不用装 Wolfram 本体。

| 入口 | 命令 | 谁用 |
| --- | --- | --- |
| CLI / REPL | `want ask "..."`，或直接敲 `want` | 人 |
| MCP server | `want mcp` | agent；stdio 传输，提供 `want_ask` / `want_short` / `want_pods` |
| Skill | `want skill --install` | agent；告诉它**什么时候**该查 W\|A |

## 安装

```sh
cargo install --path .      # 装到 ~/.cargo/bin/want
want --version
```

每个 [release](https://github.com/Iorest/want/releases) 还带 macOS arm64 和 Linux
x86_64 的预编译包。

下面各个 agent 的接入方式都用 `want` 这个命令起服务，所以先把二进制放进 `PATH`。

## 获取 AppID

API 非商业用途免费，但必须有 AppID。

1. 打开 <https://developer.wolframalpha.com>，用 Wolfram ID 登录。没有就随便找个邮箱注册，
   收一封确认信即可，不用绑卡。
2. 进 **My Apps**，点 **Sign up to get your first AppID**。
3. 填一次用途问卷。个人使用、研究、内部工具都算非商业。
4. 弹窗里填应用名和一句描述，AppID 当场生成，形如 `X7WEHY-W45KYJL3C`。
5. 存一次：

   ```sh
   want config --set-appid 'X7WEHY-W45KYJL3C'
   want short "population of France"
   ```

   它写进 `~/.config/want/config.toml`，权限 0600，以后开新终端不用再管 shell profile。
   `want config` 能看当前在用哪些 key、从哪来的，key 本身打码。

   只在当前 shell 或单条命令里用也行：

   ```sh
   export WOLFRAM_APPID='X7WEHY-W45KYJL3C'   # 只对这个 shell 生效
   want --appid 'X7WEHY-W45KYJL3C' short "population of France"   # 只对这次调用生效
   ```

几点说明：

- 免费额度 2,000 次/月，Wolfram|Alpha 所有 API 共用，`ask`、`short`、`pods`、`image`
  都从这里扣。
- 商业用途要单独授权，官网只放了销售联系表单。
- `DEMO` 已经不被 API 接受（返回 HTTP 401）。没有有效 AppID 时 `want` 直接拒绝启动，
  不会发一个注定失败的请求。
- 取值顺序：`--appid`、`WOLFRAM_APPID`、`~/.config/want/config.toml`。`want config`
  会告诉你是哪个生效。

### 多个 AppID

每个 AppID 各有 2,000 次/月，两个就是 4,000 次。`want` 在给定的 key 之间轮换，某个 key
被拒就把这次请求换到下一个重试，所以一个额度用完不会让长时间运行的 agent 直接停摆。

```sh
want config --set-appid KEY_ONE              # 第一个 key
want config --add-appid KEY_TWO              # 保留 KEY_ONE，把新 key 加进去
want config --set-appid KEY_ONE,KEY_TWO      # 用这两个覆盖整个文件
export WOLFRAM_APPID='KEY_ONE,KEY_TWO'       # 或者用环境变量
want --appid KEY_ONE --appid KEY_TWO short "2+2"
```

运行时行为：

- 请求轮流使用各个 key，单个 key 不会承担全部突发流量。
- 返回 `401`、`403` 或 `429` 的 key 在本进程内不再使用，同一次请求立刻换下一个重试；
  重试会打一行日志，不会静默发生。
- 所有 key 都用尽时，报错会说明池子里有几个 key，退出码仍是 `4`，脚本的处理方式和
  只有一个 key 时一致。
- key 对缓存透明：一个 key 取回的答案，后面不论落到哪个 key 都直接复用。
- 只有 `401`/`403`/`429` 才会换 key。查询本身解析不了时换 key 结果一样，所以不重试。

## 命令

```sh
want ask "density of osmium vs iridium"      # LLM API：为模型排好版的正文和表格
want ask "integrate x^2 sin(x)" --maxchars 800
want short "speed of light"                  # Short Answers：一行答案，最省
want pods "Mars"                             # Full Results：结构化 pods 转 markdown
want pods --primary "integrate x^2 sin(x)"   # 只留 W|A 标为 primary 的那一节
want pods --pod "Physical properties" "Mars" # 标题精确匹配，* 是通配符
want pods --json "Mars"                      # 原始 JSON：保留全部 subpod 类型
want image "plot sin(x)/x from -10 to 10" -o plot.gif
want --lang zh short "capital of France"     # 中文答案
want                                         # REPL，:quit 退出
want repl --mode pods                        # REPL 换端点
want config                                  # 当前 AppID 及来源，多个时逐个列出
want skill                                   # 打印 skill
want skill --install                         # 或直接写进 agent 的目录
```

全局选项：

- `--appid <id>` 覆盖环境变量和配置文件；可重复，或用逗号分隔多个 key
- `--lang <code>` 指定答案语言和 pod 标题（`zh`、`de`、`fr`…），**输入**仍是英文
- `--units metric|nonmetric` 指定单位制，默认按调用方 IP 判断
- `--no-source` 去掉结尾的出处行
- `--cache-ttl <秒>` 相同请求在 N 秒内复用（一次性命令默认 0，REPL 和 `want mcp` 默认 300）
- `--assumption <value>`（仅 `pods`）强制某种解释，值取自上一条回答里的 `assumption=` 行。
  `ask` 背后的 LLM API 会忽略这个参数，想换个含义只能用 `pods`
- `-v` / `-vv` 控制 stderr 日志级别，`RUST_LOG` 优先

单个子命令才有的选项：

| 命令 | 选项 | 作用 |
| --- | --- | --- |
| `ask` | `--maxchars <n>` | 答案的字符上限，API 默认约 6800 |
| `pods` | `--pod <TITLE>` | 按标题精确保留一节，可重复，`*` 是通配符 |
| `pods` | `--primary` | 只留 primary 那一节。不能和 `--json` 一起用 |
| `pods` | `--json` | 原始 Full Results JSON。不能和 `--primary` 一起用 |
| `image` | `-o, --output <path>` | 图片写到哪；不写就自动编号 |
| `repl` | `--mode ask\|short\|pods\|image` | 每行用哪个端点 |
| `config` | `--set-appid <id>` | 存 AppID，覆盖已有内容 |
| `config` | `--add-appid <id>` | 追加 AppID，保留已存的 |
| `skill` | `--install`、`--uninstall`、`--dir <root>` | 写入、移除，或换一个 skills 根目录 |

退出码：

| 码 | 含义 |
| --- | --- |
| 0 | 正常输出 |
| 1 | 其它（网络、解析、意外状态码） |
| 2 | 用法错误（参数拼错、`--units` 非法） |
| 3 | 查询没被理解，换个说法 |
| 4 | 鉴权或配额：没有 AppID、AppID 被拒，或本月额度用完 |

脚本按码分支即可，不用解析文字。

`want image` 不会覆盖已有的图。不给 `-o` 时依次写 `want.gif`、`want-1.gif`……

## 自然语言

Wolfram|Alpha 自己会断句解析，所以直接把问题丢进去，不用先拆成关键词：

```sh
want ask "how many people live in France"
want short "how tall is Mount Everest in feet"
want ask "what is the GDP of Japan in 2025"
```

关键词写法（`France population`）同样有效，长句解析失败时用它兜底。输入必须是英文；中文提问
会返回 HTTP 501，并附上几个英文改写建议。`--lang` 翻译的是答案，不是问题。

## 答案里有什么

| 类型 | 出现位置 |
| --- | --- |
| 正文和 `##` 分节 | `ask`、`pods` |
| 表格（`\|` 分隔的行） | `ask`、`pods` |
| 图、示意图 | `pods` 里的 `image: <url>` 行，`--json` 的 subpod |
| 单位换算、时间线、对比 | 同一条答案的额外分节 |
| 机器可读结构 | `pods --json`（plaintext 和 img 两类 subpod） |
| 出处 | 末尾一行：API、抓取时间、原始查询；Full Results 还会带版本号 |

出处行让数字可以核对：

```
— Wolfram|Alpha full-results 2.6 · fetched 2026-10-22T05:25:48Z · query "capital of France"
```

`want short` 故意只给裸值，方便管道取值。需要日期就用 `want ask`。

端点怎么选：单个数值给 `short`，正文、表格或对比给 `ask`，只要某几节给 `pods`。`ask`
的输出本身就是给模型看的形状（分节、`|` 分隔的表格、`image: <url>` 行），原样交给 agent
即可。

pod 标题是精确字符串（`Physical properties`、`Indefinite integral` 之类），只有 `*` 是通配符。
想缩小输出又不想猜标题，`--primary` 比 `--pod` 省事。

## 接入 agent

两步，都用客户端自带的安装器，没有需要你手抄的配置文件模板。

### 1. 注册 MCP server

```sh
# Claude Code：user 作用域，本机所有项目生效
claude mcp add -s user want -- want mcp

# opencode：默认写项目配置，加 --global 写用户级
opencode mcp add want -- want mcp

# Codex：写 ~/.codex/config.toml
codex mcp add want -- want mcp
```

这三条都是各客户端官方文档里的注册方式，改的是它们自己的配置。

服务端会自己读 `~/.config/want/config.toml`，所以这里不用传环境变量。不想把 key 落盘的话，
改用带参数的写法：Claude Code 加 `-e WOLFRAM_APPID=XXXXXXXXXX`，opencode 和 Codex 加
`--env WOLFRAM_APPID=XXXXXXXXXX`。

```sh
claude mcp list        # want: want mcp - ✔ Connected
claude mcp get want    # 看解析后的命令和环境变量
claude mcp remove want
opencode mcp list
```

全局选项可以直接跟在服务命令后面：`want mcp --lang zh` 让 agent 拿到中文答案，
`want mcp --cache-ttl 600` 把缓存延长到十分钟。

撤销同样有命令：

```sh
claude mcp remove want     # Claude Code
opencode mcp list          # opencode 没有 remove 子命令，确认后改 opencode.json
                           # Codex：删掉 ~/.codex/config.toml 里的 [mcp_servers.want]
```

### 2. 安装 skill

skill 放在 `skills/want/SKILL.md`，正好是 [skills.sh](https://skills.sh) CLI 扫描的布局，
所以能像其他已发布 skill 一样装：

```sh
npx skills add Iorest/want -g -a claude-code -a codex -a opencode
npx skills list          # 确认装上了
npx skills update want   # 之后升级
```

没有 Node，或者只想用本地 checkout，二进制自己也能装：

```sh
want skill --install     # 写 ~/.claude/skills/want 和 ~/.agents/skills/want
want skill --uninstall   # 不想留了就卸掉
want skill --install --dir ./some/skills-root
want skill               # 只打印
```

通过 CLI 装的 skill，对应 `npx skills remove want`。

两种方式都写进 agent 实际读取的目录：

| 客户端 | 扫描的 skill 目录 |
| --- | --- |
| Claude Code | `~/.claude/skills/`（项目内 `.claude/skills/`） |
| Codex | `~/.agents/skills/`（项目内 `.agents/skills/`） |
| opencode | `~/.agents/skills/`、`~/.config/opencode/skills/`、`~/.claude/skills/` |

### 跨机器

`want mcp` 走 stdio。从另一台机器用，套一层 SSH 就行，任何支持「命令加参数」的客户端都能用：

```sh
claude mcp add -s user want -- ssh myserver want mcp
```

`-e` / `--env` 不会穿过 SSH 传到远端进程，所以在 `myserver` 上先跑一次
`want config --set-appid <id>`，或者把 key 放进远端命令：
`-- ssh myserver want --appid XXXXXXXXXX mcp`。

也可以加一层 stdio 转 HTTP 的桥，让客户端走 HTTP 传输：

```sh
npx -y mcp-proxy --port 8080 -- want mcp
```

常驻服务里相同查询缓存 5 分钟，同一时刻的重复请求只打一次上游，工具调用并发执行。一串问题
既不会排队，也不会重复计费。

## MCP 工具

| 工具 | 参数 | 用途 |
| --- | --- | --- |
| `want_ask` | `query`、`maxchars?` | 正文、表格、对比、多段答案。是摘要，分节由 W\|A 挑 |
| `want_short` | `query`、`units?` | 单个数值，最便宜 |
| `want_pods` | `query`、`pod_titles?`、`primary?`、`assumption?` | 全部或指定分节，也能强制解释 |

三个工具都标了 `readOnlyHint: true`。服务端的 `instructions` 里带了查询规则（英文、自然语句
优先、一次一个属性），客户端不用额外提示词就知道怎么用。

## 测试

```sh
cargo test                                  # 44 项离线检查，不联网
WOLFRAM_APPID=... cargo test -- --ignored   # 22 项联网检查，约 35 次调用
```

离线部分覆盖 CLI 表面、skill frontmatter 规范、skill 安装布局、缺 AppID 的路径、AppID
的取值与优先级（单 key、列表、`--add-appid`）、key 池的轮换，以及 MCP 协议（跨全部版本的
initialize 协商、`server/discover`、工具 schema、只读注解、未知方法和工具、坏 JSON 容错）。
联网部分打四个端点，外加自然语言、`--lang`、出处行、`--primary`、`--pod`、`--json` 的
图片 subpod、单位制、501 错误路径，以及从失效 key 切到可用 key。

## 开发

```sh
cargo clippy --all-targets
cargo fmt --check
```

依赖：`rmcp`（官方 MCP Rust SDK）、`reqwest`、`clap`、`tokio`、`rustyline`、
`tracing`/`tracing-subscriber`、`toml`、`jiff`、`home`、`serde`/`serde_json`、`anyhow`。

## 已知边界

- 一次性命令不缓存。REPL 和 `want mcp` 默认缓存 5 分钟。
- `--lang` 只本地化输出，查询本身必须英文。
- `pods` 输出文字和图片 URL，MathML 和 imagemap 根本没请求。
- 商业套餐下，LLM API 和 Full Results API 的额度是分开的。

## 许可

MIT，见 [LICENSE](./LICENSE)。
