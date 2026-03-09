use std::time::Duration;

use anyhow::bail;
use rand::Rng;

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
        on_progress: impl Fn(PullProgress) + Send,
        on_log: impl Fn(String) + Send,
    ) -> anyhow::Result<()> {
        on_log(format!("Pulling {}...", self.image));

        // Pre-generate delays (ThreadRng is !Send)
        let delays: Vec<u64> = {
            let mut rng = rand::rng();
            (0..100).map(|_| rng.random_range(10..=60)).collect()
        };

        let total = 100_000_000u64;
        for (i, delay) in delays.into_iter().enumerate() {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let downloaded = total * (i as u64 + 1) / 100;
            on_progress(PullProgress { downloaded, total });
        }

        on_log("Pull complete".to_string());
        Ok(())
    }

    async fn boot_container(&self, on_log: impl Fn(String) + Send) -> anyhow::Result<()> {
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
            ],
            "web-frontend" => vec![
                ("Compiling assets...", 400),
                ("Serving on http://0.0.0.0:3000", 300),
            ],
            _ => vec![],
        };

        for (text, delay) in boot_lines {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            on_log(text.to_string());
        }

        if self.name == "api-server" {
            let delay = { rand::rng().random_range(200..=500u64) };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            bail!("connection refused: postgres:5432");
        }

        let startup_delay = { rand::rng().random_range(500..=1500u64) };
        tokio::time::sleep(Duration::from_millis(startup_delay)).await;
        Ok(())
    }

    async fn stop_container(&self) -> anyhow::Result<()> {
        let delay = { rand::rng().random_range(200..=800u64) };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        Ok(())
    }
}
