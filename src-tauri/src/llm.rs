use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use keyring::Entry;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

const KEYRING_SERVICE: &str = "Nomad Local AI";
const KEYRING_USER: &str = "openai-api-key";

#[derive(Debug, Clone, Serialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub instructions: String,
    pub messages: Vec<LlmMessage>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(&self, request: LlmRequest, deltas: UnboundedSender<String>) -> Result<String>;
}

pub struct OpenAiProvider {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn from_keychain() -> Result<Self> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
        let api_key = entry
            .get_password()
            .context("Cheia OpenAI nu este configurată. Deschide Setări.")?;
        if api_key.trim().is_empty() {
            bail!("Cheia OpenAI este goală");
        }
        Ok(Self {
            api_key,
            client: reqwest::Client::new(),
        })
    }

    pub fn save_key(api_key: &str) -> Result<()> {
        if api_key.trim().len() < 12 {
            bail!("Cheia API pare invalidă");
        }
        Entry::new(KEYRING_SERVICE, KEYRING_USER)?.set_password(api_key.trim())?;
        Ok(())
    }

    pub fn has_key() -> bool {
        Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|entry| entry.get_password())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }
}

fn take_sse_events(buffer: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut events = Vec::new();
    loop {
        let lf = buffer
            .windows(2)
            .position(|window| window == b"\n\n")
            .map(|index| (index, 2));
        let crlf = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| (index, 4));
        let Some((index, separator_len)) =
            [lf, crlf].into_iter().flatten().min_by_key(|value| value.0)
        else {
            break;
        };
        let event = buffer.drain(..index).collect::<Vec<_>>();
        buffer.drain(..separator_len);
        events.push(event);
    }
    events
}

fn event_data(event: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(event).ok()?;
    let values = text
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data: "));
    let combined = values.collect::<Vec<_>>().join("\n");
    (!combined.is_empty()).then_some(combined)
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn stream(&self, request: LlmRequest, deltas: UnboundedSender<String>) -> Result<String> {
        let body = serde_json::json!({
            "model": request.model,
            "instructions": request.instructions,
            "input": request.messages,
            "stream": true,
            "store": false
        });
        let response = self
            .client
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Nu mă pot conecta la OpenAI")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!(
                "OpenAI a răspuns cu {}: {}",
                status,
                detail.chars().take(600).collect::<String>()
            );
        }

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut complete = String::new();
        while let Some(chunk) = stream.next().await {
            buffer.extend_from_slice(&chunk.context("Fluxul OpenAI s-a întrerupt")?);
            for event in take_sse_events(&mut buffer) {
                let Some(data) = event_data(&event) else {
                    continue;
                };
                if data == "[DONE]" {
                    continue;
                }
                let value: Value =
                    serde_json::from_str(&data).context("Eveniment SSE OpenAI invalid")?;
                match value.get("type").and_then(Value::as_str) {
                    Some("response.output_text.delta") => {
                        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                            complete.push_str(delta);
                            let _ = deltas.send(delta.to_string());
                        }
                    }
                    Some("error") | Some("response.failed") => {
                        let message = value
                            .pointer("/error/message")
                            .or_else(|| value.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or("Răspunsul OpenAI a eșuat");
                        bail!("{message}");
                    }
                    _ => {}
                }
            }
        }
        Ok(complete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_sse_frames_and_keeps_remainder() {
        let mut buffer = b"data: {\"type\":\"one\"}\n\ndata: partial".to_vec();
        let events = take_sse_events(&mut buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(
            event_data(&events[0]).as_deref(),
            Some("{\"type\":\"one\"}")
        );
        assert_eq!(buffer, b"data: partial");
    }

    #[test]
    fn parses_crlf_sse_frames() {
        let mut buffer = b"data: ok\r\n\r\n".to_vec();
        assert_eq!(
            event_data(&take_sse_events(&mut buffer)[0]).as_deref(),
            Some("ok")
        );
    }
}
