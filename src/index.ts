#!/usr/bin/env npx tsx
import path from 'node:path'
import fs from 'node:fs'
import { loadSharedConfig, loadAgentConfig } from './config.js'
import { type Agent, type PlanContext } from './types.js'
import { phaseLoop } from './phase-loop.js'
import { ClaudeCodeAgent } from './agents/claude-code/index.js'
import { PiAgent } from './agents/pi/index.js'

function findProjectRoot(startDir: string): string {
  let dir = startDir
  while (dir !== path.dirname(dir)) {
    if (fs.existsSync(path.join(dir, '.git'))) return dir
    dir = path.dirname(dir)
  }
  throw new Error(`No .git found above ${startDir}`)
}

function buildRelatedPlans(planDir: string): string {
  const relatedPath = path.join(planDir, 'related-plans.md')
  if (!fs.existsSync(relatedPath)) return ''
  return fs.readFileSync(relatedPath, 'utf-8')
}

function usage(): never {
  console.error('Usage: llm-context [--agent claude-code|pi] <plan-directory>')
  process.exit(1)
}

async function main() {
  const args = process.argv.slice(2)
  let agentOverride: string | undefined
  let planDir: string | undefined

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--agent') {
      agentOverride = args[++i]
    } else if (!args[i].startsWith('-')) {
      planDir = path.resolve(args[i])
    }
  }

  if (!planDir) usage()

  if (!fs.existsSync(path.join(planDir, 'phase.md'))) {
    console.error(`Error: ${planDir}/phase.md not found. Is this a valid plan directory?`)
    process.exit(1)
  }

  // Find the orchestrator root (directory containing this script's package.json)
  const scriptDir = path.dirname(new URL(import.meta.url).pathname)
  const projectRoot = path.resolve(scriptDir, '..')

  const sharedConfig = loadSharedConfig(projectRoot, agentOverride)
  const agentConfig = loadAgentConfig(projectRoot, sharedConfig.agent)

  const projectDir = findProjectRoot(planDir)
  const ctx: PlanContext = {
    planDir,
    projectDir,
    devRoot: path.dirname(projectDir),
    relatedPlans: buildRelatedPlans(planDir),
    orchestratorRoot: projectRoot,
  }

  let agent: Agent
  switch (sharedConfig.agent) {
    case 'claude-code':
      agent = new ClaudeCodeAgent(agentConfig, projectRoot)
      break
    case 'pi':
      agent = new PiAgent(agentConfig, projectRoot)
      break
    default:
      console.error(`Unknown agent: ${sharedConfig.agent}`)
      process.exit(1)
  }

  console.log(`▶ Agent: ${sharedConfig.agent}`)
  console.log(`▶ Plan: ${planDir}`)
  console.log(`▶ Project: ${projectDir}`)

  await phaseLoop(agent, ctx, sharedConfig, projectRoot)
}

main().catch(err => {
  console.error(err)
  process.exit(1)
})
