// Mocked local browser checks for order notification recipients. No emails are sent.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/order-recipients-admin-browser')
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
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, locale: 'cs-CZ' })
  const errors = []
  const saves = []
  let settings = { delivery_fee_cents: 4900, free_delivery_from_cents: 100000, order_email: 'orders@example.test', delivery_slots: ['08:00–10:00'], version: 1 }
  let holdNextSave = false
  let saveStarted
  let releaseSave
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/**', async route => {
    const request = route.request()
    const url = new URL(request.url())
    const reply = (json, status = 200) => route.fulfill({ status, json })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/settings') {
      if (request.method() === 'GET') return reply(settings)
      const payload = request.postDataJSON()
      saves.push(payload)
      if (holdNextSave) {
        holdNextSave = false
        await new Promise(resolve => { releaseSave = resolve; saveStarted() })
      }
      if (payload.version !== settings.version) return reply({ error: 'Nastavení mezitím změnil jiný správce.' }, 409)
      settings = { ...payload, version: settings.version + 1 }
      return reply(settings)
    }
    errors.push(`Unexpected API request ${request.method()} ${url.pathname}`)
    return route.abort()
  })
  const mainEmail = () => page.getByLabel('Hlavní e-mail pro příjem objednávek', { exact: true })
  const extraEmails = () => page.getByLabel('Další příjemci objednávek', { exact: true })
  const saveButton = () => page.getByRole('button', { name: 'Uložit nastavení', exact: true })
  const save = async () => {
    const response = page.waitForResponse(item => item.url().endsWith('/api/admin/settings') && item.request().method() === 'PUT')
    await saveButton().click()
    return response
  }
  try {
    await page.goto(`${base}/admin/#settings`)
    await page.getByRole('heading', { name: 'Doprava a nastavení', exact: true }).waitFor()
    assert.equal(await extraEmails().inputValue(), '', 'Older settings without the new field remain editable')
    assert.equal(await mainEmail().inputValue(), 'orders@example.test')
    assert.equal(await mainEmail().getAttribute('type'), 'email')

    await extraEmails().fill('valid@example.test\nnot-an-email')
    await saveButton().click()
    await page.locator('[data-form-error]').getByText('Zkontrolujte e-mailovou adresu „not-an-email“.', { exact: true }).waitFor()
    assert.equal(saves.length, 0, 'Invalid extra addresses are rejected before PUT')
    await extraEmails().fill(Array.from({ length: 20 }, (_, index) => `recipient${index}@example.test`).join('\n'))
    await saveButton().click()
    await page.locator('[data-form-error]').getByText('Objednávky lze posílat nejvýše na 20 různých e-mailů včetně hlavního příjemce.', { exact: true }).waitFor()
    assert.equal(saves.length, 0, 'The main recipient counts toward the limit')
    assert.equal(await page.locator('[data-form-error]').count(), 1, 'Validation replaces the previous error')

    await mainEmail().fill('Orders@Example.Test')
    await extraEmails().fill(' KITCHEN@example.test, delivery@example.test ; orders@EXAMPLE.test\nKITCHEN@example.test\r\n ; delivery@EXAMPLE.test ')
    holdNextSave = true
    const started = new Promise(resolve => { saveStarted = resolve })
    const pending = save()
    await started
    assert.equal(await page.locator('form button[type=submit]').isDisabled(), true)
    assert.equal(await mainEmail().isDisabled(), true)
    assert.equal(await extraEmails().isDisabled(), true)
    assert.equal(await page.locator('[data-form-error]').count(), 0, 'A valid save clears old validation errors')
    await page.locator('form').evaluate(form => form.requestSubmit())
    assert.equal(saves.length, 1, 'An in-flight save cannot be submitted twice')
    releaseSave()
    assert.equal((await pending).status(), 200)
    await page.getByText('Nastavení bylo uloženo.', { exact: true }).waitFor()
    assert.deepEqual(saves[0], { delivery_fee_cents: 4900, free_delivery_from_cents: 100000, order_email: 'orders@example.test', additional_order_emails: ['kitchen@example.test', 'delivery@example.test'], delivery_slots: ['08:00–10:00'], version: 1 })
    await page.waitForFunction(() => document.querySelector('[name=additional_order_emails]')?.value === 'kitchen@example.test\ndelivery@example.test')
    assert.equal(await mainEmail().inputValue(), 'orders@example.test', 'The primary address remains the customer contact')

    const nineteen = Array.from({ length: 19 }, (_, index) => `recipient${index}@example.test`)
    await extraEmails().fill([...nineteen, 'ORDERS@example.test', nineteen[0], nineteen[1]].join('; '))
    assert.equal((await save()).status(), 200)
    await page.waitForFunction(() => document.querySelector('[name=additional_order_emails]')?.value.split('\n').length === 19)
    assert.deepEqual(saves[1].additional_order_emails, nineteen, 'Exactly twenty total unique addresses are accepted after deduplication')

    await extraEmails().fill(' , ; \n')
    assert.equal((await save()).status(), 200)
    await page.waitForFunction(() => document.querySelector('[name=additional_order_emails]')?.value === '')
    assert.deepEqual(saves[2].additional_order_emails, [], 'An empty list explicitly removes extra recipients')

    await mainEmail().fill('new-contact@example.test')
    await extraEmails().fill('draft@example.test\nother@example.test')
    settings = { ...settings, version: 5, additional_order_emails: ['concurrent@example.test'] }
    assert.equal((await save()).status(), 409)
    await page.locator('[data-form-error]').getByText('Nastavení mezitím změnil jiný správce.', { exact: false }).waitFor()
    assert.equal(await mainEmail().inputValue(), 'new-contact@example.test')
    assert.equal(await extraEmails().inputValue(), 'draft@example.test\nother@example.test')
    assert.equal(await extraEmails().isEnabled(), true)
    assert.equal(await saveButton().isEnabled(), true)
    assert.deepEqual(settings.additional_order_emails, ['concurrent@example.test'])
    assert.equal(saves[3].version, 4)
    assert.equal(await page.locator('#notice').isHidden(), true)
    await page.screenshot({ path: path.join(output, 'recipients-desktop.png'), fullPage: true })
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Recipient settings fit ${width}px`)
      await page.screenshot({ path: path.join(output, `recipients-${width}.png`), fullPage: true })
    }
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ result: 'passed', checks: ['legacy missing field', 'native email validation before PUT', '20 total unique limit', 'separators normalization and deduplication', 'main customer contact preserved', 'pending save controls and duplicate prevention', 'exactly 20 recipients accepted', 'empty array clears extras', 'version conflict retains draft', '390px and 320px layouts', 'no browser errors or email requests'], screenshots: output }, null, 2))
  } finally {
    if (releaseSave) releaseSave()
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
