use std::time::Duration;

use rand::Rng;
use tokio::sync::mpsc;

use crate::backend::{ContainerBackend, PullProgress};

#[derive(Clone)]
pub struct MockBackend {
    pub name: String,
    pub image: String,
}

impl MockBackend {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
        }
    }
}

impl ContainerBackend for MockBackend {
    async fn pull_image(
        &self,
        progress_tx: mpsc::UnboundedSender<PullProgress>,
        log_tx: mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        let _ = log_tx.send(format!("Pulling {}...", self.image));

        // Pre-generate delays (ThreadRng is !Send)
        let delays: Vec<u64> = {
            let mut rng = rand::rng();
            (0..10).map(|_| rng.random_range(200..=500)).collect()
        };

        let total = 100_000_000u64;
        for (i, delay) in delays.into_iter().enumerate() {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let downloaded = total * (i as u64 + 1) / 10;
            let _ = progress_tx.send(PullProgress { downloaded, total });
        }

        let _ = log_tx.send("Pull complete".to_string());
        Ok(())
    }

    async fn boot_container(&self, log_tx: mpsc::UnboundedSender<String>) -> Result<(), String> {
        let boot_lines: Vec<(&str, u64)> = match self.name.as_str() {
            "postgres" => vec![
                ("PostgreSQL init process complete", 200),
                ("LOG: listening on 0.0.0.0:5432", 300),
            ],
            "redis" => vec![
                ("oO0OoO0Oo Redis is starting oO0OoO0Oo", 150),
                ("Ready to accept connections on port 6379", 250),
            ],
            "api-server" => vec![
                ("Connecting to database...", 300),
                ("Server listening on :8080", 400),
            ],
            "web-frontend" => vec![
                ("Compiling assets...", 400),
                ("Serving on http://0.0.0.0:3000", 300),
            ],
            _ => vec![],
        };

        for (text, delay) in boot_lines {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let _ = log_tx.send(text.to_string());
        }

        let startup_delay = { rand::rng().random_range(500..=1500u64) };
        tokio::time::sleep(Duration::from_millis(startup_delay)).await;
        Ok(())
    }

    async fn stop_container(&self) -> Result<(), String> {
        let delay = { rand::rng().random_range(200..=800u64) };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        Ok(())
    }
}
