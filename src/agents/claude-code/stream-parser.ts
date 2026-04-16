import { type LLMPhase } from '../../types.js'
import { formatToolCall, type FormattedOutput } from '../../format.js'

interface ContentBlock {
  type: string
  text?: string
  name?: string
  input?: Record<string, unknown>
}

interface AssistantMessage {
  content: ContentBlock[]
}

interface StreamEvent {
  type: string
  subtype?: string
  message?: AssistantMessage
  result?: string
}

export function formatClaudeStreamLine(line: string, phase?: LLMPhase): FormattedOutput | null {
  if (!line.trim()) return null

  let event: StreamEvent
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  // Assistant messages: extract tool_use blocks only (text comes from result event)
  if (event.type === 'assistant' && event.message?.content) {
    for (const block of event.message.content) {
      if (block.type === 'tool_use' && block.name && block.input) {
        const input = block.input
        switch (block.name) {
          case 'Read':
            return formatToolCall({ name: block.name, path: input.file_path as string }, phase)
          case 'Write':
            return formatToolCall({ name: block.name, path: input.file_path as string }, phase)
          case 'Edit':
            return formatToolCall({ name: block.name, path: input.file_path as string }, phase)
          case 'Grep':
            return formatToolCall({ name: block.name, detail: `"${input.pattern}" in ${input.path ?? '.'}` }, phase)
          case 'Glob':
            return formatToolCall({ name: block.name, detail: input.pattern as string }, phase)
          case 'Bash':
            return formatToolCall({ name: block.name, detail: (input.command as string).slice(0, 120) }, phase)
          default:
            return formatToolCall({ name: block.name }, phase)
        }
      }
    }
    return null
  }

  // Final result: single source of truth for text output
  if (event.type === 'result' && event.result) {
    return { text: event.result, persist: true }
  }

  return null
}
