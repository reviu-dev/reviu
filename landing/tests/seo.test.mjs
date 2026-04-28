import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const distDir = path.resolve(__dirname, '../dist')

const readDistFile = async (relativePath) => {
  return readFile(path.join(distDir, relativePath), 'utf8')
}

test('homepage ships crawlable metadata and content', async () => {
  const html = await readDistFile('index.html')

  assert.match(html, /<title>Reviu - Rust Desktop Git Client<\/title>/)
  assert.match(
    html,
    /<meta name="description" content="Reviu is a native Rust desktop Git client for pull request review/,
  )
  assert.match(html, /<link rel="canonical" href="https:\/\/reviu\.dev\/"/)
  assert.match(html, /<meta property="og:title" content="Reviu - Rust Desktop Git Client"/)
  assert.match(html, /<meta name="twitter:card" content="summary_large_image"/)
  assert.match(html, /SoftwareApplication/)
  assert.match(html, /FAQPage/)
  assert.match(html, /Review GitHub pull requests from a native Git client\./)
  assert.match(html, /One desktop loop for daily GitHub review work\./)
  assert.match(html, /Coming from Fork\?/)
  assert.match(html, /srcset=/)
  assert.match(html, /--fit: contain;/)
  assert.doesNotMatch(html, /client="only"/)
})

test('robots and sitemap are generated', async () => {
  const robots = await readDistFile('robots.txt')
  const sitemap = await readDistFile('sitemap.xml')

  assert.match(robots, /Sitemap: https:\/\/reviu\.dev\/sitemap\.xml/)
  assert.match(sitemap, /<loc>https:\/\/reviu\.dev\/<\/loc>/)
})

test('legal pages are marked noindex', async () => {
  const privacy = await readDistFile('privacy/index.html')
  const terms = await readDistFile('terms/index.html')

  assert.match(privacy, /<title>Reviu - Privacy Policy<\/title>/)
  assert.match(terms, /<title>Reviu - Terms of Service<\/title>/)
  assert.match(privacy, /<meta name="robots" content="noindex,follow"/)
  assert.match(terms, /<meta name="robots" content="noindex,follow"/)
})
