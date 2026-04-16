import { LLMPhase } from './types.js'

const DIM = '\x1b[2m'
const BOLD = '\x1b[1m'
const GREEN = '\x1b[32m'
const CYAN = '\x1b[36m'
const RESET = '\x1b[0m'
const CLEAR_LINE = '\x1b[2K\r'

export interface ToolCall {
  name: string
  path?: string
  detail?: string
  /** For Edit: first heading or summary from old_string/new_string */
  editContext?: string
}

export interface FormattedOutput {
  text: string
  persist: boolean
}

interface HighlightRule {
  pattern: RegExp
  label: string
}

const PHASE_HIGHLIGHTS: Record<string, HighlightRule[]> = {
  [LLMPhase.AnalyseWork]: [
    { pattern: /latest-session\.md$/, label: 'Writing session log' },
    { pattern: /commit-message\.md$/, label: 'Writing commit message' },
  ],
  [LLMPhase.Reflect]: [
    { pattern: /memory\.md$/, label: 'Updating memory' },
  ],
  [LLMPhase.Dream]: [
    { pattern: /memory\.md$/, label: 'Rewriting memory' },
  ],
  [LLMPhase.Triage]: [
    { pattern: /backlog\.md$/, label: 'Updating backlog' },
    { pattern: /subagent-dispatch\.yaml$/, label: 'Dispatching to related plans' },
  ],
}

/** Track which highlight labels have already been shown this phase. */
const shownHighlights = new Set<string>()

/** Call at the start of each phase to reset highlight tracking. */
export function resetHighlights(): void {
  shownHighlights.clear()
}

export function formatToolCall(tool: ToolCall, phase?: LLMPhase): FormattedOutput {
  const isWrite = /^(write|edit|Write|Edit)$/i.test(tool.name)

  // Check for Bash cat/echo redirects to highlighted files
  const isBashWrite = /^bash$/i.test(tool.name) && tool.detail &&
    /(?:cat|echo)\s.*>\s*/.test(tool.detail)

  if ((isWrite || isBashWrite) && phase) {
    const pathToCheck = tool.path ?? tool.detail ?? ''
    const rules = PHASE_HIGHLIGHTS[phase] ?? []
    for (const rule of rules) {
      if (rule.pattern.test(pathToCheck)) {
        if (shownHighlights.has(rule.label)) {
          return { text: '', persist: false }
        }
        shownHighlights.add(rule.label)
        return {
          text: `  ${BOLD}${GREEN}★  ${rule.label}${RESET}`,
          persist: true,
        }
      }
    }

    // Silently skip phase.md writes
    if (/phase\.md/.test(pathToCheck)) {
      return { text: '', persist: false }
    }
  }

  // Strip newlines from detail to prevent multi-line progress corruption
  const desc = (tool.detail ?? tool.path ?? '').replace(/\n.*/s, '')
  return {
    text: `${DIM}  ·  ${tool.name} ${desc}${RESET}`,
    persist: false,
  }
}

/**
 * Extract a brief context string from Edit old_string/new_string.
 * Looks for markdown headings or first meaningful line.
 */
export function extractEditContext(oldStr?: string, newStr?: string): string | undefined {
  const source = newStr ?? oldStr ?? ''
  const headingMatch = source.match(/^#{1,4}\s+(.{1,60})/m)
  if (headingMatch) return headingMatch[1].trim()
  const boldMatch = source.match(/\*\*(.{1,60}?)\*\*/m)
  if (boldMatch) return boldMatch[1].trim()
  return undefined
}

/**
 * Clean up a tool name for display.
 * Strips MCP prefixes: mcp__server-name__tool_name → tool_name
 */
export function cleanToolName(name: string): string {
  const mcpMatch = name.match(/^mcp__[^_]+(?:__)?(.+)$/)
  if (mcpMatch) return mcpMatch[1]
  return name
}

/** Common parameter keys to extract as detail, in priority order. */
const DETAIL_KEYS = ['path', 'file_path', 'command', 'pattern', 'query', 'url', 'prompt', 'mode']

/**
 * Extract a meaningful detail string from a tool's input parameters.
 */
export function extractToolDetail(input: Record<string, unknown>): string {
  for (const key of DETAIL_KEYS) {
    const val = input[key]
    if (typeof val === 'string' && val.length > 0) {
      const truncated = val.length > 80 ? val.slice(0, 77) + '...' : val
      return truncated
    }
  }
  // Fallback: show first string-valued parameter
  for (const [, val] of Object.entries(input)) {
    if (typeof val === 'string' && val.length > 0) {
      const truncated = val.length > 60 ? val.slice(0, 57) + '...' : val
      return truncated
    }
  }
  return ''
}

const YELLOW = '\x1b[33m'
const MAGENTA = '\x1b[35m'
const RED = '\x1b[31m'

/** Map structured action tags to display labels and colours. */
const ACTION_STYLES: Record<string, { colour: string }> = {
  ADDED:         { colour: GREEN },
  SHARPENED:     { colour: CYAN },
  REPLACED:      { colour: YELLOW },
  REMOVED:       { colour: RED },
  MERGED:        { colour: CYAN },
  TIGHTENED:     { colour: CYAN },
  REWORDED:      { colour: DIM },
  STATS:         { colour: DIM },
  DELETED:       { colour: RED },
  PROMOTED:      { colour: YELLOW },
  REPRIORITISED: { colour: YELLOW },
  DISPATCH:      { colour: GREEN },
  'NO DISPATCH': { colour: DIM },
}

const LABEL_WIDTH = Math.max(...Object.keys(ACTION_STYLES).map(k => k.length))

/**
 * Format result text from a headless phase.
 * Recognises structured [ACTION] markers, Insight blocks, and filters noise.
 */
export function formatResultText(text: string): string {
  const lines = text.split('\n')
  const formatted: string[] = ['']  // blank line separates progress from result
  let inInsight = false

  for (const line of lines) {
    // Filter out phase.md status lines
    if (line.match(/^(?:`?phase\.md`?|Phase)\s+(?:set to|written|→)/i)) continue
    if (line.match(/phase\.md.*`git-commit-/)) continue
    // Filter lines that are just "phase.md" mentions
    if (line.match(/wrote.*phase\.md/i)) continue
    // Filter code fence lines (triple backticks)
    if (line.match(/^```\s*$/)) continue

    // Structured action markers: [ACTION] detail (case-insensitive)
    const actionMatch = line.match(/^\[([A-Za-z ]+)\]\s*(.*)$/)
    if (actionMatch) {
      const tag = actionMatch[1].toUpperCase()
      const detail = actionMatch[2]
      const style = ACTION_STYLES[tag]
      if (style) {
        const paddedTag = tag.padEnd(LABEL_WIDTH)
        formatted.push(`  ${style.colour}${BOLD}${paddedTag}${RESET}  ${style.colour}${detail}${RESET}`)
        continue
      }
    }

    // Detect insight block opening: `★ Insight ─...`
    if (line.match(/^`?★\s*Insight\s*─/)) {
      inInsight = true
      formatted.push(`  ${BOLD}${CYAN}★ Insight${RESET}`)
      continue
    }
    // Detect insight block closing: `─────...`
    if (inInsight && line.match(/^`?─{10,}`?$/)) {
      inInsight = false
      continue
    }
    // Indent insight content
    if (inInsight) {
      formatted.push(`  ${DIM}${line}${RESET}`)
      continue
    }
    // Regular result text — indent for readability
    formatted.push(`  ${DIM}${line}${RESET}`)
  }

  // Trim trailing empty lines
  while (formatted.length > 1 && formatted[formatted.length - 1].match(/^\s*$/)) {
    formatted.pop()
  }

  return formatted.join('\n')
}

export function writeLine(output: FormattedOutput): void {
  if (!output.text) return  // skip empty (e.g. suppressed phase.md)
  if (output.persist) {
    process.stderr.write(CLEAR_LINE + output.text + '\n')
  } else {
    process.stderr.write(CLEAR_LINE + output.text)
  }
}

export function clearProgress(): void {
  process.stderr.write(CLEAR_LINE)
}

export const PHASE_INFO: Record<string, { label: string; description: string }> = {
  [LLMPhase.Work]: {
    label: 'WORK',
    description: 'Pick a task, implement it, record results',
  },
  [LLMPhase.AnalyseWork]: {
    label: 'ANALYSE',
    description: 'Examine git diff, write session log and commit message',
  },
  [LLMPhase.Reflect]: {
    label: 'REFLECT',
    description: 'Distil session learnings into durable memory',
  },
  [LLMPhase.Dream]: {
    label: 'DREAM',
    description: 'Rewrite memory losslessly in tighter form',
  },
  [LLMPhase.Triage]: {
    label: 'TRIAGE',
    description: 'Reprioritise backlog, propagate to related plans',
  },
}
