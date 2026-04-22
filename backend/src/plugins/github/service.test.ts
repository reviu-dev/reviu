import { describe, expect, it } from 'vitest'

import {
  __githubServiceTestUtils,
  extractGithubRateLimitInfo,
  isGithubRateLimitNearLimit,
} from './service.js'

describe('github service rate limit parsing', () => {
  it('extracts GitHub rate limit headers from a response', () => {
    expect(extractGithubRateLimitInfo({
      'x-ratelimit-limit': '5000',
      'x-ratelimit-remaining': '4987',
      'x-ratelimit-used': 13,
      'x-ratelimit-reset': '1741801200',
      'x-ratelimit-resource': 'core',
    })).toEqual({
      limit: 5000,
      remaining: 4987,
      used: 13,
      reset: 1741801200,
      resource: 'core',
    })
  })

  it('returns null when no rate limit headers are present', () => {
    expect(extractGithubRateLimitInfo(undefined)).toBeNull()
    expect(extractGithubRateLimitInfo({})).toBeNull()
  })

  it('flags a rate limit as near when remaining is below the configured percentage', () => {
    expect(isGithubRateLimitNearLimit({
      limit: 5000,
      remaining: 499,
      resource: 'core',
    })).toBe(true)

    expect(isGithubRateLimitNearLimit({
      limit: 30,
      remaining: 2,
      resource: 'search',
    })).toBe(true)
  })

  it('does not flag a rate limit as near when remaining stays above the threshold', () => {
    expect(isGithubRateLimitNearLimit({
      limit: 5000,
      remaining: 500,
      resource: 'core',
    })).toBe(false)

    expect(isGithubRateLimitNearLimit({
      limit: 30,
      remaining: 4,
      resource: 'search',
    })).toBe(false)
  })
})

describe('github suggested change content patching', () => {
  it('replaces the anchored original lines with the suggested lines', () => {
    const content = 'fn main() {\n  println!("old");\n}\n'

    expect(__githubServiceTestUtils.applySuggestedLinesToContent(content, {
      original_start_line: 2,
      original_lines: ['  println!("old");'],
      suggested_lines: ['  println!("new");'],
    })).toBe('fn main() {\n  println!("new");\n}\n')
  })

  it('preserves crlf line endings', () => {
    const content = 'one\r\ntwo\r\nthree\r\n'

    expect(__githubServiceTestUtils.applySuggestedLinesToContent(content, {
      original_start_line: 2,
      original_lines: ['two'],
      suggested_lines: ['dos'],
    })).toBe('one\r\ndos\r\nthree\r\n')
  })

  it('rejects stale suggestions when the original lines changed', () => {
    expect(() => __githubServiceTestUtils.applySuggestedLinesToContent('one\ntwo\n', {
      original_start_line: 2,
      original_lines: ['changed'],
      suggested_lines: ['dos'],
    })).toThrow('Suggested change no longer matches the file.')
  })
})
