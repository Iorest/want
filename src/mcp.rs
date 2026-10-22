use std::sync::Arc;

use anyhow::{Result, anyhow};
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

use crate::client::{Client, Opts};

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AskArgs {
    /// Natural-language or keyword query in English.
    query: String,
    /// Character cap on the answer. Omit for the API default (~6800).
    maxchars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShortArgs {
    /// Short factual query in English, e.g. "speed of light".
    query: String,
    /// "metric" or "nonmetric".
    units: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PodsArgs {
    /// Short query in English, e.g. "Mars".
    query: String,
    /// Keep only pods whose title matches exactly, supports `*`. Omit for all pods.
    pod_titles: Option<Vec<String>>,
    /// Keep only the pod Wolfram|Alpha marks as primary. The cheapest way to bound the answer.
    primary: Option<bool>,
}

#[tool_router]
impl WantServer {
    #[tool(
        name = "want_ask",
        description = "Ask Wolfram|Alpha a question and get back prose, tables and image URLs \
                       already formatted for an LLM. Best for multi-part answers: comparisons, \
                       rankings, property tables, step results.",
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
                       -> '68.2 million'. Cheapest and fastest call; use it when one value answers \
                       the question.",
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
        // The API silently ignores an unknown units string, which would answer in
        // the caller-IP system while the agent believes it asked for another one.
        if let Some(units) = &args.units
            && !matches!(units.as_str(), "metric" | "nonmetric")
        {
            return Ok(tool_error(anyhow!(
                "units must be \"metric\" or \"nonmetric\", got {units:?}"
            )));
        }

        // A missing tool argument must not wipe the server-level --units.
        let opts = Opts {
            units: args.units.or_else(|| self.opts.units.clone()),
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
                       sections. Use when you need specific named sections (Result, Basic \
                       information, Properties, Timeline) or want to bound the output.",
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
        let titles = args.pod_titles.unwrap_or_default();
        match self
            .client
            .pods(
                &args.query,
                &self.opts,
                &titles,
                args.primary.unwrap_or(false),
            )
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
and curated factual data (people, places, chemistry, astronomy, history). Prefer these tools over \
your own recall for anything numeric or date-sensitive.

Query tips:
- Ask in plain English, the way a person would: 'how many people live in France' works.
  Fall back to keywords ('France population') only if the natural phrasing fails.
- Convert currency codes and units explicitly, e.g. '100 USD in EUR'.
- Ask for one property at a time; make several calls instead of one compound question.
- Use want_short for a single fact, want_ask for prose/tables, want_pods when you need several named sections.
- A failed call returns isError with the suggestion to try; retry once with it, then fall back
  to your own knowledge and say the lookup failed. Never invent the number."
)]
impl ServerHandler for WantServer {}

/// A failed lookup is a tool result, not a protocol error: hosts show `isError`
/// results to the model, which is what lets the agent recover instead of giving up.
///
/// The trailing guidance is the fallback ladder the agent should follow.
fn tool_error(err: anyhow::Error) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{err:#}\n\nNext step: do not resend the same query. If the message above suggests \
         alternative wording, retry once with it. If that also fails, answer from your own \
         knowledge and say the Wolfram|Alpha lookup failed. Never invent the number."
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
