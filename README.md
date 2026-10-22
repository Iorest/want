# want

**Wolfram|Alpha Network Tool**: a Rust CLI for Wolfram|Alpha that also runs as an
MCP server and ships an agent skill.

[中文](./README.zh-CN.md)

Borrowed from [`wolfram-cli`](https://github.com/ConnorGray/wolfram-cli), which wraps a
local WolframKernel over WSTP. `want` calls the Wolfram|Alpha HTTP API instead, so it
needs no Wolfram installation.

| Entry point | Command | For |
| --- | --- | --- |
| CLI / REPL | `want ask "..."`, bare `want` | people |
| MCP server | `want mcp` | agents; stdio, exposes `want_ask` / `want_short` / `want_pods` |
| Skill | `want skill --install` | agents; teaches *when* to call Wolfram\|Alpha |

## Install

```sh
cargo install --path .      # installs ~/.cargo/bin/want
want --version
```

The agent setups below launch the server as `want`, so put the binary on `PATH`
first.

## Get an AppID

The API is free for non-commercial use but always needs an AppID.

1. Open <https://developer.wolframalpha.com> and sign in with a Wolfram ID. If you
   have none, register with any email; it takes one confirmation email and no
   payment details.
2. Go to **My Apps** and press **Sign up to get your first AppID**.
3. Answer the one-time survey about intended usage. Personal, research or internal
   tooling counts as non-commercial.
4. In the dialog, give the application a name and a one-line description. The AppID
   appears immediately, shaped like `X7WEHY-W45KYJL3C`.
5. Store it once:

   ```sh
   want config --set-appid 'X7WEHY-W45KYJL3C'
   want short "population of France"
   ```

   That writes `~/.config/want/config.toml` with mode 0600, so a new terminal works
   without touching a shell profile. `want config` shows which key is in use, where
   it came from, and masks the key itself.

   A one-off shell or command still works:

   ```sh
   export WOLFRAM_APPID='X7WEHY-W45KYJL3C'   # this shell only
   want --appid 'X7WEHY-W45KYJL3C' short "population of France"   # this call only
   ```

Notes:

- The free tier is 2,000 calls per month, shared by all Wolfram|Alpha APIs, so
  `ask`, `short`, `pods` and `image` all draw from the same pool.
- Commercial use needs a separate licence. The pricing page only has a sales
  contact form.
- `DEMO` is no longer accepted (it returns HTTP 401). `want` refuses to start
  without a real AppID rather than sending a request that cannot succeed.
- Resolution order: `--appid`, then `WOLFRAM_APPID`, then
  `~/.config/want/config.toml`. `want config` prints which one won.

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
want config                                  # which AppID, and where it came from
want skill                                   # print the skill
want skill --install                         # or write it where agents look
```

Global options:

- `--appid <id>` overrides both the environment and the config file
- `--lang <code>` sets the answer language and pod titles (`zh`, `de`, `fr`, …); the
  input stays English
- `--units metric|nonmetric` sets the unit system, defaulting by caller IP
- `--no-source` drops the provenance footer
- `--cache-ttl <secs>` reuses an identical response for N seconds (default 0 for
  one-shot commands, 300 for the REPL and `want mcp`)
- `--assumption <value>` (`pods` only) forces an interpretation, using a value from
  the `assumption=` lines of a previous answer. The LLM API behind `ask` ignores this
  parameter, so `pods` is how you ask about another meaning of a word
- `-v` / `-vv` set the stderr log level; `RUST_LOG` takes precedence

Exit codes:

| Code | Meaning |
| --- | --- |
| 0 | answer printed |
| 1 | anything else (network, parse, unexpected status) |
| 2 | usage error (bad flag, bad `--units`) |
| 3 | query not understood, rephrase it |
| 4 | auth or quota, fix the AppID or wait for the month to roll over |

A script can branch on these without parsing the message.

`want image` never overwrites an earlier picture. Without `-o` it writes `want.gif`,
then `want-1.gif`, and so on.

## Natural language

Wolfram|Alpha parses questions, so send the question rather than a keyword soup:

```sh
want ask "how many people live in France"
want short "how tall is Mount Everest in feet"
want ask "what is the GDP of Japan in 2025"
```

Keyword form (`France population`) still works and is the fallback when a long
sentence is not understood. Input is English; a non-English question comes back as
HTTP 501 with suggested English phrasings. `--lang` localises the answer, not the
question.

## What an answer contains

| Type | Where it shows up |
| --- | --- |
| Prose and `##` sections | `ask`, `pods` |
| Tables (`\|`-separated rows) | `ask`, `pods` |
| Plots and diagrams | `image: <url>` lines in `pods`, plus `--json` subpods |
| Unit conversions, timelines, comparisons | additional sections in the same answer |
| Machine-readable structure | `pods --json` (plaintext, img, mathml, imagemap) |
| Provenance | trailing line: API, version, fetch time, query |

The provenance line makes a number checkable:

```
— Wolfram|Alpha full-results 2.6 · fetched 2026-10-22T05:25:48Z · query "capital of France"
```

`want short` deliberately stays a bare value so it can be piped. Use `want ask` when
you need the date.

Which endpoint: one value goes to `short`, prose, tables or comparisons to `ask`, a
few specific sections to `pods`. `ask` output is already model-shaped (sections,
`|`-separated tables, `image: <url>` lines), so hand it to the agent as-is.

Pod titles are exact strings (`Physical properties`, `Indefinite integral`, …) and
`*` is the only wildcard, so `--primary` is usually cheaper than guessing a title.

## Install into an agent

Two steps, each using the client's own installer. There is no config template to
copy.

### 1. Register the MCP server

```sh
# Claude Code: user scope, every project on this machine
claude mcp add -s user want -- want mcp

# opencode: writes the project config; add --global for user level
opencode mcp add want -- want mcp

# Codex: writes ~/.codex/config.toml
codex mcp add want -- want mcp
```

Each command is the client's documented way to add a stdio server, so the entry
lands in that client's own config.

The server reads the AppID from `~/.config/want/config.toml`, so no environment
variable is needed here. To keep the key off disk instead, add the flag form:
`-e WOLFRAM_APPID=XXXXXXXXXX` for Claude Code, `--env WOLFRAM_APPID=XXXXXXXXXX` for
opencode and Codex.

```sh
claude mcp list        # want: want mcp - ✔ Connected
claude mcp get want    # show the resolved command and env
claude mcp remove want
opencode mcp list
```

Global options ride along in the server command: `want mcp --lang zh` gives Chinese
answers inside the agent, `want mcp --cache-ttl 600` caches for ten minutes.

To undo:

```sh
claude mcp remove want     # Claude Code
opencode mcp list          # opencode has no remove subcommand; edit opencode.json
                           # Codex: drop the [mcp_servers.want] table from
                           # ~/.codex/config.toml
```

### 2. Install the skill

The skill lives at `skills/want/SKILL.md`, the layout the
[skills.sh](https://skills.sh) CLI scans, so it installs like any published skill:

```sh
npx skills add Iorest/want -g -a claude-code -a codex -a opencode
npx skills list          # confirm it landed
npx skills update want   # later: pull a newer copy
```

Without Node, or from a local checkout, the binary does the same thing:

```sh
want skill --install     # ~/.claude/skills/want + ~/.agents/skills/want
want skill --uninstall   # take it back out
want skill --install --dir ./some/skills-root
want skill               # print it instead
```

`npx skills remove want` is the equivalent for a skill installed through the CLI.

Both paths land in the directories the agents read:

| Client | Skill directory it scans |
| --- | --- |
| Claude Code | `~/.claude/skills/` (project: `.claude/skills/`) |
| Codex | `~/.agents/skills/` (project: `.agents/skills/`) |
| opencode | `~/.agents/skills/`, `~/.config/opencode/skills/`, `~/.claude/skills/` |

### Remote hosts

`want mcp` speaks stdio. From another machine, run it through SSH. Any client that
accepts a command and its arguments works:

```sh
claude mcp add -s user want -- ssh myserver want mcp
```

The `-e`/`--env` form does not reach the remote process over SSH, so run
`want config --set-appid <id>` once on `myserver`, or put the key inside the remote
command: `-- ssh myserver want --appid XXXXXXXXXX mcp`.

Another option is to bridge stdio to HTTP and use the client's HTTP transport:

```sh
npx -y mcp-proxy --port 8080 -- want mcp
```

Inside a long-lived server, identical queries are cached for 5 minutes, two callers
asking the same thing at the same moment share one upstream request, and tool calls
run concurrently. A burst of questions neither serialises nor double-bills.

## Tools exposed over MCP

| Tool | Arguments | Use it for |
| --- | --- | --- |
| `want_ask` | `query`, `maxchars?` | prose, tables, comparisons, multi-part answers. A digest: Wolfram\|Alpha picks which sections fit |
| `want_short` | `query`, `units?` | a single value; the cheapest call |
| `want_pods` | `query`, `pod_titles?`, `primary?`, `assumption?` | every section, specific sections, or a forced interpretation |

All three are annotated `readOnlyHint: true`. The server's `instructions` carry the
query rules (English, natural phrasing first, one property per call), so a client
knows how to use them without extra prompting.

## Testing

```sh
cargo test                                  # 36 offline checks, no network
WOLFRAM_APPID=... cargo test -- --ignored   # 19 live checks, ~30 API calls
```

The offline suite covers the CLI surface, skill frontmatter rules, the skill install
layout, the missing-AppID path, and the MCP protocol (initialize negotiation across
all revisions, `server/discover`, tool schemas, annotations, unknown method and
tool, malformed JSON). The live suite hits all four endpoints plus natural language,
`--lang`, the provenance footer, `--primary`, `--pod`, `--json` image subpods, units
and the 501 error path.

## Development

```sh
cargo clippy --all-targets
cargo fmt --check
```

Dependencies: `rmcp` (official MCP Rust SDK), `reqwest`, `clap`, `tokio`,
`rustyline`, `tracing`/`tracing-subscriber`, `toml`, `jiff`, `home`,
`serde`/`serde_json`, `anyhow`.

## Known limits

- One-shot commands keep no cache. The REPL and `want mcp` cache for 5 minutes by
  default.
- `--lang` localises output only; the query itself must be English.
- `pods` returns text and image URLs. MathML and imagemap are not parsed.
- On commercial plans the LLM API and Full Results API draw from separate quotas.

## License

MIT, see [LICENSE](./LICENSE).
