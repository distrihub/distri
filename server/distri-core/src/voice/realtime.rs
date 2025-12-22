use super::types::{AudioFormat, StreamingConfig, VoiceStreamEvent};
use anyhow::{Context, Result};
use async_openai::types::realtime::{
    ConversationItemCreateEvent, Item, ResponseCreateEvent, ServerEvent,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde_json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};

pub struct RealtimeVoiceClient {
    config: StreamingConfig,
    api_key: String,
    event_sender: Option<UnboundedSender<VoiceStreamEvent>>,
}

impl RealtimeVoiceClient {
    pub fn new(api_key: String, config: StreamingConfig) -> Self {
        Self {
            api_key,
            config,
            event_sender: None,
        }
    }

    pub async fn connect(&mut self) -> Result<UnboundedReceiver<VoiceStreamEvent>> {
        let url = format!(
            "wss://api.openai.com/v1/realtime?model={}",
            self.config.model
        );

        // Create request with authorization headers
        let mut request = url.into_client_request()?;
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {}", self.api_key).parse()?);
        request
            .headers_mut()
            .insert("OpenAI-Beta", "realtime=v1".parse()?);

        // Connect to WebSocket
        let (ws_stream, _) = connect_async(request)
            .await
            .context("Failed to connect to OpenAI realtime API")?;

        let (write, read) = ws_stream.split();
        let write = Arc::new(Mutex::new(write));

        // Create channels for events
        let (event_tx, event_rx) = futures_channel::mpsc::unbounded::<VoiceStreamEvent>();
        let (_input_tx, input_rx) = futures_channel::mpsc::unbounded::<Message>();

        self.event_sender = Some(event_tx.clone());

        // Handle incoming WebSocket messages
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            let mut read = read;
            while let Some(message) = read.next().await {
                match message {
                    Ok(Message::Text(_)) => {
                        let data = message.unwrap().into_data();
                        if let Ok(server_event) = serde_json::from_slice::<ServerEvent>(&data) {
                            let voice_event = Self::convert_server_event(server_event);
                            if let Some(event) = voice_event {
                                let _ = event_tx_clone.unbounded_send(event);
                            }
                        }
                    }
                    Ok(Message::Binary(data)) => {
                        // Handle audio data
                        let audio_event = VoiceStreamEvent::AudioChunk {
                            data: data.to_vec(),
                            sample_rate: 24000, // OpenAI realtime default
                            channels: 1,
                            format: AudioFormat::Mp3,
                        };
                        let _ = event_tx_clone.unbounded_send(audio_event);
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket connection closed");
                        break;
                    }
                    Err(e) => {
                        let error_event = VoiceStreamEvent::Error {
                            message: format!("WebSocket error: {}", e),
                        };
                        let _ = event_tx_clone.unbounded_send(error_event);
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Handle outgoing WebSocket messages
        let write_clone = Arc::clone(&write);
        tokio::spawn(async move {
            let mut input_rx = input_rx;
            while let Some(message) = input_rx.next().await {
                let mut write_guard = write_clone.lock().await;
                if let Err(e) = write_guard.send(message).await {
                    tracing::error!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        });

        Ok(event_rx)
    }

    pub async fn send_text(&self, text: &str) -> Result<()> {
        if let Some(sender) = &self.event_sender {
            // Create OpenAI realtime conversation item
            let item = Item::try_from(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": text
                    }
                ]
            }))?;

            // Create and send conversation item create event
            let event: ConversationItemCreateEvent = item.into();
            let event_json = serde_json::to_string(&event)?;
            let _message = Message::Text(event_json);

            // Note: This would need to be connected to the WebSocket sender
            // For now, we'll emit a text event
            let text_event = VoiceStreamEvent::TextChunk {
                text: text.to_string(),
                is_final: true,
            };
            sender.unbounded_send(text_event)?;

            // Send response create event
            let response_event = ResponseCreateEvent::default();
            let response_json = serde_json::to_string(&response_event)?;
            let _response_message = Message::Text(response_json);
            // This would also need to be sent through the WebSocket
        }
        Ok(())
    }

    pub async fn send_audio(&self, audio_data: &[u8], format: AudioFormat) -> Result<()> {
        if let Some(sender) = &self.event_sender {
            let audio_event = VoiceStreamEvent::AudioChunk {
                data: audio_data.to_vec(),
                sample_rate: self.config.sample_rate,
                channels: self.config.channels,
                format,
            };
            sender.unbounded_send(audio_event)?;
        }
        Ok(())
    }

    fn convert_server_event(server_event: ServerEvent) -> Option<VoiceStreamEvent> {
        match server_event {
            ServerEvent::ResponseOutputItemDone(event) => {
                if let Some(content) = event.item.content {
                    for content_item in content {
                        if let Some(transcript) = content_item.transcript {
                            return Some(VoiceStreamEvent::TextChunk {
                                text: transcript.trim().to_string(),
                                is_final: true,
                            });
                        }
                    }
                }
                None
            }
            ServerEvent::ResponseAudioTranscriptDelta(event) => Some(VoiceStreamEvent::TextChunk {
                text: event.delta.trim().to_string(),
                is_final: false,
            }),
            ServerEvent::Error(e) => Some(VoiceStreamEvent::Error {
                message: format!("OpenAI realtime error: {:?}", e),
            }),
            _ => None,
        }
    }
}

pub struct StreamingVoiceSession {
    client: RealtimeVoiceClient,
    event_receiver: Option<UnboundedReceiver<VoiceStreamEvent>>,
    is_connected: bool,
}

impl StreamingVoiceSession {
    pub fn new(api_key: String, config: Option<StreamingConfig>) -> Self {
        let config = config.unwrap_or_default();
        let client = RealtimeVoiceClient::new(api_key, config);

        Self {
            client,
            event_receiver: None,
            is_connected: false,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let event_receiver = self.client.connect().await?;
        self.event_receiver = Some(event_receiver);
        self.is_connected = true;
        Ok(())
    }

    pub async fn send_message(&mut self, text: &str) -> Result<()> {
        if !self.is_connected {
            return Err(anyhow::anyhow!("Voice session not connected"));
        }
        self.client.send_text(text).await
    }

    pub async fn send_audio_chunk(&mut self, audio_data: &[u8], format: AudioFormat) -> Result<()> {
        if !self.is_connected {
            return Err(anyhow::anyhow!("Voice session not connected"));
        }
        self.client.send_audio(audio_data, format).await
    }

    pub async fn next_event(&mut self) -> Option<VoiceStreamEvent> {
        if let Some(ref mut receiver) = self.event_receiver {
            receiver.next().await
        } else {
            None
        }
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
}
