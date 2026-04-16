import { type LLMPhase } from '../../types.js'
import { formatToolCall, type FormattedOutput } from '../../format.js'

export function formatClaudeStreamLine(line: string, phase?: LLMPhase): FormattedOutput | null {
  if (!line.trim()) return null

  let event: Record<string, unknown>
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  if (event.type === 'assistant' && event.subtype === 'tool_use') {
    const tool = event as Record<string, unknown>
    const name = tool.tool_name as string
    const input = tool.tool_input as Record<string, unknown>

    switch (name) {
      case 'Read':
        return formatToolCall({ name, path: input.file_path as string }, phase)
      case 'Write':
        return formatToolCall({ name, path: input.file_path as string }, phase)
      case 'Edit':
        return formatToolCall({ name, path: input.file_path as string }, phase)
      case 'Grep':
        return formatToolCall({ name, detail: `"${input.pattern}" in ${input.path ?? '.'}` }, phase)
      case 'Glob':
        return formatToolCall({ name, detail: input.pattern as string }, phase)
      case 'Bash':
        return formatToolCall({ name, detail: (input.command as string).slice(0, 120) }, phase)
      default:
        return formatToolCall({ name }, phase)
    }
  }

  if (event.type === 'assistant' && event.subtype === 'text') {
    return { text: event.text as string, persist: true }
  }

  return null
}
