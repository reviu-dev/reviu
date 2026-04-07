import { describe, expect, it } from 'vitest'

const {
  normalizeSemver,
  parseDesktopUpdateManifest,
  resolveDesktopUpdateCheck,
} = await import('./service.js')

describe('desktop update service', () => {
  it('normalizes stable and prerelease semver values', () => {
    expect(normalizeSemver('  =v0.0.4-alpha.1  ')).toBe('0.0.4-alpha.1')
    expect(normalizeSemver('0.1.0')).toBe('0.1.0')
    expect(normalizeSemver('nope')).toBeNull()
  })

  it('parses prerelease manifests with aarch64 artifacts', () => {
    const manifest = parseDesktopUpdateManifest({
      version: '0.0.4-alpha.1',
      minimumSupportedVersion: 'v0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifacts: [
        {
          platform: 'macos',
          arch: 'aarch64',
          url: 'https://github.com/joris-gallot/reviu/releases/download/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-aarch64.dmg',
          sha256: 'AA1DD070755C9E97E4B785D310BF7CEF1F440DD8F624580D3974DDC82DEE92D5',
          size: 17222631,
        },
      ],
    })

    expect(manifest).toEqual({
      version: '0.0.4-alpha.1',
      minimumSupportedVersion: '0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifacts: [
        {
          platform: 'macos',
          arch: 'aarch64',
          url: 'https://github.com/joris-gallot/reviu/releases/download/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-aarch64.dmg',
          sha256: 'aa1dd070755c9e97e4b785d310bf7cef1f440dd8f624580d3974ddc82dee92d5',
          size: 17222631,
        },
      ],
    })
  })

  it('resolves update availability for prerelease versions', () => {
    const manifest = parseDesktopUpdateManifest({
      version: '0.0.4-alpha.1',
      minimumSupportedVersion: '0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifacts: [
        {
          platform: 'macos',
          arch: 'aarch64',
          url: 'https://github.com/joris-gallot/reviu/releases/download/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-aarch64.dmg',
          sha256: 'aa1dd070755c9e97e4b785d310bf7cef1f440dd8f624580d3974ddc82dee92d5',
          size: 17222631,
        },
      ],
    })

    expect(resolveDesktopUpdateCheck(manifest, {
      currentVersion: 'v0.0.3',
      platform: 'macos',
      arch: 'aarch64',
    })).toEqual({
      updateAvailable: true,
      forceUpdate: true,
      currentVersion: '0.0.3',
      latestVersion: '0.0.4-alpha.1',
      minimumSupportedVersion: '0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifact: {
        url: 'http://localhost:4000/desktop/update/download/release/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-aarch64.dmg',
        sha256: 'aa1dd070755c9e97e4b785d310bf7cef1f440dd8f624580d3974ddc82dee92d5',
        size: 17222631,
      },
    })
  })

  it('rejects arm64 manifest artifacts', () => {
    expect(() => parseDesktopUpdateManifest({
      version: '0.0.4-alpha.1',
      minimumSupportedVersion: '0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifacts: [
        {
          platform: 'macos',
          arch: 'arm64',
          url: 'https://github.com/joris-gallot/reviu/releases/download/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-arm64.dmg',
          sha256: 'aa1dd070755c9e97e4b785d310bf7cef1f440dd8f624580d3974ddc82dee92d5',
          size: 17222631,
        },
      ],
    })).toThrowError('Invalid desktop update manifest payload')
  })

  it('rejects invalid manifest versions', () => {
    expect(() => parseDesktopUpdateManifest({
      version: '0.0',
      minimumSupportedVersion: '0.0.4-alpha.0',
      releaseNotesUrl: 'https://github.com/joris-gallot/reviu/releases/tag/v0.0.4-alpha.1',
      artifacts: [
        {
          platform: 'macos',
          arch: 'aarch64',
          url: 'https://github.com/joris-gallot/reviu/releases/download/v0.0.4-alpha.1/Reviu-0.0.4-alpha.1-macos-aarch64.dmg',
          sha256: 'aa1dd070755c9e97e4b785d310bf7cef1f440dd8f624580d3974ddc82dee92d5',
          size: 17222631,
        },
      ],
    })).toThrowError('Invalid desktop update manifest version')
  })
})
