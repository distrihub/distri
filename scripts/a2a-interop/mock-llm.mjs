// A tiny OpenAI-compatible model for the A2A interop run (scripts/a2a-interop/run.sh).
// Every turn answers through distri's `final` tool ("Hello from distri (mock)").
// A user message containing "[slow]" waits SLOW_MS (default 6000) first, so a
// task stays `working` long enough to subscribe to or cancel it.
import http from 'node:http'

const PORT = Number(process.env.MOCK_LLM_PORT || 7811)
const SLOW_MS = Number(process.env.SLOW_MS || 6000)
const text = (c) => (typeof c === 'string' ? c : Array.isArray(c) ? c.map((p) => p.text || '').join(' ') : '')

function reply(body) {
  const messages = body.messages || []
  const tools = (body.tools || []).map((t) => t.function?.name).filter(Boolean)
  const lastUser = [...messages].reverse().find((m) => m.role === 'user')
  const slow = lastUser && text(lastUser.content).includes('[slow]')
  const answer = 'Hello from distri (mock)'
  const choice = tools.includes('final') ? { tool: 'final', args: { input: answer } } : { text: answer }
  return { choice, slow }
}

function completion(model, choice, stream) {
  const id = `chatcmpl-${Date.now()}`
  const created = Math.floor(Date.now() / 1000)
  const usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 }
  const call = choice.tool && {
    id: `call_${Math.random().toString(36).slice(2, 10)}`,
    type: 'function',
    function: { name: choice.tool, arguments: JSON.stringify(choice.args) },
  }
  const finish = call ? 'tool_calls' : 'stop'
  if (!stream) {
    const message = call ? { role: 'assistant', content: null, tool_calls: [call] } : { role: 'assistant', content: choice.text }
    return JSON.stringify({ id, object: 'chat.completion', created, model, choices: [{ index: 0, message, finish_reason: finish }], usage })
  }
  const chunk = (delta, fin) =>
    `data: ${JSON.stringify({ id, object: 'chat.completion.chunk', created, model, choices: [{ index: 0, delta, finish_reason: fin }], ...(fin ? { usage } : {}) })}\n\n`
  const first = call
    ? chunk({ role: 'assistant', content: null, tool_calls: [{ index: 0, ...call }] }, null)
    : chunk({ role: 'assistant', content: choice.text }, null)
  return first + chunk({}, finish) + 'data: [DONE]\n\n'
}

http
  .createServer((req, res) => {
    const url = new URL(req.url, 'http://x')
    if (req.method === 'GET' && url.pathname.endsWith('/models')) {
      res.writeHead(200, { 'content-type': 'application/json' })
      return res.end(JSON.stringify({ object: 'list', data: [{ id: 'mock-1', object: 'model' }] }))
    }
    let raw = ''
    req.on('data', (c) => (raw += c))
    req.on('end', () => {
      let body = {}
      try {
        body = JSON.parse(raw || '{}')
      } catch {}
      const { choice, slow } = reply(body)
      const stream = body.stream === true
      setTimeout(() => {
        res.writeHead(200, { 'content-type': stream ? 'text/event-stream' : 'application/json' })
        res.end(completion(body.model || 'mock-1', choice, stream))
      }, slow ? SLOW_MS : 0)
    })
  })
  .listen(PORT, '127.0.0.1', () => console.log(`mock llm on ${PORT}`))
