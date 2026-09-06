# Voice sessions — realtime audio in the distri engine

**Date:** 2026-09-06 · **Status:** parked. The first consumer chose a client-side pipeline (browser VAD, streaming STT over the provider's WebSocket, sentence-level TTS) that needs nothing from this spec; the streaming voice experience gets its own spec and session, and this document is input to it. Read the ordering there as: cascade first, provider speech-to-speech optional. ·
**Companion docs:** the cloud surface (`distri-cloud/docs/specs/2026-09-06-voice-sessions-cloud.md`)
and the first consumer (`platform/docs/specs/2026-09-06-live-session-and-voice-chat.md`, Zippy's
live AI teacher). This document is the engine half: types, the agent loop, and the transports.

## What exists today

distri already has a complete turn-based voice stack, and nothing realtime.

| Piece | Where | State |
|---|---|---|
| TTS | `server/llm-gateway/src/tts.rs::call_tts` — OpenAI-compat, Azure OpenAI, Azure Speech (SSML), ElevenLabs, DashScope (`qwen3-tts-flash`) | Shipped. Returns a whole `Vec<u8>`; no streaming |
| STT | `distri-cloud/cloud/src/handlers/tts.rs::transcribe_speech` — Whisper, Azure Whisper, `qwen3-asr-flash` | Shipped. Whole-clip only |
| Client loop | distrijs `Chat.tsx` handsfree mode: record → transcribe → send → speak the reply | Shipped. Speaks only after `isStreaming` goes false, so latency is the whole response time |
| Provider catalog | `cloud/config/providers/*.yaml` with `completion` / `tts` / `stt` sections | Shipped |
| Mid-run input | `AgentTaskCoordinator::{cancel, deliver_message}`, drained at the top of each `AgentLoop::run` iteration (`agent_loop.rs:224`) | Shipped, but checked between iterations, not between tokens |
| Audio in the type system | none — `Part` has no audio variant, `AgentEventType` has no audio event, no LLM adapter sends `input_audio` | Missing |
| Duplex transport | none — clients are SSE + HTTP POST; no WebSocket, no WebRTC | Missing |

The 2026-05-19 provider-catalog spec made adding an OpenAI-compatible provider a YAML change. This
spec keeps that property for realtime models: a `realtime:` section in the same file.

## The decision

A **voice session** is a first-class engine object, distinct from a task. A task is one run of
the agent loop over a message; a voice session is a long-lived duplex connection that produces
many turns on one thread, where each turn may or may not go through the agent loop.

Two engines sit behind one session interface, chosen per agent (frontmatter) with a per-session
override:

- **`realtime`** — a provider speech-to-speech model (OpenAI Realtime, Gemini Live, Azure Voice
  Live) is the voice. distri does not run the token loop; it owns the session: system prompt
  from the `StandardDefinition`, tool definitions, tool execution, transcript persistence into the
  thread, usage metering, guardrails. Lowest latency, highest cost per minute, no control over the
  speech model's turn-taking beyond provider knobs.
- **`cascade`** — streaming STT → the existing `execute_stream` agent loop → streaming TTS.
  distri owns everything, including turn detection and barge-in. Cheaper, provider-swappable,
  fine-tunable STT (matters for children's speech), more to build.

Both engines expose the same events to clients and write the same `Message`s to the thread, so a
host app cannot tell which one served a session except by latency and bill. That is the point:
a consumer builds the UI once.

## 1. Types (`distri-types`)

### `Part::Audio`

```rust
// distri-types/src/core.rs — beside Image / File
Audio(AudioPart),

pub struct AudioPart {
    pub data: FileType,               // Bytes { mime_type: "audio/pcm;rate=24000" | "audio/webm" | "audio/wav" } or Url
    pub transcript: Option<String>,   // filled by STT or by the provider's input transcription
    pub duration_ms: Option<u32>,
}
```

`type_name()` returns `"audio"`. Formatters that flatten parts to text render the transcript when
present and `[audio 12s]` otherwise, so channels and compaction keep working with no changes.
The LLM adapters (`llm.rs`, `claude_llm.rs`, `openai_responses_llm.rs`) send `transcript` as text;
none of them sends `input_audio` in this spec. Native audio understanding in a completion call is
a later, separate change.

### Audio events

`AgentEventType` (`distri-types/src/events.rs`) gains, all carrying `session_id`:

| Event | Payload | Emitted by |
|---|---|---|
| `VoiceSessionStarted` | `engine`, `model`, `transport` | both engines |
| `UserSpeechStarted` / `UserSpeechEnded` | `at_ms` | VAD (cascade) or provider (realtime) |
| `UserTranscript` | `text`, `is_final` | STT / provider input transcription |
| `AgentSpeechStarted` / `AgentSpeechEnded` | `message_id` | when the first / last audio frame of a reply is sent |
| `AgentTranscriptDelta` | `message_id`, `delta` | text of what the agent is saying, aligned to playback so captions match audio |
| `AudioDelta` | `message_id`, `seq`, `mime`, `data_b64` | **cascade only, on the WebSocket transport.** Never on SSE |
| `Interrupted` | `message_id`, `spoken_chars` | barge-in cut a reply; `spoken_chars` is how much the user heard, and the persisted assistant message is truncated to it |
| `VoiceSessionEnded` | `reason`, `audio_in_ms`, `audio_out_ms`, `turns` | both engines |

Text events (`TextMessageStart/Content/End`, `ToolCalls`, `ToolResults`) fire as today. A client
that only understands the text stream still gets a usable transcript.

### `[voice]` frontmatter

`StandardDefinition` gains a real field (unknown TOML keys are refused by
`validate_frontmatter_keys`, `agent.rs:2184`, so this cannot be smuggled in):

```toml
[voice]
engine = "realtime"                    # "realtime" | "cascade" | "off" (default off)
realtime_model = "openai/gpt-realtime-mini"   # provider/model, realtime engine only
stt_model = "alibaba_cloud/qwen3-asr-flash"   # cascade engine
tts_model = "azure_openai/gpt-4o-mini-tts"    # cascade engine; realtime engines carry their own voice
voice = "coral"
barge_in = true                        # user speech cancels the agent's reply
speak_tool_results = false             # cascade: whether tool-result text is voiced (default: only the agent's prose)

[voice.turn_detection]
type = "semantic"                      # "silence" | "semantic" | "manual" (push-to-talk)
eagerness = "low"                      # low | medium | high — how fast to decide a pause is the end of a turn
max_silence_ms = 2500
```

`runtime = [...]` is the precedent: a capability constraint that the request path enforces. A
session request against an agent with `engine = "off"` is a 400. Every key is overridable per
session by the host, so Zippy can set `eagerness` per lesson phase without a second agent.

## 2. The session runner (`distri-core`)

```
VoiceSession { id, thread_id, agent, engine, transport, started_at, meter }
  ├─ realtime: RealtimeBridge   — holds the provider socket (or the OpenAI sideband)
  └─ cascade:  CascadeLoop      — VAD → STT → execute_stream → TTS
```

Both live in `server/distri-core/src/voice/` and register with the `AgentRuntime` so `cancel`
and `deliver_message` reach them; `InProcessRuntime` and `RedisRuntime` need no new trait.

### 2.1 Realtime engine (`RealtimeBridge`)

One provider adapter trait, three implementations to start:

```rust
#[async_trait]
pub trait RealtimeProvider: Send + Sync {
    /// Mint what the *client* needs to connect directly to the provider.
    async fn client_credentials(&self, cfg: &SessionConfig) -> Result<ClientTransport>;
    /// Open distri's own control connection to the same session.
    async fn attach(&self, cfg: &SessionConfig, handle: ClientTransport) -> Result<Box<dyn RealtimeControl>>;
}

#[async_trait]
pub trait RealtimeControl: Send + Sync {
    async fn update_session(&self, instructions: &str, tools: &[ToolDefinition], turn: &TurnDetection) -> Result<()>;
    async fn inject_context(&self, text: &str) -> Result<()>;      // out-of-band, not spoken
    async fn say(&self, text: &str) -> Result<()>;                 // force a spoken line (scripted stems)
    async fn submit_tool_result(&self, call_id: &str, parts: Vec<Part>) -> Result<()>;
    async fn interrupt(&self) -> Result<()>;
    fn events(&self) -> mpsc::Receiver<ProviderEvent>;             // transcripts, function calls, usage, errors
    async fn close(&self) -> Result<()>;
}
```

| Provider | Client transport | distri control connection |
|---|---|---|
| OpenAI Realtime | WebRTC. Client gets an ephemeral secret (`POST /v1/realtime/client_secrets`, server-side) and posts its SDP to `POST /v1/realtime/calls` | **Sideband WebSocket** `wss://api.openai.com/v1/realtime?call_id=…` — distri sees every event and answers every function call; the browser never holds tool logic. Known: the sideband drops after long silence, so the bridge reconnects and re-sends `session.update` |
| Gemini Live | WebSocket with an ephemeral token (`newSessionExpireTime` 1 min, `expireTime` 30 min, `uses: 1`, `liveConnectConstraints` locking model and modalities) | No sideband exists. Function calls arrive on the client socket; the client relays them to distri over the session WebSocket (§3) and relays the result back. Or distri holds the provider socket and proxies audio — that is the cascade transport with a realtime model, and is how the desktop shell does it |
| Azure Voice Live | WebSocket, same shape as Gemini | Same as Gemini. Interesting because Zippy already holds the Azure Speech account |

Turn flow: provider emits `input_audio_transcription.completed` → distri appends a user `Message`
with `Part::Audio { transcript }`; provider emits `response.done` → distri appends the assistant
`Message` (text from the output transcript) and usage; `function_call` → distri runs the tool
exactly as `AgentExecutor` would (`execute_tool_with_executor_context`, MCP adapters, external
tools via the existing `/agents/{id}/complete-tool` round trip) and submits the result. The
thread therefore reads as an ordinary conversation, compaction sees ordinary messages, and
`distri traces` sees ordinary tool spans.

What the realtime engine does **not** do: run `PlanningStrategy`. The speech model plans. If a
host needs distri's reasoning in the loop, it registers a tool (`think`, `next_step`) whose
execution is a normal `execute()` on a sub-agent; the speech model calls it and speaks the
result. The cost is one extra LLM round trip on those turns, so hosts should reserve it for
decisions and keep chit-chat on the speech model. Zippy's director tools are the worked example.

### 2.2 Cascade engine (`CascadeLoop`)

```
mic frames (PCM16, 16 kHz) ──► VAD (Silero via ort) ──► streaming STT ──► end-of-turn
        ▲                                                                    │
        │ barge_in: cancel task, flush TTS queue, emit Interrupted            ▼
        │                                                     Message::user(transcript)
 speaker frames ◄── streaming TTS ◄── sentence chunker ◄── execute_stream(TextMessageContent deltas)
```

Three engine changes make this work and are useful on their own:

1. **Intra-stream cancellation.** `execute_step_stream` checks `context.cancellation_signal`
   on every delta, not only at the top of `AgentLoop::run`. A cancelled run persists the
   partial assistant text (what was actually spoken, from `Interrupted.spoken_chars`) rather than
   dropping it, so the model's next turn knows what the user heard.
2. **Sentence-chunked TTS.** A `SentenceChunker` over `TextMessageContent` deltas emits a chunk
   at `. ! ? :` or 120 characters, whichever first, and `call_tts` gains a streaming sibling
   (`call_tts_stream -> impl Stream<Item = Bytes>`; OpenAI, ElevenLabs and Azure Speech all
   support chunked transfer). First audio lands after the first sentence, roughly 600–900 ms
   after the user stops speaking on a fast completion model, instead of after the whole reply.
   distrijs handsfree mode should switch to this the same day.
3. **Streaming STT.** A `SttStream` adapter (Deepgram, Azure Speech continuous, OpenAI
   `gpt-4o-mini-transcribe` over WebSocket, or on-device `sherpa-onnx` in the desktop shell)
   yielding interim and final transcripts. The existing whole-clip `transcribe_speech` stays for
   the turn-based path.

Turn detection in the cascade: Silero VAD for speech/no-speech, plus a pluggable end-of-turn
model. Pipecat's Smart Turn v3 is an 8 MB open ONNX model (23 languages) that runs in `ort` in
about 10–60 ms on CPU; it is the default. LiveKit's detector is under a non-Apache licence and
stays out.

### 2.3 Barge-in semantics (both engines)

- A user speech start while the agent is speaking with `barge_in = true` → `interrupt()`,
  `Interrupted { spoken_chars }`, the assistant message is truncated to what was heard.
- `turn_detection.type = "manual"` disables automatic end-of-turn; the client sends
  `commit_turn` (push-to-talk, or a young learner's "I'm done" button). Recommended default for
  under-12s, where silence-based detection cuts off thinking pauses.
- `eagerness` maps to provider knobs (`semantic_vad.eagerness` on OpenAI, `max_silence_ms` on
  the cascade). Hosts change it mid-session through `update_session`, e.g. `low` while the
  student works a problem, `high` during quick recall.

## 3. Transports

| Transport | Used by | Notes |
|---|---|---|
| Direct-to-provider (WebRTC / WS) | realtime engine, browser | Audio never touches distri. Browser echo cancellation works because playback is a WebRTC track |
| **distri session WebSocket** `GET /v1/voice/sessions/{id}/ws` | control channel for every engine; audio channel for the cascade | Binary frames = PCM16 audio both ways; text frames = the JSON events above plus client commands (`commit_turn`, `interrupt`, `tool_result`, `update_session`). First transport to build: it is what OpenAI, Gemini and Azure use themselves, it works from a browser `AudioWorklet` and from `cpal` on desktop |
| LiveKit room | cascade on mobile networks and for recording | The `livekit` Rust crate (0.8.x) can join a room, publish and subscribe to audio, and use data channels, so distri joins as a participant. Adds jitter buffering, NAT traversal, and libwebrtc's echo canceller. Deferred until the WebSocket transport has shown where it hurts |

One trap to design around from day one: a browser's echo canceller only cancels audio it plays
through a media element or WebRTC track. PCM decoded from a WebSocket and scheduled on an
`AudioContext` is invisible to it, so the microphone hears the agent and barge-in fires on the
agent's own voice. The browser client for the cascade transport therefore plays through a local
`RTCPeerConnection` loopback (or LiveKit), never raw `AudioContext` buffers.

SSE stays as it is. `AudioDelta` is never sent over SSE, and the open issue on SSE worker-thread
exhaustion (distri-cloud issue #3) is a prerequisite for any product that multiplies long-lived
streams.

## 4. The desktop shell

The "distri as voice AI inside other apps" idea is the cascade engine plus the runtime running
in-process:

- **Tauri 2**, distri-core compiled into the app. No sidecar, no IPC tax, 5–10 MB bundle.
- **Microphone in Rust, not in the WebView.** `cpal` capture (or the `livekit` crate's audio
  device module, which brings libwebrtc's AEC/NS/AGC). This sidesteps the open WKWebView and
  WebKitGTK `getUserMedia` permission bugs, and `cpal` alone has no echo cancellation, so the
  APM is not optional.
- **Push-to-talk** via `tauri-plugin-global-shortcut`; `turn_detection = "manual"` by default.
- **On-device STT** through `sherpa-onnx` (official Rust crate, streaming Zipformer models) when
  the agent's `stt_model` is `local/…`; cloud completion and TTS otherwise. Fully offline needs
  a local completion model (`mistralrs`) and Kokoro TTS; possible, but first-token latency on a
  laptop is the limit, so it is a mode, not the default.
- **Host-app integration is MCP.** The app being voice-enabled exposes its actions as an MCP
  server (stdio locally); the agent's `[[tools.mcp]]` entry is the only glue. `cloud/src/mcp_app/`
  is the server-side precedent. OS-level fallbacks (accessibility tree, frontmost-window
  screenshot) are a later plugin.
- `distri-cli/src/chat.rs` is the seed: it already streams a thread and runs local tools. The
  shell is that loop with audio I/O and a tray window.

`distri-ui/src-tauri` is referenced in three docs and exists in none of them; this is the plan
that makes it real.

## 5. Metering and observability

- Every session records `audio_in_ms`, `audio_out_ms`, provider audio tokens where reported, and
  STT/TTS minutes for the cascade, as usage records on the thread, so a workspace budget rule
  can cap voice minutes the same way it caps tokens.
- `gen_ai.tts` spans exist; add `gen_ai.stt` and a `voice.session` root span with turn count,
  interruptions, and end-of-turn latency (`UserSpeechEnded` → `AgentSpeechStarted`). That last
  number is the one to watch weekly.
- Audio is not stored by default. Transcripts are. A session config flag `record = true` keeps
  the audio on the transport that has it (LiveKit egress, or the cascade's own frames); consumers
  serving children must not turn it on without consent handling on their side.

## 6. Build order

| Phase | Delivers | Depends on |
|---|---|---|
| 1 | `Part::Audio`, the audio events, `[voice]` on `StandardDefinition`, sentence-chunked streaming TTS in the gateway, distrijs handsfree speaking per sentence | nothing; ships value to every existing voice user |
| 2 | Realtime engine with the OpenAI adapter (WebRTC client + sideband), session WebSocket for control, tool execution through the bridge, thread persistence, metering | 1 |
| 3 | Gemini Live and Azure Voice Live adapters (client-relayed tool calls) | 2 |
| 4 | Cascade engine: VAD, streaming STT adapter, Smart Turn, intra-stream cancel, PCM over the session WebSocket, `RTCPeerConnection` loopback playback in distrijs | 1; SSE issue #3 for scale |
| 5 | Tauri shell with in-process runtime and Rust mic capture | 4 |
| 6 | LiveKit transport for the cascade | 4, driven by mobile-network data |

## Open questions

1. **Where does the realtime system prompt come from?** Proposal: the same `Formatter` that
   builds the first message of a task, minus the scratchpad. Speech models follow long
   instruction blocks worse than text models; a `[voice] instructions_override` may be needed.
2. **Should a realtime session be a task?** Proposal: no. It is a session that produces messages
   on a thread; `active_task_id` stays empty, and a host that wants agent-loop reasoning calls a
   tool. Revisit if hosts keep re-implementing "run the agent after each turn".
3. **Turn-detector licensing.** Smart Turn is BSD-2. If Krisp VIVA's interruption model is worth
   its fee it becomes a plugin behind the same trait.
