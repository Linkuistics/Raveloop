import { type LLMPhase } from '../../types.js'
import { formatToolCall, type FormattedOutput } from '../../format.js'

export function formatPiStreamLine(line: string, phase?: LLMPhase): FormattedOutput | null {
  if (!line.trim()) return null

  let event: Record<string, unknown>
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  if (event.type === 'tool_execution_start') {
    const name = event.tool_name as string
    const input = event.tool_input as Record<string, unknown>

    switch (name) {
      case 'read':
        return formatToolCall({ name, path: (input.file_path ?? input.path ?? '') as string }, phase)
      case 'write':
        return formatToolCall({ name, path: (input.file_path ?? input.path ?? '') as string }, phase)
      case 'edit':
        return formatToolCall({ name, path: (input.file_path ?? input.path ?? '') as string }, phase)
      case 'grep':
        return formatToolCall({ name, detail: `"${input.pattern}" in ${input.path ?? '.'}` }, phase)
      case 'find':
        return formatToolCall({ name, detail: (input.pattern ?? input.glob ?? '') as string }, phase)
      case 'bash':
        return formatToolCall({ name, detail: (input.command as string).slice(0, 120) }, phase)
      default:
        return formatToolCall({ name }, phase)
    }
  }

  if (event.type === 'tool_execution_end' && event.isError) {
    return { text: `  \x1b[31m✗  tool error\x1b[0m`, persist: true }
  }

  if (event.type === 'message_end') {
    const content = event.content as Array<{ type: string; text?: string }>
    if (Array.isArray(content)) {
      const text = content
        .filter(c => c.type === 'text' && c.text)
        .map(c => c.text)
        .join('\n')
      if (text) return { text, persist: true }
    }
  }

  return null
}
