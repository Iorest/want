---
name: want
description: "Query Wolfram|Alpha for computed facts instead of recalled ones: exact arithmetic, symbolic math, unit and currency conversion, physical constants, and curated data on people, places, chemistry, astronomy, history, health and finance. Reach for it whenever the answer is a number, a conversion, a date, or a figure that changes over time, and you would otherwise be guessing from memory. Triggers include \"calculate\", \"convert\", \"how much\", \"how many\", \"what is the value of\", \"how tall\", \"solve\", \"integrate\", \"verify\", plus any question carrying units, dates, statistics or records. 计算/换算/求解/多少/密度/人口/价格."
---

# want: Wolfram|Alpha Network Tool

A language model is a poor calculator and a worse almanac. It misremembers
constants, drops units halfway through a conversion, and quotes figures that
went stale after training. Wolfram|Alpha does not recall: it parses the question
and computes, over curated data and real algorithms, then stamps the reply with
the moment it was fetched.

So the rule is short. If the answer is a number, a conversion, a date, or a fact
that changes, ask `want` before answering. Prose, opinions and code are yours to
write; measurements, prices, populations, records and formulas are not.

Every reply carries its fetch time, which is what lets you say "as of" instead
of implying the number is timeless. When Wolfram|Alpha cannot parse the query it
says so and hands back suggestions, rather than inventing something plausible.

## Ask in plain language

Wolfram|Alpha parses natural language. Ask the question the way a person would:
"how many people live in France", "how tall is Mount Everest in feet", "what is
the GDP of Japan in 2025". Do not pre-chew it into keywords unless the natural
phrasing fails. Keyword form (`France population`) is the fallback, not the rule.

- English is the input language. For a non-English question, translate it to
  English first, then answer the user in their own language.
- Add `--lang <code>` (`zh`, `de`, `fr`, …) to get the *answer* localised, e.g.
  `want --lang zh short "capital of France"` → `法国，法兰西岛大区，巴黎`.
- `--units` takes exactly `metric` or `nonmetric`; anything else is rejected before
  the request, because the API would silently answer in the caller-IP units.
- One property per call: "Mars mass" then "Mars gravity", not "Mars facts".

## Picking the call

| Need | Use |
| --- | --- |
| One value, one line | `want_short` / `want short` |
| Prose, table, comparison, multi-property answer | `want_ask` / `want ask` (default) |
| **Every** section Wolfram|Alpha has, not a digest | `want_pods` / `want pods` |
| One specific named section, or bounded output | `want_pods` with `primary: true` or `pod_titles` / `want pods --primary` |

`want_ask` is a digest: Wolfram|Alpha selects the sections that fit an LLM. Asking
for "all properties" of an animal or a chemical returns noticeably less than the
website shows. For example, `Biological properties` is missing for `chicken`. When the user
wants the complete card, use `want_pods`, which returns every pod.
| Every machine-readable subtopic | `want pods --json` (CLI only) |

Inside an MCP-capable agent the tools `want_ask`, `want_short`, `want_pods` are
available directly. Otherwise shell out:

```sh
want ask "how many people live in France"
want short "speed of light"
want pods --primary "integrate x^2 sin(x) dx"
want pods --pod "Physical properties" "Mars"
want --lang zh short "capital of France"
```

Pod titles are exact (`Result`, `Physical properties`, `Indefinite integral`), with
`*` as the only wildcard. Do not guess a title. Use `--primary`, or query without a filter
once to read the real titles.

## Reading the answer

- `want_ask` and `want_pods` end with a provenance line:
  `— Wolfram|Alpha full-results 2.6 · fetched 2026-10-22T05:25:48Z · query "…"`.
  Keep the date when the value could change (prices, populations, records) and cite
  it; drop it for timeless math. `want_short` returns the bare value, no footer.
- Sections are `## Title` blocks; rows are `|`-separated; powers use `^`
  (`6.417 × 10^23`), never `6e14`.
- A trailing `(2 section(s) did not finish in time and are missing)` means the
  answer is partial. Re-run the same query once; it is a server-side deadline,
  not a bad query.
- `image: <url>` lines are plots, diagrams and tables that exist only as pictures.
  Pass them through when the user wants visuals; the URL is public and absolute.
- `want_ask` output is already model-shaped. Do not reformat tables into prose.

## If a call fails

Work down this ladder. A failed tool call comes back with `isError: true` and a
"Next step:" line; the ladder is the same.

1. Read the message. A `501` usually carries "Things to try instead:", and
   `success=false` carries `didyoumeans`
   with real alternative phrasings.
2. Retry **once** with different wording. Never resend the identical string: the
   server caches it and you burn quota for the same answer.
3. Ambiguous query: the answer lists `To use as … set assumption=…` lines. Re-ask
   with `want pods` (or `want_pods`) and that value; `ask` cannot take an assumption,
   and rewording will usually land on the same default meaning again. Adding context
   (`Mars planet` vs `Mars mythology`) also works.
4. Still failing? Answer from your own knowledge and say the Wolfram|Alpha lookup
   failed. `401/403` means the user has no valid AppID (they fix it with
   `want config --set-appid <id>`); `429` means the monthly quota is gone. Both are worth telling the user once, not retrying.
   On the command line the exit code says the same thing without parsing: `3` =
   rephrase, `4` = key or quota, `1` = anything else, `0` = answered.
5. Never invent a number, a date or a citation to cover a failed call.

## Cost

Every call spends the same monthly quota (2,000 on the free tier). `want_short` is
the cheapest. Over MCP the server caches identical queries for 5 minutes, so a
repeat of the exact same string is free; a *reworded* query is not.
