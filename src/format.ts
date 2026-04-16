import { LLMPhase } from './types.js'

const DIM = '\x1b[2m'
const BOLD = '\x1b[1m'
const GREEN = '\x1b[32m'
const YELLOW = '\x1b[33m'
const RESET = '\x1b[0m'

export interface ToolCall {
  name: string
  path?: string
  detail?: string
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

export function formatToolCall(tool: ToolCall, phase?: LLMPhase): string {
  const isWrite = /^(write|edit|Write|Edit)$/i.test(tool.name)

  if (isWrite && tool.path && phase) {
    const rules = [...(PHASE_HIGHLIGHTS[phase] ?? []), ...ALWAYS_HIGHLIGHT]
    for (const rule of rules) {
      if (rule.pattern.test(tool.path)) {
        return `  ${BOLD}${GREEN}★  ${rule.label}${RESET}`
      }
    }
  }

  const desc = tool.detail ?? tool.path ?? ''
  return `${DIM}  ·  ${tool.name} ${desc}${RESET}`
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
