//! Voice Command Layer hook.
//!
//! Before a finished transcription is pasted into the focused app, ask a
//! local server whether the text is a spoken command it wants to handle
//! itself (in which case the paste is suppressed) or plain dictation to
//! paste as-is or rewritten.
//!
//! This is intentionally decoupled from the rest of Handy: the server is
//! optional, external, and addressed over loopback HTTP. If it isn't
//! running, times out, or answers with anything unexpected, [`intercept`]
//! fails open and the caller pastes the original transcription — stock
//! Handy behavior is never blocked by this layer.

use log::{debug, warn};
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const VOICE_COMMAND_ENDPOINT: &str = "http://localhost:8765/process";
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

static CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|e| {
            warn!("Failed to build voice-command HTTP client, using default: {e}");
            Client::new()
        })
});

#[derive(Debug, Serialize)]
struct VoiceCommandRequest<'a> {
    text: &'a str,
}

#[derive(Debug, Deserialize)]
struct VoiceCommandResponse {
    action: String,
    #[serde(default)]
    text: Option<String>,
}

/// What the caller should do with a finished transcription after asking the
/// voice-command server.
pub enum VoiceCommandDecision {
    /// Paste this text — either the original transcription unchanged (fail-open,
    /// or an explicit `"paste"` with no `text` field) or text the server rewrote.
    Paste(String),
    /// The transcription was a command the server already acted on; nothing
    /// should be pasted.
    Suppress,
}

/// Ask the local voice-command server what to do with `original` before it is
/// pasted. See the module docs for the fail-open contract.
pub async fn intercept(original: &str) -> VoiceCommandDecision {
    let response = match CLIENT
        .post(VOICE_COMMAND_ENDPOINT)
        .json(&VoiceCommandRequest { text: original })
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => {
            debug!("Voice-command server unreachable, pasting as-is: {e}");
            return VoiceCommandDecision::Paste(original.to_string());
        }
    };

    if !response.status().is_success() {
        warn!(
            "Voice-command server returned HTTP {}, pasting as-is",
            response.status()
        );
        return VoiceCommandDecision::Paste(original.to_string());
    }

    let parsed = match response.json::<VoiceCommandResponse>().await {
        Ok(parsed) => parsed,
        Err(e) => {
            warn!("Voice-command server response was not valid JSON, pasting as-is: {e}");
            return VoiceCommandDecision::Paste(original.to_string());
        }
    };

    match parsed.action.as_str() {
        "suppress" => VoiceCommandDecision::Suppress,
        "paste" => {
            VoiceCommandDecision::Paste(parsed.text.unwrap_or_else(|| original.to_string()))
        }
        other => {
            warn!("Voice-command server returned unknown action '{other}', pasting as-is");
            VoiceCommandDecision::Paste(original.to_string())
        }
    }
}
