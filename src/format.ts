import { LLMPhase } from './types.js'

const DIM = '\x1b[2m'
const BOLD = '\x1b[1m'
const GREEN = '\x1b[32m'
const RESET = '\x1b[0m'
const CLEAR_LINE = '\x1b[2K\r'

export interface ToolCall {
  name: string
  path?: string
  detail?: string
}

export interface FormattedOutput {
  text: string
  persist: boolean  // true = print with \n (stays); false = overwrite in place (progress)
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

const ALWAYS_HIGHLIGHT: HighlightRule[] = [
  { pattern: /phase\.md$/, label: 'Advancing phase' },
]

export function formatToolCall(tool: ToolCall, phase?: LLMPhase): FormattedOutput {
  const isWrite = /^(write|edit|Write|Edit)$/i.test(tool.name)

  if (isWrite && tool.path && phase) {
    const rules = [...(PHASE_HIGHLIGHTS[phase] ?? []), ...ALWAYS_HIGHLIGHT]
    for (const rule of rules) {
      if (rule.pattern.test(tool.path)) {
        return {
          text: `  ${BOLD}${GREEN}★  ${rule.label}${RESET}`,
          persist: true,
        }
      }
    }
  }

  const desc = tool.detail ?? tool.path ?? ''
  return {
    text: `${DIM}  ·  ${tool.name} ${desc}${RESET}`,
    persist: false,
  }
}

/**
 * Write a formatted line to stderr, either overwriting the current
 * progress line or persisting as a permanent line.
 */
export function writeLine(output: FormattedOutput): void {
  if (output.persist) {
    // Clear any lingering progress line, then print permanently
    process.stderr.write(CLEAR_LINE + output.text + '\n')
  } else {
    // Overwrite in place — acts as a progress indicator
    process.stderr.write(CLEAR_LINE + output.text)
  }
}

/**
 * Write persistent text (LLM response, errors). Clears progress line first.
 */
export function writeText(text: string): void {
  process.stderr.write(CLEAR_LINE + text + '\n')
}

/**
 * Clear any lingering progress line (call after headless phase completes).
 */
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
