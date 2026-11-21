// Exercise the built storefront against isolated public settings fixtures.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const output = 'output/site-settings-browser'
const defaultImage = 'images/brunch-spread.webp'
const defaultAlt = 'Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole'
const selectedImage = '/api/images/8f14207b-3f35-4c15-99fa-1591443cac33'
const selectedAlt = 'Vybraná fotografie svačiny z pekárny Mysterium Gaudi'
const settings = { delivery_fee_cents: 0, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], online_payment_enabled: false }
const product = { id: 1, name: 'Testovací houstička', description: 'Svačina z pekárny Mysterium Gaudi', category: 'Slané', tag: '', weight: '100 g', price_cents: 12000, vegan: false, ingredients: '', allergens: '', image: '', active: true, stock: 99, version: 1 }
const errors = []
const passed = []

async function staticServer() {
  const root = path.resolve('dist')
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const contentType = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.webp': 'image/webp', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': contentType, 'Content-Security-Policy': "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self'; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'self'" })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  return { server, base: `http://127.0.0.1:${server.address().port}` }
}

async function run(browser, base, name, options = {}) {
  const viewport = options.viewport || { width: 1280, height: 1000 }
  const context = await browser.newContext({ viewport, reducedMotion: 'reduce' })
  const page = await context.newPage()
  const imageRequests = []
  page.setDefaultTimeout(10000)
  page.on('pageerror', error => errors.push(`${name}: ${error.message}`))
  await page.clock.setFixedTime(new Date('2026-11-21T12:00:00Z'))
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (route.request().method() !== 'GET') {
      errors.push(`${name}: unexpected mutation ${url.pathname}`)
      return json({ error: 'Unexpected mutation' }, 500)
    }
    if (url.pathname === selectedImage) {
      imageRequests.push(url.pathname)
      if (options.imageFailure === 'missing') return route.fulfill({ status: 404, body: 'Not found' })
      if (options.imageFailure === 'malformed') return route.fulfill({ contentType: 'image/webp', body: 'invalid image bytes' })
      return route.fulfill({ contentType: 'image/webp', body: await fs.readFile('public/images/brunch-spread.webp') })
    }
    if (url.pathname === '/api/settings') return json({ ...settings, ...options.settings })
    if (url.pathname === '/api/products') return json([product])
    if (url.pathname === '/api/delivery-options') return json({ dates: [{ value: '2026-11-23', label: 'Pondělí 23. listopadu' }], slots: ['09:00–10:00'] })
    if (url.pathname === '/api/terms') return json({ version: 1, title: 'Obchodní podmínky', content: 'Testovací podmínky.', updated_at: '2026-11-21T12:00:00Z' })
    if (url.pathname === '/api/auth/me') return json({ error: 'Nepřihlášen' }, 401)
    errors.push(`${name}: unexpected request ${url.pathname}`)
    return json({ error: 'Unexpected request' }, 500)
  })
  try {
    await page.goto(base)
    await page.locator('.product-card').waitFor()
    const expectedSource = options.settings?.hero_image && !options.imageFailure ? selectedImage : defaultImage
    const expectedAlt = expectedSource === defaultImage ? defaultAlt : selectedAlt
    await page.waitForFunction(({ source, alt }) => {
      const image = document.querySelector('.hero-visual > img')
      return image?.getAttribute('src') === source && image.alt === alt && image.complete && image.naturalWidth > 0
    }, { source: expectedSource, alt: expectedAlt })
    await page.evaluate(() => document.fonts.ready)
    assert.equal(await page.locator('.hero-visual > img').count(), 1, 'one hero image')
    assert.equal(await page.locator('.hero-copy').getByText('Z PEKÁRNY MYSTERIUM GAUDI', { exact: true }).count(), 1)
    assert.match(await page.locator('.hero-copy > p:not(.hero-delivery)').innerText(), /houstičky.*Mysterium Gaudi.*originální domácí receptury/)
    const geometry = await page.evaluate(() => {
      const box = selector => {
        const rect = document.querySelector(selector).getBoundingClientRect()
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height, right: rect.right, bottom: rect.bottom }
      }
      return { image: box('.hero-visual > img'), copy: box('.hero-copy'), title: box('#hero-title'), bodyWidth: document.documentElement.scrollWidth, viewport: innerWidth, fit: getComputedStyle(document.querySelector('.hero-visual > img')).objectFit }
    })
    assert(geometry.bodyWidth <= geometry.viewport, `no horizontal overflow: ${JSON.stringify(geometry)}`)
    assert(geometry.image.x >= 0 && geometry.image.right <= viewport.width, 'hero photo inside viewport')
    assert(geometry.image.height > 0 && geometry.image.height <= 400, 'bounded photo height')
    assert.equal(geometry.fit, 'cover')
    if (viewport.width <= 600) assert(geometry.image.y > geometry.copy.bottom, 'mobile photo follows copy')
    else assert(geometry.image.x > geometry.copy.right, 'desktop photo beside copy')
    if (options.settings?.hero_image) assert.equal(imageRequests.length, 1, 'selected image loads once, including on failure')
    else assert.equal(imageRequests.length, 0, 'default settings do not request uploaded photo')
    if (options.screenshot) {
      await page.screenshot({ path: `${output}/${options.screenshot}`, fullPage: true, animations: 'disabled', clip: { x: 0, y: 0, width: viewport.width, height: Math.ceil(geometry.image.bottom + 30) } })
    }
    passed.push(name)
  } finally {
    await page.unrouteAll({ behavior: 'ignoreErrors' })
    await context.close()
  }
}

async function main() {
  await fs.mkdir(output, { recursive: true })
  const { server, base } = await staticServer()
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  try {
    await run(browser, base, 'Older settings without hero fields retain default photo', { screenshot: 'default-desktop.png' })
    await run(browser, base, 'Empty hero fields retain default photo and description', { settings: { hero_image: '', hero_image_alt: '' } })
    const selected = { hero_image: selectedImage, hero_image_alt: selectedAlt }
    await run(browser, base, 'Uploaded hero photo and description render on desktop', { settings: selected, screenshot: 'selected-desktop.png' })
    await run(browser, base, 'Uploaded hero photo fits mobile layout', { settings: selected, viewport: { width: 390, height: 844 }, screenshot: 'selected-mobile.png' })
    await run(browser, base, 'Uploaded hero photo fits narrow mobile layout', { settings: selected, viewport: { width: 320, height: 740 } })
    await run(browser, base, 'Uploaded hero photo fits tablet layout', { settings: selected, viewport: { width: 768, height: 1024 }, screenshot: 'selected-tablet.png' })
    await run(browser, base, 'Missing uploaded hero photo falls back to default and description', { settings: selected, imageFailure: 'missing' })
    await run(browser, base, 'Malformed uploaded image falls back without repeated requests', { settings: selected, imageFailure: 'malformed' })
    assert.deepEqual(errors, [], 'no uncaught errors or unexpected API requests')
    const result = { passed, errors, screenshots: output }
    await fs.writeFile(`${output}/results.json`, `${JSON.stringify(result, null, 2)}\n`)
    console.log(JSON.stringify(result, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}

main().catch(error => { console.error(error); process.exitCode = 1 })
