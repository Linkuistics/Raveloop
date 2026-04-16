import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

export interface CommitResult {
  committed: boolean
  message: string
}

export function gitCommitPlan(planDir: string, planName: string, phaseName: string): CommitResult {
  const commitMsgPath = path.join(planDir, 'commit-message.md')
  let message: string

  if (fs.existsSync(commitMsgPath)) {
    message = fs.readFileSync(commitMsgPath, 'utf-8').trim()
    fs.unlinkSync(commitMsgPath)
  } else {
    message = `run-plan: ${phaseName} (${planName})`
  }

  execSync(`git add "${planDir}"`, { stdio: 'pipe' })

  try {
    execSync('git diff --cached --quiet', { stdio: 'pipe' })
    return { committed: false, message }
  } catch {
    // git diff --cached --quiet exits non-zero when there are staged changes
    execSync(`git commit -m "${message.replace(/"/g, '\\"')}"`, { stdio: 'pipe' })
    return { committed: true, message }
  }
}

export function gitSaveWorkBaseline(planDir: string): void {
  const baselinePath = path.join(planDir, 'work-baseline')
  try {
    const sha = execSync('git rev-parse HEAD', { stdio: 'pipe' }).toString().trim()
    fs.writeFileSync(baselinePath, sha)
  } catch {
    fs.writeFileSync(baselinePath, '')
  }
}
