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
import { shouldDream, updateDreamBaseline } from './dream.js'
import { gitCommitPlan, gitSaveWorkBaseline } from './git.js'
import { dispatchSubagents } from './subagent-dispatch.js'

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

async function askContinue(): Promise<boolean> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  })
  return new Promise(resolve => {
    rl.question('\nContinue to next phase? [Y/n] ', answer => {
      rl.close()
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
    case ScriptPhase.GitCommitWork:
      gitCommitPlan(planDir, name, 'work')
      writePhase(planDir, LLMPhase.Reflect)
      return askContinue()

    case ScriptPhase.GitCommitReflect:
      gitCommitPlan(planDir, name, 'reflect')
      writePhase(planDir, LLMPhase.Dream)
      return true

    case ScriptPhase.GitCommitDream:
      gitCommitPlan(planDir, name, 'dream')
      writePhase(planDir, LLMPhase.Triage)
      return true

    case ScriptPhase.GitCommitTriage:
      gitCommitPlan(planDir, name, 'triage')
      writePhase(planDir, LLMPhase.Work)
      return askContinue()
  }
}

export async function phaseLoop(
  agent: Agent,
  ctx: PlanContext,
  config: SharedConfig,
  projectRoot: string
): Promise<void> {
  const tokens = agent.tokens()

  if (agent.setup) {
    await agent.setup(ctx)
  }

  while (true) {
    const phase = readPhase(ctx.planDir)
    console.log(`\n▶ Phase: ${phase}`)

    if (isScriptPhase(phase)) {
      const shouldContinue = await handleScriptPhase(phase, ctx.planDir)
      if (!shouldContinue) {
        console.log('Exiting.')
        return
      }
      continue
    }

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
      console.error(`⚠ Phase did not advance from ${phase}. Stopping.`)
      return
    }

    // Dream trigger: after reflect commits, check if dream is needed
    if (newPhase === LLMPhase.Dream || readPhase(ctx.planDir) === LLMPhase.Dream) {
      if (!shouldDream(ctx.planDir, config.headroom)) {
        console.log('  ⏭ Dream skipped (memory within headroom)')
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
