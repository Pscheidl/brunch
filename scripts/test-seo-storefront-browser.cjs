// Local WASM regression for crawlable links and the server-content handoff.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const http = require('node:http')
const path = require('node:path')

const products = [{
  id: 12, name: 'Houska se sýrem', description: 'Čerstvé pečivo se sýrem.',
  category: 'Slané', tag: '', weight: '100 g', ingredients: 'Pečivo, sýr',
  allergens: '1,7', image: '/images/brunch-spread.webp', active: true,
  stock: null, variants: [], vegan: false, banquet: false,
  minimum_quantity: 0, version: 1, price_cents: 4900
}]
const homeTitle = 'Brunch Era · Dobrá pauza začíná tady'
async function main() {
  const root = path.resolve('dist')
  const output = path.resolve('output/seo-storefront-browser')
  await fs.mkdir(output, { recursive: true })
  let base
  const server = http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url, base)
      const filename = path.resolve(root, `.${url.pathname === '/' ? '/index.html' : url.pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      let body = await fs.readFile(filename)
      if (url.pathname === '/') {
        const canonical = `${base}/${url.searchParams.has('product') ? '?product=12' : ''}`
        body = Buffer.from(body.toString().replace('<!--server-content-->', '<div id="server-content"><main><h1>Nabídka pečiva</h1><a href="/?product=12">Houska se sýrem</a><p>Čerstvé pečivo se sýrem.</p></main></div>')
          .replace('<meta name="robots" content="noindex,follow">', '<meta name="robots" content="index,follow">')
          .replace('</head>', `<link rel="canonical" href="${canonical}"><meta property="og:url" content="${canonical}"></head>`))
      }
      response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.webp': 'image/webp' }[path.extname(filename)] || 'application/octet-stream' })
      response.end(body)
    } catch {
      response.writeHead(404)
      response.end('Not found')
    }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true })
  const errors = [], checks = []
  async function fixture({ fail = false, hold = false, javaScriptEnabled = true } = {}) {
    const context = await browser.newContext({ javaScriptEnabled, viewport: { width: 1280, height: 900 } })
    const page = await context.newPage()
    let release
    const gate = hold ? new Promise(resolve => { release = resolve }) : Promise.resolve()
    const model = { fail, release }
    page.on('pageerror', error => errors.push(error.message))
    await context.route('**/api/**', async route => {
      const url = new URL(route.request().url())
      const reply = (json, status = 200) => route.fulfill({ json, status })
      if (url.pathname === '/api/products') {
        await gate
        return model.fail ? reply({ error: 'Nabídku teď nelze načíst.' }, 503) : reply(products)
      }
      if (url.pathname === '/api/settings') return reply({ delivery_fee_cents: 4900, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], online_payment_enabled: false })
      if (url.pathname === '/api/delivery-options') return reply({ dates: [], slots: ['08:00–17:00'], postal_slots: {} })
      if (url.pathname === '/api/auth/me') return reply({ error: 'Nepřihlášen' }, 401)
      if (url.pathname === '/api/terms') return reply({ version: 1, title: 'Podmínky', content: 'Testovací podmínky', updated_at: '2026-11-21T12:00:00Z' })
      errors.push(`Unexpected request ${url.pathname}`)
      return reply({ error: 'Unexpected request' }, 500)
    })
    return { context, page, model }
  }
  try {
    const delayed = await fixture({ hold: true })
    await delayed.page.goto(base)
    await delayed.page.locator('#client-content').waitFor({ state: 'attached' })
    assert(await delayed.page.locator('#server-content').isVisible())
    assert(await delayed.page.locator('#client-content').isHidden())
    assert.equal(await delayed.page.locator('#server-content a').getAttribute('href'), '/?product=12')
    delayed.model.release()
    await delayed.page.locator('#client-content .product-card').waitFor()
    assert.equal(await delayed.page.locator('#server-content').count(), 0)
    assert.equal(await delayed.page.locator('#app-loading').count(), 0)
    assert.equal(await delayed.page.locator('main').count(), 1)
    checks.push('Server content remains visible until APIs are ready, then hands off without duplicate catalog')

    const page = delayed.page
    const titleLink = page.locator('.product-card h3 a')
    const photoLink = page.locator('.product-card a.product-photo')
    assert.equal(await titleLink.getAttribute('href'), '/?product=12')
    assert.equal(await photoLink.getAttribute('href'), '/?product=12')
    await page.evaluate(() => { window.linkNavigationSentinel = 'unchanged' })
    await titleLink.focus()
    await page.keyboard.press('Enter')
    await page.locator('.product-dialog[open]').waitFor()
    assert.equal(await page.title(), 'Houska se sýrem | Brunch Era')
    assert.equal(await page.locator('link[rel=canonical]').getAttribute('href'), `${base}/?product=12`)
    assert.equal(await page.locator('meta[name=description]').getAttribute('content'), products[0].description)
    assert.equal(await page.evaluate(() => window.linkNavigationSentinel), 'unchanged')
    await page.getByRole('button', { name: 'Zavřít detail svačiny' }).click()
    await page.waitForFunction(title => document.title === title, homeTitle)
    assert.equal(await page.locator('link[rel=canonical]').getAttribute('href'), `${base}/`)
    assert.equal(new URL(page.url()).search, '')
    checks.push('Keyboard activation keeps dialog UX and updates product metadata, closing restores homepage metadata')

    await photoLink.click()
    await page.locator('.product-dialog[open]').waitFor()
    await page.goBack()
    await page.locator('.product-dialog[open]').waitFor({ state: 'hidden' })
    await page.goForward()
    await page.locator('.product-dialog[open]').waitFor()
    assert.equal(await page.title(), 'Houska se sýrem | Brunch Era')
    await page.getByRole('button', { name: 'Zavřít detail svačiny' }).click()
    const popupPromise = delayed.context.waitForEvent('page')
    await titleLink.click({ modifiers: ['Control'] })
    const popup = await popupPromise
    await popup.locator('.product-dialog[open]').waitFor()
    assert.equal(new URL(popup.url()).search, '?product=12')
    assert.equal(await page.locator('.product-dialog[open]').count(), 0)
    await popup.close()
    checks.push('Back/forward and Ctrl-click preserve normal link and new-tab behavior')

    const failed = await fixture({ fail: true })
    await failed.page.goto(base)
    await failed.page.locator('#loading-retry').waitFor()
    assert(await failed.page.locator('#server-content').isVisible())
    assert(await failed.page.locator('#client-content').isHidden())
    assert.equal(await failed.page.locator('#loading-message').innerText(), 'Nabídku teď nelze načíst.')
    failed.model.fail = false
    await failed.page.locator('#loading-retry').click()
    await failed.page.locator('#client-content .product-card').waitFor()
    assert.equal(await failed.page.locator('#server-content').count(), 0)
    checks.push('API failure retains readable server content and a working retry')

    const noScript = await fixture({ javaScriptEnabled: false })
    await noScript.page.goto(`${base}/?product=12`)
    assert(await noScript.page.locator('#server-content').isVisible())
    assert(await noScript.page.getByText('Čerstvé pečivo se sýrem.', { exact: true }).isVisible())
    assert(await noScript.page.locator('#app-loading').isHidden())
    checks.push('Disabled JavaScript leaves public HTML readable')
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ checks }, null, 2))
  } finally {
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors }, null, 2))
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
