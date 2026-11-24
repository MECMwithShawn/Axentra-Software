//! Read-only contract checks against the real, signed-in `proton-drive`.
//! Ignored by default; run with
//! `cargo test -p axentra-clouddrive-provider --test proton_cli_live -- --ignored`.
//!
//! Only `version` and `filesystem list --json /my-files` are run: nothing
//! is uploaded, created or downloaded. Nothing read is printed but counts.
//! `PROTON_DRIVE_BIN` overrides where the binary is looked for.

use std::path::PathBuf;
use std::time::Duration;

use axentra_clouddrive_provider::proton_cli::{parse_list, CheckedArgv, CliCommand, RunLimits};
use axentra_clouddrive_provider::{
    CliRunner, ItemKind, ProcessRunner, ProtonCliConfig, ProtonCliDestination, RemotePath,
};

fn binary() -> PathBuf {
    std::env::var_os("PROTON_DRIVE_BIN")
        .map(PathBuf::from)
        .or_else(ProtonCliConfig::default_binary)
        .expect("no HOME or XDG_DATA_HOME to find proton-drive under")
}

#[tokio::test]
#[ignore = "runs the real proton-drive against the signed-in account (read-only)"]
async fn live_version_and_my_files_listing() {
    let config = ProtonCliConfig::new(binary());
    let runner = ProcessRunner::new(&config.binary, None).unwrap();
    let d = ProtonCliDestination::new(config, runner.clone()).unwrap();
    // The pinned version and SHA-512.
    d.check_binary().await.unwrap();

    let cmd = CliCommand::List(RemotePath::my_files());
    assert!(!cmd.is_mutating());
    let argv = CheckedArgv::new(cmd.argv()).unwrap();
    let out = runner
        .run(
            &argv,
            RunLimits {
                timeout: Duration::from_secs(300),
                max_stdout: 64 << 20,
                max_stderr: 1 << 20,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        out.code,
        Some(0),
        "proton-drive list failed (stderr not shown)"
    );
    assert!(!out.truncated);
    let nodes = parse_list(&out.stdout).unwrap();
    let folders = nodes.iter().filter(|n| n.kind == ItemKind::Folder).count();
    eprintln!(
        "/my-files: {} entries ({folders} folders, {} files)",
        nodes.len(),
        nodes.len() - folders
    );
}
