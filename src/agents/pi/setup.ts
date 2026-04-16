import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import YAML from 'yaml'
import { type PlanContext } from '../../types.js'

function isPiInstalled(): boolean {
  try {
    execSync('which pi', { stdio: 'pipe' })
    return true
  } catch {
    return false
  }
}

function isSubagentExtensionInstalled(): boolean {
  const settingsPath = path.join(os.homedir(), '.pi', 'agent', 'settings.json')
  if (!fs.existsSync(settingsPath)) return false

  try {
    const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf-8')) as {
      packages?: string[]
    }
    return settings.packages?.some(p => p.includes('pi-subagent')) ?? false
  } catch {
    return false
  }
}

function installSubagentExtension(): void {
  console.log('  Installing Pi subagent extension...')
  execSync('pi install npm:@mjakl/pi-subagent', { stdio: 'inherit' })
  console.log('  ✓ Subagent extension installed')
}

interface SkillFrontmatter {
  name: string
  description: string
  tools?: string[]
  model?: string
  thinking?: string
}

function parseSkillFrontmatter(content: string): { frontmatter: SkillFrontmatter; body: string } {
  const match = content.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/)
  if (!match) {
    throw new Error('Skill file missing YAML frontmatter')
  }
  const frontmatter = YAML.parse(match[1]) as SkillFrontmatter
  return { frontmatter, body: match[2] }
}

function generateAgentDefinition(skill: { frontmatter: SkillFrontmatter; body: string }): string {
  const fm = skill.frontmatter
  const lines = ['---']
  lines.push(`name: ${fm.name}`)
  lines.push(`description: ${fm.description}`)
  if (fm.tools) lines.push(`tools: ${fm.tools.join(', ')}`)
  if (fm.model) lines.push(`model: ${fm.model}`)
  if (fm.thinking) lines.push(`thinking: ${fm.thinking}`)
  lines.push('---')
  lines.push('')
  lines.push(skill.body)
  return lines.join('\n')
}

export async function setupPi(projectRoot: string, ctx: PlanContext): Promise<void> {
  console.log('▶ Pi setup...')

  // Check prerequisites
  if (!isPiInstalled()) {
    throw new Error(
      'pi is not installed. Install with: npm install -g @mariozechner/pi-coding-agent'
    )
  }

  if (!process.env.ANTHROPIC_API_KEY) {
    console.warn('  ⚠ ANTHROPIC_API_KEY not set — pi may fail to authenticate')
  }

  // Install subagent extension if needed
  if (!isSubagentExtensionInstalled()) {
    installSubagentExtension()
  } else {
    console.log('  ✓ Subagent extension already installed')
  }

  // Generate agent definitions from skills
  const skillsDir = path.join(projectRoot, 'skills')
  if (!fs.existsSync(skillsDir)) {
    console.log('  ⏭ No skills/ directory — skipping agent definition generation')
    return
  }

  const agentsDir = path.join(ctx.projectDir, '.pi', 'agents')
  fs.mkdirSync(agentsDir, { recursive: true })

  const skillFiles = fs.readdirSync(skillsDir).filter(f => f.endsWith('.md'))
  for (const file of skillFiles) {
    const content = fs.readFileSync(path.join(skillsDir, file), 'utf-8')
    try {
      const skill = parseSkillFrontmatter(content)
      const agentDef = generateAgentDefinition(skill)
      fs.writeFileSync(path.join(agentsDir, file), agentDef)
    } catch (err) {
      console.warn(`  ⚠ Skipping ${file}: ${err}`)
    }
  }

  console.log(`  ✓ Generated ${skillFiles.length} agent definition(s) in ${agentsDir}`)
}
