// Run the real public admin UI with controlled API responses, without a database or build.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const output = path.resolve('output/order-dates-browser')
const results = []
const errors = []
const makeOrder = (index, overrides = {}) => ({
  id: `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
  version: 1,
  number: `BR-MATCH-${String(index).padStart(3, '0')}`,
  created_at: '2026-10-24T08:00:00Z',
  delivery_date: '2026-11-02', delivery_slot: '09:00–10:00',
  name: 'Testovací zákazník', email: 'customer@example.test', city: 'Praha',
  status: 'new', payment_status: 'unpaid', payment_method: 'cash_on_delivery', total_cents: 16900,
  ...overrides
})
const orders = [
  ...Array.from({ length: 55 }, (_, index) => makeOrder(index + 1)),
  makeOrder(56, { delivery_date: '2026-11-03' }),
  makeOrder(57, { status: 'confirmed' }),
  makeOrder(58, { number: 'BR-OTHER-058' })
]

async function staticServer() {
  const root = path.resolve('public')
  await fs.access(path.join(root, 'admin/index.html'))
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const contentType = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': contentType })
      response.end(body)
    } catch {
      response.writeHead(404)
      response.end('Not found')
    }
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  return { server, base: `http://127.0.0.1:${server.address().port}` }
}

async function setup(browser, base, options = {}) {
  const context = await browser.newContext({
    viewport: options.viewport || { width: 1280, height: 1000 },
    locale: 'cs-CZ', timezoneId: 'America/Los_Angeles'
  })
  const page = await context.newPage()
  page.setDefaultTimeout(10000)
  await page.clock.setFixedTime(new Date('2026-10-24T22:30:00Z'))
  page.on('pageerror', error => errors.push(error.message))
  const api = { requests: [], failNext: false }
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/auth/me') return json({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/mail') return json([])
    if (url.pathname === '/api/admin/mail/broadcast') return json({ recipient_count: 0 })
    if (url.pathname === '/api/admin/orders/bulk-status') {
      api.bulk = route.request().postDataJSON()
      return json({ updated: api.bulk.orders.length })
    }
    if (url.pathname === '/api/admin/orders') {
      const query = Object.fromEntries(url.searchParams)
      api.requests.push(query)
      if (api.failNext) {
        api.failNext = false
        return json({ error: 'Testovací výpadek načítání objednávek.' }, 503)
      }
      const filtered = orders.filter(order =>
        (!query.delivery_date || order.delivery_date === query.delivery_date) &&
        (!query.status || order.status === query.status) &&
        (!query.q || [order.number, order.email, order.name, order.city].some(value => value.toLowerCase().includes(query.q.toLowerCase())))
      )
      const currentPage = Number(query.page || 1)
      return json({ orders: filtered.slice((currentPage - 1) * 50, currentPage * 50), total: filtered.length, page: currentPage })
    }
    errors.push(`Unexpected API request: ${url.pathname}`)
    return json({ error: 'Unexpected test request' }, 500)
  })
  await page.goto(`${base}/admin/`)
  await page.getByRole('heading', { name: 'Objednávky', exact: true }).waitFor()
  return { context, page, api }
}

async function listAction(test, action, expected) {
  const response = test.page.waitForResponse(value => new URL(value.url()).pathname === '/api/admin/orders')
  await action()
  await response
  await test.page.getByRole('heading', { name: 'Objednávky', exact: true }).waitFor()
  assert.deepEqual(test.api.requests.at(-1), expected)
}

async function assertNoOverflow(page) {
  const dimensions = await page.evaluate(() => ({
    viewport: innerWidth,
    document: document.documentElement.scrollWidth,
    body: document.body.scrollWidth
  }))
  assert(dimensions.document <= dimensions.viewport + 1, `Document overflow: ${JSON.stringify(dimensions)}`)
  assert(dimensions.body <= dimensions.viewport + 1, `Body overflow: ${JSON.stringify(dimensions)}`)
}

async function run(browser, base, name, work, options) {
  const test = await setup(browser, base, options)
  try {
    await work(test)
    results.push({ scenario: name, passed: true })
  } catch (error) {
    results.push({ scenario: name, passed: false, error: error.message })
    await test.page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true })
    throw error
  } finally {
    await test.page.unrouteAll({ behavior: 'ignoreErrors' })
    await test.context.close()
  }
}

async function main() {
  await fs.mkdir(output, { recursive: true })
  const { server, base } = await staticServer()
  let browser
  try {
    browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
    await run(browser, base, 'Combined delivery day, search and status survive pagination, refresh and navigation', async test => {
      const { page, api } = test
      assert.deepEqual(api.requests.at(-1), { page: '1', status: '', q: '', delivery_date: '' })
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-02'), { page: '1', status: '', q: '', delivery_date: '2026-11-02' })
      await page.getByLabel('Vyhledat objednávku', { exact: true }).fill(' MATCH ')
      await page.getByLabel('Stav objednávky', { exact: true }).selectOption('new')
      const expected = { page: '1', status: 'new', q: 'MATCH', delivery_date: '2026-11-02' }
      await listAction(test, () => page.getByRole('button', { name: 'Vyhledat', exact: true }).click(), expected)
      assert.equal(await page.locator('tbody tr').count(), 50)
      assert.match(await page.locator('.pagination').innerText(), /Celkem 55/)
      await listAction(test, () => page.getByRole('button', { name: 'Další →', exact: true }).click(), { ...expected, page: '2' })
      assert.equal(await page.locator('tbody tr').count(), 5)
      await listAction(test, () => page.getByRole('button', { name: 'Obnovit', exact: true }).click(), { ...expected, page: '2' })
      await page.getByRole('link', { name: 'E-maily', exact: true }).click()
      await page.getByRole('heading', { name: 'E-maily', exact: true }).waitFor()
      await listAction(test, () => page.getByRole('link', { name: 'Objednávky', exact: true }).click(), { ...expected, page: '2' })
      assert.equal(await page.getByLabel('Den doručení / odběru', { exact: true }).inputValue(), '2026-11-02')
      assert.equal(await page.getByLabel('Stav objednávky', { exact: true }).inputValue(), 'new')
      assert.equal(await page.getByLabel('Vyhledat objednávku', { exact: true }).inputValue(), 'MATCH')
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-03'), { ...expected, delivery_date: '2026-11-03' })
      assert.equal(await page.locator('tbody tr').count(), 1)
      await page.getByText('BR-MATCH-056', { exact: true }).waitFor()
      await page.screenshot({ path: path.join(output, 'filtered-orders-desktop.png'), fullPage: true })
    })

    await run(browser, base, 'Bulk updates select one current state and use the displayed versions', async test => {
      const { page, api } = test
      await page.getByLabel('Vyhledat objednávku', { exact: true }).fill('MATCH-05')
      await page.getByRole('button', { name: 'Vyhledat', exact: true }).click()
      const checkbox = i => page.getByRole('checkbox', { name: `Vybrat objednávku BR-MATCH-${i}`, exact: true })
      await checkbox('050').check()
      assert.ok(await checkbox('057').isDisabled(), 'another starting status cannot be selected')
      await checkbox('051').check()
      await page.getByLabel('Nový stav').selectOption('confirmed')
      page.once('dialog', prompt => prompt.accept())
      const update = page.waitForResponse(response => response.url().endsWith('/api/admin/orders/bulk-status'))
      await page.getByRole('button', { name: 'Změnit stav vybraných' }).click()
      await update
      assert.deepEqual(api.bulk, {
        orders: [orders[49], orders[50]].map(({ id, version }) => ({ id, version })),
        from_status: 'new', status: 'confirmed', confirm_payments: false
      })
    })

    await run(browser, base, 'Empty delivery day and clearing shortcuts preserve other filters and reset pagination', async test => {
      const { page } = test
      await page.getByLabel('Vyhledat objednávku', { exact: true }).fill('MATCH')
      await page.getByLabel('Stav objednávky', { exact: true }).selectOption('new')
      const expected = { page: '1', status: 'new', q: 'MATCH', delivery_date: '2026-11-02' }
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-02'), expected)
      await listAction(test, () => page.getByRole('button', { name: 'Další →', exact: true }).click(), { ...expected, page: '2' })
      await listAction(test, () => page.getByRole('button', { name: 'Zítra', exact: true }).click(), { ...expected, delivery_date: '2026-10-26' })
      await page.getByText('Žádné objednávky', { exact: true }).waitFor()
      assert.match(await page.locator('.empty').innerText(), /Zkuste změnit hledání nebo filtr\./)
      assert(await page.getByRole('button', { name: 'Další →', exact: true }).isDisabled())
      assert.equal(await page.getByRole('button', { name: 'Zítra', exact: true }).getAttribute('aria-pressed'), 'true')
      await listAction(test, () => page.getByRole('button', { name: 'Všechny dny', exact: true }).click(), { ...expected, delivery_date: '' })
      assert.equal(await page.getByLabel('Den doručení / odběru', { exact: true }).inputValue(), '')
      assert.match(await page.locator('.pagination').innerText(), /Celkem 56/)
      assert.equal(await page.getByRole('button', { name: 'Všechny dny', exact: true }).getAttribute('aria-pressed'), 'true')
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-03'), { ...expected, delivery_date: '2026-11-03' })
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill(''), { ...expected, delivery_date: '' })
    })

    await run(browser, base, 'A failed request displays a recoverable error without losing the delivery day', async test => {
      const { page, api } = test
      api.failNext = true
      const failed = page.waitForResponse(value => new URL(value.url()).pathname === '/api/admin/orders' && value.status() === 503)
      await page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-03')
      await failed
      await page.getByRole('heading', { name: 'Data nelze načíst', exact: true }).waitFor()
      await page.getByRole('alert').filter({ hasText: 'Testovací výpadek načítání objednávek.' }).waitFor()
      await listAction(test, () => page.getByRole('button', { name: 'Obnovit', exact: true }).click(), { page: '1', status: '', q: '', delivery_date: '2026-11-03' })
      assert.equal(await page.getByLabel('Den doručení / odběru', { exact: true }).inputValue(), '2026-11-03')
      assert.equal(await page.locator('tbody tr').count(), 1)
    })

    await run(browser, base, 'Today and tomorrow use Prague calendar dates across midnight, DST and New Year', async test => {
      const { page } = test
      assert.equal(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone), 'America/Los_Angeles')
      const cases = [
        ['2026-03-28T23:30:00Z', '2026-03-29', '2026-03-30'],
        ['2026-10-24T22:30:00Z', '2026-10-25', '2026-10-26'],
        ['2026-10-25T22:30:00Z', '2026-10-25', '2026-10-26'],
        ['2026-12-31T22:59:00Z', '2026-12-31', '2027-01-01'],
        ['2026-12-31T23:01:00Z', '2027-01-01', '2027-01-02']
      ]
      for (const [instant, today, tomorrow] of cases) {
        await page.clock.setFixedTime(new Date(instant))
        await listAction(test, () => page.getByRole('button', { name: 'Dnes', exact: true }).click(), { page: '1', status: '', q: '', delivery_date: today })
        assert.equal(await page.getByLabel('Den doručení / odběru', { exact: true }).inputValue(), today)
        assert.equal(await page.getByRole('button', { name: 'Dnes', exact: true }).getAttribute('aria-pressed'), 'true')
        await listAction(test, () => page.getByRole('button', { name: 'Zítra', exact: true }).click(), { page: '1', status: '', q: '', delivery_date: tomorrow })
        assert.equal(await page.getByLabel('Den doručení / odběru', { exact: true }).inputValue(), tomorrow)
        assert.equal(await page.getByRole('button', { name: 'Zítra', exact: true }).getAttribute('aria-pressed'), 'true')
      }
    })

    await run(browser, base, 'Delivery day controls and empty results remain usable at 390 pixels', async test => {
      const { page } = test
      await listAction(test, () => page.getByLabel('Den doručení / odběru', { exact: true }).fill('2026-11-03'), { page: '1', status: '', q: '', delivery_date: '2026-11-03' })
      await assertNoOverflow(page)
      const controls = page.getByRole('group', { name: 'Rychlá volba dne doručení nebo odběru', exact: true })
      for (const label of ['Dnes', 'Zítra', 'Všechny dny']) {
        const bounds = await controls.getByRole('button', { name: label, exact: true }).boundingBox()
        assert(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 391, `${label} must fit within the viewport`)
      }
      await page.screenshot({ path: path.join(output, 'filtered-orders-mobile.png'), fullPage: true })
      await listAction(test, () => page.getByRole('button', { name: 'Dnes', exact: true }).click(), { page: '1', status: '', q: '', delivery_date: '2026-10-25' })
      await page.getByText('Žádné objednávky', { exact: true }).waitFor()
      await assertNoOverflow(page)
      await page.screenshot({ path: path.join(output, 'empty-orders-mobile.png'), fullPage: true })
    }, { viewport: { width: 390, height: 844 } })

    assert.deepEqual(errors, [], 'No unhandled browser errors or unexpected API calls')
    console.log(`Passed ${results.length} order delivery date browser scenarios`)
  } finally {
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ results, errors }, null, 2))
    if (browser) await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
