//! `want` — Wolfram|Alpha Network Tool.
//!
//! A CLI with a REPL for people, a `want mcp` stdio server for agents, and
//! `want skill` to print, install or remove the SKILL.md they load. `want config`
//! reports or stores the AppID.

mod client;
mod config;
mod mcp;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use tracing::level_filters::LevelFilter;

use crate::client::{Client, Opts};

/// Shipped verbatim so the file in the repo is the single source of truth.
const SKILL_MD: &str = include_str!("../skills/want/SKILL.md");

#[derive(Debug, Parser)]
#[command(
    name = "want",
    version,
    about = "Wolfram|Alpha Network Tool — an AI-native W|A CLI, REPL and MCP server",
    after_help = "Examples:\n  want ask \"density of osmium\"\n  want short \"population of France\"\n  \
                  want pods --primary \"integrate x^2 sin(x)\"\n  want image \"plot sin(x)/x\" -o plot.gif\n  \
                  want                       # REPL\n  \
                  \nAgent setup (see README for Codex and opencode):\n  \
                  want config --set-appid <id>\n  \
                  claude mcp add -s user want -- want mcp\n  \
                  want skill --install       # or: npx skills add Iorest/want -g\n  \
                  \nUndo:\n  claude mcp remove want\n  want skill --uninstall\n  \
                  \nExit codes: 0 ok, 2 usage, 3 query not understood, 4 auth or quota, 1 other"
)]
struct Cli {
    /// Wolfram|Alpha AppID. Repeat it, or separate keys with commas, to spread
    /// load across several monthly quotas. Falls back to WOLFRAM_APPID, then
    /// ~/.config/want/config.toml
    #[arg(long, global = true, value_delimiter = ',', hide_env_values = true)]
    appid: Vec<String>,

    /// Log verbosity on stderr: -v info, -vv debug. RUST_LOG takes precedence
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Unit system: metric or nonmetric; defaults by caller IP
    #[arg(long, global = true, value_parser = ["metric", "nonmetric"])]
    units: Option<String>,

    /// Output language for answers and pod titles, e.g. zh, de, fr. The query itself stays English
    #[arg(long, global = true)]
    lang: Option<String>,

    /// Drop the provenance footer (endpoint, fetch time, query)
    #[arg(long, global = true)]
    no_source: bool,

    /// Reuse an identical response for N seconds; 0 disables. Default 0 for one-shot
    /// commands, 300 for the REPL and `want mcp`
    #[arg(long, global = true)]
    cache_ttl: Option<u64>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Ask via the LLM API; plain text already shaped for a language model (the agent default)
    #[command(visible_alias = "q")]
    Ask {
        /// Natural-language question. English keyword forms work best
        #[arg(value_parser = client::non_empty)]
        query: String,
        /// Character cap on the answer; the API default is ~6800
        #[arg(long)]
        maxchars: Option<u32>,
    },
    /// One-line answer via the Short Answers API; the smallest and fastest call
    Short {
        /// A question with a single-value answer
        #[arg(value_parser = client::non_empty)]
        query: String,
    },
    /// Structured pods via the Full Results API, rendered as markdown
    Pods {
        /// Query
        #[arg(value_parser = client::non_empty)]
        query: String,
        /// Keep only pods whose title matches exactly; repeatable, * is a wildcard
        #[arg(long = "pod", value_name = "TITLE")]
        pod: Vec<String>,
        /// Keep only the pod Wolfram|Alpha marks as primary. Not with --json
        #[arg(long, conflicts_with = "json")]
        primary: bool,
        /// Force an interpretation, using a value from a previous answer's assumptions
        /// list. Only the Full Results API honours this; `ask` cannot be disambiguated
        #[arg(long)]
        assumption: Option<String>,
        /// Print the raw Full Results JSON instead of markdown, with every subpod
        /// it contains. Not with --primary, which filters on the client side
        #[arg(long)]
        json: bool,
    },
    /// Render one image via the Simple API and save it to disk
    Image {
        /// Query, e.g. "plot sin(x)/x from -10 to 10"
        #[arg(value_parser = client::non_empty)]
        query: String,
        /// Output path; defaults to want.gif / want.png by image type
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Interactive REPL; also the default when no subcommand is given
    Repl {
        /// Endpoint used for each query
        #[arg(long, value_enum, default_value_t = Mode::Ask)]
        mode: Mode,
    },
    /// Serve MCP over stdio
    Mcp,
    /// Show which AppIDs will be used and where they came from, or store them
    Config {
        /// Write these AppIDs to the config file (created 0600), replacing what is
        /// stored. Repeat the flag or separate keys with commas
        #[arg(
            long,
            value_name = "APPID",
            value_delimiter = ',',
            group = "appid_action"
        )]
        set_appid: Vec<String>,
        /// Append these AppIDs to the ones already stored, keeping the pool intact
        #[arg(
            long,
            value_name = "APPID",
            value_delimiter = ',',
            group = "appid_action"
        )]
        add_appid: Vec<String>,
    },
    /// Print SKILL.md, install it into the agent skill directories, or remove it again
    Skill {
        /// Write SKILL.md into the skills directories instead of printing it
        #[arg(long, group = "skill_action")]
        install: bool,
        /// Remove the installed SKILL.md and leave everything else alone
        #[arg(long, group = "skill_action")]
        uninstall: bool,
        /// Skills root to act on; repeatable. Defaults to ~/.claude/skills and ~/.agents/skills
        #[arg(long, value_name = "DIR", requires = "skill_action")]
        dir: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Ask,
    Short,
    Pods,
    Image,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {err:#}");
        std::process::exit(exit_code(&err));
    }
}

/// Exit codes a script can branch on: 3 means "rephrase", 4 means "fix the key
/// or wait for the quota to reset".
fn exit_code(err: &anyhow::Error) -> i32 {
    match err
        .downcast_ref::<client::Failure>()
        .map(client::Failure::kind)
    {
        Some(client::Kind::Usage) => 2,
        Some(client::Kind::Uninterpretable) => 3,
        Some(client::Kind::Access) => 4,
        _ => 1,
    }
}

/// Build the command, then hide the network options from the one subcommand that
/// cannot use them: globals are inherited by every subcommand, so `want skill
/// --help` would otherwise advertise `--appid`, `--units`, `--lang` and friends.
fn parse_cli() -> Cli {
    let mut command = Cli::command();
    command.build();

    let matches = command
        .mut_subcommand("skill", |skill| skill.mut_args(hide_from_skill))
        .get_matches();

    Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
}

fn hide_from_skill(arg: clap::Arg) -> clap::Arg {
    match arg.get_id().as_str() {
        "appid" | "units" | "lang" | "no_source" | "cache_ttl" => arg.hide(true),
        _ => arg,
    }
}

async fn run() -> Result<()> {
    let cli = parse_cli();

    // stdout is the MCP channel, so all logging goes to stderr.
    let level = match cli.verbose {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        _ => LevelFilter::DEBUG,
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(level.into())
                .from_env_lossy(),
        )
        .with_writer(std::io::stderr)
        // Colour codes in a redirected stderr would end up in log files and in
        // anything parsing them (test harnesses included).
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    // `want skill` is pure stdout plus optional file writes, no network, so it
    // must not demand an AppID.
    if let Some(Cmd::Skill {
        install,
        uninstall,
        dir,
    }) = &cli.cmd
    {
        return skill(*install, *uninstall, dir);
    }

    let resolved = config::resolve(cli.appid.clone())?;

    if let Some(Cmd::Config {
        set_appid,
        add_appid,
    }) = &cli.cmd
    {
        return show_config(set_appid, add_appid, resolved.as_ref());
    }

    // DEMO is no longer accepted by the API, so a missing AppID is a hard error
    // pointing at every way to supply one, instead of a confusing 401. It exits
    // with the same code as a rejected key: both mean "fix the credentials".
    let Some(resolved) = resolved else {
        return Err(client::Failure::access(missing_appid_hint()).into());
    };

    warn_about_origin(&resolved);

    let client = Arc::new(Client::new(resolved.appids)?);
    let opts = Opts {
        units: cli.units.clone(),
        lang: cli.lang.clone(),
        no_source: cli.no_source,
        cache_ttl: cli.cache_ttl.unwrap_or(0),
        assumption: None,
    };

    // A long-lived mode caches by default: repeating a query there is common.
    let live_opts = || Opts {
        cache_ttl: cli.cache_ttl.unwrap_or(300),
        ..opts.clone()
    };

    match cli.cmd {
        None => repl(&client, Mode::Ask, &live_opts()).await?,
        Some(Cmd::Repl { mode }) => repl(&client, mode, &live_opts()).await?,
        Some(Cmd::Ask { query, maxchars }) => {
            println!("{}", client.llm(&query, &opts, maxchars).await?)
        }
        Some(Cmd::Short { query }) => println!("{}", client.short(&query, &opts).await?),
        Some(Cmd::Pods {
            query,
            pod,
            primary,
            assumption,
            json,
        }) => {
            let opts = Opts { assumption, ..opts };
            if json {
                println!("{}", client.pods_json(&query, &opts, &pod).await?)
            } else {
                println!("{}", client.pods(&query, &opts, &pod, primary).await?)
            }
        }
        Some(Cmd::Image { query, output }) => {
            println!(
                "{}",
                save_image(&client, &query, &opts, output).await?.display()
            )
        }
        Some(Cmd::Mcp) => mcp::serve(client, &live_opts()).await?,
        Some(Cmd::Config { .. }) => unreachable!("handled before the AppID check"),
        Some(Cmd::Skill { .. }) => unreachable!("handled before the AppID check"),
    }

    Ok(())
}

/// What to print when no AppID was found in any of the three places.
fn missing_appid_hint() -> String {
    let path = config::path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.config/want/config.toml".to_owned());
    format!(
        "No Wolfram|Alpha AppID. Get a free one at https://developer.wolframalpha.com, \
         then pick one:\n  \
         want config --set-appid <id>      # writes {path}; repeat for more keys\n  \
         export WOLFRAM_APPID=<id>         # this shell only, comma-separated for a pool\n  \
         want --appid <id> <command>       # this command only, repeat for more keys"
    )
}

/// Point out the two ways an AppID can be exposed: the process list, or a config
/// file other users can read.
fn warn_about_origin(resolved: &config::Resolved) {
    match resolved.origin {
        config::Origin::Flag => tracing::warn!(
            "--appid is visible in the process list; prefer `want config --set-appid` or WOLFRAM_APPID"
        ),
        config::Origin::File => {
            if let Ok(path) = config::path() {
                config::warn_if_world_readable(&path);
            }
        }
        config::Origin::Env => {}
    }
}

/// `want config` — say where the AppIDs come from, or store them.
fn show_config(
    set_appid: &[String],
    add_appid: &[String],
    resolved: Option<&config::Resolved>,
) -> Result<()> {
    if !set_appid.is_empty() || !add_appid.is_empty() {
        // `--set-appid` starts from nothing so a dead key can be dropped;
        // `--add-appid` starts from the file so adding one keeps the rest.
        let mut ids = if set_appid.is_empty() {
            config::read()?
        } else {
            Vec::new()
        };
        let incoming: Vec<&str> = set_appid
            .iter()
            .chain(add_appid)
            .map(String::as_str)
            .collect();
        ids.extend(config::split(&incoming.join(",")));
        // Re-adding a key must not double it, so fold the list once more before
        // it is written; the count printed is the pool the user will get.
        let ids = config::split(&ids.join(","));

        let path = config::write(&ids)?;
        let plural = if ids.len() == 1 { "" } else { "s" };
        println!("stored {} AppID{plural} in {}", ids.len(), path.display());
        return Ok(());
    }

    let path = config::path()?;
    if let Some(resolved) = resolved {
        warn_about_origin(resolved);
    }

    match resolved {
        Some(resolved) if resolved.appids.len() == 1 => println!(
            "AppID: {}  from {}",
            config::mask(&resolved.appids[0]),
            resolved.origin.describe()
        ),
        Some(resolved) => {
            // Several keys: say how many, and which ones, still masked.
            println!(
                "AppIDs: {} keys  from {}",
                resolved.appids.len(),
                resolved.origin.describe()
            );
            for (i, appid) in resolved.appids.iter().enumerate() {
                println!("  {}. {}", i + 1, config::mask(appid));
            }
        }
        None => println!("AppID: none found"),
    }

    println!(
        "file:  {} ({})",
        path.display(),
        if path.exists() { "exists" } else { "missing" }
    );
    println!(
        "env:   WOLFRAM_APPID {}",
        if std::env::var("WOLFRAM_APPID").is_ok_and(|v| !v.trim().is_empty()) {
            "set"
        } else {
            "not set"
        }
    );

    if resolved.is_none() {
        println!("\nStore one with: want config --set-appid <id>");
    }

    Ok(())
}

/// Print the skill, write it where every supported agent looks for it, or take it back.
///
/// `~/.claude/skills` is Claude Code's own path; `~/.agents/skills` is read by
/// Codex (user scope) and by opencode (agent-compatible path), so those two
/// cover Claude Code, opencode and Codex with one install.
fn skill(install: bool, uninstall: bool, dirs: &[PathBuf]) -> Result<()> {
    if !install && !uninstall {
        print!("{SKILL_MD}");
        return Ok(());
    }

    let roots: Vec<PathBuf> = if dirs.is_empty() {
        let home = home::home_dir().context("cannot determine the home directory")?;
        vec![home.join(".claude/skills"), home.join(".agents/skills")]
    } else {
        dirs.to_vec()
    };

    for root in roots {
        let dir = root.join("want");
        let path = dir.join("SKILL.md");

        if uninstall {
            if !path.exists() {
                println!("not installed: {}", path.display());
                continue;
            }
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            // Only drop the folder if nothing else lives there.
            let _ = std::fs::remove_dir(&dir);
            println!("removed {}", path.display());
            continue;
        }

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        std::fs::write(&path, SKILL_MD)
            .with_context(|| format!("failed to write {}", path.display()))?;
        println!("{}", path.display());
    }

    Ok(())
}

/// Multi-turn prompt loop, mirroring `wolfram-cli`'s no-subcommand REPL.
async fn repl(client: &Client, mode: Mode, opts: &Opts) -> Result<()> {
    eprintln!(
        "want {} — {:?} mode, Ctrl-D or :quit to exit",
        env!("CARGO_PKG_VERSION"),
        mode
    );

    let mut rl = rustyline::DefaultEditor::new().context("failed to start the REPL")?;
    while let Some(line) = prompt(&mut rl) {
        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        if matches!(query, ":q" | ":quit" | ":exit" | "quit" | "exit") {
            break;
        }

        let _ = rl.add_history_entry(query);

        match ask_once(client, mode, opts, query).await {
            Ok(answer) => println!("{answer}\n"),
            Err(err) => eprintln!("error: {err:#}"),
        }
    }

    Ok(())
}

/// `None` on EOF or a hard read error; Ctrl-C just gives a fresh prompt.
fn prompt(rl: &mut rustyline::DefaultEditor) -> Option<String> {
    loop {
        match rl.readline("want> ") {
            Ok(line) => return Some(line),
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(_) => return None,
        }
    }
}

/// One REPL turn: dispatch the typed line to the endpoint the REPL is in.
async fn ask_once(client: &Client, mode: Mode, opts: &Opts, query: &str) -> Result<String> {
    match mode {
        Mode::Ask => client.llm(query, opts, None).await,
        Mode::Short => client.short(query, opts).await,
        Mode::Pods => client.pods(query, opts, &[], false).await,
        Mode::Image => Ok(save_image(client, query, opts, None)
            .await?
            .display()
            .to_string()),
    }
}

/// Render one image to disk. Without an explicit `-o` the name is numbered, so a
/// later picture never overwrites an earlier one.
async fn save_image(
    client: &Client,
    query: &str,
    opts: &Opts,
    output: Option<PathBuf>,
) -> Result<PathBuf> {
    let (bytes, ext) = client.image(query, opts).await?;

    let path = match output {
        Some(path) => path,
        None => {
            let mut candidate = PathBuf::from(format!("want.{ext}"));
            let mut n = 1;
            while candidate.exists() {
                candidate = PathBuf::from(format!("want-{n}.{ext}"));
                n += 1;
            }
            candidate
        }
    };

    std::fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}
