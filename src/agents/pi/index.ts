import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { type Agent, type PlanContext, type AgentConfig, LLMPhase } from '../../types.js'
import { formatPiStreamLine } from './stream-parser.js'
import { writeLine, clearProgress } from '../../format.js'
import { setupPi } from './setup.js'

export class PiAgent implements Agent {
  private config: AgentConfig
  private projectRoot: string

  constructor(config: AgentConfig, projectRoot: string) {
    this.config = config
    this.projectRoot = projectRoot
  }

  private loadPromptFile(name: string, ctx: PlanContext): string {
    const filePath = path.join(this.projectRoot, 'agents', 'pi', 'prompts', name)
    let content = fs.readFileSync(filePath, 'utf-8')
    content = content.replaceAll('{{PROJECT}}', ctx.projectDir)
    content = content.replaceAll('{{DEV_ROOT}}', ctx.devRoot)
    content = content.replaceAll('{{PLAN}}', ctx.planDir)
    return content
  }

  async invokeInteractive(prompt: string, ctx: PlanContext): Promise<void> {
    const systemPrompt = this.loadPromptFile('system-prompt.md', ctx)
    const memoryPrompt = this.loadPromptFile('memory-prompt.md', ctx)
    const fullSystemPrompt = systemPrompt + '\n\n' + memoryPrompt + '\n\n' + prompt

    const args: string[] = [
      '--no-session',
      '--append-system-prompt', fullSystemPrompt,
      '--provider', this.config.provider ?? 'anthropic',
      '--model', this.config.models[LLMPhase.Work],
    ]

    const thinking = this.config.thinking?.[LLMPhase.Work]
    if (thinking) args.push('--thinking', thinking)

    return new Promise((resolve, reject) => {
      const child = spawn('pi', args, {
        cwd: ctx.projectDir,
        stdio: ['inherit', 'inherit', 'inherit'],
      })
      child.on('close', code => {
        if (code === 0) resolve()
        else reject(new Error(`pi exited with code ${code}`))
      })
    })
  }

  async invokeHeadless(prompt: string, ctx: PlanContext, phase: LLMPhase): Promise<string> {
    const systemPrompt = this.loadPromptFile('system-prompt.md', ctx)

    const args: string[] = [
      '--no-session',
      '--append-system-prompt', systemPrompt,
      '--provider', this.config.provider ?? 'anthropic',
      '--model', this.config.models[phase],
      '--mode', 'json',
      '-p', prompt,
    ]

    const thinking = this.config.thinking?.[phase]
    if (thinking) args.push('--thinking', thinking)

    return new Promise((resolve, reject) => {
      const chunks: string[] = []
      const child = spawn('pi', args, {
        cwd: ctx.projectDir,
        stdio: ['pipe', 'pipe', 'inherit'],
      })

      child.stdout.on('data', (data: Buffer) => {
        const lines = data.toString().split('\n')
        for (const line of lines) {
          const formatted = formatPiStreamLine(line, phase)
          if (formatted) {
            writeLine(formatted)
            if (formatted.persist) chunks.push(formatted.text)
          }
        }
      })

      child.on('close', code => {
        clearProgress()
        if (code === 0) resolve(chunks.join('\n'))
        else reject(new Error(`pi exited with code ${code}`))
      })
    })
  }

  async dispatchSubagent(prompt: string, targetPlan: string): Promise<string> {
    return this.invokeHeadless(prompt, {
      planDir: targetPlan,
      projectDir: path.dirname(path.dirname(targetPlan)),
      devRoot: path.dirname(path.dirname(path.dirname(targetPlan))),
      relatedPlans: '',
      orchestratorRoot: this.projectRoot,
    }, LLMPhase.Triage)
  }

  async setup(ctx: PlanContext): Promise<void> {
    await setupPi(this.projectRoot, ctx)
  }

  tokens(): Record<string, string> {
    const tokensPath = path.join(this.projectRoot, 'agents', 'pi', 'tokens.yaml')
    return YAML.parse(fs.readFileSync(tokensPath, 'utf-8')) as Record<string, string>
  }
}
