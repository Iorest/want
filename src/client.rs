use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use tokio::sync::oneshot;

const LLM_URL: &str = "https://www.wolframalpha.com/api/v1/llm-api";
const SHORT_URL: &str = "https://api.wolframalpha.com/v1/result";
const SIMPLE_URL: &str = "https://api.wolframalpha.com/v1/simple";
const PODS_URL: &str = "https://api.wolframalpha.com/v2/query";

/// Why a call failed, so a caller can branch without parsing English.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The API could not interpret the query (HTTP 501 or `success: false`).
    Uninterpretable,
    /// Bad or missing AppID, or the monthly quota is gone.
    Access,
    /// Anything else: network, parsing, unexpected status.
    Other,
}

#[derive(Debug)]
pub struct Failure {
    kind: Kind,
    message: String,
}

impl Failure {
    fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

fn fail<T>(kind: Kind, message: impl Into<String>) -> Result<T> {
    Err(Failure::new(kind, message).into())
}

/// A 501 covers three very different failures, and only the body says which:
/// unparseable input (`Kind::Uninterpretable`), a server timeout, or a result the
/// LLM API could not render — the last one usually means `--maxchars` is too small.
/// Telling an agent to "rephrase" a query that was merely slow or too short is
/// wasted effort, so only the parse failure keeps that code.
fn classify_501(body: &str) -> (Kind, &'static str) {
    if body.contains("timeout") {
        return (
            Kind::Other,
            " (Wolfram|Alpha was too slow; the same query may work on a retry)",
        );
    }
    if body.contains("could not generate a result") {
        return (
            Kind::Other,
            " (the LLM API says this when the answer cannot fit --maxchars; try a larger value)",
        );
    }
    (Kind::Uninterpretable, "")
}

/// Query options that apply across endpoints.
#[derive(Default, Clone)]
pub struct Opts {
    /// `metric` or `nonmetric`; the API defaults by caller IP.
    pub units: Option<String>,
    /// Interpretation to force, taken from `didyoumeans` in a prior response.
    pub assumption: Option<String>,
    /// W|A `languagecode`, e.g. `zh`. Localises pod titles and answers.
    pub lang: Option<String>,
    /// Suppress the provenance footer.
    pub no_source: bool,
    /// Seconds to reuse an identical response; 0 disables caching.
    pub cache_ttl: u64,
}

impl Opts {
    /// Parameters shared by the endpoints that understand them.
    fn pairs(&self, assumption: bool) -> Vec<(&'static str, &str)> {
        let mut v = Vec::new();
        if let Some(u) = &self.units {
            v.push(("units", u.as_str()));
        }
        if assumption && let Some(a) = &self.assumption {
            v.push(("assumption", a.as_str()));
        }
        if let Some(l) = &self.lang {
            v.push(("languagecode", l.as_str()));
        }
        v
    }
}

/// Thin client over the Wolfram|Alpha HTTP endpoints `want` exposes.
///
/// One client lives for the whole process, so the connection pool inside
/// `reqwest::Client` is reused, identical queries are coalesced, and the cache
/// makes an agent asking the same thing twice free.
pub struct Client {
    http: reqwest::Client,
    appid: String,
    /// key -> (TTL clock, fetch time for the provenance line, body)
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// key -> callers waiting for the request that is already running
    inflight: Mutex<HashMap<String, Vec<oneshot::Sender<Shared>>>>,
}

type CacheEntry = (Instant, jiff::Timestamp, Vec<u8>);
/// What a finished request hands to everyone who was waiting on it.
type Shared = std::result::Result<(Vec<u8>, jiff::Timestamp), String>;

enum Role<'a> {
    /// First caller: runs the request and answers the followers.
    Leader(LeaderGuard<'a>),
    /// Later caller: waits for the leader instead of spending quota.
    Follower(oneshot::Receiver<Shared>),
}

/// Owns the in-flight slot for one key. Dropping it without publishing means the
/// leader was cancelled, so every follower is released with an error instead of
/// waiting forever.
struct LeaderGuard<'a> {
    client: &'a Client,
    key: String,
}

impl Drop for LeaderGuard<'_> {
    fn drop(&mut self) {
        let waiters = self
            .client
            .inflight
            .lock()
            .unwrap()
            .remove(&self.key)
            .unwrap_or_default();

        for waiter in waiters {
            let _ = waiter.send(Err("the request for this query was cancelled".to_owned()));
        }
    }
}

impl Client {
    pub fn new(appid: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("want/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .context("failed to build the HTTP client")?;

        Ok(Self {
            http,
            appid: appid.to_owned(),
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    /// GET with the AppID appended, mapping W|A's status codes onto typed errors.
    async fn send(&self, base: &str, params: &[(&str, &str)]) -> Result<reqwest::Response> {
        let mut url = reqwest::Url::parse(base).context("invalid URL")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("appid", &self.appid);
            for (k, v) in params {
                pairs.append_pair(k, v);
            }
        }

        let started = Instant::now();
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("request to {base} failed"))?;
        let status = resp.status().as_u16();
        // Never log the URL: it carries the AppID.
        tracing::debug!(
            base,
            status,
            ms = started.elapsed().as_millis(),
            "wolfram|alpha"
        );

        if status == 200 {
            return Ok(resp);
        }

        let body = resp.text().await.unwrap_or_default();
        match status {
            501 => {
                let (kind, hint) = classify_501(&body);
                let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
                fail(
                    kind,
                    format!("Wolfram|Alpha returned HTTP 501: {body}{hint}"),
                )
            }
            401 | 403 => fail(
                Kind::Access,
                format!(
                    "Invalid or missing AppID (HTTP {status}): {}. Store it with \
                     `want config --set-appid`, or pass --appid / WOLFRAM_APPID.",
                    body.trim()
                ),
            ),
            429 => fail(
                Kind::Access,
                format!("Wolfram|Alpha quota exceeded (HTTP 429): {}", body.trim()),
            ),
            _ => fail(
                Kind::Other,
                format!("Wolfram|Alpha returned HTTP {status}: {}", body.trim()),
            ),
        }
    }

    /// `send` plus body, memoised for `opts.cache_ttl` seconds and coalesced
    /// while in flight: two callers asking at the same moment cost one request.
    ///
    /// Returns the time the data was actually fetched, which a cache hit keeps:
    /// the provenance line must not claim a fresh fetch for an old answer.
    async fn get(
        &self,
        base: &str,
        params: &[(&str, &str)],
        opts: &Opts,
    ) -> Result<(Vec<u8>, jiff::Timestamp)> {
        let key = format!("{base}?{params:?}");

        if opts.cache_ttl > 0
            && let Some(hit) = self.cached(&key, opts.cache_ttl)
        {
            tracing::debug!(base, "cache hit");
            return Ok(hit);
        }

        match self.join(&key) {
            Role::Follower(rx) => match rx.await {
                Ok(Ok(shared)) => {
                    tracing::debug!(base, "joined an in-flight request");
                    Ok(shared)
                }
                Ok(Err(message)) => Err(anyhow!(message)),
                Err(_) => Err(anyhow!("the request for this query was cancelled")),
            },
            Role::Leader(guard) => {
                let result = match self.send(base, params).await {
                    Ok(resp) => match resp.bytes().await {
                        Ok(bytes) => Ok((bytes.to_vec(), jiff::Timestamp::now())),
                        Err(err) => Err(anyhow!("failed to read the response body: {err}")),
                    },
                    Err(err) => Err(err),
                };

                self.publish(&guard, &result, opts);
                result
            }
        }
    }

    fn cached(&self, key: &str, ttl: u64) -> Option<(Vec<u8>, jiff::Timestamp)> {
        let mut cache = self.cache.lock().unwrap();
        match cache.get(key) {
            Some((at, fetched, body)) if at.elapsed().as_secs() < ttl => {
                Some((body.clone(), *fetched))
            }
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    /// Become the leader for `key`, or register as a follower of the current one.
    fn join(&self, key: &str) -> Role<'_> {
        let mut inflight = self.inflight.lock().unwrap();
        match inflight.get_mut(key) {
            Some(waiters) => {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                Role::Follower(rx)
            }
            None => {
                inflight.insert(key.to_owned(), Vec::new());
                Role::Leader(LeaderGuard {
                    client: self,
                    key: key.to_owned(),
                })
            }
        }
    }

    /// Hand the outcome to every follower, then cache it.
    fn publish(
        &self,
        guard: &LeaderGuard<'_>,
        result: &Result<(Vec<u8>, jiff::Timestamp)>,
        opts: &Opts,
    ) {
        let key = guard.key.clone();
        let waiters = self
            .inflight
            .lock()
            .unwrap()
            .remove(&key)
            .unwrap_or_default();

        let shared = match result {
            Ok(value) => Ok(value.clone()),
            Err(err) => Err(format!("{err:#}")),
        };
        for waiter in waiters {
            let _ = waiter.send(shared.clone());
        }

        if opts.cache_ttl == 0 {
            return;
        }
        if let Ok((bytes, fetched)) = result {
            let mut cache = self.cache.lock().unwrap();
            // shortcut: entries are dropped on re-query or overflow, never on a
            // timer, so cap the map instead of growing for the process lifetime.
            if cache.len() >= 1024 {
                cache.clear();
            }
            cache.insert(key, (Instant::now(), *fetched, bytes.clone()));
        }
    }

    /// LLM API: prose and tables already shaped for a language model.
    ///
    /// Natural language goes in as-is — Wolfram|Alpha parses it, so
    /// "how many people live in France" works without rewriting.
    pub async fn llm(&self, input: &str, opts: &Opts, maxchars: Option<u32>) -> Result<String> {
        let maxchars = maxchars.map(|m| m.to_string());
        let mut params = vec![("input", input)];
        if let Some(m) = maxchars.as_deref() {
            params.push(("maxchars", m));
        }
        params.extend(opts.pairs(true));

        let (body, fetched) = self.get(LLM_URL, &params, opts).await?;
        let text = String::from_utf8(body).context("the answer was not valid UTF-8")?;
        Ok(annotate(
            "llm-api",
            input,
            opts,
            text.trim_end().to_owned(),
            None,
            fetched,
        ))
    }

    /// Short Answers API: a single line, cheapest call. Never annotated, so the
    /// output stays pipeable as a bare value.
    pub async fn short(&self, input: &str, opts: &Opts) -> Result<String> {
        // The API budgets 5s by default, which a localised answer can exceed; a
        // 501 from that budget looks like "could not interpret" but is just slow.
        let mut params = vec![("i", input), ("timeout", "20")];
        params.extend(opts.pairs(false));

        let (body, _) = self.get(SHORT_URL, &params, opts).await?;
        let text = String::from_utf8(body).context("the answer was not valid UTF-8")?;
        Ok(text.trim().to_owned())
    }

    /// Full Results API: structured pods, rendered as markdown for an agent.
    pub async fn pods(
        &self,
        input: &str,
        opts: &Opts,
        podtitles: &[String],
        primary_only: bool,
    ) -> Result<String> {
        let (body, fetched) = self.pods_raw(input, opts, podtitles).await?;
        let body = String::from_utf8(body).context("the response was not valid UTF-8")?;
        let parsed: Value =
            serde_json::from_str(&body).context("failed to parse the Full Results JSON")?;

        let version = parsed
            .get("queryresult")
            .and_then(|qr| qr.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let text = render_pods(input, &parsed, primary_only, !podtitles.is_empty())?;
        Ok(annotate(
            "full-results",
            input,
            opts,
            text,
            version.as_deref(),
            fetched,
        ))
    }

    /// Raw Full Results JSON, for callers that need every subpod type
    /// (plaintext, img, mathml, imagemap) instead of the markdown rendering.
    pub async fn pods_json(
        &self,
        input: &str,
        opts: &Opts,
        podtitles: &[String],
    ) -> Result<String> {
        let (body, _) = self.pods_raw(input, opts, podtitles).await?;
        String::from_utf8(body).context("the response was not valid UTF-8")
    }

    async fn pods_raw(
        &self,
        input: &str,
        opts: &Opts,
        podtitles: &[String],
    ) -> Result<(Vec<u8>, jiff::Timestamp)> {
        let mut params = vec![
            ("input", input),
            ("output", "json"),
            // plaintext alone silently drops image-only pods such as plots.
            ("format", "plaintext,image"),
        ];
        params.extend(opts.pairs(true));
        params.extend(podtitles.iter().map(|t| ("podtitle", t.as_str())));

        self.get(PODS_URL, &params, opts).await
    }

    /// Simple API: one rendered image. Returns the bytes plus a file extension.
    pub async fn image(&self, input: &str, opts: &Opts) -> Result<(Vec<u8>, &'static str)> {
        let mut params = vec![("i", input)];
        params.extend(opts.pairs(false));

        let (bytes, _) = self.get(SIMPLE_URL, &params, opts).await?;
        // Sniff the magic bytes: the response headers are gone once it is cached.
        let ext = if bytes.starts_with(b"GIF8") {
            "gif"
        } else if bytes.starts_with(b"\x89PNG") {
            "png"
        } else if bytes.starts_with(b"\xff\xd8") {
            "jpg"
        } else {
            "gif"
        };

        Ok((bytes, ext))
    }
}

/// Append the provenance line that makes a claim attributable and dated.
fn annotate(
    endpoint: &str,
    query: &str,
    opts: &Opts,
    mut text: String,
    api_version: Option<&str>,
    fetched: jiff::Timestamp,
) -> String {
    if opts.no_source {
        return text;
    }

    text.push_str(&format!(
        "\n\n— Wolfram|Alpha {endpoint}{} · fetched {} · query \"{query}\"",
        api_version.map(|v| format!(" {v}")).unwrap_or_default(),
        fetched,
    ));
    text
}

fn render_pods(input: &str, body: &Value, primary_only: bool, filtered: bool) -> Result<String> {
    let qr = body
        .get("queryresult")
        .context("response has no queryresult")?;

    // `error` is a bool attribute on success and an object when the query failed.
    if let Some(err) = qr.get("error").filter(|e| e.is_object()) {
        return fail(
            Kind::Other,
            format!("Wolfram|Alpha rejected the query: {err}"),
        );
    }

    if !qr.get("success").and_then(Value::as_bool).unwrap_or(false) {
        let hints: Vec<String> = ["didyoumeans", "tips", "futuretopic", "reinterpret"]
            .iter()
            .filter_map(|k| qr.get(*k).map(|v| format!("{k}: {v}")))
            .collect();
        let mut msg = format!("No result for: {input}");
        if !hints.is_empty() {
            msg.push('\n');
            msg.push_str(&hints.join("\n"));
        }
        return fail(Kind::Uninterpretable, msg);
    }

    let pods: Vec<&Value> = qr
        .get("pods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| !primary_only || p.get("primary").and_then(Value::as_bool).unwrap_or(false))
        .collect();

    // W|A answers 200 with `numpods: 0` and no `pods` key, so explain which
    // filter emptied the result instead of a generic "no content".
    if pods.is_empty() {
        if primary_only && !filtered {
            return fail(
                Kind::Uninterpretable,
                format!("No section is marked primary for: {input}"),
            );
        }
        if filtered {
            return fail(
                Kind::Uninterpretable,
                "No section matched the given --pod title(s). Titles must match exactly \
                 (a trailing * is a wildcard); drop --pod to see every section.",
            );
        }
        return fail(
            Kind::Uninterpretable,
            format!("Wolfram|Alpha returned no sections for: {input}"),
        );
    }

    let mut out = String::new();
    for pod in pods {
        let mut lines: Vec<String> = Vec::new();

        for subpod in pod
            .get("subpods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = subpod.get("plaintext").and_then(Value::as_str) {
                let text = text.trim();
                if !text.is_empty() {
                    lines.push(text.to_owned());
                }
            }

            // Plots, diagrams and periodic tables only exist as images, so keep
            // the URL for a multimodal agent even when plaintext is empty.
            if let Some(src) = subpod
                .get("img")
                .and_then(|img| img.get("src"))
                .and_then(Value::as_str)
            {
                let alt = subpod
                    .get("img")
                    .and_then(|img| img.get("alt"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                lines.push(if alt.is_empty() {
                    format!("image: {src}")
                } else {
                    format!("image: {src} ({alt})")
                });
            }
        }

        if lines.is_empty() {
            continue;
        }

        out.push_str("## ");
        out.push_str(
            pod.get("title")
                .and_then(Value::as_str)
                .unwrap_or("(untitled)"),
        );
        out.push('\n');
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }

    if out.is_empty() {
        return fail(
            Kind::Uninterpretable,
            format!("Query returned no readable content: {input}"),
        );
    }

    Ok(out.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::render_pods;
    use serde_json::json;

    #[test]
    fn renders_pods_as_markdown() {
        let body = json!({"queryresult": {"success": true, "pods": [
            {"title": "Input", "subpods": [{"plaintext": "2 + 2"}]},
            {"title": "Result", "subpods": [{"plaintext": "4"}]},
            {"title": "Empty", "subpods": [{"plaintext": ""}]},
        ]}});

        assert_eq!(
            render_pods("2+2", &body, false, false).unwrap(),
            "## Input\n2 + 2\n\n## Result\n4"
        );
    }

    #[test]
    fn keeps_image_urls_from_subpods() {
        let body = json!({"queryresult": {"success": true, "pods": [
            {"title": "Plot", "subpods": [{
                "plaintext": "",
                "img": {"src": "https://example.test/plot.gif", "alt": "plot of sin(x)"}
            }]},
        ]}});

        assert_eq!(
            render_pods("plot", &body, false, false).unwrap(),
            "## Plot\nimage: https://example.test/plot.gif (plot of sin(x))"
        );
    }

    #[test]
    fn reports_uninterpreted_query_with_hints() {
        let body = json!({"queryresult": {
            "success": false,
            "didyoumeans": {"didyoumean": [{"val": "rain"}]},
        }});

        let err = render_pods("rian", &body, false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("rian"), "{err}");
        assert!(err.contains("rain"), "{err}");
    }

    #[test]
    fn a_501_is_classified_from_its_body() {
        let slow = super::classify_501("could not give a response in time for timeout=5");
        assert_eq!(slow.0, super::Kind::Other);
        assert!(slow.1.contains("retry"), "{}", slow.1);

        let short = super::classify_501("could not generate a result for this input");
        assert_eq!(short.0, super::Kind::Other);
        assert!(short.1.contains("maxchars"), "{}", short.1);

        let unclear = super::classify_501("did not understand your input");
        assert_eq!(unclear.0, super::Kind::Uninterpretable);
        assert!(unclear.1.is_empty());
    }

    #[test]
    fn uninterpreted_queries_are_typed() {
        let body = json!({"queryresult": {"success": false}});
        let err = render_pods("zzz", &body, false, false).unwrap_err();
        let failure = err.downcast_ref::<super::Failure>().expect("typed error");
        assert_eq!(failure.kind(), super::Kind::Uninterpretable);
    }

    #[test]
    fn primary_filter_keeps_only_the_primary_pod() {
        let body = json!({"queryresult": {"success": true, "pods": [
            {"title": "Result", "primary": true, "subpods": [{"plaintext": "4"}]},
            {"title": "Plot", "subpods": [{"plaintext": "chart"}]},
        ]}});

        assert_eq!(
            render_pods("2+2", &body, true, false).unwrap(),
            "## Result\n4"
        );
    }

    #[test]
    fn explains_an_empty_pod_filter() {
        // The API answers 200 with numpods: 0 and no `pods` key.
        let body = json!({"queryresult": {"success": true, "numpods": 0}});

        let err = render_pods("Mars", &body, false, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--pod"), "{err}");
    }
}
