import { describe, expect, it } from 'vitest'
import { formatCommitMessage } from '../providers.js'

describe('formatCommitMessage', () => {
  it('joins subject and body with a blank line', () => {
    expect(formatCommitMessage('feat: add thing', 'Explains why.')).toBe(
      'feat: add thing\n\nExplains why.',
    )
  })

  it('returns just the subject when the body is empty or missing', () => {
    expect(formatCommitMessage('fix: typo', '')).toBe('fix: typo')
    expect(formatCommitMessage('fix: typo')).toBe('fix: typo')
    expect(formatCommitMessage('fix: typo', '   ')).toBe('fix: typo')
  })

  it('trims surrounding whitespace', () => {
    expect(formatCommitMessage('  chore: bump  ', '  body  ')).toBe('chore: bump\n\nbody')
  })
})
