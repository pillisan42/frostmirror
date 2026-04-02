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
}

impl IncomingWatcher {
    pub fn new(incoming_dir: PathBuf, mirror_dir: PathBuf) -> Self {
        Self {
            incoming_dir,
            mirror_dir,
        }
    }

    /// Start watching. This runs in a loop and never returns under normal operation.
    pub async fn watch(&self) -> Result<()> {
        // Ensure subdirectories exist
        std::fs::create_dir_all(self.incoming_dir.join("done"))?;
        std::fs::create_dir_all(self.incoming_dir.join("failed"))?;

        // Process any existing .pkg files first
        self.process_existing().await;

        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            notify::Config::default().with_poll_interval(Duration::from_secs(2)),
        )?;

        watcher.watch(&self.incoming_dir, RecursiveMode::NonRecursive)?;
        tracing::info!(
            "watching {} for .pkg files",
            self.incoming_dir.display()
        );

        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(event) => {
                    if matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_)
                    ) {
                        // Wait a moment for the file write to complete
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        self.process_existing().await;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Periodic check for any .pkg files that might have been missed
                    self.process_existing().await;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::error!("watcher channel disconnected");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn process_existing(&self) {
        let entries: Vec<_> = match std::fs::read_dir(&self.incoming_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .ends_with(".pkg")
                })
                .collect(),
            Err(_) => return,
        };

        for entry in entries {
            let path = entry.path();
            let filename = entry.file_name().to_string_lossy().to_string();
            tracing::info!("detected package: {}", filename);

            let importer = Importer::new(self.mirror_dir.clone());
            match importer.import(&path) {
                Ok(result) => {
                    tracing::info!(
                        "successfully imported {} ({} crates)",
                        filename,
                        result.crate_count
                    );
                    // Move to done/
                    let done_path = self.incoming_dir.join("done").join(&filename);
                    if let Err(e) = std::fs::rename(&path, &done_path) {
                        tracing::warn!("failed to move {} to done/: {}", filename, e);
                        // Fallback: copy + remove
                        let _ = std::fs::copy(&path, &done_path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
                Err(e) => {
                    tracing::error!("failed to import {}: {}", filename, e);
                    // Move to failed/
                    let failed_path = self.incoming_dir.join("failed").join(&filename);
                    if let Err(e2) = std::fs::rename(&path, &failed_path) {
                        tracing::warn!("failed to move {} to failed/: {}", filename, e2);
                        let _ = std::fs::copy(&path, &failed_path);
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
}
