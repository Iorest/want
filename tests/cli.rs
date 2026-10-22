//! End-to-end checks for the CLI surface.
//!
//! Offline tests run with `cargo test`. Tests marked `#[ignore]` call the real
//! Wolfram|Alpha API and consume quota; run them with
//! `WOLFRAM_APPID=... cargo test -- --ignored`.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_want"))
        .args(args)
        .env_remove("WOLFRAM_APPID")
        .output()
        .expect("failed to run want")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

//==========================================================
// Offline
//==========================================================

#[test]
fn help_lists_every_subcommand() {
    let out = run(&["--help"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let help = stdout(&out);
    for cmd in ["ask", "short", "pods", "image", "repl", "mcp", "skill"] {
        assert!(help.contains(cmd), "help does not mention `{cmd}`:\n{help}");
    }
}

#[test]
fn version_comes_from_cargo() {
    let out = run(&["--version"]);
    assert_eq!(
        stdout(&out).trim(),
        format!("want {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn skill_is_a_valid_agent_skill() {
    let out = run(&["skill"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let text = stdout(&out);
    let rest = text
        .strip_prefix("---\n")
        .expect("SKILL.md must open with frontmatter");
    let (front, body) = rest
        .split_once("\n---\n")
        .expect("frontmatter must be closed with ---");

    let field = |key: &str| {
        front
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}: ")))
            .map(str::trim)
    };

    // Agent Skills name rules: 1-64 chars, lowercase alphanumeric, single hyphens.
    let name = field("name").expect("name is required");
    assert_eq!(name, "want");
    assert!((1..=64).contains(&name.len()));
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    );
    assert!(!name.starts_with('-') && !name.ends_with('-') && !name.contains("--"));

    let description = field("description").expect("description is required");
    assert!(
        (1..=1024).contains(&description.chars().count()),
        "description must be 1-1024 chars, got {}",
        description.chars().count()
    );

    assert!(body.trim().len() > 200, "skill body is too thin");
    // The skill must not promise tools the server does not expose.
    for tool in ["want_ask", "want_short", "want_pods"] {
        assert!(body.contains(tool), "skill never mentions {tool}");
    }
}

#[test]
fn skill_install_writes_the_layout_agents_expect() {
    let root = std::env::temp_dir().join(format!("want-skill-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let out = run(&["skill", "--install", "--dir", root.to_str().unwrap()]);
    assert!(out.status.success(), "{}", stderr(&out));

    let path = root.join("want").join("SKILL.md");
    let written = std::fs::read_to_string(&path).expect("SKILL.md was not written");
    assert_eq!(written, stdout(&run(&["skill"])));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn skill_help_hides_the_options_it_cannot_use() {
    // Globals are inherited by every subcommand; the skill one never touches the
    // network, so advertising them is noise.
    let out = run(&["skill", "--help"]);
    let help = stdout(&out);

    for flag in ["--appid", "--units", "--lang", "--no-source", "--cache-ttl"] {
        assert!(
            !help.contains(flag),
            "{flag} leaked into skill --help:\n{help}"
        );
    }
    assert!(help.contains("--install"), "{help}");
    assert!(help.contains("--uninstall"), "{help}");
}

#[test]
fn skill_install_can_be_undone() {
    let root = std::env::temp_dir().join(format!("want-uninstall-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();

    let install = run(&["skill", "--install", "--dir", root.to_str().unwrap()]);
    assert!(install.status.success(), "{}", stderr(&install));

    let path = root.join("want").join("SKILL.md");
    assert!(path.exists(), "install did not write {}", path.display());

    let remove = run(&["skill", "--uninstall", "--dir", root.to_str().unwrap()]);
    assert!(remove.status.success(), "{}", stderr(&remove));
    assert!(!path.exists(), "uninstall left {} behind", path.display());

    // Running it twice must not fail.
    let again = run(&["skill", "--uninstall", "--dir", root.to_str().unwrap()]);
    assert!(again.status.success(), "{}", stderr(&again));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn primary_and_json_cannot_be_combined() {
    // --primary filters client-side; raw JSON cannot honour it, so refuse early
    // instead of silently returning everything.
    let out = run(&["pods", "--json", "--primary", "Mars"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--primary"), "{}", stderr(&out));
}

#[test]
fn a_bad_key_exits_4() {
    // 4 is the "fix the key or wait for quota" code.
    let out = Command::new(env!("CARGO_BIN_EXE_want"))
        .args(["--appid", "definitely-not-a-real-appid", "short", "2+2"])
        .env_remove("WOLFRAM_APPID")
        .output()
        .expect("failed to run want");

    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
}

#[test]
fn a_bad_unit_system_is_rejected_before_any_request() {
    let out = run(&["--units", "metirc", "short", "2+2"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

/// Run with an isolated HOME so the config file under test is the only one.
fn run_at(home: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_want"));
    cmd.args(args).env("HOME", home).env_remove("WOLFRAM_APPID");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run want")
}

#[test]
fn config_file_is_written_private_and_read_back() {
    let home = std::env::temp_dir().join(format!("want-config-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    std::fs::create_dir_all(&home).expect("temp home");

    let write = run_at(&home, &["config", "--set-appid", "TESTKEY123456"], &[]);
    assert!(write.status.success(), "{}", stderr(&write));

    let path = home.join(".config/want/config.toml");
    assert!(path.exists(), "config file was not created");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config holds a secret, mode was {mode:o}");
    }

    let show = run_at(&home, &["config"], &[]);
    let text = stdout(&show);
    assert!(text.contains("from config file"), "{text}");
    assert!(
        !text.contains("TESTKEY123456"),
        "the key was printed: {text}"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn appid_precedence_is_flag_then_env_then_file() {
    let home = std::env::temp_dir().join(format!("want-precedence-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    std::fs::create_dir_all(&home).expect("temp home");

    let write = run_at(&home, &["config", "--set-appid", "FILEKEY"], &[]);
    assert!(write.status.success(), "{}", stderr(&write));

    let from_file = run_at(&home, &["config"], &[]);
    assert!(
        stdout(&from_file).contains("from config file"),
        "{}",
        stdout(&from_file)
    );

    let from_env = run_at(&home, &["config"], &[("WOLFRAM_APPID", "ENVKEY")]);
    assert!(
        stdout(&from_env).contains("from WOLFRAM_APPID"),
        "{}",
        stdout(&from_env)
    );

    let from_flag = run_at(
        &home,
        &["--appid", "FLAGKEY", "config"],
        &[("WOLFRAM_APPID", "ENVKEY")],
    );
    assert!(
        stdout(&from_flag).contains("from --appid"),
        "{}",
        stdout(&from_flag)
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn a_broken_config_file_is_reported_not_ignored() {
    let home = std::env::temp_dir().join(format!("want-badconfig-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    std::fs::create_dir_all(home.join(".config/want")).expect("temp home");
    std::fs::write(home.join(".config/want/config.toml"), "appid = [oops").unwrap();

    let out = run_at(&home, &["short", "2+2"], &[]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("config.toml"), "{}", stderr(&out));

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn help_never_prints_the_env_value() {
    // The AppID is resolved by hand instead of through clap's `env`, because
    // clap renders `[env: NAME=value]` and would leak the key into any pasted
    // `--help` output.
    let out = Command::new(env!("CARGO_BIN_EXE_want"))
        .arg("--help")
        .env("WOLFRAM_APPID", "super-secret-appid")
        .output()
        .expect("failed to run want");

    let help = stdout(&out);
    assert!(
        !help.contains("super-secret-appid"),
        "the AppID leaked: {help}"
    );
    assert!(help.contains("WOLFRAM_APPID"), "{help}");
}

#[test]
fn passing_the_appid_on_the_command_line_warns() {
    // `config` resolves the AppID without touching the network, so the warning
    // can be checked offline.
    let out = Command::new(env!("CARGO_BIN_EXE_want"))
        .args(["--appid", "x", "config"])
        .env_remove("WOLFRAM_APPID")
        .output()
        .expect("failed to run want");

    assert!(stderr(&out).contains("process list"), "{}", stderr(&out));
}

#[test]
fn missing_appid_points_at_every_way_to_set_one() {
    let out = run(&["short", "2+2"]);
    assert!(!out.status.success(), "want should fail without an AppID");

    let err = stderr(&out);
    assert!(err.contains("developer.wolframalpha.com"), "{err}");
    assert!(err.contains("want config --set-appid"), "{err}");
    assert!(err.contains("WOLFRAM_APPID"), "{err}");
    assert!(err.contains("--appid"), "{err}");
}

//==========================================================
// Live (needs WOLFRAM_APPID)
//==========================================================

fn live(args: &[&str]) -> Output {
    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live tests");
    Command::new(env!("CARGO_BIN_EXE_want"))
        .args(args)
        .env("WOLFRAM_APPID", appid)
        .output()
        .expect("failed to run want")
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn short_answers_a_known_fact() {
    let out = live(&["short", "speed of light"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("300"), "{}", stdout(&out));
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn ask_returns_llm_shaped_text() {
    let out = live(&["ask", "density of osmium", "--maxchars", "400"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let text = stdout(&out);
    assert!(text.contains("Result"), "{text}");
    // maxchars is a soft cap: the API truncates on section boundaries, so a
    // 400-char request lands a little over. Well under the ~6800 default.
    assert!(
        text.chars().count() <= 600,
        "maxchars was not honoured: {} chars",
        text.chars().count()
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn pods_primary_returns_exactly_one_section() {
    let out = live(&["pods", "--primary", "integrate x^2 sin(x) dx"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let text = stdout(&out);
    assert_eq!(text.matches("## ").count(), 1, "{text}");
    assert!(text.contains("sin(x)"), "{text}");
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn pods_unknown_title_explains_the_filter() {
    let out = live(&["pods", "--pod", "No Such Section", "Mars"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--pod"), "{}", stderr(&out));
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn units_switch_changes_the_answer() {
    let out = live(&["--units", "nonmetric", "short", "height of Mount Everest"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("feet"), "{}", stdout(&out));
}

fn live_in(dir: &std::path::Path, args: &[&str]) -> Output {
    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live tests");
    Command::new(env!("CARGO_BIN_EXE_want"))
        .args(args)
        .env("WOLFRAM_APPID", appid)
        .current_dir(dir)
        .output()
        .expect("failed to run want")
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn the_repl_serves_a_repeated_image_from_cache() {
    use std::io::{Read, Write};
    use std::process::Stdio;

    let appid = std::env::var("WOLFRAM_APPID").expect("export WOLFRAM_APPID to run the live tests");
    let dir = std::env::temp_dir().join(format!("want-repl-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let mut child = Command::new(env!("CARGO_BIN_EXE_want"))
        .args(["repl", "--mode", "image"])
        .env("WOLFRAM_APPID", appid)
        .env("RUST_LOG", "want=debug")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn want repl");

    let mut stderr = child.stderr.take().expect("stderr");
    let logger = std::thread::spawn(move || {
        let mut buf = String::new();
        stderr.read_to_string(&mut buf).expect("read stderr");
        buf
    });

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        // The same query twice: the second one must not reach the API.
        writeln!(stdin, "plot sin(x)").unwrap();
        writeln!(stdin, "plot sin(x)").unwrap();
    }
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("repl did not exit");
    let log = logger.join().expect("join logger");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("want.gif").exists(), "first picture missing");
    assert!(dir.join("want-1.gif").exists(), "second picture missing");

    let upstream = log.matches("status=200").count();
    assert_eq!(upstream, 1, "expected one upstream request:\n{log}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn image_never_overwrites_an_earlier_file() {
    let dir = std::env::temp_dir().join(format!("want-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    for _ in 0..2 {
        let out = live_in(&dir, &["image", "plot sin(x)"]);
        assert!(out.status.success(), "{}", stderr(&out));
    }

    assert!(dir.join("want.gif").exists(), "first picture missing");
    assert!(
        dir.join("want-1.gif").exists(),
        "second picture overwrote the first"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn image_writes_a_real_gif() {
    let path = std::env::temp_dir().join(format!("want-plot-{}.gif", std::process::id()));
    let out = live(&[
        "image",
        "plot sin(x)/x from -10 to 10",
        "-o",
        path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", stderr(&out));

    let bytes = std::fs::read(&path).expect("image was not written");
    assert!(bytes.starts_with(b"GIF8"), "not a GIF: {:?}", &bytes[..8]);
    std::fs::remove_file(&path).ok();
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn uninterpretable_input_exits_3() {
    let out = live(&["short", "zzzz qqqq xxxx"]);
    // 3 is the "rephrase it" code a script can branch on.
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(stderr(&out).contains("501"), "{}", stderr(&out));
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn natural_language_is_understood_without_rewriting() {
    let out = live(&["ask", "how many people live in France", "--maxchars", "300"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let text = stdout(&out);
    assert!(text.contains("population"), "{text}");
    assert!(text.contains("million"), "{text}");
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn lang_option_localises_the_answer() {
    let out = live(&["--lang", "zh", "short", "capital of France"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("巴黎"),
        "expected a Chinese answer, got: {}",
        stdout(&out)
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn every_answer_carries_a_dated_source_line() {
    let annotated = live(&["ask", "2+2", "--maxchars", "200"]);
    let text = stdout(&annotated);
    // The footer is what makes a number attributable: which API, when, for what.
    assert!(text.contains("— Wolfram|Alpha llm-api"), "{text}");
    assert!(text.contains("fetched 20"), "{text}");
    assert!(text.contains("query \"2+2\""), "{text}");

    let bare = live(&["ask", "2+2", "--maxchars", "200", "--no-source"]);
    assert!(
        !stdout(&bare).contains("— Wolfram|Alpha"),
        "{}",
        stdout(&bare)
    );
}

#[test]
#[ignore = "needs WOLFRAM_APPID and consumes API quota"]
fn pods_json_keeps_image_only_sections() {
    let out = live(&["pods", "--json", "plot sin(x)/x"]);
    assert!(out.status.success(), "{}", stderr(&out));

    let body: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    let pods = body["queryresult"]["pods"].as_array().expect("pods array");

    // `format=plaintext` alone would drop these entirely.
    let has_image = pods.iter().any(|pod| {
        pod["subpods"]
            .as_array()
            .is_some_and(|subs| subs.iter().any(|s| s["img"]["src"].is_string()))
    });
    assert!(has_image, "no image subpod in {}", stdout(&out));
}
