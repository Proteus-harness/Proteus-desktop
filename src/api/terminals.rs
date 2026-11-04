//! Direct process control; input is never a model request or a retried command.
use super::Api;
use anyhow::{Result, bail};
use base64::Engine as _;
use proteus_contracts::contracts::{ToolTerminalResponse, ToolTerminalSnapshot};
use serde::Serialize;
use std::{io::Read as _, path::Path, time::Duration};
use uuid::Uuid;

#[cfg(test)]
#[path = "terminals_tests.rs"]
mod tests;

impl Api {
    pub fn terminal_read(
        &self,
        session: &Path,
        id: Uuid,
        cursor: Option<u64>,
    ) -> Result<ToolTerminalSnapshot> {
        let mut request = self
            .agent
            .get(format!("{}/terminals/read", self.origin))
            .header("Authorization", format!("Bearer {}", self.token))
            .query("session_dir", session.to_string_lossy())
            .query("id", id.to_string())
            .query("max_bytes", "65536")
            .query("wait_ms", "1000");
        if let Some(cursor) = cursor {
            request = request.query("cursor", cursor.to_string());
        }
        let mut response = request
            .config()
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .call()
            .map_err(|_| anyhow::anyhow!("Терминал недоступен"))?;
        if !response.status().is_success() {
            bail!("Терминал недоступен");
        }
        let mut data = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(128 * 1024)
            .read_to_end(&mut data)?;
        match serde_json::from_slice::<ToolTerminalResponse>(&data)? {
            ToolTerminalResponse::Read(snapshot) => Ok(snapshot),
            _ => bail!("Сервер не вернул вывод терминала"),
        }
    }

    pub fn terminal_input(&self, session: &Path, id: Uuid, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 65_536 {
            bail!("Слишком большой блок ввода терминала");
        }
        // Intentionally no Debug; body may contain a password.
        #[derive(Serialize)]
        struct Input<'a> {
            session_dir: &'a Path,
            id: Uuid,
            data_base64: String,
        }
        self.terminal_command(
            "input",
            &Input {
                session_dir: session,
                id,
                data_base64: base64::prelude::BASE64_STANDARD.encode(bytes),
            },
        )
    }

    pub fn terminal_resize(&self, session: &Path, id: Uuid, columns: u16, rows: u16) -> Result<()> {
        #[derive(Serialize)]
        struct Resize<'a> {
            session_dir: &'a Path,
            id: Uuid,
            columns: u16,
            rows: u16,
        }
        self.terminal_command(
            "resize",
            &Resize {
                session_dir: session,
                id,
                columns,
                rows,
            },
        )
    }

    pub fn terminal_stop(&self, session: &Path, id: Uuid) -> Result<()> {
        #[derive(Serialize)]
        struct Stop<'a> {
            session_dir: &'a Path,
            id: Uuid,
        }
        self.terminal_command(
            "stop",
            &Stop {
                session_dir: session,
                id,
            },
        )
    }

    fn terminal_command(&self, operation: &str, body: &impl Serialize) -> Result<()> {
        let mut response = self
            .agent
            .post(format!("{}/terminals/{operation}", self.origin))
            .header("Authorization", format!("Bearer {}", self.token))
            .config()
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .send_json(body)
            .map_err(|_| anyhow::anyhow!("Операция терминала не подтверждена"))?;
        if !response.status().is_success() {
            bail!("Операция терминала не подтверждена");
        }
        let mut data = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(4096)
            .read_to_end(&mut data)?;
        match serde_json::from_slice::<ToolTerminalResponse>(&data)? {
            ToolTerminalResponse::Accepted => Ok(()),
            _ => bail!("Операция терминала не подтверждена"),
        }
    }
}
