use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use keyring::Entry;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
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
    pub workspace: Option<PathBuf>,
    pub max_output_tokens: Option<u32>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(&self, request: LlmRequest, deltas: UnboundedSender<String>) -> Result<String>;
}

pub struct OpenAiProvider {
    api_key: Option<String>,
    endpoint: String,
    protocol: ApiProtocol,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy)]
enum ApiProtocol {
    OpenAiCompatible,
    AnthropicMessages,
    GoogleGemini,
    GoogleVertexExpress,
    AzureOpenAi,
}

impl ApiProtocol {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "openai-compatible" => Ok(Self::OpenAiCompatible),
            "anthropic-messages" => Ok(Self::AnthropicMessages),
            "google-gemini" => Ok(Self::GoogleGemini),
            "google-vertex-express" => Ok(Self::GoogleVertexExpress),
            "azure-openai" => Ok(Self::AzureOpenAi),
            _ => bail!("Formatul API selectat nu este disponibil"),
        }
    }
}

impl OpenAiProvider {
    pub fn from_keychain(
        configured_endpoint: Option<&str>,
        credential_id: &str,
        environment_variable: Option<&str>,
        protocol: &str,
    ) -> Result<Self> {
        let entry = Entry::new(KEYRING_SERVICE, credential_id)?;
        let configured_endpoint = configured_endpoint.unwrap_or("https://api.openai.com");
        let endpoint = configured_endpoint.trim().trim_end_matches('/').to_string();
        let protocol = ApiProtocol::parse(protocol)?;
        let endpoint_url = url::Url::parse(&endpoint)?;
        let host = endpoint_url.host_str().unwrap_or_default();
        let local_http =
            endpoint_url.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        if endpoint_url.scheme() != "https" && !local_http {
            bail!(
                "Endpoint-ul trebuie să folosească HTTPS (HTTP este permis doar pentru localhost)"
            );
        }
        let api_key = environment_variable
            .and_then(|name| std::env::var(name).ok())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                entry
                    .get_password()
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
        if api_key.is_none() && !local_http {
            bail!("Cheia API nu este configurată. Deschide Setări AI.");
        }
        Ok(Self {
            api_key,
            endpoint,
            protocol,
            client: reqwest::Client::new(),
        })
    }

    pub fn save_key(api_key: &str) -> Result<()> {
        Self::save_key_for(KEYRING_USER, api_key)
    }

    pub fn save_key_for(credential_id: &str, api_key: &str) -> Result<()> {
        if api_key.trim().len() < 12 {
            bail!("Cheia API pare invalidă");
        }
        Entry::new(KEYRING_SERVICE, credential_id)?.set_password(api_key.trim())?;
        Ok(())
    }

    pub fn has_key() -> bool {
        Self::has_key_for(KEYRING_USER)
    }

    pub fn has_key_for(credential_id: &str) -> bool {
        Entry::new(KEYRING_SERVICE, credential_id)
            .and_then(|entry| entry.get_password())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn delete_key() -> Result<()> {
        Self::delete_key_for(KEYRING_USER)
    }

    pub fn delete_key_for(credential_id: &str) -> Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, credential_id)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

pub fn chat_completions_endpoint(base: &str) -> Result<String> {
    let mut url =
        url::Url::parse(base.trim()).context("Endpoint-ul providerului nu este un URL valid")?;
    let host = url.host_str().unwrap_or_default();
    let local_http = url.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if url.scheme() != "https" && !local_http {
        bail!("Endpoint-ul trebuie să folosească HTTPS (HTTP este permis doar pentru localhost)");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Endpoint-ul nu poate include credențiale, query sau fragment");
    }
    let path = url.path().trim_end_matches('/');
    if path.is_empty() || path == "/" {
        url.set_path("/v1/chat/completions");
    } else if path.ends_with("/chat/completions") {
        // The caller already supplied a full endpoint.
    } else {
        url.set_path(&format!("{path}/chat/completions"));
    }
    Ok(url.to_string())
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

fn append_endpoint_path(base: &str, suffix: &str) -> Result<String> {
    let mut url = url::Url::parse(base)?;
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/{suffix}"));
    Ok(url.to_string())
}

impl OpenAiProvider {
    fn stream_endpoint(&self, model: &str) -> Result<String> {
        match self.protocol {
            ApiProtocol::OpenAiCompatible | ApiProtocol::AzureOpenAi => {
                chat_completions_endpoint(&self.endpoint)
            }
            ApiProtocol::AnthropicMessages => {
                if self.endpoint.ends_with("/v1/messages") {
                    Ok(self.endpoint.clone())
                } else if self.endpoint.ends_with("/v1") {
                    append_endpoint_path(&self.endpoint, "messages")
                } else {
                    append_endpoint_path(&self.endpoint, "v1/messages")
                }
            }
            ApiProtocol::GoogleGemini | ApiProtocol::GoogleVertexExpress => {
                let encoded_model =
                    url::form_urlencoded::byte_serialize(model.as_bytes()).collect::<String>();
                let model_path = if matches!(self.protocol, ApiProtocol::GoogleVertexExpress) {
                    format!("publishers/google/models/{encoded_model}")
                } else {
                    format!("models/{encoded_model}")
                };
                let endpoint = append_endpoint_path(
                    &self.endpoint,
                    &format!("{model_path}:streamGenerateContent"),
                )?;
                let mut url = url::Url::parse(&endpoint)?;
                url.query_pairs_mut().append_pair("alt", "sse");
                Ok(url.to_string())
            }
        }
    }

    fn request_body(&self, request: &LlmRequest) -> Value {
        match self.protocol {
            ApiProtocol::OpenAiCompatible | ApiProtocol::AzureOpenAi => {
                let mut messages =
                    vec![serde_json::json!({ "role": "system", "content": request.instructions })];
                messages.extend(request.messages.iter().map(|message| {
                    serde_json::json!({ "role": message.role, "content": message.content })
                }));
                let mut body = serde_json::json!({ "model": request.model, "messages": messages, "stream": true });
                if let Some(max_tokens) = request.max_output_tokens {
                    body["max_tokens"] = serde_json::json!(max_tokens);
                }
                body
            }
            ApiProtocol::AnthropicMessages => {
                let messages = request
                    .messages
                    .iter()
                    .filter(|message| message.role == "user" || message.role == "assistant")
                    .map(|message| {
                        serde_json::json!({ "role": message.role, "content": message.content })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "model": request.model,
                    "max_tokens": request.max_output_tokens.unwrap_or(4096),
                    "system": request.instructions,
                    "messages": messages,
                    "stream": true
                })
            }
            ApiProtocol::GoogleGemini | ApiProtocol::GoogleVertexExpress => {
                let contents = request
                    .messages
                    .iter()
                    .filter(|message| message.role == "user" || message.role == "assistant")
                    .map(|message| {
                        let role = if message.role == "assistant" {
                            "model"
                        } else {
                            "user"
                        };
                        serde_json::json!({
                            "role": role,
                            "parts": [{ "text": message.content }]
                        })
                    })
                    .collect::<Vec<_>>();
                let mut body = serde_json::json!({
                    "systemInstruction": { "parts": [{ "text": request.instructions }] },
                    "contents": contents
                });
                if let Some(max_tokens) = request.max_output_tokens {
                    body["generationConfig"] = serde_json::json!({ "maxOutputTokens": max_tokens });
                }
                body
            }
        }
    }

    fn text_delta<'a>(&self, value: &'a Value) -> Option<&'a str> {
        let pointer = match self.protocol {
            ApiProtocol::OpenAiCompatible | ApiProtocol::AzureOpenAi => "/choices/0/delta/content",
            ApiProtocol::AnthropicMessages => "/delta/text",
            ApiProtocol::GoogleGemini | ApiProtocol::GoogleVertexExpress => {
                "/candidates/0/content/parts/0/text"
            }
        };
        value.pointer(pointer).and_then(Value::as_str)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn stream(&self, request: LlmRequest, deltas: UnboundedSender<String>) -> Result<String> {
        let endpoint = self.stream_endpoint(&request.model)?;
        let body = self.request_body(&request);
        let mut builder = self
            .client
            .post(endpoint)
            .header("Content-Type", "application/json");
        if let Some(api_key) = &self.api_key {
            builder = match self.protocol {
                ApiProtocol::OpenAiCompatible => builder.bearer_auth(api_key),
                ApiProtocol::AnthropicMessages => builder
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01"),
                ApiProtocol::GoogleGemini | ApiProtocol::GoogleVertexExpress => {
                    builder.header("x-goog-api-key", api_key)
                }
                ApiProtocol::AzureOpenAi => builder.header("api-key", api_key),
            };
        }
        let response = builder
            .json(&body)
            .send()
            .await
            .context("Nu mă pot conecta la endpoint-ul AI")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!(
                "Providerul AI a răspuns cu {}: {}",
                status,
                detail.chars().take(600).collect::<String>()
            );
        }

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut complete = String::new();
        while let Some(chunk) = stream.next().await {
            buffer.extend_from_slice(&chunk.context("Fluxul AI s-a întrerupt")?);
            if buffer.len() > 1024 * 1024 {
                bail!("Evenimentul SSE OpenAI depășește limita de 1 MB");
            }
            for event in take_sse_events(&mut buffer) {
                let Some(data) = event_data(&event) else {
                    continue;
                };
                if data == "[DONE]" {
                    continue;
                }
                let value: Value =
                    serde_json::from_str(&data).context("Eveniment SSE AI invalid")?;
                if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
                    bail!("{message}");
                }
                if let Some(delta) = self.text_delta(&value) {
                    complete.push_str(delta);
                    let _ = deltas.send(delta.to_string());
                }
            }
        }
        if !buffer.is_empty() {
            if let Some(data) = event_data(&buffer) {
                if data != "[DONE]" {
                    let value: Value =
                        serde_json::from_str(&data).context("Ultimul eveniment SSE AI invalid")?;
                    if let Some(delta) = self.text_delta(&value) {
                        complete.push_str(delta);
                        let _ = deltas.send(delta.to_string());
                    }
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

    #[test]
    fn configures_openai_compatible_endpoints_safely() {
        assert_eq!(
            chat_completions_endpoint("https://api.openai.com").unwrap(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_endpoint("https://openrouter.ai/api/v1").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_endpoint("http://localhost:11434/v1").unwrap(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert!(chat_completions_endpoint("http://example.com/v1").is_err());
        assert!(chat_completions_endpoint("https://user:secret@example.com/v1").is_err());
    }
}
