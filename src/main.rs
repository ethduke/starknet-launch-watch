use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use starknet_launch_watch::{
    config::{Registry, Settings},
    decode::{Receipt, decode_receipt},
    engine::Engine,
    enrich,
    rpc::Rpc,
};
use std::{path::PathBuf, sync::atomic::Ordering};

#[derive(Parser)]
#[command(version, about = "Read-only Starknet launch and pool observations")]
struct Cli {
    #[arg(long, global = true)]
    console: bool,
    #[arg(long, global = true, default_value = "config/mainnet.json")]
    registry: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Decode committed public launch receipts offline. No credentials or network.
    Demo {
        #[arg(long, default_value = "tests/fixtures/mainnet-receipts.json")]
        fixtures: String,
    },
    /// Check RPC chain, deployment hashes and event ABI compatibility. Does not send alerts.
    Doctor,
    /// Watch accepted blocks. Console output unless both Telegram variables are configured.
    Watch {
        #[arg(long)]
        once: bool,
    },
    /// Replay an explicit range to console. Historical Telegram sending requires --telegram.
    Replay {
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
        #[arg(long, default_value = "data/replay.sqlite")]
        database: PathBuf,
        #[arg(long)]
        telegram: bool,
    },
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    if std::path::Path::new(".env").exists() {
        dotenvy::from_filename(".env")
            .map_err(|_| anyhow::anyhow!("invalid local .env file (contents suppressed)"))?;
    }
    let cli = Cli::parse();
    let path = if cli.registry == "config/mainnet.json" {
        std::env::var("REGISTRY_PATH").unwrap_or(cli.registry)
    } else {
        cli.registry
    };
    let registry = Registry::load(&path)?;
    if let Command::Demo { fixtures } = cli.command {
        let receipts: Vec<Receipt> = serde_json::from_slice(&std::fs::read(fixtures)?)?;
        let mut count = 0;
        for r in receipts {
            for l in decode_receipt(&registry, &r)? {
                let mut d = enrich::Details {
                    snapshot_block: l.block_number,
                    ..Default::default()
                };
                for a in &l.tokens {
                    d.tokens.push(enrich::TokenInfo {
                        address: a.clone(),
                        ..Default::default()
                    });
                }
                println!("{}\n", enrich::message(&l, &d));
                count += 1;
            }
        }
        eprintln!("Offline demo: {count} observations decoded; enrichment deliberately unknown");
        return Ok(());
    }
    let force_console = cli.console
        || matches!(
            &cli.command,
            Command::Doctor
                | Command::Replay {
                    telegram: false,
                    ..
                }
        );
    let mut settings = Settings::load(force_console)?;
    if let Command::Doctor = cli.command {
        let rpc = Rpc::new(settings.rpc_url)?;
        let head = rpc.head().await?;
        rpc.verify(&registry, head, true).await?;
        let version = rpc
            .call("starknet_specVersion", serde_json::json!([]))
            .await?;
        println!("RPC verified: Starknet mainnet | accepted head {head} | spec {version}");
        for s in registry.sources {
            println!(
                "{}: address {}, reviewed class {}",
                s.name, s.address, s.class_hash
            );
        }
        println!(
            "Telegram: {}",
            if settings.telegram.is_some() {
                "configured"
            } else {
                "disabled for this check"
            }
        );
        return Ok(());
    }
    if let Command::Replay { ref database, .. } = cli.command {
        settings.db_path = database.clone();
    }
    if let Some(parent) = settings
        .db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = settings.db_path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().map_err(|_| {
        anyhow::anyhow!("another worker owns this database; use a separate DATABASE_PATH")
    })?;
    let mut engine = Engine::new(&settings, registry)?;
    match cli.command {
        Command::Watch { once } => engine.watch(&settings, once).await?,
        Command::Replay {
            from, to, telegram, ..
        } => {
            ensure!(from <= to, "--from must be <= --to");
            ensure!(
                !telegram || settings.telegram.is_some(),
                "historical Telegram delivery requires both Telegram variables and no --console flag"
            );
            ensure!(
                to <= engine.rpc.head().await?,
                "requested range exceeds accepted head"
            );
            engine.rpc.verify(&engine.registry, to, false).await?;
            if let Some((cursor, _)) = engine.db.cursor()? {
                ensure!(
                    cursor >= from.saturating_sub(1) && cursor <= to,
                    "replay database cursor outside requested range; use a new --database path"
                );
            }
            if engine.db.cursor()?.is_none() && from > 0 {
                engine
                    .db
                    .commit(from - 1, &engine.rpc.hash(from - 1).await?, &[])?;
            }
            engine.catch_up(from, to).await?;
        }
        _ => unreachable!(),
    }
    let (events, delivered) = engine.db.counts()?;
    eprintln!(
        "Stored observations: {events}; delivered: {delivered}; RPC requests this run: {}",
        engine.rpc.calls.load(Ordering::Relaxed)
    );
    drop(lock);
    Ok(())
}
