// Isolated browser checks for terms editing and private order provenance. No production requests.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/terms-admin-browser')
  await fs.mkdir(output, { recursive: true })
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const contentType = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': contentType })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
  const errors = []
  const saves = []
  const unsafeText = '<script>window.__termsInjected = true</script>\n<img src=x onerror="window.__termsInjected = true">\n\nBěžný text podmínek.'
  let terms = { version: 1, title: 'Obchodní podmínky', content: unsafeText, updated_at: '2026-11-21T12:00:00Z' }
  const orders = [
    { id: 'new-order', number: 'BR-TEST-1', created_from_ip: '2001:db8::1', terms_version: 2 },
    { id: 'old-order', number: 'BR-TEST-2', created_from_ip: null, terms_version: null }
  ].map(order => ({ ...order, name: 'Testovací zákazník', organization: '', email: 'customer@example.test', phone: '+420 777 123 456',
    street: 'Testovací 1', city: 'Praha', postal_code: '11000', delivery_date: '2026-11-23', delivery_slot: '08:00–10:00',
    status: 'confirmed', payment_status: 'unpaid', payment_method: 'cash_on_delivery', created_at: '2026-11-21T12:00:00Z',
    subtotal_cents: 10000, delivery_fee_cents: 0, total_cents: 10000, items: [], version: 1 }))
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/**', route => {
    const request = route.request()
    const url = new URL(request.url())
    const reply = (json, status = 200) => route.fulfill({ status, json })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/terms') {
      if (request.method() === 'GET') return reply(terms)
      const payload = request.postDataJSON()
      saves.push(payload)
      if (payload.version !== terms.version) return reply({ error: 'Podmínky mezitím změnil jiný správce.' }, 409)
      terms = { ...payload, version: terms.version + 1, updated_at: '2026-11-21T13:00:00Z' }
      return reply(terms)
    }
    if (url.pathname === '/api/admin/orders') return reply({ page: 1, total: orders.length, orders })
    const order = orders.find(item => url.pathname === `/api/admin/orders/${item.id}`)
    if (order) return reply(order)
    errors.push(`Unexpected API request ${request.method()} ${url.pathname}`)
    return route.abort()
  })
  try {
    await page.goto(`${base}/admin/#terms`)
    await page.getByRole('heading', { name: 'Obchodní podmínky', exact: true }).waitFor()
    assert.equal(await page.getByLabel('Název podmínek').getAttribute('maxlength'), '160')
    assert.equal(await page.getByLabel('Znění obchodních podmínek').getAttribute('maxlength'), '100000')
    await page.getByRole('button', { name: 'Náhled', exact: true }).click()
    assert.equal(await page.locator('.terms-text').textContent(), unsafeText)
    assert.equal(await page.locator('.terms-preview script, .terms-preview img').count(), 0)
    assert.equal(await page.evaluate(() => window.__termsInjected), undefined)
    await page.getByRole('button', { name: 'Zpět k úpravám' }).click()

    await page.getByLabel('Název podmínek').fill('  Nové obchodní podmínky  ')
    await page.getByLabel('Znění obchodních podmínek').fill('  Nové znění.\n\nDruhý odstavec.  ')
    await page.getByRole('button', { name: 'Uložit a zveřejnit novou verzi' }).click()
    await page.getByText('Obchodní podmínky byly uloženy jako verze 2.', { exact: true }).waitFor()
    assert.deepEqual(saves[0], { version: 1, title: 'Nové obchodní podmínky', content: 'Nové znění.\n\nDruhý odstavec.' })
    await page.getByText('Aktuální verze: 2', { exact: false }).waitFor()

    await page.getByLabel('Znění obchodních podmínek').fill('   ')
    await page.getByRole('button', { name: 'Uložit a zveřejnit novou verzi' }).click()
    await page.locator('[data-form-error]').getByText('Vyplňte název i znění obchodních podmínek.').waitFor()
    assert.equal(saves.length, 1, 'Whitespace-only text must not be submitted')

    await page.getByLabel('Znění obchodních podmínek').fill('Rozpracované znění, které musí zůstat ve formuláři.')
    terms = { ...terms, version: 3, content: 'Znění upravené druhým správcem.' }
    await page.getByRole('button', { name: 'Uložit a zveřejnit novou verzi' }).click()
    await page.locator('[data-form-error]').getByText('Podmínky mezitím změnil jiný správce.', { exact: false }).waitFor()
    assert.equal(saves[1].version, 2)
    assert.equal(await page.getByLabel('Znění obchodních podmínek').inputValue(), 'Rozpracované znění, které musí zůstat ve formuláři.')
    assert.equal(terms.version, 3, 'A stale editor must not overwrite the latest version')
    await page.getByRole('button', { name: 'Obnovit', exact: true }).click()
    await page.getByText('Aktuální verze: 3', { exact: false }).waitFor()
    assert.equal(await page.getByLabel('Znění obchodních podmínek').inputValue(), 'Znění upravené druhým správcem.')
    await page.screenshot({ path: path.join(output, 'editor-desktop.png'), fullPage: true })
    await page.setViewportSize({ width: 320, height: 844 })
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Terms editor fits 320 px')
    await page.screenshot({ path: path.join(output, 'editor-mobile.png'), fullPage: true })
    await page.getByRole('button', { name: 'Náhled', exact: true }).click()
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Terms preview fits 320 px')
    await page.getByRole('button', { name: 'Zpět k úpravám' }).click()

    await page.getByRole('link', { name: 'Objednávky', exact: true }).click()
    await page.getByRole('row').filter({ hasText: 'BR-TEST-1' }).getByRole('button', { name: 'Detail' }).click()
    await page.getByRole('dialog').getByText('2001:db8::1', { exact: false }).waitFor()
    assert.equal(await page.getByRole('link', { name: 'Obchodní podmínky přijaté při objednání, verze 2' }).getAttribute('href'), '/obchodni-podminky/?version=2')
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await page.getByRole('row').filter({ hasText: 'BR-TEST-2' }).getByRole('button', { name: 'Detail' }).click()
    await page.getByRole('dialog').getByText('U starších objednávek neevidována', { exact: false }).waitFor()
    assert.equal(await page.getByRole('link', { name: /Obchodní podmínky přijaté při objednání/ }).count(), 0)
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ result: 'passed', checks: ['safe text preview', 'field limits', 'trimmed save with version', 'empty text rejection', 'version conflict preserves draft', 'refresh current version', '320px layout', 'private order IP', 'legacy IP fallback', 'accepted terms version link', 'no browser errors'], screenshots: output }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
