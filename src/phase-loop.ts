import fs from 'node:fs'
import path from 'node:path'
import readline from 'node:readline'
import {
  type Agent,
  type PlanContext,
  type SharedConfig,
  type Phase,
  LLMPhase,
  ScriptPhase,
  isScriptPhase,
} from './types.js'
import { composePrompt } from './prompt-composer.js'
import { PHASE_INFO } from './format.js'
import { shouldDream, updateDreamBaseline } from './dream.js'
import { gitCommitPlan, gitSaveWorkBaseline, type CommitResult } from './git.js'
import { dispatchSubagents } from './subagent-dispatch.js'

const DIM = '\x1b[2m'
const BOLD = '\x1b[1m'
const CYAN = '\x1b[36m'
const YELLOW = '\x1b[33m'
const RED = '\x1b[31m'
const RESET = '\x1b[0m'
const HR = '────────────────────────────────────────────────────'

function phaseHeader(label: string, description: string, plan: string): void {
  console.log(`\n${DIM}${HR}${RESET}`)
  console.log(`  ${BOLD}${CYAN}◆  ${label}${RESET}${DIM}  ·  ${plan}${RESET}`)
  console.log(`  ${DIM}${description}${RESET}`)
  console.log(`${DIM}${HR}${RESET}`)
}

function errorBanner(msg: string): void {
  console.error(`\n  ${RED}✗  ${msg}${RESET}\n`)
}

function showCommit(phaseName: string, plan: string, result: CommitResult): void {
  if (result.committed) {
    const firstLine = result.message.split('\n')[0]
    console.log(`\n  ${BOLD}${CYAN}⚙  COMMIT · ${phaseName}${RESET}${DIM}  ·  ${plan}  ·${RESET}  ${firstLine}`)
  } else {
    console.log(`\n  ${BOLD}${CYAN}⚙  COMMIT · ${phaseName}${RESET}${DIM}  ·  ${plan}  ·  nothing to commit${RESET}`)
  }
}

function readPhase(planDir: string): Phase {
  const phasePath = path.join(planDir, 'phase.md')
  return fs.readFileSync(phasePath, 'utf-8').trim() as Phase
}

function writePhase(planDir: string, phase: Phase): void {
  fs.writeFileSync(path.join(planDir, 'phase.md'), phase)
}

function planName(planDir: string): string {
  return path.basename(planDir)
}

async function askContinue(nextLabel: string): Promise<boolean> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  })
  return new Promise(resolve => {
    const prompt = `\n${DIM}${HR}${RESET}\n  ${BOLD}${YELLOW}▶  Proceed to ${nextLabel}? [Y/n]${RESET} `
    rl.question(prompt, answer => {
      rl.close()
      console.log(`${DIM}${HR}${RESET}`)
      resolve(answer.trim().toLowerCase() !== 'n')
    })
  })
}

async function handleScriptPhase(
  phase: ScriptPhase,
  planDir: string
): Promise<boolean> {
  const name = planName(planDir)

  switch (phase) {
    case ScriptPhase.GitCommitWork: {
      showCommit('work', name, gitCommitPlan(planDir, name, 'work'))
      writePhase(planDir, LLMPhase.Reflect)
      return askContinue('reflect phase')
    }

    case ScriptPhase.GitCommitReflect: {
      showCommit('reflect', name, gitCommitPlan(planDir, name, 'reflect'))
      writePhase(planDir, LLMPhase.Dream)
      return true
    }

    case ScriptPhase.GitCommitDream: {
      showCommit('dream', name, gitCommitPlan(planDir, name, 'dream'))
      writePhase(planDir, LLMPhase.Triage)
      return true
    }

    case ScriptPhase.GitCommitTriage: {
      showCommit('triage', name, gitCommitPlan(planDir, name, 'triage'))
      writePhase(planDir, LLMPhase.Work)
      return askContinue('next work phase')
    }
  }
}

export async function phaseLoop(
  agent: Agent,
  ctx: PlanContext,
  config: SharedConfig,
  projectRoot: string
): Promise<void> {
  const tokens = agent.tokens()
  const name = planName(ctx.planDir)

  if (agent.setup) {
    await agent.setup(ctx)
  }

  while (true) {
    const phase = readPhase(ctx.planDir)

    if (isScriptPhase(phase)) {
      const shouldContinue = await handleScriptPhase(phase, ctx.planDir)
      if (!shouldContinue) {
        console.log(`\n${DIM}Exiting.${RESET}`)
        return
      }
      continue
    }

    // Display phase header
    const info = PHASE_INFO[phase]
    phaseHeader(info?.label ?? phase, info?.description ?? '', name)

    // Pre-work: save baseline for analyse-work diff
    if (phase === LLMPhase.Work) {
      gitSaveWorkBaseline(ctx.planDir)
      fs.rmSync(path.join(ctx.planDir, 'latest-session.md'), { force: true })
    }

    const prompt = composePrompt(projectRoot, phase, ctx, tokens)

    if (phase === LLMPhase.Work) {
      await agent.invokeInteractive(prompt, ctx)
    } else {
      await agent.invokeHeadless(prompt, ctx, phase)
    }

    // Post-phase: check phase advanced
    const newPhase = readPhase(ctx.planDir)
    if (newPhase === phase) {
      errorBanner(`Phase did not advance from ${phase}. Stopping.`)
      return
    }

    // Dream trigger: after reflect, check if dream is needed
    if (readPhase(ctx.planDir) === LLMPhase.Dream) {
      if (!shouldDream(ctx.planDir, config.headroom)) {
        console.log(`  ${DIM}⏭  Dream skipped (memory within headroom)${RESET}`)
        writePhase(ctx.planDir, ScriptPhase.GitCommitDream)
      }
    }

    // After dream phase completes, update baseline
    if (phase === LLMPhase.Dream) {
      updateDreamBaseline(ctx.planDir)
    }

    // After triage, dispatch subagents
    if (phase === LLMPhase.Triage) {
      await dispatchSubagents(agent, ctx.planDir)
    }
  }
}
