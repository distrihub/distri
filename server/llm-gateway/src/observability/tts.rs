//! TTS (text-to-speech) span creation and recording.

/// Create a tracing span for a TTS (text-to-speech) call.
pub fn create_tts_span(
    model: &str,
    provider: &str,
    voice: &str,
    audio_format: &str,
) -> tracing::Span {
    tracing::info_span!(
        "gen_ai.tts",
        "gen_ai.provider.name" = provider,
        "gen_ai.request.model" = model,
        "tts.voice" = voice,
        "tts.audio_format" = audio_format,
        "tts.duration_ms" = tracing::field::Empty,
    )
}

/// Record TTS response duration.
pub fn record_tts_response(span: &tracing::Span, duration_ms: u64) {
    span.record("tts.duration_ms", duration_ms as i64);
}
