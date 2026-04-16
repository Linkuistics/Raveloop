export function formatPiStreamLine(line: string): string | null {
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
        return `  ▸ read ${input.file_path ?? input.path ?? ''}`
      case 'write':
        return `  ▸ write ${input.file_path ?? input.path ?? ''}`
      case 'edit':
        return `  ▸ edit ${input.file_path ?? input.path ?? ''}`
      case 'grep':
        return `  ▸ grep "${input.pattern}" in ${input.path ?? '.'}`
      case 'find':
        return `  ▸ find ${input.pattern ?? input.glob ?? ''}`
      case 'bash':
        return `  ▸ bash: ${(input.command as string).slice(0, 120)}`
      default:
        return `  ▸ ${name}`
    }
  }

  if (event.type === 'tool_execution_end' && event.isError) {
    return `  ✗ tool error`
  }

  if (event.type === 'message_end') {
    const content = event.content as Array<{ type: string; text?: string }>
    if (Array.isArray(content)) {
      return content
        .filter(c => c.type === 'text' && c.text)
        .map(c => c.text)
        .join('\n')
    }
  }

  return null
}
