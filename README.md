# want

**Wolfram|Alpha Network Tool** — a Rust CLI for Wolfram|Alpha that is also an MCP
server and an agent skill.

[中文](./README.zh-CN.md)

Inspired by [`wolfram-cli`](https://github.com/ConnorGray/wolfram-cli). That project
wraps a local WolframKernel over WSTP; `want` talks to the Wolfram|Alpha HTTP API
instead, so it needs no Wolfram installation.

| Entry point | Command | For |
| --- | --- | --- |
| CLI / REPL | `want ask "..."`, bare `want` | humans |
| MCP server | `want mcp` | agents; stdio, exposes `want_ask` / `want_short` / `want_pods` |
| Skill | `want skill --install` | agents; teaches *when* to call Wolfram\|Alpha |

## Install

```sh
cargo install --path .      # installs ~/.cargo/bin/want
want --version
```

Every agent config below starts the server with `want`, so the binary has to be on
`PATH` before you wire it up.

## Get an AppID

The API is free for non-commercial use, but it always needs an AppID.

1. Open <https://developer.wolframalpha.com> and sign in with a Wolfram ID (register
   with any email if you do not have one — an email confirmation round trip, no
   payment details).
2. Go to **My Apps** and press **Sign up to get your first AppID**.
3. Answer the one-time survey about intended usage. Personal, research or internal
   tooling counts as non-commercial.
4. In the dialog that appears, give the application a name and a one-line
   description. The AppID appears immediately, shaped like `X7WEHY-W45KYJL3C`.
5. Export it:

   ```sh
   export WOLFRAM_APPID='X7WEHY-W45KYJL3C'   # add to ~/.zshrc or ~/.bashrc
   want short "population of France"
   ```

Details worth knowing:

- The free tier is **2,000 calls per month**, shared by all Wolfram|Alpha APIs, so
  `ask`, `short`, `pods` and `image` all draw from the same pool.
- Commercial use needs a separate licence; the pricing page only has a sales
  contact form.
- `DEMO` is no longer accepted (it returns HTTP 401), so `want` refuses to start
  without a real AppID instead of sending a request that cannot succeed.
- Resolution order: `--appid` > `WOLFRAM_APPID`.

## Commands

```sh
want ask "density of osmium vs iridium"      # LLM API: prose and tables shaped for a model
want ask "integrate x^2 sin(x)" --maxchars 800
want short "speed of light"                  # Short Answers: one line, cheapest
want pods "Mars"                             # Full Results: structured pods -> markdown
want pods --primary "integrate x^2 sin(x)"   # only the section Wolfram|Alpha calls primary
want pods --pod "Physical properties" "Mars" # exact title, * is a wildcard
want pods --json "Mars"                      # raw JSON: every subpod type, machine-readable
want image "plot sin(x)/x from -10 to 10" -o plot.gif
want --lang zh short "capital of France"     # answer in Chinese
want                                         # REPL, :quit to leave
want repl --mode pods                        # REPL on another endpoint
want skill                                   # print the skill
want skill --install                         # or write it where agents look
```

Global options:

- `--lang <code>` — answer language and pod titles (`zh`, `de`, `fr`, …); input
  stays English
- `--units metric|nonmetric` — unit system, defaults by caller IP
- `--no-source` — drop the provenance footer
- `--cache-ttl <secs>` — reuse an identical response for N seconds (default: 0 for
  one-shot commands, 300 for the REPL and `want mcp`)
- `--assumption <value>` (`ask` and `pods`) — force an interpretation, value taken
  from `didyoumeans` in a previous response
- `-v` / `-vv` — stderr log level, `RUST_LOG` takes precedence

Exit codes, so a script can branch without parsing English:

| Code | Meaning |
| --- | --- |
| 0 | answer printed |
| 1 | anything else (network, parse, unexpected status) |
| 2 | usage error (bad flag, bad `--units`) |
| 3 | query not understood — rephrase it |
| 4 | auth or quota — fix the AppID, or wait for the month to roll over |

`want image` never overwrites an earlier picture: without `-o` it writes `want.gif`,
then `want-1.gif`, and so on.

## Natural language

Wolfram|Alpha parses questions, so send the question, not a keyword soup:

```sh
want ask "how many people live in France"
want short "how tall is Mount Everest in feet"
want ask "what is the GDP of Japan in 2025"
```

Keyword form (`France population`) still works and is the fallback when a long
sentence is not understood. Input is English; a non-English question comes back as
HTTP 501 with suggested English phrasings. `--lang` localises the *answer*, not the
question.

## What an answer contains

| Type | Where it shows up |
| --- | --- |
| Prose and `##` sections | `ask`, `pods` |
| Tables (`\|`-separated rows) | `ask`, `pods` |
| Plots and diagrams | `image: <url>` lines in `pods`, plus `--json` subpods |
| Unit conversions, timelines, comparisons | additional sections in the same answer |
| Machine-readable structure | `pods --json` (plaintext, img, mathml, imagemap) |
| Provenance | trailing line: API + version + fetch time + query |

The provenance line is what makes a number checkable:

```
— Wolfram|Alpha full-results 2.6 · fetched 2026-10-22T05:25:48Z · query "capital of France"
```

`want short` deliberately stays a bare value so it can be piped; use `want ask`
when you need the date.

Which endpoint: one value → `short`; prose, tables or comparisons → `ask`; a few
specific sections → `pods`. `ask` output is already model-shaped (sections,
`|`-separated tables, `image: <url>` lines), so hand it to the agent as-is.

Pod titles are exact strings (`Physical properties`, `Indefinite integral`, …) and
`*` is the only wildcard, so `--primary` is usually cheaper than guessing a title.

## Install into an agent

Two commands, using each host's own installer — `want` ships no config file to copy.

### 1. Register the MCP server

```sh
# Claude Code — user scope, every project on this machine
claude mcp add -s user want -e WOLFRAM_APPID=XXXXXXXXXX -- want mcp

# opencode — writes the project config; add --global for user level
opencode mcp add want --env WOLFRAM_APPID=XXXXXXXXXX -- want mcp

# Codex — writes ~/.codex/config.toml
codex mcp add want --env WOLFRAM_APPID=XXXXXXXXXX -- want mcp
```

Each command is the host's documented way to add a stdio server, so the result is a
real entry in that host's config, not a file you maintain by hand.

```sh
claude mcp list        # want: want mcp - ✔ Connected
claude mcp get want    # show the resolved command and env
claude mcp remove want
opencode mcp list
```

Global options ride along in the server command: `want mcp --lang zh` delivers
Chinese answers inside the agent, `want mcp --cache-ttl 600` caches for ten minutes.

Undo it again:

```sh
claude mcp remove want     # Claude Code
opencode mcp list          # opencode: check what is there, then edit opencode.json
                           # (no remove subcommand); Codex: drop the
                           # [mcp_servers.want] table from ~/.codex/config.toml
```

### 2. Install the skill

The skill lives at `skills/want/SKILL.md`, the layout the
[skills.sh](https://skills.sh) CLI scans, so it installs like any published skill:

```sh
npx skills add <your-github>/want -g -a claude-code -a codex -a opencode
npx skills list          # confirm it landed
npx skills update want   # later: pull a newer copy
```

Without Node, or from a local checkout, the binary does the same thing:

```sh
want skill --install     # ~/.claude/skills/want + ~/.agents/skills/want
want skill --uninstall   # take it back out again
want skill --install --dir ./some/skills-root
want skill               # print it instead
```

`npx skills remove want` is the equivalent for a skill installed through the CLI.

Both paths land in the directories the agents actually read:

| Host | Skill directory it scans |
| --- | --- |
| Claude Code | `~/.claude/skills/` (project: `.claude/skills/`) |
| Codex | `~/.agents/skills/` (project: `.agents/skills/`) |
| opencode | `~/.agents/skills/`, `~/.config/opencode/skills/`, `~/.claude/skills/` |

### Remote / networked hosts

`want mcp` speaks stdio. From another machine, run it through SSH — every host that
accepts a command plus args works:

```sh
claude mcp add -s user want -e WOLFRAM_APPID=XXXXXXXXXX -- ssh myserver want mcp
```

or bridge stdio to HTTP and use the host's HTTP transport:

```sh
npx -y mcp-proxy --port 8080 -- want mcp
```

Inside a long-lived server, identical queries are cached for 5 minutes, two callers
asking the same thing at the same moment share one upstream request, and tool calls
run concurrently — a burst of questions neither serialises nor double-bills.

## Tools exposed over MCP

| Tool | Arguments | Use it for |
| --- | --- | --- |
| `want_ask` | `query`, `maxchars?` | prose, tables, comparisons, multi-part answers |
| `want_short` | `query`, `units?` | a single value; cheapest call |
| `want_pods` | `query`, `pod_titles?`, `primary?` | specific result sections, bounded output |

All three are annotated `readOnlyHint: true`, and the server's `instructions` carry
the query-rewriting rules (English, keyword form, one property per call) so a host
knows how to use them without extra prompting.

## Testing

```sh
cargo test                                  # 31 offline checks, no network
WOLFRAM_APPID=... cargo test -- --ignored   # 18 live checks, ~30 API calls
```

The offline suite covers CLI surface, skill frontmatter rules, the skill install
layout, the missing-AppID path, and the MCP protocol (initialize negotiation across
all revisions, `server/discover`, tool schemas, annotations, unknown method/tool,
malformed JSON resilience). The live suite hits all four endpoints plus natural
language, `--lang`, the provenance footer, `--primary`, `--pod`, `--json` image
subpods, units and the 501 error path.

## Development

```sh
cargo clippy --all-targets
cargo fmt --check
```

Dependencies: `rmcp` (official MCP Rust SDK), `reqwest`, `clap`, `tokio`,
`rustyline`, `tracing`, `home`, `serde`/`serde_json`, `anyhow`.

## Known limits

- No cache between CLI invocations; only `want mcp` keeps one (5 min default).
- `--lang` localises output only; the query itself must be English.
- `pods` keeps `plaintext` only; MathML and imagemap are not parsed.
- LLM API and Full Results API quotas are separate pools on commercial plans.

## License

MIT — see [LICENSE](./LICENSE).
