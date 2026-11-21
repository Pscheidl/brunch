// Run against built dist with mocked API responses. No live orders are created.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const product = { id: 1, name: 'Testovací svačina', description: 'Test termínu', category: 'Slané', tag: '', weight: '100 g', price_cents: 12000, vegan: false, ingredients: '', allergens: '', image: '', active: true, stock: 99, version: 1 }
const results = []
const errors = []
const out = 'output/delivery-date-safety-browser'

async function staticServer() {
  const root = path.resolve('dist')
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const type = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.webp': 'image/webp', '.svg': 'image/svg+xml' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': type })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  return { server, base: `http://127.0.0.1:${server.address().port}` }
}

async function setup(browser, base, options = {}) {
  // A visitor in another time zone must still follow the Prague delivery day.
  const context = await browser.newContext({ viewport: { width: 1280, height: 1000 }, timezoneId: 'America/Los_Angeles' })
  const page = await context.newPage()
  page.setDefaultTimeout(10000)
  page.on('pageerror', error => errors.push(error.message))
  await page.clock.setFixedTime(new Date(options.now || '2026-11-22T22:59:59Z'))
  const requests = []
  await page.route('**/api/**', async route => {
    const pathname = new URL(route.request().url()).pathname
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (pathname === '/api/products') return json([product])
    if (pathname === '/api/settings') return json({ delivery_fee_cents: 0, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], online_payment_enabled: false })
    if (pathname === '/api/delivery-options') return json({ dates: [{ value: '2026-11-23', label: 'Pondělí 23. listopadu' }], slots: ['09:00–10:00'] })
    if (pathname === '/api/auth/me') return json({ error: 'Nepřihlášen' }, 401)
    if (pathname === '/api/address-suggestions') return json([])
    if (pathname === '/api/orders') {
      const body = route.request().postDataJSON()
      requests.push(body)
      if (options.lostResponse && requests.length === 1) return route.abort('failed')
      return json({ ...body, id: 'e86f212a-0465-4c22-a2d8-1a6157d98f28', number: 'TEST-ROLLOVER', status: 'new', payment_status: 'unpaid', subtotal_cents: 12000, delivery_fee_cents: 0, total_cents: 12000, items: [] })
    }
    errors.push(`Unexpected request ${pathname}`)
    return json({ error: 'Unexpected' }, 500)
  })
  await page.goto(base)
  await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
  await page.locator('.header-cart').click()
  await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
  const form = page.locator('.checkout-form')
  const calendar = form.getByRole('group', { name: 'Kalendář doručení', exact: true })
  return { page, context, requests, form, calendar }
}

async function review(test) {
  const { page, form } = test
  await form.locator('[name=name]').fill('Jana Testovací')
  await form.locator('[name=tel]').fill('+420 777 123 456')
  await form.locator('[name=email]').fill('test@example.test')
  await form.locator('[name=street-address]').fill('Testovací 12')
  await form.locator('[name=city]').fill('Praha')
  await form.locator('[name=postal-code]').fill('11000')
  await form.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
  await page.getByRole('heading', { name: 'Už jen poslední krok.', exact: true }).waitFor()
}

async function run(browser, base, name, options, work) {
  const test = await setup(browser, base, options)
  try {
    await work(test)
    results.push(name)
  } finally {
    await test.page.unrouteAll({ behavior: 'ignoreErrors' })
    await test.context.close()
  }
}

async function main() {
  await fs.mkdir(out, { recursive: true })
  const { server, base } = await staticServer()
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  try {
    await run(browser, base, 'Stale API dates and manually enabled past buttons cannot select today or the past', { now: '2026-11-24T08:00:00Z' }, async ({ page, calendar }) => {
      const selected = calendar.locator('button[aria-pressed=true]')
      assert.equal(await selected.getAttribute('aria-label'), 'Středa 25. 11. 2026')
      for (const label of ['Pondělí 23. 11. 2026', 'Úterý 24. 11. 2026', 'Sobota 28. 11. 2026']) {
        const button = calendar.getByRole('button', { name: label, exact: true })
        assert(await button.isDisabled())
        await button.evaluate(element => { element.disabled = false; element.click() })
        assert.equal(await selected.getAttribute('aria-label'), 'Středa 25. 11. 2026')
      }
      await calendar.getByRole('button', { name: 'Předchozí měsíc', exact: true }).evaluate(element => { element.disabled = false; element.click() })
      assert.equal(await calendar.locator('strong').innerText(), 'Listopad 2026')
      await page.screenshot({ path: `${out}/past-dates-disabled.png` })
    })

    await run(browser, base, 'Crossing Prague midnight at final submission returns to delivery without creating an order', {}, async test => {
      await review(test)
      await test.page.clock.setFixedTime(new Date('2026-11-22T23:00:01Z'))
      await test.page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await test.page.getByRole('heading', { name: 'Kam to přivezeme?', exact: true }).waitFor()
      assert.equal(test.requests.length, 0)
      assert.equal(await test.calendar.locator('button[aria-pressed=true]').getAttribute('aria-label'), 'Úterý 24. 11. 2026')
      await test.page.locator('.checkout-body').getByText(/Původně zvolený den už není dostupný/).waitFor()
      await test.page.clock.setFixedTime(new Date('2026-11-24T08:00:00Z'))
      await test.page.evaluate(() => window.dispatchEvent(new Event('focus')))
      assert.equal(await test.calendar.locator('button[aria-pressed=true]').getAttribute('aria-label'), 'Středa 25. 11. 2026')
    })

    await run(browser, base, 'A lost successful response retries the original payload and key across midnight', { lostResponse: true }, async test => {
      await review(test)
      await test.page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await test.page.getByRole('alert').filter({ hasText: 'Spojení se nepodařilo' }).waitFor()
      assert.equal(test.requests.length, 1)
      await test.page.clock.setFixedTime(new Date('2026-11-24T08:00:00Z'))
      await test.page.evaluate(() => window.dispatchEvent(new Event('focus')))
      await test.page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await test.page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(test.requests.length, 2)
      assert.deepEqual(test.requests[1], test.requests[0])
      assert.equal(test.requests[1].delivery_date, '2026-11-23')
    })
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ passed: results, errors }, null, 2))
  } finally {
    await fs.writeFile(`${out}/results.json`, JSON.stringify({ passed: results, errors }, null, 2))
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
