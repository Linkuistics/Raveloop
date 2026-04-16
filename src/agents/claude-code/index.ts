import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { type Agent, type PlanContext, type AgentConfig, LLMPhase } from '../../types.js'
import { formatClaudeStreamLine } from './stream-parser.js'

export class ClaudeCodeAgent implements Agent {
  private config: AgentConfig
  private projectRoot: string
  private dangerous: boolean

  constructor(config: AgentConfig, projectRoot: string, dangerous = false) {
    this.config = config
    this.projectRoot = projectRoot
    this.dangerous = dangerous
  }

  async invokeInteractive(prompt: string, ctx: PlanContext): Promise<void> {
    const args: string[] = ['--allow-dangerously-skip-permissions']
    const model = this.config.models[LLMPhase.Work]
    if (model) args.push('--model', model)
    if (this.dangerous) args.push('--dangerously-skip-permissions')
    args.push('--output-format', 'stream-json')
    args.push(prompt)

    return new Promise((resolve, reject) => {
      const child = spawn('claude', args, {
        cwd: ctx.projectDir,
        stdio: ['inherit', 'inherit', 'inherit'],
      })
      child.on('close', code => {
        if (code === 0) resolve()
        else reject(new Error(`claude exited with code ${code}`))
      })
    })
  }

  async invokeHeadless(prompt: string, ctx: PlanContext, phase: LLMPhase): Promise<string> {
    const args = ['--allow-dangerously-skip-permissions', '-p', prompt, '--output-format', 'stream-json']
    const model = this.config.models[phase]
    if (model) args.push('--model', model)
    if (this.dangerous) args.push('--dangerously-skip-permissions')

    return new Promise((resolve, reject) => {
      const chunks: string[] = []
      const child = spawn('claude', args, {
        cwd: ctx.projectDir,
        stdio: ['pipe', 'pipe', 'inherit'],
      })

      child.stdout.on('data', (data: Buffer) => {
        const lines = data.toString().split('\n')
        for (const line of lines) {
          const formatted = formatClaudeStreamLine(line, phase)
          if (formatted) {
            process.stderr.write(formatted + '\n')
            chunks.push(formatted)
          }
        }
      })

      child.on('close', code => {
        if (code === 0) resolve(chunks.join('\n'))
        else reject(new Error(`claude exited with code ${code}`))
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

  tokens(): Record<string, string> {
    const tokensPath = path.join(this.projectRoot, 'agents', 'claude-code', 'tokens.yaml')
    return YAML.parse(fs.readFileSync(tokensPath, 'utf-8')) as Record<string, string>
  }
}
