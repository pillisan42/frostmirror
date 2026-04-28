use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "frostmirror")]
#[command(about = "Lightweight dependency-scoped Rust mirror for air-gapped environments")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch dependencies and produce a .pkg bundle
    Fetch {
        /// Path to depends.toml
        #[arg(short, long, default_value = "depends.toml")]
        config: PathBuf,

        /// Output directory for .pkg files
        #[arg(short, long, env = "FROSTMIRROR_OUTPUT", default_value = "./output")]
        output: PathBuf,

        /// Only download crates not in the previous bundle (delta)
        #[arg(long)]
        incremental: bool,

        /// Skip downloading rustup-init binaries and toolchain components
        #[arg(long, conflicts_with = "include_rustup")]
        skip_rustup: bool,

        /// Force download of rustup data (suppress the interactive prompt)
        #[arg(long)]
        include_rustup: bool,
    },

    /// Import a .pkg bundle into the local mirror
    Import {
        /// Path to the .pkg file
        file: PathBuf,

        /// Mirror directory
        #[arg(long, env = "FROSTMIRROR_MIRROR", default_value = "/mirror")]
        mirror: PathBuf,

        /// Config directory. When importing a snapshot bundle, frostmirror.toml
        /// and depends.toml from the bundle are written here. Has no effect on
        /// regular fetch-produced bundles, which carry no config sections.
        #[arg(long, default_value = "/config")]
        config_dir: PathBuf,
    },

    /// Start the HTTP registry server
    Serve {
        /// Bind address
        #[arg(long, env = "FROSTMIRROR_BIND", default_value = "0.0.0.0:8080")]
        bind: String,

        /// Base URL for generated configs
        #[arg(long, env = "FROSTMIRROR_BASE_URL", default_value = "http://localhost:8080")]
        base_url: String,

        /// Mirror directory
        #[arg(long, env = "FROSTMIRROR_MIRROR", default_value = "/mirror")]
        mirror: PathBuf,

        /// Incoming directory for .pkg files
        #[arg(long, env = "FROSTMIRROR_INCOMING", default_value = "/incoming")]
        incoming: PathBuf,

        /// Watch incoming directory and auto-import .pkg files
        #[arg(long)]
        watch_incoming: bool,
    },

    /// Show mirror status
    Status {
        /// Mirror directory
        #[arg(long, env = "FROSTMIRROR_MIRROR", default_value = "/mirror")]
        mirror: PathBuf,
    },

    /// Verify a .pkg bundle
    Verify {
        /// Path to the .pkg file
        file: PathBuf,
    },

    /// Garbage collect unused crates from the mirror
    Gc {
        /// Mirror directory
        #[arg(long, env = "FROSTMIRROR_MIRROR", default_value = "/mirror")]
        mirror: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Fetch {
            config,
            output,
            incremental,
            skip_rustup,
            include_rustup,
        } => {
            let history_dir = frostmirror_fetch::default_history_dir();
            let skip = decide_skip_rustup(skip_rustup, include_rustup, &history_dir);
            let fetch_config = frostmirror_fetch::fetcher::FetchConfig::from_env(
                config,
                output,
                incremental,
                skip,
            );
            let fetcher = frostmirror_fetch::Fetcher::new(fetch_config);
            let pkg_path = fetcher.run().await?;
            println!("Bundle written to: {}", pkg_path.display());
        }

        Commands::Import {
            file,
            mirror,
            config_dir,
        } => {
            let importer = frostmirror_import::Importer::new(mirror).with_config_dir(config_dir);
            let result = importer.import(&file)?;
            println!(
                "Imported {} crates, {} rustup artifacts",
                result.crate_count, result.rustup_count
            );
        }

        Commands::Serve {
            bind,
            base_url,
            mirror,
            incoming,
            watch_incoming,
        } => {
            let server = frostmirror_serve::Server {
                mirror_dir: mirror,
                incoming_dir: incoming,
                config_path: PathBuf::from("/config/frostmirror.toml"),
                depends_path: PathBuf::from("/config/depends.toml"),
                bind_addr: bind,
                base_url,
                watch_incoming,
            };
            server.run().await?;
        }

        Commands::Status { mirror } => {
            let importer = frostmirror_import::Importer::new(mirror);
            let status = importer.status()?;
            println!("Crate count:  {}", status.crate_count);
            println!("Total size:   {} bytes", status.total_size);
            println!(
                "Last import:  {}",
                status.last_import.as_deref().unwrap_or("never")
            );
        }

        Commands::Verify { file } => {
            println!("Verifying {}...", file.display());
            let bundle = frostmirror_core::BundleReader::read_file(&file)?;
            frostmirror_core::BundleReader::verify(&bundle)?;
            println!(
                "OK — {} crates, {} rustup artifacts",
                bundle.manifest.crates.len(),
                bundle.manifest.rustup.len()
            );
        }

        Commands::Gc { mirror } => {
            let gc = frostmirror_import::GarbageCollector::new(mirror);
            let result = gc.run()?;
            println!(
                "Removed {} crates, freed {} bytes",
                result.removed, result.freed_bytes
            );
        }
    }

    Ok(())
}

/// Decide whether to skip the rustup download for this fetch.
///
/// Precedence:
/// 1. `--skip-rustup`        → skip.
/// 2. `--include-rustup`     → download.
/// 3. No prior rustup data   → download (nothing to "re"-download; no prompt).
/// 4. Stdin not a TTY        → download (preserve old non-interactive behavior).
/// 5. Otherwise              → prompt; default `N` (skip on Enter / non-yes input).
fn decide_skip_rustup(skip_flag: bool, include_flag: bool, history_dir: &Path) -> bool {
    if skip_flag {
        return true;
    }
    if include_flag {
        return false;
    }
    if !frostmirror_fetch::latest_manifest_has_rustup(history_dir) {
        return false;
    }
    if !std::io::stdin().is_terminal() {
        return false;
    }

    let mut stdout = std::io::stdout();
    let _ = write!(
        stdout,
        "Rustup data already present in the previous bundle. Redownload? [y/N]: "
    );
    let _ = stdout.flush();

    let mut line = String::new();
    let stdin = std::io::stdin();
    let answer = match stdin.lock().read_line(&mut line) {
        Ok(_) => line.trim().to_ascii_lowercase(),
        Err(_) => String::new(),
    };
    let download = matches!(answer.as_str(), "y" | "yes");
    if download {
        eprintln!("→ including rustup data");
        false
    } else {
        eprintln!("→ skipping rustup data");
        true
    }
}
