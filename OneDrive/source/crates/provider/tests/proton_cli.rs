//! The Proton Drive CLI adapter, offline: the command allowlist and its
//! second check at the runner, the scrubbed environment, the version and
//! SHA-512 gate, parsing against fixtures taken from the v0.9.0 source and
//! (sanitized) from the real CLI, error mapping, the real process runner's
//! time limit and output cap, and the destination's calls through a fake
//! runner.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axentra_clouddrive_provider::proton_cli::{
    check_version, classify_message, parse_folder_size, parse_list, parse_node,
    parse_transfer_summary, parse_version, remote_name_problem, scrubbed_env, upload_name_problem,
    CheckedArgv, CliCommand, CliOutput, FakeReply, FileConflict, RunError, RunLimits,
    ARGV_LITERALS, PINNED_SHA512, PINNED_VERSION,
};
use axentra_clouddrive_provider::{
    Capability, CliRunner, ContentDigests, FakeRunner, ItemKind, MirrorDestination, ProcessRunner,
    ProtonCliConfig, ProtonCliDestination, ProviderError, RelPath, RemotePath,
};

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/proton_cli")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn rp(s: &str) -> RemotePath {
    RemotePath::parse(s).unwrap()
}

fn os(v: &[&str]) -> Vec<OsString> {
    v.iter().map(OsString::from).collect()
}

fn strs(cmd: &CliCommand) -> Vec<String> {
    cmd.argv()
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// A destination over a fake runner that has already passed the version
/// check (one `version` call answered).
fn dest() -> ProtonCliDestination<FakeRunner> {
    dest_with(|_| {})
}

fn dest_with(f: impl FnOnce(&mut ProtonCliConfig)) -> ProtonCliDestination<FakeRunner> {
    let mut config = ProtonCliConfig::new("/opt/test/proton-drive");
    config.expected_sha512 = None;
    f(&mut config);
    let runner = FakeRunner::new();
    runner.reply(0, fixture("version.txt"), "");
    ProtonCliDestination::new(config, runner).unwrap()
}

fn not_found(name: &str) -> (i32, String, String) {
    (1, String::new(), format!("Node not found: {name}\n"))
}

// ----- commands and the allowlist ----------------------------------------------

#[test]
fn commands_build_exact_argument_lists() {
    let root = rp("/my-files/OneDrive");
    assert_eq!(strs(&CliCommand::Version), ["version"]);
    assert_eq!(strs(&CliCommand::AuthLogin), ["auth", "login"]);
    assert_eq!(
        strs(&CliCommand::List(root.clone())),
        ["filesystem", "list", "--json", "--", "/my-files/OneDrive"]
    );
    assert_eq!(
        strs(&CliCommand::Info(RemotePath::my_files())),
        ["filesystem", "info", "--json", "--", "/my-files"]
    );
    assert_eq!(
        strs(&CliCommand::Size(root.clone())),
        ["filesystem", "size", "--json", "--", "/my-files/OneDrive"]
    );
    assert_eq!(
        strs(&CliCommand::CreateFolder {
            parent: root.clone(),
            name: "-Docs".into()
        }),
        [
            "filesystem",
            "create-folder",
            "--json",
            "--",
            "/my-files/OneDrive",
            "-Docs"
        ]
    );
    assert_eq!(
        strs(&CliCommand::Upload {
            local: "/stage/x.d/a b.txt".into(),
            parent: root.clone(),
            on_conflict: FileConflict::Skip,
            thumbnails: false,
        }),
        [
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--skip-thumbnails",
            "--",
            "/stage/x.d/a b.txt",
            "/my-files/OneDrive"
        ]
    );
    assert_eq!(
        strs(&CliCommand::Upload {
            local: "/stage/x.d/a".into(),
            parent: root.clone(),
            on_conflict: FileConflict::CreateNewRevision,
            thumbnails: true,
        }),
        [
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "create-new-revision",
            "--",
            "/stage/x.d/a",
            "/my-files/OneDrive"
        ]
    );
    assert_eq!(
        strs(&CliCommand::Download {
            file: root.join("a").unwrap(),
            into: "/stage/y.d".into(),
        }),
        [
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/my-files/OneDrive/a",
            "/stage/y.d"
        ]
    );
    assert!(CliCommand::CreateFolder {
        parent: root.clone(),
        name: "x".into()
    }
    .is_mutating());
    assert!(!CliCommand::List(root).is_mutating());
}

/// A small deterministic generator, so the "property" runs are repeatable.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn name(&mut self) -> String {
        const PIECES: &[&str] = &[
            "a", "Z", "0", " ", "-", "--", ".", "..", "'", "\"", "$HOME", "`id`", ";", "&", "|",
            "é", "日本", "📄", "replace", "remove", "rename", "delete", "trash", "token=", "-f",
            "--json", "~", "=", "_", "\u{202e}", "%s", "\\", "/", "*", "[", "{", "?", "\u{1b}",
        ];
        let n = 1 + self.next() % 6;
        (0..n)
            .map(|_| PIECES[(self.next() % PIECES.len() as u64) as usize])
            .collect()
    }
}

#[test]
fn every_command_built_from_any_valid_name_passes_and_carries_nothing_extra() {
    let mut rng = Lcg(0xA11CE);
    let mut built = 0;
    for _ in 0..3000 {
        let (a, b) = (rng.name(), rng.name());
        let Ok(parent) = rp("/my-files/OneDrive").join(&a) else {
            assert!(remote_name_problem(&a).is_some(), "{a:?}");
            continue;
        };
        let local = PathBuf::from(format!("/stage/.axentra-part-1-2.d/{b}"));
        let mut cmds = vec![
            CliCommand::List(parent.clone()),
            CliCommand::Info(parent.clone()),
            CliCommand::Size(parent.clone()),
        ];
        let mut inputs = vec![parent.to_string()];
        if remote_name_problem(&b).is_none() {
            cmds.push(CliCommand::CreateFolder {
                parent: parent.clone(),
                name: b.clone(),
            });
            cmds.push(CliCommand::Download {
                file: parent.join(&b).unwrap(),
                into: "/stage/dl.d".into(),
            });
            inputs.push(b.clone());
            inputs.push(parent.join(&b).unwrap().to_string());
            inputs.push("/stage/dl.d".into());
        }
        if upload_name_problem(&b).is_none() {
            for on_conflict in [FileConflict::Skip, FileConflict::CreateNewRevision] {
                cmds.push(CliCommand::Upload {
                    local: local.clone(),
                    parent: parent.clone(),
                    on_conflict,
                    thumbnails: rng.next() & 1 == 0,
                });
            }
            inputs.push(local.to_str().unwrap().into());
        }
        for cmd in cmds {
            let argv = cmd.argv();
            let checked = CheckedArgv::new(argv.clone())
                .unwrap_or_else(|e| panic!("{cmd:?} was refused: {e}"));
            assert_eq!(checked.is_mutating(), cmd.is_mutating());
            let args: Vec<&str> = argv.iter().map(|a| a.to_str().unwrap()).collect();
            // Allowed subcommands only.
            assert!(
                matches!(
                    args.as_slice(),
                    [
                        "filesystem",
                        "list" | "info" | "size" | "create-folder" | "upload" | "download",
                        ..
                    ]
                ),
                "{args:?}"
            );
            // Options all before the "--", and only the safe strategies.
            let split = args.iter().position(|a| *a == "--").unwrap();
            for (i, a) in args[..split].iter().enumerate() {
                assert!(ARGV_LITERALS.contains(a), "{a:?} in {args:?}");
                if *a == "--file-conflict-strategy" {
                    assert!(["skip", "create-new-revision"].contains(&args[i + 1]));
                }
            }
            assert!(!args[..split].contains(&"replace"));
            assert!(!args[..split].contains(&"remove"));
            // Every other word is one of the paths or names it was given:
            // nothing from the environment, no secrets, nothing added.
            for a in &args[split + 1..] {
                assert!(inputs.iter().any(|i| i == a), "{a:?} in {args:?}");
            }
            built += 1;
        }
    }
    assert!(built > 1000, "only {built} commands were built");
}

#[test]
fn the_runner_boundary_refuses_anything_else() {
    let refused: &[&[&str]] = &[
        &[],
        &["--help"],
        &["auth", "logout"],
        &["auth", "login", "--json"],
        &["version", "--json"],
        &["filesystem", "delete", "--json", "--", "/my-files/a"],
        &["filesystem", "trash", "--json", "--", "/my-files/a"],
        &["filesystem", "empty-trash", "--json", "--"],
        &[
            "filesystem",
            "move",
            "--json",
            "--",
            "/my-files/a",
            "/my-files/b",
        ],
        &["filesystem", "rename", "--json", "--", "/my-files/a", "b"],
        &["filesystem", "restore", "--json", "--", "/trash/a"],
        &[
            "filesystem",
            "copy",
            "--json",
            "--",
            "/my-files/a",
            "/my-files/b",
        ],
        &[
            "sharing",
            "invite",
            "--json",
            "--",
            "/my-files/a",
            "x@example.invalid",
        ],
        &["albums", "list", "--json", "--"],
        &["photos", "upload", "--json", "--", "/tmp/x"],
        &["takeout", "run", "--json", "--", "/tmp/x"],
        // The CLI's aliases and prefix matching are not accepted either.
        &["fs", "list", "--json", "--", "/my-files"],
        &["filesystem", "lis", "--json", "--", "/my-files"],
        &["files", "list", "--json", "--", "/my-files"],
        // Missing --json, missing --, extra options.
        &["filesystem", "list", "--", "/my-files"],
        &["filesystem", "list", "--json", "/my-files"],
        &["filesystem", "list", "--json", "-v", "--", "/my-files"],
        &["filesystem", "list", "--json", "--json", "--", "/my-files"],
        &["filesystem", "list", "-j", "--", "/my-files"],
        &[
            "filesystem",
            "list",
            "--json",
            "--type",
            "file",
            "--",
            "/my-files",
        ],
        // Strategies that trash, delete, rename, or abbreviate.
        &[
            "filesystem",
            "upload",
            "--json",
            "-f",
            "replace",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "replace",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy=replace",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "rep",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "r",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "rename",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "Skip",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--file-conflict-strategy",
            "replace",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "-d",
            "replace",
            "--",
            "/s/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--folder-conflict-strategy",
            "merge",
            "--",
            "/s/a",
            "/my-files",
        ],
        &["filesystem", "upload", "--json", "--", "/s/a", "/my-files"],
        &[
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "remove",
            "--",
            "/my-files/a",
            "/s",
        ],
        &[
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "create-new-revision",
            "--",
            "/my-files/a",
            "/s",
        ],
        &[
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--skip-thumbnails",
            "--",
            "/my-files/a",
            "/s",
        ],
        // Paths outside /my-files, or that the CLI would read differently.
        &["filesystem", "list", "--json", "--", "/"],
        &["filesystem", "list", "--json", "--", "/trash"],
        &["filesystem", "list", "--json", "--", "/shared-with-me/x"],
        &["filesystem", "list", "--json", "--", "/my-filesX/a"],
        &["filesystem", "list", "--json", "--", "my-files/a"],
        &["filesystem", "list", "--json", "--", "/my-files/"],
        &["filesystem", "list", "--json", "--", "/my-files//a"],
        &["filesystem", "list", "--json", "--", "/my-files/../trash"],
        &["filesystem", "list", "--json", "--", "/my-files/a\\/b"],
        &[
            "filesystem",
            "list",
            "--json",
            "--",
            "/my-files/abcdefghijklmnopqrstuv~ABCDEFGHIJKLMNOPQRSTUV",
        ],
        &[
            "filesystem",
            "list",
            "--json",
            "--",
            "/my-files",
            "/my-files",
        ],
        &[
            "filesystem",
            "create-folder",
            "--json",
            "--",
            "/my-files",
            "a/b",
        ],
        &[
            "filesystem",
            "create-folder",
            "--json",
            "--",
            "/my-files",
            "..",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "relative/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "~/a",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/s/*",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/s/a[1].txt",
            "/my-files",
        ],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/s/a",
            "/s/b",
            "/my-files",
        ],
        &[
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/my-files/a",
            "/s/{x}",
        ],
    ];
    for argv in refused {
        assert!(
            CheckedArgv::new(os(argv)).is_err(),
            "{argv:?} should have been refused"
        );
    }
    // And what the commands build is accepted.
    for ok in [
        &["version"][..],
        &["auth", "login"],
        &["filesystem", "list", "--json", "--", "/my-files"],
        &[
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/s/-a",
            "/my-files",
        ],
    ] {
        CheckedArgv::new(os(ok)).unwrap();
    }
    assert!(CheckedArgv::new(os(&["auth", "login"])).unwrap().is_login());
}

#[test]
fn names_that_the_cli_would_misread_are_refused() {
    for ok in [
        "a",
        "-a",
        "a b",
        "Ünïcødé",
        "日本語 📄.txt",
        "a~b",
        ".hidden",
        "x'y\"z",
    ] {
        assert_eq!(remote_name_problem(ok), None, "{ok:?}");
        assert_eq!(upload_name_problem(ok), None, "{ok:?}");
    }
    for bad in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a\0b",
        "a\nb",
        "\u{1b}[31m",
        "\u{9b}x",
    ] {
        assert!(remote_name_problem(bad).is_some(), "{bad:?}");
    }
    // Shaped like a node id ("<22 chars>~<22 chars>", or 88..108 long parts).
    let uid = format!("{}~{}", "A".repeat(22), "b_-9".repeat(22));
    assert!(remote_name_problem(&uid).is_some());
    assert!(remote_name_problem(&format!("{}~{}", "A".repeat(22), "B".repeat(22))).is_some());
    assert!(remote_name_problem(&format!("{}~{}", "A".repeat(21), "B".repeat(22))).is_none());
    // Glob characters only matter for uploads.
    for glob in ["a*", "a?", "a[1]", "a{b}"] {
        assert_eq!(remote_name_problem(glob), None, "{glob:?}");
        assert!(upload_name_problem(glob).is_some(), "{glob:?}");
    }
    assert!(RemotePath::parse("/my-files/OneDrive").is_ok());
    assert!(RemotePath::parse("/my-files/a/../b").is_err());
    assert_eq!(
        rp("/my-files/a/b").parent().unwrap().to_string(),
        "/my-files/a"
    );
}

// ----- environment ------------------------------------------------------------------

fn sample_env() -> Vec<(OsString, OsString)> {
    [
        ("HOME", "/home/u"),
        ("USER", "u"),
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "C.UTF-8"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("XDG_CACHE_HOME", "/home/u/.cache"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("DISPLAY", ":0"),
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("PROTON_DRIVE_LOG_LEVEL", "DEBUG"),
        ("PROTON_DRIVE_BASE_URL", "evil.example"),
        ("PROTON_DRIVE_UNSAFE_CACHE", "1"),
        ("PROTON_DRIVE_CACHE_DIR", "/elsewhere"),
        ("PROTON_DRIVE_CREDENTIALS_STORE", "keychain"),
        ("LD_PRELOAD", "/tmp/x.so"),
        ("NODE_OPTIONS", "--require /tmp/x.js"),
        ("BUN_CONFIG", "x"),
        ("SSH_AUTH_SOCK", "/run/ssh"),
        ("AWS_SECRET_ACCESS_KEY", "s3cr3t"),
        ("GITHUB_TOKEN", "ghp_x"),
        ("HTTPS_PROXY", "http://proxy"),
        ("SHELL", "/bin/bash"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn keys(env: &[(OsString, OsString)]) -> Vec<String> {
    env.iter()
        .map(|(k, _)| k.to_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_environment_is_an_allowlist_with_quiet_logging() {
    let env = scrubbed_env(sample_env(), false, None).unwrap();
    assert_eq!(
        keys(&env),
        [
            "DBUS_SESSION_BUS_ADDRESS",
            "HOME",
            "LANG",
            "LC_ALL",
            "PATH",
            "PROTON_DRIVE_LOG_LEVEL",
            "USER",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
        ]
    );
    let level = env
        .iter()
        .find(|(k, _)| k == "PROTON_DRIVE_LOG_LEVEL")
        .unwrap();
    assert_eq!(level.1, "WARNING");

    let login = scrubbed_env(sample_env(), true, Some(Path::new("/cache/pd"))).unwrap();
    let k = keys(&login);
    assert!(k.contains(&"DISPLAY".to_string()));
    assert!(k.contains(&"WAYLAND_DISPLAY".to_string()));
    let cache = login
        .iter()
        .find(|(k, _)| k == "PROTON_DRIVE_CACHE_DIR")
        .unwrap();
    assert_eq!(cache.1, "/cache/pd");
    assert!(!k.contains(&"PROTON_DRIVE_BASE_URL".to_string()));
}

#[test]
fn an_unsafe_credentials_store_is_refused() {
    for v in ["unsafe_file", "UNSAFE_FILE", " unsafe_file "] {
        let mut env = sample_env();
        env.push(("PROTON_DRIVE_CREDENTIALS_STORE".into(), v.into()));
        let err = scrubbed_env(env, false, None).unwrap_err();
        assert!(err.to_string().contains("unsafe_file"), "{err}");
        let mut env = sample_env();
        env.push(("PROTON_DRIVE_CREDENTIALS_STORE".into(), v.into()));
        assert!(ProcessRunner::with_env("/bin/true", env, None).is_err());
    }
    assert!(ProcessRunner::with_env("proton-drive", sample_env(), None).is_err());
}

// ----- the real process runner, with stand-in scripts ------------------------------

fn script(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("proton-drive");
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    p
}

fn limits(timeout_ms: u64, cap: usize) -> RunLimits {
    RunLimits {
        timeout: Duration::from_millis(timeout_ms),
        max_stdout: cap,
        max_stderr: cap,
    }
}

/// Run, retrying the rare ETXTBSY from a script that was just written
/// while another test thread forked.
async fn run(runner: &ProcessRunner, argv: &[&str], lim: RunLimits) -> Result<CliOutput, RunError> {
    let argv = CheckedArgv::new(os(argv)).unwrap();
    for _ in 0..50 {
        match runner.run(&argv, lim).await {
            Err(RunError::Spawn(e)) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => return other,
        }
    }
    panic!("the script stayed busy");
}

#[tokio::test]
async fn the_process_gets_only_the_scrubbed_environment_and_no_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let bin = script(
        dir.path(),
        "env; cat; echo \"args:$*\"; pwd; echo err >&2; exit 3",
    );
    let runner = ProcessRunner::with_env(&bin, sample_env(), None).unwrap();
    let out = run(&runner, &["version"], limits(10_000, 1 << 20))
        .await
        .unwrap();
    assert_eq!(out.code, Some(3));
    assert_eq!(out.stderr, b"err\n");
    let text = String::from_utf8(out.stdout).unwrap();
    let mut vars: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, _)| k))
        .collect();
    // sh may add PWD, SHLVL and _ of its own.
    vars.retain(|k| !["PWD", "SHLVL", "_", "OLDPWD"].contains(k));
    vars.sort();
    assert_eq!(
        vars,
        [
            "DBUS_SESSION_BUS_ADDRESS",
            "HOME",
            "LANG",
            "LC_ALL",
            "PATH",
            "PROTON_DRIVE_LOG_LEVEL",
            "USER",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
        ]
    );
    assert!(text.contains("PROTON_DRIVE_LOG_LEVEL=WARNING"));
    assert!(text.contains("args:version\n/\n"), "{text}");

    let login = run(&runner, &["auth", "login"], limits(10_000, 1 << 20))
        .await
        .unwrap();
    let text = String::from_utf8(login.stdout).unwrap();
    assert!(text.contains("DISPLAY=:0"));
    assert!(text.contains("WAYLAND_DISPLAY=wayland-0"));
}

#[tokio::test]
async fn a_slow_run_is_killed_with_its_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let bin = script(
        dir.path(),
        &format!("sleep 30 & echo $! > '{}'; wait", pidfile.display()),
    );
    let runner = ProcessRunner::with_env(&bin, sample_env(), None).unwrap();
    let started = std::time::Instant::now();
    let err = run(&runner, &["version"], limits(1500, 1024))
        .await
        .unwrap_err();
    assert!(matches!(err, RunError::TimedOut(_)), "{err}");
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // The grandchild is gone too (or at most a zombie awaiting its reaper).
    let mut alive = true;
    for _ in 0..100 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat.rsplit(") ").next().and_then(|s| s.chars().next());
        if stat.is_empty() || state == Some('Z') {
            alive = false;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!alive, "sleep {pid} survived the timeout");
}

#[tokio::test]
async fn dropping_a_run_kills_it() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let bin = script(
        dir.path(),
        &format!("echo $$ > '{}'; sleep 30", pidfile.display()),
    );
    let runner = ProcessRunner::with_env(&bin, sample_env(), None).unwrap();
    let argv = CheckedArgv::new(os(&["version"])).unwrap();
    let cancelled = tokio::time::timeout(
        Duration::from_millis(1500),
        runner.run(&argv, limits(60_000, 1024)),
    )
    .await;
    assert!(cancelled.is_err());
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    for _ in 0..100 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat.rsplit(") ").next().and_then(|s| s.chars().next());
        if stat.is_empty() || state == Some('Z') {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "{pid} survived being cancelled");
}

#[tokio::test]
async fn output_is_capped_and_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let bin = script(
        dir.path(),
        "head -c 300000 /dev/zero; head -c 5000 /dev/zero >&2",
    );
    let runner = ProcessRunner::with_env(&bin, sample_env(), None).unwrap();
    let out = run(&runner, &["version"], limits(10_000, 1000))
        .await
        .unwrap();
    assert_eq!(out.code, Some(0));
    assert!(out.truncated);
    assert_eq!(out.stdout.len(), 1000);
    assert_eq!(out.stderr.len(), 1000);

    // And the destination trusts none of it.
    let mut config = ProtonCliConfig::new(&bin);
    config.expected_sha512 = None;
    config.max_stdout = 1000;
    let d = ProtonCliDestination::new(config, runner).unwrap();
    let err = d.check_binary().await.unwrap_err();
    assert!(err.to_string().contains("output limit"), "{err}");
}

// ----- the binary gate ------------------------------------------------------------

#[test]
fn only_the_pinned_version_is_accepted() {
    assert_eq!(
        parse_version(&fixture("version.txt")).unwrap(),
        "cli-drive@0.9.0+75612299"
    );
    check_version("cli-drive@0.9.0+75612299").unwrap();
    check_version(PINNED_VERSION).unwrap();
    for bad in [
        "cli-drive@0.9.1",
        "cli-drive@0.9.0.1",
        "cli-drive@0.9.01",
        "cli-drive@0.10.0+0badc0de",
        "external-drive@0.9.0",
        "",
    ] {
        assert!(check_version(bad).is_err(), "{bad:?}");
    }
    assert!(parse_version(b"Proton Drive SDK js@0.22.2\n").is_err());
    assert!(parse_version(b"").is_err());
}

#[tokio::test]
async fn an_unknown_version_stops_everything() {
    let mut config = ProtonCliConfig::new("/opt/test/proton-drive");
    config.expected_sha512 = None;
    let runner = FakeRunner::new();
    runner.reply(0, fixture("version_newer.txt"), "");
    runner.reply(0, fixture("version_newer.txt"), "");
    let d = ProtonCliDestination::new(config, runner).unwrap();
    let err = d.list(&RelPath::root()).await.unwrap_err();
    assert!(matches!(err, ProviderError::Other(_)), "{err}");
    assert!(err.to_string().contains("cli-drive@0.10.0"), "{err}");
    assert!(!err.is_retryable());
    // Asked again, it checks again (a failure is not remembered as a pass),
    // and nothing but `version` ever ran.
    assert!(d.ensure_folder(&rel("a")).await.is_err());
    assert_eq!(
        d.runner().calls(),
        [vec!["version".to_string()], vec!["version".to_string()]]
    );
    assert_eq!(d.runner().mutating_calls(), 0);
}

#[tokio::test]
async fn the_version_is_checked_once() {
    let d = dest();
    d.runner().reply(0, fixture("list_empty.json"), "");
    d.runner().reply(0, fixture("list_empty.json"), "");
    assert!(d.list(&RelPath::root()).await.unwrap().is_empty());
    assert!(d.list(&RelPath::root()).await.unwrap().is_empty());
    let calls = d.runner().calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], ["version"]);
    assert_eq!(d.runner().timeouts()[0], Duration::from_secs(60));
}

#[tokio::test]
async fn the_binary_hash_is_checked_before_it_runs() {
    use sha2::{Digest, Sha512};
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("proton-drive");
    std::fs::write(&bin, b"not really proton-drive").unwrap();
    let good: String = Sha512::digest(b"not really proton-drive")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let mut config = ProtonCliConfig::new(&bin);
    assert_eq!(config.expected_sha512.as_deref(), Some(PINNED_SHA512));
    let d = ProtonCliDestination::new(config.clone(), FakeRunner::new()).unwrap();
    let err = d.check_binary().await.unwrap_err();
    assert!(err.to_string().contains("SHA-512"), "{err}");
    assert!(d.runner().calls().is_empty());

    config.expected_sha512 = Some(good.to_ascii_uppercase());
    let runner = FakeRunner::new();
    runner.reply(0, fixture("version.txt"), "");
    let d = ProtonCliDestination::new(config.clone(), runner).unwrap();
    d.check_binary().await.unwrap();

    config.binary = dir.path().join("missing");
    let d = ProtonCliDestination::new(config, FakeRunner::new()).unwrap();
    assert!(d.check_binary().await.is_err());
}

// ----- parsing ------------------------------------------------------------------------

#[test]
fn listings_parse_from_the_fixtures() {
    let folders = parse_list(&fixture("list_folders.json")).unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders.iter().all(|n| n.kind == ItemKind::Folder));
    assert_eq!(folders[1].name.as_deref(), Some("OneDrive"));
    assert_eq!(
        folders[0].uid,
        "VOLUMEPLACEHOLDER0000A~FOLDERPLACEHOLDER00001"
    );
    assert!(parse_list(&fixture("list_empty.json")).unwrap().is_empty());
    assert!(parse_list(b"[]").unwrap().is_empty());

    let mixed = parse_list(&fixture("list_mixed.json")).unwrap();
    assert_eq!(mixed.len(), 3);
    let file = &mixed[1];
    assert_eq!(file.kind, ItemKind::File);
    assert_eq!(file.name.as_deref(), Some("report.txt"));
    assert_eq!(file.size, Some(5));
    assert_eq!(
        file.sha1.as_deref(),
        Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
    );
    assert!(file.sha1_verified);
    assert!(file
        .revision
        .as_deref()
        .unwrap()
        .ends_with("REVISIONPLACEHOLDER001"));
    // A name Proton couldn't decrypt.
    assert_eq!(mixed[2].name, None);
}

#[test]
fn nodes_parse_from_the_fixtures() {
    let root = parse_node(&fixture("info_my_files.json")).unwrap();
    assert_eq!(root.kind, ItemKind::Folder);
    assert_eq!(root.owner_email.as_deref(), Some("owner@example.invalid"));
    assert_eq!(root.size, None);

    let file = parse_node(&fixture("info_file.json")).unwrap();
    assert_eq!(file.size, Some(5));
    assert_eq!(
        file.sha1.as_deref(),
        Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
    );
    // Older uploads may claim no digest at all.
    let legacy = parse_node(&fixture("info_file_no_sha1.json")).unwrap();
    assert_eq!(legacy.size, Some(1000));
    assert_eq!(legacy.sha1, None);
    assert!(!legacy.sha1_verified);

    let folder = parse_node(&fixture("create_folder.json")).unwrap();
    assert_eq!(folder.name.as_deref(), Some("New Folder"));
    let size = parse_folder_size(&fixture("size.json")).unwrap();
    assert_eq!((size.size, size.number_of_descendants), (4096, 3));
}

#[test]
fn transfer_summaries_parse_from_the_fixtures() {
    let ok = parse_transfer_summary(&fixture("upload_ok.json")).unwrap();
    assert_eq!((ok.transferred_items, ok.skipped_items), (1, 0));
    let skipped = parse_transfer_summary(&fixture("upload_skipped.json")).unwrap();
    assert_eq!(skipped.skipped_items, 1);
    let bad = parse_transfer_summary(&fixture("download_integrity.json")).unwrap();
    assert_eq!(bad.failures.len(), 1);
    assert!(bad.failures[0].error.starts_with("IntegrityError"));
    assert!(bad.failures[0].node_uid.is_some());
}

#[test]
fn output_that_does_not_parse_fails_closed() {
    for (what, bytes) in [
        ("list object", fixture("list_unknown_shape.json")),
        ("album", fixture("list_unknown_type.json")),
        ("garbage", b"Proton Drive CLI".to_vec()),
        ("undefined", b"undefined\n".to_vec()),
        ("truncated", b"[\n{\"uid\":\"x\",".to_vec()),
        ("trailing", b"[]\n[]".to_vec()),
        ("empty", Vec::new()),
    ] {
        let err = parse_list(&bytes).unwrap_err();
        assert!(matches!(err, ProviderError::Other(_)), "{what}: {err}");
    }
    assert!(parse_node(b"").is_err());
    assert!(parse_node(b"null").is_err());
    assert!(parse_node(br#"{"uid":"x","name":{"ok":true},"type":"file"}"#).is_err());
    assert!(parse_node(
        br#"{"uid":"x","name":{"ok":true,"value":"a"},"type":"file","activeRevision":{"uid":"r","claimedDigests":{"sha1":"zz","sha1Verified":true}}}"#
    )
    .is_err());
    assert!(parse_transfer_summary(&fixture("upload_unknown_shape.json")).is_err());
    assert!(parse_transfer_summary(
        br#"{"transferredItems":0,"transferredBytes":0,"skippedItems":0,"failedItems":2,"failures":[]}"#
    )
    .is_err());
    // An error message must not echo what it failed on (it may be a name).
    let err = parse_list(br#"[{"uid":"x","name":{"ok":true,"value":"secret-name.txt"},"type":7}]"#)
        .unwrap_err();
    assert!(!err.to_string().contains("secret-name"), "{err}");
}

// ----- error mapping ---------------------------------------------------------------

#[test]
fn messages_map_to_what_the_caller_can_do() {
    let txt = |f: &str| String::from_utf8(fixture(f)).unwrap();
    assert!(matches!(
        classify_message(&txt("stderr_auth_required.txt")),
        ProviderError::AuthRequired
    ));
    assert!(matches!(
        classify_message("Your session expired, please log in again"),
        ProviderError::AuthRequired
    ));
    assert!(matches!(
        classify_message(&txt("stderr_rate_limited.txt")),
        ProviderError::RateLimited { .. }
    ));
    assert!(matches!(
        classify_message("RateLimitedError: Too many server requests, please try again later"),
        ProviderError::RateLimited { .. }
    ));
    assert!(matches!(
        classify_message(&txt("stderr_offline.txt")),
        ProviderError::Offline(_)
    ));
    assert!(matches!(
        classify_message("ConnectionError: x"),
        ProviderError::Offline(_)
    ));
    assert!(matches!(
        classify_message(&txt("stderr_server_errors.txt")),
        ProviderError::Transient(_)
    ));
    assert!(matches!(
        classify_message("IntegrityError: Integrity verification failed"),
        ProviderError::Integrity(_)
    ));
    assert!(matches!(
        classify_message("DecryptionError: Failed to decrypt block"),
        ProviderError::Integrity(_)
    ));
    assert!(matches!(
        classify_message(
            "NodeWithSameNameExistsValidationError: A file or folder with that name already exists"
        ),
        ProviderError::Conflict(_)
    ));
    // A keyring that can't be reached is not "sign in again".
    let keyring = classify_message(&txt("stderr_keyring.txt"));
    assert!(matches!(keyring, ProviderError::Other(_)), "{keyring}");
    assert!(keyring.to_string().contains("keyring"));
    for other in [
        txt("stderr_path_not_supported.txt"),
        "ValidationError: Storage quota exceeded".into(),
        "something new".into(),
        String::new(),
    ] {
        let e = classify_message(&other);
        assert!(matches!(e, ProviderError::Other(_)), "{other:?}: {e}");
        assert!(!e.is_retryable());
    }
    // Control characters never pass through.
    assert!(!classify_message("bad\u{1b}[2Jthing")
        .to_string()
        .contains('\u{1b}'));
}

#[tokio::test]
async fn not_found_only_when_the_cli_says_so() {
    let d = dest();
    d.runner().reply(1, "", fixture("stderr_not_found.txt"));
    let err = d.stat(&rel("Missing Folder/x")).await.unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");

    // Not with a different exit code, or other output alongside.
    d.runner().reply(2, "", fixture("stderr_not_found.txt"));
    let err = d.stat(&rel("a")).await.unwrap_err();
    assert!(!matches!(err, ProviderError::NotFound(_)), "{err}");
    d.runner().reply(1, "{}", fixture("stderr_not_found.txt"));
    let err = d.stat(&rel("a")).await.unwrap_err();
    assert!(!matches!(err, ProviderError::NotFound(_)), "{err}");
    d.runner()
        .reply(1, "", "Node not found: a\nand something else\n");
    let err = d.stat(&rel("a")).await.unwrap_err();
    assert!(!matches!(err, ProviderError::NotFound(_)), "{err}");
    // A path type the CLI doesn't support is not "missing".
    d.runner()
        .reply(1, "", fixture("stderr_path_not_supported.txt"));
    let err = d.stat(&rel("a")).await.unwrap_err();
    assert!(matches!(err, ProviderError::Other(_)), "{err}");
    // Success with output that doesn't parse is not success, nor missing.
    d.runner().reply(0, "", "");
    let err = d.stat(&rel("a")).await.unwrap_err();
    assert!(matches!(err, ProviderError::Other(_)), "{err}");
    // Signed out, and rate limited.
    d.runner().reply(1, "", fixture("stderr_auth_required.txt"));
    assert!(matches!(
        d.list(&RelPath::root()).await.unwrap_err(),
        ProviderError::AuthRequired
    ));
    d.runner().reply(1, "", fixture("stderr_rate_limited.txt"));
    assert!(d.list(&RelPath::root()).await.unwrap_err().is_retryable());
    assert_eq!(d.runner().mutating_calls(), 0);
}

/// The CLI's once-per-process update notices (api/driveRequirementHook.ts)
/// don't hide what the error was.
#[tokio::test]
async fn update_notices_on_stderr_are_set_aside() {
    let d = dest();
    for notice in [
        "Update recommended: suggested SDK version 0.30.0 or newer (currently using 0.22.2)",
        "Update required: required SDK version 0.30.0 or newer (currently using 0.22.2)",
        "Update needed: unsupported feature for drive",
    ] {
        d.runner()
            .reply(1, "", format!("{notice}\nNode not found: Missing Folder\n"));
        let err = d.stat(&rel("Missing Folder/x")).await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)), "{err}");

        d.runner()
            .reply(1, "", format!("{notice}\nsomething unexpected\n"));
        let err = d.stat(&rel("a")).await.unwrap_err().to_string();
        assert!(
            err.contains("something unexpected") && err.contains("also said: Update"),
            "{err}"
        );
    }
    assert_eq!(d.runner().mutating_calls(), 0);
}

#[tokio::test]
async fn a_timeout_is_temporary_and_a_missing_binary_is_not() {
    let d = dest();
    d.runner().push(FakeReply::TimedOut);
    let err = d.list(&RelPath::root()).await.unwrap_err();
    assert!(matches!(err, ProviderError::Transient(_)), "{err}");
    assert!(err.is_retryable());

    let mut config = ProtonCliConfig::new("/nonexistent/proton-drive");
    config.expected_sha512 = None;
    let runner = ProcessRunner::with_env("/nonexistent/proton-drive", sample_env(), None).unwrap();
    let d = ProtonCliDestination::new(config, runner).unwrap();
    let err = d.check_binary().await.unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
}

// ----- the destination -----------------------------------------------------------

#[tokio::test]
async fn capabilities_never_include_more_than_it_does() {
    let d = dest();
    let caps = d.capabilities();
    assert!(caps.sha1 && caps.create_folder && caps.round_trip);
    assert!(!caps.revisions && !caps.change_feed && !caps.quick_xor_hash && !caps.sha256);
    // Revisions are off by default, and refused without running anything.
    let local = tempfile::NamedTempFile::new().unwrap();
    let err = d
        .upload_revision(&rel("a"), local.path())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ProviderError::Unsupported(Capability::Revisions)
    ));
    assert!(d.runner().calls().is_empty());
    assert!(dest_with(|c| c.revisions = true).capabilities().revisions);
}

#[tokio::test]
async fn account_comes_from_the_root_folder() {
    let d = dest();
    d.runner().reply(0, fixture("info_my_files.json"), "");
    let a = d.account().await.unwrap();
    assert_eq!(a.provider, "proton");
    assert_eq!(a.id, "VOLUMEPLACEHOLDER0000A~ROOTNODEPLACEHOLDER000");
    assert_eq!(a.email.as_deref(), Some("owner@example.invalid"));
    assert_eq!(
        d.runner().calls()[1],
        ["filesystem", "info", "--json", "--", "/my-files"]
    );
}

#[tokio::test]
async fn list_maps_entries_and_skips_unreadable_names() {
    let d = dest();
    d.runner().reply(0, fixture("list_mixed.json"), "");
    let entries = d.list(&rel("Docs")).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "Docs");
    assert_eq!(entries[0].kind, ItemKind::Folder);
    assert_eq!(entries[0].size, None);
    assert_eq!(entries[1].name, "report.txt");
    assert_eq!(entries[1].size, Some(5));
    assert_eq!(
        d.runner().calls()[1],
        [
            "filesystem",
            "list",
            "--json",
            "--",
            "/my-files/OneDrive/Docs"
        ]
    );
    assert_eq!(d.runner().timeouts()[1], Duration::from_secs(300));
}

#[tokio::test]
async fn ensure_folder_creates_each_missing_level_and_nothing_else() {
    let d = dest();
    let r = d.runner();
    let (c, o, e) = not_found("Docs");
    r.reply(c, o, e); // info /my-files/OneDrive/Docs/2026
    r.reply(0, fixture("info_my_files.json"), ""); // info .../OneDrive exists... as a folder
    let (c, o, e) = not_found("Docs");
    r.reply(c, o, e); // info .../OneDrive/Docs
    let docs = String::from_utf8(fixture("create_folder.json"))
        .unwrap()
        .replace("New Folder", "Docs");
    r.reply(0, docs, ""); // create-folder .../OneDrive Docs
    let (c, o, e) = not_found("2026");
    r.reply(c, o, e); // info .../Docs/2026
    let y = String::from_utf8(fixture("create_folder.json"))
        .unwrap()
        .replace("New Folder", "2026")
        .replace("FOLDERPLACEHOLDER00004", "FOLDERPLACEHOLDER00005");
    r.reply(0, y, "");
    let uid = d.ensure_folder(&rel("Docs/2026")).await.unwrap();
    assert_eq!(uid, "VOLUMEPLACEHOLDER0000A~FOLDERPLACEHOLDER00005");
    let calls: Vec<String> = r.calls().iter().map(|c| c.join(" ")).collect();
    assert_eq!(
        calls,
        [
            "version",
            "filesystem info --json -- /my-files/OneDrive/Docs/2026",
            "filesystem info --json -- /my-files/OneDrive",
            "filesystem info --json -- /my-files/OneDrive/Docs",
            "filesystem create-folder --json -- /my-files/OneDrive Docs",
            "filesystem info --json -- /my-files/OneDrive/Docs/2026",
            "filesystem create-folder --json -- /my-files/OneDrive/Docs 2026",
        ]
    );
    assert_eq!(r.mutating_calls(), 2);
    assert_eq!(r.pending(), 0);

    // Already there: one look, nothing made.
    r.reply(0, fixture("create_folder.json"), "");
    d.ensure_folder(&rel("New Folder")).await.unwrap();
    assert_eq!(r.mutating_calls(), 2);

    // A file in the way is a conflict.
    r.reply(0, fixture("info_file.json"), "");
    let err = d.ensure_folder(&rel("report.txt")).await.unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
}

#[tokio::test]
async fn a_folder_made_meanwhile_counts_as_made() {
    let d = dest();
    let r = d.runner();
    let (c, o, e) = not_found("New Folder");
    r.reply(c, o.clone(), e.clone()); // target
    r.reply(0, fixture("info_my_files.json"), ""); // root folder
    r.reply(c, o, e); // level
    r.reply(1, "", "A file or folder with that name already exists\n"); // create
    r.reply(0, fixture("create_folder.json"), ""); // look again: there
    let uid = d.ensure_folder(&rel("New Folder")).await.unwrap();
    assert!(uid.ends_with("FOLDERPLACEHOLDER00004"));
}

#[tokio::test]
async fn upload_new_stages_the_exact_name_alone_and_cleans_up() {
    let d = dest();
    let stage = tempfile::tempdir().unwrap();
    let local = stage.path().join("staged-blob");
    std::fs::write(&local, b"hello").unwrap();
    std::fs::write(stage.path().join("other"), b"x").unwrap();
    d.runner().reply(0, fixture("upload_ok.json"), "");
    d.runner().reply(0, fixture("info_file.json"), "");

    let stat = d
        .upload_new(&rel("Docs"), "report.txt", &local)
        .await
        .unwrap();
    assert_eq!(stat.claimed.size, 5);
    assert_eq!(stat.claimed, {
        let all = ContentDigests::of_bytes(b"hello");
        ContentDigests {
            size: all.size,
            sha1: all.sha1,
            ..Default::default()
        }
    });
    let uploads = d.runner().uploads();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].file_name, "report.txt");
    assert_eq!(uploads[0].siblings, ["report.txt"]);
    assert_eq!(uploads[0].data, b"hello");
    let calls = d.runner().calls();
    let up = &calls[1];
    assert_eq!(
        &up[..6],
        [
            "filesystem",
            "upload",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--skip-thumbnails"
        ]
    );
    assert!(up[7].starts_with(stage.path().to_str().unwrap()));
    assert!(up[7].ends_with("/report.txt"));
    assert_eq!(up[8], "/my-files/OneDrive/Docs");
    assert_eq!(
        calls[2],
        [
            "filesystem",
            "info",
            "--json",
            "--",
            "/my-files/OneDrive/Docs/report.txt"
        ]
    );
    // The staging folder holds what it did before.
    assert_eq!(listing(stage.path()), ["other", "staged-blob"]);
    assert_eq!(std::fs::read(&local).unwrap(), b"hello");
}

#[tokio::test]
async fn a_taken_name_is_a_conflict_and_nothing_is_replaced() {
    let d = dest();
    let stage = tempfile::tempdir().unwrap();
    let local = stage.path().join("blob");
    std::fs::write(&local, b"hello").unwrap();
    d.runner().reply(0, fixture("upload_skipped.json"), "");
    let err = d
        .upload_new(&rel(""), "report.txt", &local)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    // No stat after a skip, and only "skip" was ever asked for.
    assert_eq!(d.runner().calls().len(), 2);
    assert!(d.runner().calls()[1].contains(&"skip".to_string()));
    assert_eq!(listing(stage.path()), ["blob"]);
}

#[tokio::test]
async fn upload_failures_map_by_their_error_class() {
    let stage = tempfile::tempdir().unwrap();
    let local = stage.path().join("blob");
    std::fs::write(&local, b"hello").unwrap();
    type Check = fn(&ProviderError) -> bool;
    let cases: [(&str, i32, Check); 4] = [
        ("upload_rate_limited.json", 1, |e| {
            matches!(e, ProviderError::RateLimited { .. })
        }),
        ("upload_quota.json", 1, |e| {
            matches!(e, ProviderError::Other(_))
        }),
        // Success with a summary that doesn't add up is not success.
        ("upload_unknown_shape.json", 0, |e| {
            matches!(e, ProviderError::Other(_))
        }),
        ("upload_ok.json", 1, |e| {
            matches!(e, ProviderError::Other(_))
        }),
    ];
    for (f, code, check) in cases {
        let d = dest();
        d.runner()
            .reply(code, fixture(f), fixture("stderr_upload_failed.txt"));
        let err = d.upload_new(&rel(""), "a.txt", &local).await.unwrap_err();
        assert!(check(&err), "{f}: {err}");
        assert_eq!(d.runner().calls().len(), 2, "{f}");
    }
    // The folder vanished: the CLI fails before printing a summary.
    let d = dest();
    let (c, o, e) = not_found("Docs");
    d.runner().reply(c, o, e);
    let err = d
        .upload_new(&rel("Docs"), "a.txt", &local)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");
    assert_eq!(listing(stage.path()), ["blob"]);
}

#[tokio::test]
async fn names_the_cli_would_misread_are_never_uploaded() {
    let d = dest();
    let stage = tempfile::tempdir().unwrap();
    let local = stage.path().join("blob");
    std::fs::write(&local, b"x").unwrap();
    for name in ["a[1].txt", "*", "a\\b", "x?"] {
        assert!(
            d.upload_new(&rel(""), name, &local).await.is_err(),
            "{name}"
        );
    }
    // A staging path with a glob character is refused at the boundary.
    let globby = stage.path().join("st[a]ge");
    std::fs::create_dir(&globby).unwrap();
    std::fs::write(globby.join("blob"), b"x").unwrap();
    let err = d
        .upload_new(&rel(""), "ok.txt", &globby.join("blob"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("refusing"), "{err}");
    // A link is not uploaded.
    std::os::unix::fs::symlink(&local, stage.path().join("link")).unwrap();
    assert!(d
        .upload_new(&rel(""), "ok.txt", &stage.path().join("link"))
        .await
        .is_err());
    // Nothing ran but the version check; nothing was left behind.
    assert_eq!(d.runner().calls().len(), 1);
    assert_eq!(listing(&globby), ["blob"]);
    assert_eq!(listing(stage.path()), ["blob", "link", "st[a]ge"]);
}

#[tokio::test]
async fn revisions_when_enabled_use_create_new_revision_only() {
    let d = dest_with(|c| c.revisions = true);
    let stage = tempfile::tempdir().unwrap();
    let local = stage.path().join("blob");
    std::fs::write(&local, b"hello").unwrap();
    let r = d.runner();
    r.reply(0, fixture("info_file.json"), "");
    r.reply(0, fixture("upload_ok.json"), "");
    r.reply(0, fixture("info_file.json"), "");
    d.upload_revision(&rel("Docs/report.txt"), &local)
        .await
        .unwrap();
    let up = &r.calls()[2];
    assert_eq!(up[4], "create-new-revision");
    assert_eq!(up.last().unwrap(), "/my-files/OneDrive/Docs");
    assert_eq!(r.uploads()[0].file_name, "report.txt");
    // Of nothing, it is not found, and nothing is uploaded.
    let (c, o, e) = not_found("x");
    r.reply(c, o, e);
    let err = d.upload_revision(&rel("x"), &local).await.unwrap_err();
    assert!(matches!(err, ProviderError::NotFound(_)), "{err}");
    assert_eq!(r.uploads().len(), 1);
}

#[tokio::test]
async fn stat_reports_only_verified_claims() {
    let d = dest();
    d.runner().reply(0, fixture("info_file.json"), "");
    let s = d.stat(&rel("Docs/report.txt")).await.unwrap();
    assert_eq!(s.uid, "VOLUMEPLACEHOLDER0000A~FILEPLACEHOLDER0000001");
    assert_eq!(
        s.claimed.sha1.as_deref(),
        Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
    );
    assert!(s.revision.is_some());

    // No digest claimed: no SHA-1, so L3 can't be reached for it.
    d.runner().reply(0, fixture("info_file_no_sha1.json"), "");
    let s = d.stat(&rel("legacy.bin")).await.unwrap();
    assert_eq!((s.claimed.size, s.claimed.sha1), (1000, None));

    // A claim whose signature didn't verify is dropped.
    let unverified = String::from_utf8(fixture("info_file.json"))
        .unwrap()
        .replace("\"sha1Verified\":true", "\"sha1Verified\":false");
    d.runner().reply(0, unverified, "");
    assert_eq!(d.stat(&rel("a")).await.unwrap().claimed.sha1, None);

    // A folder is not a file.
    d.runner().reply(0, fixture("create_folder.json"), "");
    assert!(d.stat(&rel("New Folder")).await.is_err());
    // A trashed file is in the way, not missing.
    let trashed = String::from_utf8(fixture("info_file.json"))
        .unwrap()
        .replace(
            "\"treeEventScopeId\"",
            "\"trashTime\":\"2026-02-01T00:00:00.000Z\",\"treeEventScopeId\"",
        );
    d.runner().reply(0, trashed, "");
    assert!(matches!(
        d.stat(&rel("a")).await.unwrap_err(),
        ProviderError::Conflict(_)
    ));
}

#[tokio::test]
async fn download_goes_through_a_fresh_folder_and_is_hashed_here() {
    let d = dest();
    let stage = tempfile::tempdir().unwrap();
    let dest_path = stage.path().join("roundtrip");
    let r = d.runner();
    r.reply(0, fixture("info_file.json"), "");
    r.push(FakeReply::Download {
        files: vec![("report.txt".into(), b"hello".to_vec())],
        output: CliOutput::new(0, fixture("download_ok.json"), ""),
    });
    let got = d
        .download_to(&rel("Docs/report.txt"), &dest_path)
        .await
        .unwrap();
    assert_eq!(got, ContentDigests::of_bytes(b"hello"));
    assert_eq!(std::fs::read(&dest_path).unwrap(), b"hello");
    assert_eq!(listing(stage.path()), ["roundtrip"]);
    let dl = &r.calls()[2];
    assert_eq!(
        &dl[..7],
        [
            "filesystem",
            "download",
            "--json",
            "--file-conflict-strategy",
            "skip",
            "--",
            "/my-files/OneDrive/Docs/report.txt"
        ]
    );
    assert!(dl[7].starts_with(stage.path().to_str().unwrap()));
    // The time limit grows with the claimed size.
    assert_eq!(r.timeouts()[2], Duration::from_secs(300));
    assert_eq!(r.mutating_calls(), 0);
}

#[tokio::test]
async fn a_failed_download_leaves_nothing() {
    let stage = tempfile::tempdir().unwrap();
    let dest_path = stage.path().join("roundtrip");

    // The CLI's own integrity check failed.
    let d = dest();
    d.runner().reply(0, fixture("info_file.json"), "");
    d.runner().reply(
        1,
        fixture("download_integrity.json"),
        fixture("stderr_download_failed.txt"),
    );
    let err = d
        .download_to(&rel("report.txt"), &dest_path)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Integrity(_)), "{err}");

    // Skipped (a Proton Doc, say).
    let d = dest();
    d.runner().reply(0, fixture("info_file.json"), "");
    d.runner().reply(0, fixture("download_skipped.json"), "");
    assert!(d.download_to(&rel("report.txt"), &dest_path).await.is_err());

    // More than one file appeared, or none.
    for files in [
        vec![
            ("a".to_string(), b"x".to_vec()),
            ("b".to_string(), b"y".to_vec()),
        ],
        vec![],
    ] {
        let d = dest();
        d.runner().reply(0, fixture("info_file.json"), "");
        d.runner().push(FakeReply::Download {
            files,
            output: CliOutput::new(0, fixture("download_ok.json"), ""),
        });
        let err = d
            .download_to(&rel("report.txt"), &dest_path)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("one file"), "{err}");
    }
    assert!(listing(stage.path()).is_empty());

    // Something there already is never replaced.
    let d = dest();
    std::fs::write(&dest_path, b"keep").unwrap();
    d.runner().reply(0, fixture("info_file.json"), "");
    d.runner().push(FakeReply::Download {
        files: vec![("report.txt".into(), b"hello".to_vec())],
        output: CliOutput::new(0, fixture("download_ok.json"), ""),
    });
    let err = d
        .download_to(&rel("report.txt"), &dest_path)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Conflict(_)), "{err}");
    assert_eq!(std::fs::read(&dest_path).unwrap(), b"keep");
    assert_eq!(listing(stage.path()), ["roundtrip"]);
}

#[tokio::test]
async fn reading_never_mutates() {
    let d = dest();
    let r = d.runner();
    r.reply(0, fixture("info_my_files.json"), "");
    r.reply(0, fixture("list_folders.json"), "");
    r.reply(0, fixture("info_file.json"), "");
    r.reply(0, fixture("size.json"), "");
    d.account().await.unwrap();
    d.list(&RelPath::root()).await.unwrap();
    d.stat(&rel("a")).await.unwrap();
    assert_eq!(d.folder_size(&RelPath::root()).await.unwrap().size, 4096);
    assert_eq!(r.mutating_calls(), 0);
    assert_eq!(r.calls().len(), 5);
}

#[test]
fn the_root_must_be_in_my_files() {
    for bad in [
        "/",
        "/trash",
        "OneDrive",
        "/my-files/../x",
        "/my-files/a\\/b",
    ] {
        let mut c = ProtonCliConfig::new("/x/proton-drive");
        c.root = bad.into();
        assert!(
            ProtonCliDestination::new(c, FakeRunner::new()).is_err(),
            "{bad:?}"
        );
    }
    let mut c = ProtonCliConfig::new("/x/proton-drive");
    c.root = "/my-files/Backups/OneDrive".into();
    let d = ProtonCliDestination::new(c, FakeRunner::new()).unwrap();
    assert_eq!(
        d.remote_path(&rel("a/b")).unwrap().to_string(),
        "/my-files/Backups/OneDrive/a/b"
    );
}
