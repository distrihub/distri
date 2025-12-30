
import { Chat, ChatProps } from "@distri/react"
import { Skeleton } from "@/components/ui/skeleton"
import { Button } from "@/components/ui/button"
import { useState, useEffect, useRef, useMemo } from "react"
import { Play, Pause, SkipBack, SkipForward, RotateCcw, MessageSquare } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useChatMessages, useChatStateStore } from "@distri/react";

export interface ReplayChatOptions extends ChatProps {
  threadId: string
  playMode?: "replay" | "chat"
}

// Removed useChatState since useChat already manages the state

export const ReplayChat = (options: ReplayChatOptions) => {
  const { threadId, playMode = "chat" } = options
  const [idx, setIdx] = useState(0)
  const [isPlaying, setIsPlaying] = useState(false)
  const [playbackSpeed, setPlaybackSpeed] = useState(2)
  const intervalRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Access individual state values to prevent object recreation issues
  const messageCount = useChatStateStore((state) => state.messages.length);
  const isStreaming = useChatStateStore((state) => state.isStreaming);
  const streamingIndicator = useChatStateStore((state) => state.streamingIndicator);
  const toolCallsCount = useChatStateStore((state) => state.toolCalls.size);
  const stepsCount = useChatStateStore((state) => state.steps.size);

  // Get toolCalls data for debugging - use a stable selector that only changes when actual data changes
  const toolCallsData = useChatStateStore((state) => {
    if (playMode !== "replay") return "not-in-replay-mode";
    // Create a stable string representation instead of recreating objects
    const toolCallEntries = Array.from(state.toolCalls.entries()).slice(0, 5);
    return toolCallEntries.map(([id, tc]) => `${id}:${tc.tool_name}:${tc.status}`).join('|');
  });

  // Get detailed toolCalls only when we need to display them (memoized)
  const detailedToolCalls = useMemo(() => {
    if (playMode !== "replay" || toolCallsData === "not-in-replay-mode") return [];

    // Only fetch detailed data when we have tool calls and we're actually displaying
    const state = useChatStateStore.getState();
    return Array.from(state.toolCalls.entries()).slice(0, 5).map(([id, toolCall]) => ({
      id,
      tool_name: toolCall.tool_name,
      status: toolCall.status,
      startTime: toolCall.startTime,
      endTime: toolCall.endTime,
      isExternal: toolCall.isExternal,
      isLiveStream: toolCall.isLiveStream,
      input: toolCall.input,
      error: toolCall.error
    }));
  }, [playMode, toolCallsData]);

  // Only create debug snapshot when needed and memoize it
  const globalChatState = useMemo(() => {
    if (playMode !== "replay") {
      return { message: 'Not available in chat mode' };
    }
    return {
      messageCount,
      isStreaming,
      streamingIndicator,
      toolCallsCount,
      stepsCount,
      toolCallsSummary: toolCallsData,
      toolCalls: detailedToolCalls,
      timestamp: Date.now()
    };
  }, [playMode, messageCount, isStreaming, streamingIndicator, toolCallsCount, stepsCount, toolCallsData, detailedToolCalls])

  // const messages = threadMessages.map(decodeA2AStreamEvent).filter(Boolean) as (DistriChatMessage)[];
  const { messages } = useChatMessages({ threadId, agent: options.agent });
  
  const filteredMessages = playMode === "replay" ? messages.slice(0, idx) : messages;
  useEffect(() => {
    if (playMode === "replay") {
      setIsPlaying(true)
    } else {
      setIsPlaying(false)
      setIdx(0)
    }
  }, [playMode])

  // Auto-play functionality
  useEffect(() => {
    if (isPlaying && idx < messages.length && playMode === "replay") {
      intervalRef.current = setTimeout(() => {
        setIdx(prev => Math.min(prev + 1, messages.length))
      }, 1000 / playbackSpeed)
    } else if (idx >= messages.length) {
      setIsPlaying(false)
    }

    return () => {
      if (intervalRef.current) {
        clearTimeout(intervalRef.current)
      }
    }
  }, [isPlaying, idx, messages.length, playbackSpeed, playMode])

  const handlePlayPause = () => {
    if (idx >= messages.length) {
      setIdx(0)
    }
    setIsPlaying(!isPlaying)
  }

  const handleReset = () => {
    setIdx(0)
    setIsPlaying(false)
  }

  const handleSkipBack = () => {
    setIdx(prev => Math.max(0, prev - 1))
  }

  const handleSkipForward = () => {
    setIdx(prev => Math.min(messages.length, prev + 1))
  }

  const handleProgressClick = (e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    const clickX = e.clientX - rect.left
    const percentage = clickX / rect.width
    const newIdx = Math.floor(percentage * messages.length)
    setIdx(Math.max(0, Math.min(newIdx, messages.length)))
  }

  const formatTime = (seconds: number) => {
    const mins = Math.floor(seconds / 60)
    const secs = seconds % 60
    return `${mins}:${secs.toString().padStart(2, '0')}`
  }

  const currentTime = idx
  const totalTime = messages.length
  const progress = totalTime > 0 ? (currentTime / totalTime) * 100 : 0

  const getCurrentMessage = () => {
    if (idx > 0 && idx <= messages.length) {
      return messages[idx - 1]
    }
    return null
  }

  const getEventExplanation = (message: any | null) => {
    return message.type;

  }


  return (
    <div className="flex h-full space-x-4">
      {/* Left Panel - Raw Message and Status - Only show in replay mode */}
      {playMode === "replay" && (
        <div className="w-80 flex-shrink-0">
          <Card className="h-full flex flex-col">
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm flex items-center gap-2">
                  <MessageSquare className="h-4 w-4" />
                  Events
                </CardTitle>

              </div>
            </CardHeader>

            <CardContent className="flex-1 overflow-hidden">
              {/* Raw Message Section */}
              {getCurrentMessage() && (
                <div className="h-full flex flex-col space-y-4">

                  <div className="flex-1 flex flex-col space-y-4 min-h-0">
                    <div className="flex-1 flex flex-col min-h-0">
                      <div className="text-xs font-medium mb-2">Latest Event: <span className="text-muted-foreground text-sm ml-2">{getEventExplanation(getCurrentMessage())}</span></div>
                      <pre className="text-xs bg-muted p-2 rounded flex-1 overflow-auto" style={{ whiteSpace: 'pre-wrap' }}>{JSON.stringify(getCurrentMessage(), null, 2)}</pre>
                    </div>
                    <div className="flex-1 flex flex-col min-h-0">
                      <div className="text-xs font-medium mb-2">Current State:</div>
                      <pre className="text-xs bg-muted p-2 rounded flex-1 overflow-auto" style={{ whiteSpace: 'pre-wrap' }}>
                        {playMode === "replay" ? JSON.stringify(globalChatState, null, 2) : 'Not available in chat mode'}
                      </pre>
                    </div>
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Main Chat Area */}

      <div className="flex-1 flex flex-col space-y-4">
        <Chat
          agent={options.agent}
          threadId={threadId}
          initialMessages={filteredMessages}
          initialInput={options.initialInput}
          theme="auto"
          voiceEnabled={true}
          useSpeechRecognition={true}
          ttsConfig={{
            model: 'openai', // or 'gemini'
            voice: 'alloy',  // OpenAI voices: alloy, echo, fable, onyx, nova, shimmer
            speed: 1.0
          }}
        />

        {/* Player Controls - Only show in replay mode */}
        {playMode === "replay" && (
          <div className="bg-card border rounded-lg p-4">
            {/* Progress Bar */}
            <div className="space-y-2 mb-4">
              <div className="flex justify-between text-sm text-muted-foreground">
                <span>{formatTime(currentTime)}</span>
                <span>{formatTime(totalTime)}</span>
              </div>
              <div
                className="relative h-2 bg-secondary rounded-full cursor-pointer"
                onClick={handleProgressClick}
              >
                <div
                  className="absolute top-0 left-0 h-full bg-primary rounded-full transition-all duration-200"
                  style={{ width: `${progress}%` }}
                />
                <div
                  className="absolute top-1/2 w-4 h-4 bg-primary rounded-full border-2 border-background transform -translate-y-1/2 -translate-x-2 cursor-pointer hover:scale-110 transition-transform"
                  style={{ left: `${progress}%` }}
                />
              </div>
            </div>

            {/* Control Buttons - All on same row */}
            <div className="flex items-center justify-between">
              <div className="flex items-center space-x-2">
                <Button
                  variant="outline"
                  size="icon"
                  onClick={handleReset}
                  disabled={idx === 0}
                  title="Reset to beginning"
                >
                  <RotateCcw className="h-4 w-4" />
                </Button>

                <Button
                  variant="outline"
                  size="icon"
                  onClick={handleSkipBack}
                  disabled={idx === 0}
                  title="Previous message"
                >
                  <SkipBack className="h-4 w-4" />
                </Button>

                <Button
                  variant="default"
                  size="icon"
                  onClick={handlePlayPause}
                  title={isPlaying ? "Pause" : "Play"}
                >
                  {isPlaying ? (
                    <Pause className="h-4 w-4" />
                  ) : (
                    <Play className="h-4 w-4" />
                  )}
                </Button>

                <Button
                  variant="outline"
                  size="icon"
                  onClick={handleSkipForward}
                  disabled={idx >= messages.length}
                  title="Next message"
                >
                  <SkipForward className="h-4 w-4" />
                </Button>
              </div>

              <div className="text-xs text-muted-foreground space-y-1">
                {idx === 0 ? (
                  <div>Ready to play</div>
                ) : idx >= messages.length ? (
                  <div>Playback complete</div>
                ) : (
                  <div>Message {idx} of {messages.length}</div>
                )}
                <div>Duration: {formatTime(currentTime)} / {formatTime(totalTime)}</div>
              </div>
              {/* Playback Speed - Only 2x and 3x */}
              <div className="flex items-center space-x-2">

                <div className="flex space-x-1">
                  {[2, 3].map((speed) => (
                    <Button
                      key={speed}
                      variant={playbackSpeed === speed ? "default" : "outline"}
                      size="sm"
                      onClick={() => setPlaybackSpeed(speed)}
                      className="text-xs"
                    >
                      {speed}x
                    </Button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
export function SkeletonCard() {
  return (
    <div className="flex flex-col space-y-3">
      <Skeleton className="h-[125px] w-[250px] rounded-xl" />
      <div className="space-y-2">
        <Skeleton className="h-4 w-[250px]" />
        <Skeleton className="h-4 w-[200px]" />
      </div>
    </div>
  )
}
