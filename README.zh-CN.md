# want

**Wolfram|Alpha Network Tool** — 用 Rust 写的 W|A 命令行工具，同时是 MCP server 和 agent skill。

[English](./README.md)

灵感来自 [`wolfram-cli`](https://github.com/ConnorGray/wolfram-cli)。那个项目封装的是本地
WolframKernel（WSTP），本项目走 Wolfram|Alpha 的 HTTP API，因此不依赖任何 Wolfram 安装。

| 入口 | 命令 | 用途 |
| --- | --- | --- |
| CLI / REPL | `want ask "..."`，或直接敲 `want` | 人用 |
| MCP server | `want mcp` | agent 用，stdio 传输，暴露 `want_ask` / `want_short` / `want_pods` |
| Skill | `want skill --install` | agent 用，教它**什么时候**该调 W\|A |

## 安装

```sh
cargo install --path .      # 装到 ~/.cargo/bin/want
want --version
```

下面所有 agent 配置都是用 `want` 这个命令拉起 server 的，所以先把二进制放进 `PATH`。

## 获取 AppID

API 对非商业用途免费，但必须有 AppID。

1. 打开 <https://developer.wolframalpha.com>，用 Wolfram ID 登录（没有就用任意邮箱注册，
   只需邮箱验证，不填任何付款信息）。
2. 进 **My Apps**，点 **Sign up to get your first AppID**。
3. 填一次性用途问卷。个人使用、研究、内部工具都算非商业。
4. 弹窗里填应用名和一句描述，AppID 立刻生成，形如 `X7WEHY-W45KYJL3C`。
5. 配到环境里：

   ```sh
   export WOLFRAM_APPID='X7WEHY-W45KYJL3C'   # 写进 ~/.zshrc 或 ~/.bashrc
   want short "population of France"
   ```

几个要点：

- 免费额度是 **2000 次/月**，Wolfram|Alpha 所有 API 共用这个池子，`ask`、`short`、`pods`、
  `image` 都从里面扣。
- 商业用途需要单独授权，官网只有询价表单。
- `DEMO` 已经不被 API 接受（返回 HTTP 401），所以没有 AppID 时 `want` 会直接拒绝启动，
  而不是发一个注定失败的请求。
- 读取顺序：`--appid` > `WOLFRAM_APPID`。

## 命令

```sh
want ask "density of osmium vs iridium"      # LLM API：为模型排版好的正文/表格
want ask "integrate x^2 sin(x)" --maxchars 800
want short "speed of light"                  # Short Answers：一行答案，最省
want pods "Mars"                             # Full Results：结构化 pods → markdown
want pods --primary "integrate x^2 sin(x)"   # 只留 W|A 标为 primary 的那一节
want pods --pod "Physical properties" "Mars" # 标题精确匹配，* 是通配
want pods --json "Mars"                      # 原始 JSON：保留全部 subpod 类型
want image "plot sin(x)/x from -10 to 10" -o plot.gif
want --lang zh short "capital of France"     # 中文答案
want                                         # REPL，:quit 退出
want repl --mode pods                        # REPL 换端点
want skill                                   # 打印 skill
want skill --install                         # 或直接写进 agent 的目录
```

全局选项：

- `--lang <code>` — 答案与 pod 标题的语言（`zh`、`de`、`fr`…），**输入**仍是英文
- `--units metric|nonmetric` — 单位制，默认按调用方 IP
- `--no-source` — 去掉出处行
- `--cache-ttl <秒>` — 相同请求在 N 秒内复用（默认：一次性命令 0，REPL 与 `want mcp` 300）
- `--assumption <value>`（`ask` / `pods`）— 强制某种解释，值取自上一次响应里的 `didyoumeans`
- `-v` / `-vv` — stderr 日志级别，`RUST_LOG` 优先

退出码，脚本不用解析英文就能分支：

| 码 | 含义 |
| --- | --- |
| 0 | 正常输出 |
| 1 | 其它（网络、解析、意外状态码） |
| 2 | 用法错误（参数拼错、`--units` 非法） |
| 3 | 查询没被理解 —— 换个说法 |
| 4 | 鉴权或配额 —— 换 AppID，或等额度重置 |

`want image` 不会覆盖已存在的图：不给 `-o` 时依次写 `want.gif`、`want-1.gif`……

## 自然语言

Wolfram|Alpha 自己会解析问题，所以直接问，不要先拆成关键词：

```sh
want ask "how many people live in France"
want short "how tall is Mount Everest in feet"
want ask "what is the GDP of Japan in 2025"
```

关键词写法（`France population`）同样有效，是长句解析失败时的退路。输入必须是英文；中文提问
会返回 HTTP 501，并附上可用的英文改写建议。`--lang` 翻译的是**答案**，不是问题。

## 一条答案里有什么

| 类型 | 出现位置 |
| --- | --- |
| 正文与 `##` 分节 | `ask`、`pods` |
| 表格（`\|` 分隔的行） | `ask`、`pods` |
| 图、示意图 | `pods` 里的 `image: <url>` 行，`--json` 的 subpod |
| 单位换算、时间线、对比 | 同一答案里的额外分节 |
| 机器可读结构 | `pods --json`（plaintext、img、mathml、imagemap） |
| 出处 | 末尾一行：API + 版本 + 抓取时间 + 原始查询 |

出处行是让数字可核对的东西：

```
— Wolfram|Alpha full-results 2.6 · fetched 2026-10-22T05:25:48Z · query "capital of France"
```

`want short` 故意只输出裸值，方便管道取值；需要日期就用 `want ask`。

三个端点怎么选：一个问题一个值 → `short`；要正文/表格/对比 → `ask`；只要某几节 → `pods`。
`ask` 的返回已经是给模型看的形状（分节、`|` 分隔的表格、`image: <url>`），直接喂给 agent 即可。

pod 标题是精确字符串（`Physical properties`、`Indefinite integral` 之类），只有 `*` 能通配；
想缩小输出又不想猜标题时，用 `--primary` 比 `--pod` 省事。

## 接进 agent

两条命令，全用各 host 自己的安装器——`want` 不提供让你手动维护的配置文件。

### 1. 注册 MCP server

```sh
# Claude Code —— user 作用域，本机所有项目生效
claude mcp add -s user want -e WOLFRAM_APPID=XXXXXXXXXX -- want mcp

# opencode —— 默认写项目配置，加 --global 写用户级
opencode mcp add want --env WOLFRAM_APPID=XXXXXXXXXX -- want mcp

# Codex —— 写 ~/.codex/config.toml
codex mcp add want --env WOLFRAM_APPID=XXXXXXXXXX -- want mcp
```

这三条都是各 host 文档里添加 stdio server 的正规入口，落到的是 host 自己的配置，不是由你
手写维护的文件。

```sh
claude mcp list        # want: want mcp - ✔ Connected
claude mcp get want    # 查看解析后的命令与环境变量
claude mcp remove want
opencode mcp list
```

全局选项可以直接跟在 server 命令后面：`want mcp --lang zh` 让 agent 里拿到中文答案，
`want mcp --cache-ttl 600` 把缓存延长到十分钟。

撤掉也是命令：

```sh
claude mcp remove want     # Claude Code
opencode mcp list          # opencode 没有 remove 子命令，确认后改 opencode.json
                           # Codex：删掉 ~/.codex/config.toml 里的 [mcp_servers.want]
```

### 2. 安装 skill

skill 放在 `skills/want/SKILL.md`，这正是 [skills.sh](https://skills.sh) CLI 扫描的布局，
所以它能像其它已发布 skill 一样安装：

```sh
npx skills add <your-github>/want -g -a claude-code -a codex -a opencode
npx skills list          # 确认装上了
npx skills update want   # 之后升级
```

没有 Node、或只想用本地 checkout 时，二进制自己也能装：

```sh
want skill --install     # 写 ~/.claude/skills/want 和 ~/.agents/skills/want
want skill --uninstall   # 装回去还能撤
want skill --install --dir ./some/skills-root
want skill               # 只打印
```

用 CLI 装的 skill，对应的是 `npx skills remove want`。

两条路径落的都是 agent 真正会读的目录：

| Host | 扫描的 skill 目录 |
| --- | --- |
| Claude Code | `~/.claude/skills/`（项目内 `.claude/skills/`） |
| Codex | `~/.agents/skills/`（项目内 `.agents/skills/`） |
| opencode | `~/.agents/skills/`、`~/.config/opencode/skills/`、`~/.claude/skills/` |

### 远程 / 跨机器

`want mcp` 走 stdio。跨机器就用 SSH 拉起——任何支持「命令 + 参数」的 host 都行：

```sh
claude mcp add -s user want -e WOLFRAM_APPID=XXXXXXXXXX -- ssh myserver want mcp
```

或者加一层 stdio→HTTP 桥，host 用 HTTP 传输连：

```sh
npx -y mcp-proxy --port 8080 -- want mcp
```

常驻 server 里相同查询缓存 5 分钟，同一时刻的重复请求只打一次上游，工具调用并发执行——
突发问题既不排队也不重复计费。

## MCP 暴露的工具

| 工具 | 参数 | 什么时候用 |
| --- | --- | --- |
| `want_ask` | `query`、`maxchars?` | 正文、表格、对比、多段答案 |
| `want_short` | `query`、`units?` | 单个数值，最便宜 |
| `want_pods` | `query`、`pod_titles?`、`primary?` | 指定结果分节、限定输出 |

三个工具都标了 `readOnlyHint: true`；server 的 `instructions` 里带了查询改写规则（英文、
关键词化、一次一个属性），host 不用额外提示词就知道怎么用。

## 测试

```sh
cargo test                                  # 31 项离线检查，不联网
WOLFRAM_APPID=... cargo test -- --ignored   # 18 项联网检查，约 30 次调用
```

离线部分覆盖 CLI 表面、skill frontmatter 规范、skill 安装布局、缺 AppID 路径，以及 MCP 协议
（跨全部版本的 initialize 协商、`server/discover`、工具 schema、只读注解、未知方法/工具、
坏 JSON 容错）。联网部分覆盖四个端点，外加自然语言、`--lang`、出处行、`--primary`、`--pod`、
`--json` 的图片 subpod、units 和 501 错误路径。

## 开发

```sh
cargo clippy --all-targets
cargo fmt --check
```

依赖：`rmcp`（官方 MCP Rust SDK）、`reqwest`、`clap`、`tokio`、`rustyline`、`tracing`、
`home`、`serde`/`serde_json`、`anyhow`。

## 已知边界

- CLI 之间不共享缓存；只有 `want mcp` 带缓存（默认 5 分钟）。
- `--lang` 只本地化输出，查询本身必须英文。
- `pods` 只取 `plaintext`，不解析 MathML / imagemap。
- 商业套餐下 LLM API 与 Full Results API 的额度是分开的。
