use super::providers::TtsService;
use super::types::{AudioFormat, StreamingConfig, TtsModel, TtsRequest, VoiceStreamEvent};
use anyhow::Result;
use async_stream::stream;
use futures_util::{Stream, StreamExt};
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

pub struct VoiceStreamProcessor {
    _tts_service: TtsService,
    _config: StreamingConfig,
    _audio_buffer: VecDeque<VoiceStreamEvent>,
}

impl VoiceStreamProcessor {
    pub fn new(tts_service: TtsService, config: StreamingConfig) -> Self {
        Self {
            _tts_service: tts_service,
            _config: config,
            _audio_buffer: VecDeque::new(),
        }
    }

    /// Process text chunks and convert them to audio streams
    pub fn process_text_stream<S>(&mut self, text_stream: S) -> impl Stream<Item = VoiceStreamEvent>
    where
        S: Stream<Item = String> + Send + 'static,
    {
        let tts_service = self._tts_service.clone();
        let config = self._config.clone();

        stream! {
            tokio::pin!(text_stream);

            let mut text_buffer = String::new();
            let mut sentence_boundary = false;

            while let Some(text_chunk) = text_stream.next().await {
                text_buffer.push_str(&text_chunk);

                // Check for sentence boundaries (., !, ?, or significant pauses)
                if text_chunk.ends_with('.') || text_chunk.ends_with('!') || text_chunk.ends_with('?') {
                    sentence_boundary = true;
                }

                // If we have a complete sentence or buffer is getting large
                if sentence_boundary || text_buffer.len() > 200 {
                    if !text_buffer.trim().is_empty() {
                        // Emit text event first
                        yield VoiceStreamEvent::TextChunk {
                            text: text_buffer.clone(),
                            is_final: sentence_boundary,
                        };

                        // Convert to audio
                        let tts_request = TtsRequest {
                            text: text_buffer.clone(),
                            model: TtsModel::OpenAI,
                            voice: config.voice.clone(),
                            speed: Some(1.0),
                        };

                        match tts_service.synthesize(&tts_request).await {
                            Ok(audio_bytes) => {
                                yield VoiceStreamEvent::AudioChunk {
                                    data: audio_bytes.to_vec(),
                                    sample_rate: config.sample_rate,
                                    channels: config.channels,
                                    format: AudioFormat::Mp3,
                                };
                            }
                            Err(e) => {
                                yield VoiceStreamEvent::Error {
                                    message: format!("TTS synthesis failed: {}", e),
                                };
                            }
                        }
                    }

                    text_buffer.clear();
                    sentence_boundary = false;
                }
            }

            // Process any remaining text
            if !text_buffer.trim().is_empty() {
                yield VoiceStreamEvent::TextChunk {
                    text: text_buffer.clone(),
                    is_final: true,
                };

                let tts_request = TtsRequest {
                    text: text_buffer,
                    model: TtsModel::OpenAI,
                    voice: config.voice.clone(),
                    speed: Some(1.0),
                };

                match tts_service.synthesize(&tts_request).await {
                    Ok(audio_bytes) => {
                        yield VoiceStreamEvent::AudioChunk {
                            data: audio_bytes.to_vec(),
                            sample_rate: config.sample_rate,
                            channels: config.channels,
                            format: AudioFormat::Mp3,
                        };
                    }
                    Err(e) => {
                        yield VoiceStreamEvent::Error {
                            message: format!("TTS synthesis failed: {}", e),
                        };
                    }
                }
            }
        }
    }

    /// Process audio chunks and convert them to text
    pub fn process_audio_stream<S>(
        &mut self,
        audio_stream: S,
    ) -> impl Stream<Item = VoiceStreamEvent>
    where
        S: Stream<Item = Vec<u8>> + Send + 'static,
    {
        let tts_service = self._tts_service.clone();

        stream! {
            tokio::pin!(audio_stream);

            let mut audio_buffer = Vec::new();
            let buffer_threshold = 1024 * 16; // 16KB buffer

            while let Some(audio_chunk) = audio_stream.next().await {
                audio_buffer.extend_from_slice(&audio_chunk);

                // Process when buffer is large enough or stream ends
                if audio_buffer.len() >= buffer_threshold {
                    match tts_service.transcribe(&audio_buffer).await {
                        Ok(text) => {
                            if !text.trim().is_empty() {
                                yield VoiceStreamEvent::TextChunk {
                                    text: text.trim().to_string(),
                                    is_final: false, // Streaming transcription
                                };
                            }
                        }
                        Err(e) => {
                            yield VoiceStreamEvent::Error {
                                message: format!("Speech recognition failed: {}", e),
                            };
                        }
                    }
                    audio_buffer.clear();
                }
            }

            // Process any remaining audio
            if !audio_buffer.is_empty() {
                match tts_service.transcribe(&audio_buffer).await {
                    Ok(text) => {
                        if !text.trim().is_empty() {
                            yield VoiceStreamEvent::TextChunk {
                                text: text.trim().to_string(),
                                is_final: true,
                            };
                        }
                    }
                    Err(e) => {
                        yield VoiceStreamEvent::Error {
                            message: format!("Speech recognition failed: {}", e),
                        };
                    }
                }
            }
        }
    }
}

/// Bidirectional voice streaming handler
pub struct BidirectionalVoiceStream {
    _tts_service: TtsService,
    _config: StreamingConfig,
    input_sender: mpsc::UnboundedSender<StreamInput>,
    output_receiver: Option<mpsc::UnboundedReceiver<VoiceStreamEvent>>,
}

#[derive(Debug)]
pub enum StreamInput {
    Text(String),
    Audio(Vec<u8>),
    EndOfInput,
}

impl BidirectionalVoiceStream {
    pub fn new(tts_service: TtsService, config: StreamingConfig) -> Self {
        let (input_sender, mut input_receiver) = mpsc::unbounded_channel::<StreamInput>();
        let (output_sender, output_receiver) = mpsc::unbounded_channel::<VoiceStreamEvent>();

        let processor_tts = tts_service.clone();
        let processor_config = config.clone();

        // Spawn background processor
        tokio::spawn(async move {
            let mut processor = VoiceStreamProcessor::new(processor_tts, processor_config);

            while let Some(input) = input_receiver.recv().await {
                match input {
                    StreamInput::Text(text) => {
                        // Process single text input
                        let text_stream = futures_util::stream::once(async { text });
                        let voice_stream = processor.process_text_stream(text_stream);
                        tokio::pin!(voice_stream);

                        while let Some(event) = voice_stream.next().await {
                            if output_sender.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    StreamInput::Audio(audio_data) => {
                        // Process single audio chunk
                        let audio_stream = futures_util::stream::once(async { audio_data });
                        let voice_stream = processor.process_audio_stream(audio_stream);
                        tokio::pin!(voice_stream);

                        while let Some(event) = voice_stream.next().await {
                            if output_sender.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    StreamInput::EndOfInput => {
                        break;
                    }
                }
            }
        });

        Self {
            _tts_service: tts_service,
            _config: config,
            input_sender,
            output_receiver: Some(output_receiver),
        }
    }

    pub async fn send_text(&self, text: &str) -> Result<()> {
        self.input_sender
            .send(StreamInput::Text(text.to_string()))?;
        Ok(())
    }

    pub async fn send_audio(&self, audio_data: Vec<u8>) -> Result<()> {
        self.input_sender.send(StreamInput::Audio(audio_data))?;
        Ok(())
    }

    pub fn take_output_stream(&mut self) -> Option<mpsc::UnboundedReceiver<VoiceStreamEvent>> {
        self.output_receiver.take()
    }

    pub async fn close(&self) {
        let _ = self.input_sender.send(StreamInput::EndOfInput);
    }
}

// Stream wrapper for easier integration
pub struct VoiceEventStream {
    receiver: mpsc::UnboundedReceiver<VoiceStreamEvent>,
}

impl VoiceEventStream {
    pub fn new(receiver: mpsc::UnboundedReceiver<VoiceStreamEvent>) -> Self {
        Self { receiver }
    }
}

impl Stream for VoiceEventStream {
    type Item = VoiceStreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
