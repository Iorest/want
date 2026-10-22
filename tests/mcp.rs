//! MCP protocol checks, driving `want mcp` over a real stdio session.
//!
//! Offline tests run with `cargo test`. The `#[ignore]` tests call the API and
//! need `WOLFRAM_APPID`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Spawn a server, send `lines`, and read exactly `expected` replies.
///
/// Reading before closing stdin matters: the server shuts down on EOF, which
/// would cancel a tool call that is still waiting on the network.
fn run_server(args: &[&str], appid: Option<&str>, lines: &[String], expected: usize) -> Vec<Value> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_want"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    match appid {
        Some(appid) => {
            command.env("WOLFRAM_APPID", appid);
        }
        None => {
            command
                .args(["--appid", "protocol-test"])
                .env_remove("WOLFRAM_APPID");
        }
    }

    let mut child = command.spawn().expect("failed to spawn want mcp");

    {
        let stdin = child.stdin.as_mut().expect("stdin is piped");
        for line in lines {
            writeln!(stdin, "{line}").expect("failed to write a request");
        }
    }

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut reader = BufReader::new(stdout);
    let mut responses = Vec::with_capacity(expected);

    for _ in 0..expected {
        let mut line = String::new();
        reader.read_line(&mut line).expect("failed to read a reply");
        assert!(
            !line.is_empty(),
            "server exited before sending all replies; got {responses:#?}"
        );
        responses.push(serde_json::from_str(&line).expect("one JSON message per line"));
    }

    drop(child.stdin.take());
    let _ = child.wait();
    responses
}

/// Protocol-only session with a bogus AppID: tool calls fail, schema does not.
fn session(lines: &[String], expected: usize) -> Vec<Value> {
    run_server(&["mcp"], None, lines, expected)
}

fn live(lines: &[String], expected: usize, args: &[&str]) -> Vec<Value> {
    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live tests");
    run_server(args, Some(&appid), lines, expected)
}

fn init(id: u64, version: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "want-tests", "version": "1"}
        }
    })
    .to_string()
}

const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

/// `initialize` + `notifications/initialized`, the handshake every host sends.
fn open(version: &str) -> Vec<String> {
    vec![init(1, version), INITIALIZED.to_owned()]
}

fn response(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("no response for id {id}: {responses:#?}"))
}

fn call(id: u64, tool: &str, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    })
    .to_string()
}

//==========================================================
// Offline
//==========================================================

#[test]
fn initialize_reports_server_identity() {
    let responses = session(&open("2025-06-18"), 1);
    let result = &response(&responses, 1)["result"];

    assert_eq!(result["serverInfo"]["name"], "want");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["capabilities"]["tools"], json!({}));
    assert!(
        result["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("want_short")),
        "instructions should teach the host which tool to pick"
    );
}

#[test]
fn negotiates_every_version_that_still_has_initialize() {
    for version in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
        let responses = session(&open(version), 1);
        let result = &response(&responses, 1)["result"];
        assert_eq!(
            result["protocolVersion"],
            json!(version),
            "version {version}"
        );
    }
}

#[test]
fn supports_the_stateless_2026_revision() {
    // 2026-07-28 dropped the initialize handshake for `server/discover` plus
    // per-request `_meta`, so this is a second, version-less entry path.
    let lines = vec![
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {}
            }}
        })
        .to_string(),
    ];

    let responses = session(&lines, 1);
    let result = &response(&responses, 1)["result"];

    assert!(
        result["supportedVersions"]
            .as_array()
            .is_some_and(|v| v.contains(&json!("2026-07-28"))),
        "{result:#?}"
    );
    assert_eq!(result["capabilities"]["tools"], json!({}));
}

#[test]
fn tools_are_listed_as_read_only() {
    let mut lines = open("2025-06-18");
    lines.push(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string());

    let responses = session(&lines, 2);
    let tools = response(&responses, 2)["result"]["tools"]
        .as_array()
        .expect("tools must be an array")
        .clone();

    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["want_ask", "want_pods", "want_short"]);

    for tool in &tools {
        assert_eq!(
            tool["annotations"]["readOnlyHint"], true,
            "{} should be advertised as read-only",
            tool["name"]
        );
        assert!(
            !tool["description"].as_str().unwrap_or_default().is_empty(),
            "{} has no description",
            tool["name"]
        );
    }
}

#[test]
fn every_tool_requires_a_query_string() {
    let mut lines = open("2025-06-18");
    lines.push(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string());

    for tool in response(&session(&lines, 2), 2)["result"]["tools"]
        .as_array()
        .unwrap()
    {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object", "{}", tool["name"]);
        assert_eq!(
            schema["properties"]["query"]["type"], "string",
            "{}",
            tool["name"]
        );
        assert!(
            schema["required"]
                .as_array()
                .is_some_and(|r| r.contains(&json!("query"))),
            "{} does not require query",
            tool["name"]
        );
    }
}

#[test]
fn unknown_method_is_rejected() {
    let mut lines = open("2025-06-18");
    lines.push(json!({"jsonrpc": "2.0", "id": 2, "method": "does/not/exist"}).to_string());

    let responses = session(&lines, 2);
    assert_eq!(response(&responses, 2)["error"]["code"], json!(-32601));
}

#[test]
fn unknown_tool_is_rejected() {
    let mut lines = open("2025-06-18");
    lines.push(call(2, "not_a_tool", json!({})));

    let responses = session(&lines, 2);
    assert_eq!(response(&responses, 2)["error"]["code"], json!(-32602));
}

#[test]
fn missing_query_is_a_tool_error_not_a_crash() {
    let mut lines = open("2025-06-18");
    lines.push(call(2, "want_short", json!({})));

    let responses = session(&lines, 2);
    let result = &response(&responses, 2)["result"];
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("query"),
        "{result:#?}"
    );
}

#[test]
fn a_probe_that_never_initializes_exits_cleanly() {
    // Health checks spawn the server and close it again, and the 2026-07-28
    // revision never sends `initialize`. Neither is a failure.
    let frames: [Vec<&str>; 3] = [
        vec![r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        vec![
            r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        ],
        vec![],
    ];

    for payload in frames {
        let mut child = Command::new(env!("CARGO_BIN_EXE_want"))
            .args(["mcp", "--appid", "protocol-test"])
            .env_remove("WOLFRAM_APPID")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn want mcp");

        {
            let stdin = child.stdin.as_mut().expect("stdin");
            for frame in &payload {
                writeln!(stdin, "{frame}").expect("write a frame");
            }
        }

        let out = child.wait_with_output().expect("server did not exit");
        assert!(
            out.status.success(),
            "exit {} for {payload:?}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn malformed_json_does_not_kill_the_session() {
    let mut lines = vec!["this is not json".to_owned()];
    lines.extend(open("2025-06-18"));
    lines.push(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string());

    let responses = session(&lines, 2);
    assert_eq!(
        response(&responses, 1)["result"]["serverInfo"]["name"],
        "want"
    );
    assert_eq!(
        response(&responses, 2)["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn failing_tool_calls_are_results_not_protocol_errors() {
    let mut lines = open("2025-06-18");
    // Bogus AppID: both calls fail at the API.
    lines.push(call(2, "want_short", json!({"query": "2+2"})));
    lines.push(call(3, "want_ask", json!({"query": "2+2"})));

    let responses = session(&lines, 3);
    assert_eq!(responses.len(), 3);

    for id in [2, 3] {
        let reply = response(&responses, id);
        assert!(
            reply.get("error").is_none(),
            "id {id} should not be a protocol error: {reply:#?}"
        );

        let result = &reply["result"];
        assert_eq!(result["isError"], true, "id {id}: {result:#?}");

        // The agent needs a way out, not just a stack of text.
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("Next step"), "id {id}: {text}");
        assert!(text.contains("Never invent"), "id {id}: {text}");
    }
}

#[test]
fn invalid_units_is_a_tool_error() {
    let mut lines = open("2025-06-18");
    lines.push(call(
        2,
        "want_short",
        json!({"query": "2+2", "units": "metirc"}),
    ));

    let responses = session(&lines, 2);
    let result = &response(&responses, 2)["result"];

    assert_eq!(result["isError"], true, "{result:#?}");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("metric"),
        "{result:#?}"
    );
}

#[test]
fn unknown_arguments_are_rejected_not_ignored() {
    // The CLI spells the same thing `--pod`. Ignoring the argument used to look
    // like a successful filter: the agent got the whole card and never knew.
    let mut lines = open("2025-06-18");
    lines.push(call(
        2,
        "want_pods",
        json!({"query": "Mars", "pod": ["Result"]}),
    ));

    let responses = session(&lines, 2);
    let result = &response(&responses, 2)["result"];

    assert_eq!(result["isError"], true, "{result:#?}");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("pod"),
        "{result:#?}"
    );
}

#[test]
fn empty_arguments_are_refused_before_any_request() {
    let mut lines = open("2025-06-18");
    lines.push(call(2, "want_short", json!({"query": "   "})));
    lines.push(call(3, "want_ask", json!({"query": "2+2", "maxchars": 0})));
    lines.push(call(
        4,
        "want_pods",
        json!({"query": "Mars", "pod_titles": []}),
    ));

    let responses = session(&lines, 4);

    for id in [2, 3, 4] {
        let result = &response(&responses, id)["result"];
        assert_eq!(result["isError"], true, "id {id}: {result:#?}");
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("Invalid tool argument"), "id {id}: {text}");
        // A stray run of spaces comes from a mangled string continuation, which
        // no reader would notice but every agent would read.
        assert!(!text.contains("  "), "id {id} has padded text: {text:?}");
    }
}

//==========================================================
// Live (needs WOLFRAM_APPID)
//==========================================================

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn assumption_disambiguates_an_ambiguous_query() {
    // "chicken" defaults to the animal; the food interpretation is a different card.
    let mut lines = open("2025-06-18");
    lines.push(call(
        2,
        "want_pods",
        json!({
            "query": "chicken",
            "assumption": "*C.chicken-_*ExpandedFood-",
            "primary": true
        }),
    ));

    let responses = live(&lines, 2, &["mcp"]);
    let text = response(&responses, 2)["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    assert!(
        text.to_lowercase().contains("nutrition") || text.to_lowercase().contains("calories"),
        "assumption was ignored: {text}"
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn server_options_reach_the_tools() {
    let mut lines = open("2025-06-18");
    lines.push(call(2, "want_short", json!({"query": "capital of France"})));

    let responses = live(&lines, 2, &["mcp", "--lang", "zh"]);
    let text = response(&responses, 2)["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    assert!(
        text.contains("巴黎"),
        "--lang did not reach the tool: {text}"
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn duplicate_concurrent_calls_share_one_request() {
    use std::io::Read;

    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live test");

    let mut child = Command::new(env!("CARGO_BIN_EXE_want"))
        .args(["mcp"])
        .env("WOLFRAM_APPID", appid)
        // Only our own log line, so counting upstream calls stays exact, and no
        // ANSI colour codes that would split `status=200` apart.
        .env("RUST_LOG", "want=debug")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn want mcp");

    let mut stderr = child.stderr.take().expect("stderr");
    let logger = std::thread::spawn(move || {
        let mut buf = String::new();
        stderr.read_to_string(&mut buf).expect("read stderr");
        buf
    });

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    for line in open("2025-06-18") {
        writeln!(stdin, "{line}").unwrap();
    }
    let first = call(2, "want_short", json!({"query": "speed of light"}));
    let second = call(3, "want_short", json!({"query": "speed of light"}));
    // Same moment on purpose: the second must ride the first request.
    writeln!(stdin, "{first}").unwrap();
    writeln!(stdin, "{second}").unwrap();

    for _ in 0..3 {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        assert!(!line.is_empty(), "server exited early");
    }

    drop(stdin);
    let _ = child.wait();
    let log = logger.join().expect("join logger");

    // Either coalesced in flight or served from the cache, but never two calls.
    let upstream = log.matches("status=200").count();
    assert_eq!(upstream, 1, "expected one upstream request:\n{log}");
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn cached_answers_keep_their_original_fetch_time() {
    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live test");

    let mut child = Command::new(env!("CARGO_BIN_EXE_want"))
        .args(["mcp"])
        .env("WOLFRAM_APPID", appid)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn want mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    let read_one = |reader: &mut BufReader<_>| -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("one JSON message per line")
    };

    let question = call(2, "want_ask", json!({"query": "2+2", "maxchars": 200}));
    for line in open("2025-06-18") {
        writeln!(stdin, "{line}").unwrap();
    }
    writeln!(stdin, "{question}").unwrap();
    read_one(&mut reader);
    let first = read_one(&mut reader);

    // Sequential on purpose: the second call must be served from the cache.
    writeln!(stdin, "{question}").unwrap();
    let second = read_one(&mut reader);

    drop(stdin);
    let _ = child.wait();

    let stamp = |reply: &Value| -> String {
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        text.split("fetched ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or_default()
            .to_owned()
    };

    let first = stamp(&first);
    assert!(!first.is_empty(), "no provenance line in the reply");
    assert_eq!(
        first,
        stamp(&second),
        "a cache hit must not claim a new fetch time"
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn server_units_survive_a_tool_call_without_units() {
    let mut lines = open("2025-06-18");
    lines.push(call(
        2,
        "want_short",
        json!({"query": "height of Mount Everest"}),
    ));

    let responses = live(&lines, 2, &["mcp", "--units", "nonmetric"]);
    let text = response(&responses, 2)["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    assert!(text.contains("feet"), "--units was lost: {text}");
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn want_pods_returns_more_than_the_want_ask_digest() {
    // The whole point of want_pods: the LLM API picks a subset, so the full card
    // is longer. If that ever stops being true, the skill is telling a lie.
    let mut lines = open("2025-06-18");
    lines.push(call(
        2,
        "want_ask",
        json!({"query": "chicken", "maxchars": 6800}),
    ));
    lines.push(call(3, "want_pods", json!({"query": "chicken"})));

    let responses = live(&lines, 3, &["mcp"]);

    let text_of = |id: u64| {
        response(&responses, id)["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };

    // The LLM API writes `Title:` lines, the pods renderer `## Title`. A table
    // row never ends with a colon, so the filter stays on section headers.
    let ask_text = text_of(2);
    let pods_text = text_of(3);

    let ask: std::collections::HashSet<&str> = ask_text
        .lines()
        .filter_map(|line| line.strip_suffix(':'))
        .filter(|name| !name.contains('|'))
        .collect();
    let pods: std::collections::HashSet<&str> = pods_text
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .collect();

    let mut extra: Vec<&&str> = pods.difference(&ask).collect();
    extra.sort_unstable();
    assert!(
        !extra.is_empty(),
        "want_ask already returns every section, so want_pods has no reason to exist"
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn live_tool_calls_return_answers() {
    let mut lines = open("2025-06-18");
    lines.push(call(2, "want_short", json!({"query": "speed of light"})));
    lines.push(call(
        3,
        "want_pods",
        json!({"query": "Mars", "primary": true}),
    ));

    let responses = live(&lines, 3, &["mcp"]);

    for id in [2, 3] {
        let result = &response(&responses, id)["result"];
        assert_eq!(result["isError"], false, "{result:#?}");
        assert!(
            !result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
        );
    }
}
