use anyhow::Result;
use frostmirror_import::Importer;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Watches the `/incoming/` directory for new `.pkg` files and auto-imports them.
pub struct IncomingWatcher {
    incoming_dir: PathBuf,
    mirror_dir: PathBuf,
    config_dir: Option<PathBuf>,
}

impl IncomingWatcher {
    pub fn new(incoming_dir: PathBuf, mirror_dir: PathBuf) -> Self {
        Self {
            incoming_dir,
            mirror_dir,
            config_dir: None,
        }
    }

    /// When set, snapshot bundles dropped into the incoming directory will
    /// have their Config sections written to this directory.
    pub fn with_config_dir(mut self, config_dir: PathBuf) -> Self {
        self.config_dir = Some(config_dir);
        self
    }

    /// Start watching. Runs a blocking loop on a dedicated thread so it never
    /// starves the tokio async runtime that serves HTTP requests.
    pub async fn watch(&self) -> Result<()> {
        // Ensure subdirectories exist
        std::fs::create_dir_all(self.incoming_dir.join("done"))?;
        std::fs::create_dir_all(self.incoming_dir.join("failed"))?;

        let incoming_dir = self.incoming_dir.clone();
        let mirror_dir = self.mirror_dir.clone();
        let config_dir = self.config_dir.clone();

        // Run the entire watcher loop on a dedicated blocking thread so the
        // async runtime (and therefore the HTTP server / UI) stays responsive.
        tokio::task::spawn_blocking(move || {
            // Process any existing .pkg files first
            process_existing(&incoming_dir, &mirror_dir, config_dir.as_deref());

            let (tx, rx) = mpsc::channel();
            let mut watcher = match RecommendedWatcher::new(
                move |res: std::result::Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                },
                notify::Config::default().with_poll_interval(Duration::from_secs(2)),
            ) {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!("failed to create file watcher: {}", e);
                    return;
                }
            };

            if let Err(e) = watcher.watch(&incoming_dir, RecursiveMode::NonRecursive) {
                tracing::error!("failed to watch {}: {}", incoming_dir.display(), e);
                return;
            }

            tracing::info!(
                "watching {} for .pkg files",
                incoming_dir.display()
            );

            loop {
                match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(event) => {
                        if matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_)
                        ) {
                            // Wait a moment for the file write to complete
                            std::thread::sleep(Duration::from_secs(2));
                            process_existing(&incoming_dir, &mirror_dir, config_dir.as_deref());
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Periodic check for any .pkg files that might have been missed
                        process_existing(&incoming_dir, &mirror_dir, config_dir.as_deref());
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        tracing::error!("watcher channel disconnected");
                        break;
                    }
                }
            }
        })
        .await?;

        Ok(())
    }
}

/// Process all `.pkg` files currently sitting in the incoming directory.
/// Runs entirely on a blocking thread -- no async, no tokio runtime interaction.
fn process_existing(incoming_dir: &PathBuf, mirror_dir: &PathBuf, config_dir: Option<&std::path::Path>) {
    let entries: Vec<_> = match std::fs::read_dir(incoming_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.ends_with(".pkg") && e.file_type().map(|t| t.is_file()).unwrap_or(false)
            })
            .collect(),
        Err(_) => return,
    };

    for entry in entries {
        let path = entry.path();
        let filename = entry.file_name().to_string_lossy().to_string();

        // Check the file is not still being written (size stable over 1 second)
        let size_before = path.metadata().map(|m| m.len()).unwrap_or(0);
        std::thread::sleep(Duration::from_secs(1));
        let size_after = path.metadata().map(|m| m.len()).unwrap_or(0);
        if size_before != size_after {
            tracing::debug!("skipping {} (still being written)", filename);
            continue;
        }

        tracing::info!("detected package: {}", filename);

        let importer = match config_dir {
            Some(dir) => Importer::new(mirror_dir.clone()).with_config_dir(dir.to_path_buf()),
            None => Importer::new(mirror_dir.clone()),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            importer.import(&path)
        })) {
            Ok(Ok(result)) => {
                tracing::info!(
                    "successfully imported {} ({} crates)",
                    filename,
                    result.crate_count
                );
                // Move to done/
                let done_path = incoming_dir.join("done").join(&filename);
                move_file(&path, &done_path, &filename, "done");
            }
            Ok(Err(e)) => {
                tracing::error!("failed to import {}: {:#}", filename, e);
                // Move to failed/
                let failed_path = incoming_dir.join("failed").join(&filename);
                move_file(&path, &failed_path, &filename, "failed");
            }
            Err(_) => {
                tracing::error!("import panicked for {}", filename);
                let failed_path = incoming_dir.join("failed").join(&filename);
                move_file(&path, &failed_path, &filename, "failed");
            }
        }
    }
}

fn move_file(from: &std::path::Path, to: &std::path::Path, name: &str, dest_label: &str) {
    if let Err(e) = std::fs::rename(from, to) {
        tracing::warn!("failed to move {} to {}/: {} (falling back to copy)", name, dest_label, e);
        if let Err(e2) = std::fs::copy(from, to) {
            tracing::error!("failed to copy {} to {}/: {}", name, dest_label, e2);
            return;
        }
        let _ = std::fs::remove_file(from);
    }
}
