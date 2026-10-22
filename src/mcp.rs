use std::sync::Arc;

use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars,
    service::ServerInitializeError,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;

use crate::client::{self, Client, Opts};

#[derive(Clone)]
pub struct WantServer {
    client: Arc<Client>,
    opts: Opts,
}

impl WantServer {
    pub fn new(client: Arc<Client>, opts: Opts) -> Self {
        Self { client, opts }
    }
}

// Spelled as the API spells it, so the schema can list the two values. Every
// argument struct also denies unknown fields: one the server silently ignored
// used to look like it worked, and the CLI names the same things differently
// (`--pod` against `pod_titles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Metric,
    Nonmetric,
}

impl Units {
    fn as_str(self) -> &'static str {
        match self {
            Units::Metric => "metric",
            Units::Nonmetric => "nonmetric",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskArgs {
    /// Natural-language or keyword query in English.
    query: String,
    /// Character cap on the answer. Omit for the API default (~6800).
    #[schemars(range(min = 1))]
    maxchars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShortArgs {
    /// Short factual query in English, e.g. "speed of light".
    query: String,
    /// Unit system. Omit to use the server default.
    units: Option<Units>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PodsArgs {
    /// Short query in English, e.g. "Mars".
    query: String,
    /// Keep only pods whose title matches exactly, supports `*`. Omit for all pods.
    #[schemars(length(min = 1))]
    pod_titles: Option<Vec<String>>,
    /// Keep only the pod Wolfram|Alpha marks as primary. Returns less text, but a
    /// pods call still costs one API call either way.
    primary: Option<bool>,
    /// Interpretation to force, e.g. `*C.chicken-_*ExpandedFood-`, taken from a
    /// previous answer's "To use as ... set assumption=" line. This is the only way
    /// to disambiguate: the LLM API behind want_ask ignores this parameter. Check
    /// the answer's first section; if it still shows the default meaning, the
    /// value was not applied.
    assumption: Option<String>,
}

#[tool_router]
impl WantServer {
    #[tool(
        name = "want_ask",
        description = "Ask Wolfram|Alpha a question and get back prose, tables and image URLs \
                       already formatted for an LLM. Best for multi-part answers: comparisons, \
                       rankings, property tables, step results. Wolfram|Alpha picks a curated \
                       subset of sections here; use want_pods when every section matters.",
        annotations(
            title = "Wolfram|Alpha answer",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn want_ask(
        &self,
        Parameters(args): Parameters<AskArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(bad) = check_query(&args.query) {
            return Ok(bad);
        }
        if args.maxchars == Some(0) {
            return Ok(arg_error(
                "`maxchars` must be at least 1; omit it for the API default",
            ));
        }

        match self
            .client
            .llm(&args.query, &self.opts, args.maxchars)
            .await
        {
            Ok(text) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
            Err(err) => Ok(tool_error(err)),
        }
    }

    #[tool(
        name = "want_short",
        description = "Get a single short answer from Wolfram|Alpha, e.g. 'population of France' \
                       -> '68.2 million'. The smallest and fastest call; use it when one value \
                       answers the question.",
        annotations(
            title = "Wolfram|Alpha short answer",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn want_short(
        &self,
        Parameters(args): Parameters<ShortArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(bad) = check_query(&args.query) {
            return Ok(bad);
        }

        // A missing tool argument must not wipe the server-level --units.
        let opts = Opts {
            units: args
                .units
                .map(|u| u.as_str().to_owned())
                .or_else(|| self.opts.units.clone()),
            ..self.opts.clone()
        };
        match self.client.short(&args.query, &opts).await {
            Ok(text) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
            Err(err) => Ok(tool_error(err)),
        }
    }

    #[tool(
        name = "want_pods",
        description = "Fetch the structured Wolfram|Alpha result pods for a query as markdown \
                       sections. Returns every section Wolfram|Alpha has, or only the ones you \
                       name. Use it when the user wants the complete card, when want_ask left \
                       sections out, or to force an interpretation with `assumption`.",
        annotations(
            title = "Wolfram|Alpha result sections",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn want_pods(
        &self,
        Parameters(args): Parameters<PodsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(bad) = check_query(&args.query) {
            return Ok(bad);
        }
        if let Some(titles) = &args.pod_titles {
            if titles.is_empty() {
                return Ok(arg_error(
                    "`pod_titles` must not be empty; omit it to get every section",
                ));
            }
            if titles.iter().any(|title| client::is_blank(title)) {
                return Ok(arg_error(
                    "`pod_titles` contains an empty title; remove it or omit the filter",
                ));
            }
        }
        // An empty assumption is not a forced interpretation; the API ignores it
        // and answers with the default meaning while the agent believes otherwise.
        if args
            .assumption
            .as_ref()
            .is_some_and(|a| client::is_blank(a))
        {
            return Ok(arg_error(
                "`assumption` must not be empty; omit it for the default interpretation",
            ));
        }

        let titles = args.pod_titles.unwrap_or_default();
        let opts = Opts {
            assumption: args.assumption,
            ..self.opts.clone()
        };
        match self
            .client
            .pods(&args.query, &opts, &titles, args.primary.unwrap_or(false))
            .await
        {
            Ok(text) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
            Err(err) => Ok(tool_error(err)),
        }
    }
}

#[tool_handler(
    name = "want",
    instructions = "\
Tools backed by Wolfram|Alpha: exact math, unit and currency conversion, physical constants, \
and curated factual data (people, places, chemistry, astronomy, history). Recall is unreliable for \
numbers, units and dated facts, so compute them here instead of guessing.

Query tips:
- Ask in plain English, the way a person would: 'how many people live in France' works.
  Fall back to keywords ('France population') only if the natural phrasing fails.
- Convert currency codes and units explicitly, e.g. '100 USD in EUR'.
- Ask for one property at a time; make several calls instead of one compound question.
- want_short answers a single value. want_ask returns a digest: Wolfram|Alpha picks the
  sections, so parts of the full card can be missing. want_pods returns every section, or
  just the ones you name, and is the right call when the user asks for everything.
- want_ask and want_pods end with the time the answer was fetched; cite it when the value
  can change. want_short returns a bare value with no date.
- Resending the identical query is free, because it is cached; rewording costs a call.
- A failed call returns isError with a Next step; follow it, and never invent the number.
- Ambiguous input (e.g. 'chicken' = animal, food, or movie) comes back with lines like
  `To use as a food set assumption=*C.chicken-_*ExpandedFood-`. Call want_pods with the
  same query and that value in `assumption`. want_ask cannot take an assumption, so do
  not reword the query hoping for the other meaning."
)]
impl ServerHandler for WantServer {}

/// Refuse an argument before spending a call, and say which one to fix.
///
/// An argument error is the caller's mistake, not a failed lookup: the advice of
/// "reword the query" would send the agent down the wrong path.
fn arg_error(message: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "Invalid tool argument: {message}. Fix the argument and call again; \
nothing was sent, so no quota was spent."
    ))])
}

/// An empty query costs a call and comes back as an API error written for a
/// developer ("Query Parameter i is required"), so refuse it here.
fn check_query(query: &str) -> Option<CallToolResult> {
    client::is_blank(query).then(|| arg_error("`query` must not be empty"))
}

/// A failed lookup is a tool result, not a protocol error: hosts show `isError`
/// results to the model, which is what lets the agent recover instead of giving up.
///
/// The guidance has to agree with the skill: an identical resend is cached and
/// free, rewording costs a call, and an auth or quota failure will not change no
/// matter how the question is phrased.
fn tool_error(err: anyhow::Error) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{err:#}\n\nNext step: an auth or quota error will not change, so tell the user and \
         stop. Otherwise resending the identical query is free (the server caches it) and \
         fixes a timeout or a missing section; if the message names a change to make, make \
         it once. If it still fails, answer from your own knowledge and say the \
         Wolfram|Alpha lookup failed. Never invent the number."
    ))])
}

/// Serve MCP over stdio.
pub async fn serve(client: Arc<Client>, opts: &Opts) -> Result<()> {
    let service = match WantServer::new(client, opts.clone()).serve(stdio()).await {
        Ok(service) => service,
        // A host that spawns the server and closes stdin without initializing is
        // normal: health checks do it, and the 2026-07-28 revision never sends
        // `initialize` at all. Nothing failed.
        Err(ServerInitializeError::ConnectionClosed(_) | ServerInitializeError::Cancelled) => {
            tracing::debug!("client disconnected before initializing");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    service.waiting().await?;
    Ok(())
}
