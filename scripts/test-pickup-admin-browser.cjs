// Local browser regression for pickup settings and fulfillment-specific administration.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/pickup-admin-browser')
  await fs.mkdir(output, { recursive: true })
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const file = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!file.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }[path.extname(file)] || 'application/octet-stream' })
      response.end(await fs.readFile(file))
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, locale: 'cs-CZ' })
  const errors = [], saves = [], deliverySaves = [], patches = [], bulk = [], confirms = [], checks = []
  let settings = { delivery_fee_cents: 4900, free_delivery_from_cents: 35000, order_email: 'orders@example.test', additional_order_emails: [], delivery_slots: ['07:00–09:00'], version: 1 }
  let pickup = { enabled: false, name: '', street: '', city: '', postal_code: '', instructions: '', parking_free: true, slots: [], version: 1 }
  let failSave = 0, holdSave = false, releaseSave, saveStarted
  const snapshot = { name: 'Původní prodejna', street: 'Původní 12', city: 'Praha', postal_code: '11000', instructions: 'Vchod ze dvora <img src=x onerror=alert(1)>', parking_free: true }
  const common = { version: 1, name: 'Jana Testovací', email: 'jana@example.test', phone: '+420 777 123 456', organization: '', street: 'Jiná 1', city: 'Brno', postal_code: '60200', notes: 'Vyzvedne kolegyně', delivery_date: '2026-11-23', delivery_slot: '08:00–10:00', payment_method: 'cash_on_delivery', payment_status: 'unpaid', status: 'confirmed', subtotal_cents: 3500, delivery_fee_cents: 0, total_cents: 3500, created_at: '2026-11-21T10:00:00Z', items: [{ product_id: 1, name: 'Svačina', quantity: 1, unit_price_cents: 3500, total_cents: 3500 }], history: [{ status: 'new', payment_status: 'unpaid', created_at: '2026-11-21T10:00:00Z' }] }
  let orders = [
    { ...common, id: 'pickup-1', number: 'BR-PICKUP-1', fulfillment_method: 'pickup', pickup_location: snapshot },
    { ...common, id: 'pickup-2', number: 'BR-PICKUP-2', fulfillment_method: 'pickup', pickup_location: { ...snapshot, parking_free: false } },
    { ...common, id: 'delivery-1', number: 'BR-DELIVERY-1', delivery_fee_cents: 4900, total_cents: 8400 }
  ]
  page.on('pageerror', error => errors.push(error.message))
  page.on('dialog', async prompt => { confirms.push(prompt.message()); await prompt.accept() })
  await page.route('**/api/**', async route => {
    const request = route.request(), url = new URL(request.url()), method = request.method()
    const reply = (json, status = 200) => route.fulfill({ json, status })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/settings') {
      if (method === 'GET') return reply(settings)
      const data = request.postDataJSON()
      deliverySaves.push(data)
      settings = { ...data, version: settings.version + 1 }
      return reply(settings)
    }
    if (url.pathname === '/api/admin/pickup-settings') {
      if (method === 'GET') return reply(pickup)
      const data = request.postDataJSON()
      saves.push(data)
      if (holdSave) { holdSave = false; await new Promise(resolve => { releaseSave = resolve; saveStarted() }) }
      if (failSave) { const status = failSave; failSave = 0; return reply({ error: status === 409 ? 'Adresu mezitím změnil jiný správce.' : 'Spojení se nepodařilo.' }, status) }
      if (data.version !== pickup.version) return reply({ error: 'Adresu mezitím změnil jiný správce.' }, 409)
      pickup = { ...data, version: pickup.version + 1 }
      return reply(pickup)
    }
    if (url.pathname === '/api/admin/orders') return reply({ orders, page: 1, total: orders.length })
    if (url.pathname === '/api/admin/orders/bulk-status') {
      const data = request.postDataJSON()
      bulk.push(data)
      orders = orders.map(order => data.orders.some(item => item.id === order.id) ? { ...order, status: data.status, version: order.version + 1 } : order)
      return reply({ updated: data.orders.length })
    }
    const order = orders.find(order => url.pathname === `/api/admin/orders/${order.id}`)
    if (order) {
      if (method === 'PATCH') {
        const data = request.postDataJSON()
        patches.push({ id: order.id, ...data })
        Object.assign(order, data, { version: order.version + 1, history: [...order.history, { status: data.status, payment_status: data.payment_status, created_at: '2026-11-21T10:30:00Z' }] })
      }
      return reply(order)
    }
    errors.push(`Unexpected API request ${method} ${url.pathname}`)
    return route.abort()
  })
  const form = () => page.locator('.pickup-settings')
  const pickupSave = async () => {
    const response = page.waitForResponse(r => r.url().endsWith('/api/admin/pickup-settings') && r.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit osobní odběr', exact: true }).click()
    return response
  }
  const row = number => page.getByRole('row').filter({ hasText: number })
  try {
    await page.goto(`${base}/admin/#settings`)
    await page.getByRole('heading', { name: 'Osobní odběr', exact: true }).waitFor()
    assert.equal(await page.getByLabel('Povolit osobní odběr', { exact: true }).isChecked(), false)
    assert.equal(await page.getByLabel('Parkování zdarma', { exact: true }).isChecked(), true)
    assert.equal((await pickupSave()).status(), 200)
    assert.equal(saves[0].enabled, false)
    assert.deepEqual(saves[0].slots, [])
    checks.push('disabled blank draft and free parking default')

    await page.getByLabel('Povolit osobní odběr', { exact: true }).check()
    await page.getByRole('button', { name: 'Uložit osobní odběr', exact: true }).click()
    assert.equal(saves.length, 1, 'Incomplete enabled pickup never sends PUT')
    await page.getByLabel('Název odběrného místa', { exact: true }).fill('Nová prodejna')
    await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).fill('Nová 20')
    await page.getByLabel('Město odběrného místa', { exact: true }).fill('Praha')
    await page.getByLabel('PSČ odběrného místa', { exact: true }).fill('110 00')
    await page.getByLabel('Pokyny k odběru', { exact: true }).fill('Vchod z ulice')
    await page.getByLabel('Časová okna osobního odběru', { exact: true }).fill('10:00–12:00\n11:00–13:00')
    await page.getByRole('button', { name: 'Uložit osobní odběr', exact: true }).click()
    await form().getByText('Časová okna osobního odběru se nesmějí překrývat.', { exact: true }).waitFor()
    assert.equal(saves.length, 1)
    await page.getByLabel('Časová okna osobního odběru', { exact: true }).fill('14:00 - 16:00\n08:00–10:00')
    checks.push('required address and overlapping slot validation')

    await page.getByLabel('Hlavní e-mail pro příjem objednávek', { exact: true }).fill('office@example.test')
    await page.getByRole('button', { name: 'Uložit nastavení', exact: true }).click()
    await page.getByText('Nastavení bylo uloženo.', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).inputValue(), 'Nová 20')
    assert.equal(saves.length, 1)
    assert(!Object.hasOwn(deliverySaves[0], 'pickup'))
    await page.getByLabel('Hlavní e-mail pro příjem objednávek', { exact: true }).fill('unsaved@example.test')
    holdSave = true
    const started = new Promise(resolve => { saveStarted = resolve })
    const pending = pickupSave()
    await started
    assert.equal(await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).isDisabled(), true)
    assert.equal(await page.getByRole('button', { name: 'Obnovit osobní odběr', exact: true }).isDisabled(), true)
    await form().evaluate(form => form.requestSubmit())
    assert.equal(saves.length, 2)
    releaseSave()
    assert.equal((await pending).status(), 200)
    await page.getByText('Nastavení osobního odběru bylo uloženo.', { exact: true }).waitFor()
    assert.deepEqual(pickup.slots, ['08:00–10:00', '14:00–16:00'])
    assert.equal(pickup.postal_code, '11000')
    assert.equal(await page.getByLabel('Hlavní e-mail pro příjem objednávek', { exact: true }).inputValue(), 'unsaved@example.test')
    checks.push('independent forms preserve drafts and block duplicate saves')

    await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).fill('Po výpadku 30')
    failSave = 500
    assert.equal((await pickupSave()).status(), 500)
    await form().getByText('Spojení se nepodařilo.', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).inputValue(), 'Po výpadku 30')
    assert.equal((await pickupSave()).status(), 200)
    await page.getByText('Nastavení osobního odběru bylo uloženo.', { exact: true }).waitFor()
    pickup = { ...pickup, street: 'Souběžná 40', version: pickup.version + 1 }
    await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).fill('Rozepsaná 50')
    assert.equal((await pickupSave()).status(), 409)
    await form().getByText(/Rozepsané údaje zatím zůstávají/).waitFor()
    assert.equal(await page.getByLabel('Ulice a číslo odběrného místa', { exact: true }).inputValue(), 'Rozepsaná 50')
    await page.getByRole('button', { name: 'Obnovit osobní odběr', exact: true }).click()
    await page.waitForFunction(() => document.querySelector('[name=pickup_street]').value === 'Souběžná 40')
    assert.equal(await page.getByLabel('Hlavní e-mail pro příjem objednávek', { exact: true }).inputValue(), 'unsaved@example.test')
    checks.push('failed save retry and version conflict preserve drafts')
    await page.screenshot({ path: path.join(output, 'settings-desktop.png'), fullPage: true })
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Settings fit ${width}px`)
      await form().scrollIntoViewIfNeeded()
      await page.screenshot({ path: path.join(output, `settings-${width}.png`), fullPage: true })
    }
    checks.push('pickup settings fit 390 and 320 px')

    await page.setViewportSize({ width: 1440, height: 1100 })
    await page.getByRole('link', { name: 'Objednávky', exact: true }).click()
    await row('BR-PICKUP-1').getByText('Osobní odběr', { exact: true }).waitFor()
    await row('BR-DELIVERY-1').getByText('Rozvoz', { exact: true }).waitFor()
    await page.getByRole('checkbox', { name: 'Vybrat objednávku BR-PICKUP-1', exact: true }).check()
    assert.equal(await page.getByRole('checkbox', { name: 'Vybrat objednávku BR-DELIVERY-1', exact: true }).isDisabled(), true)
    await page.getByRole('button', { name: 'Vybrat všechny ve stejném stavu na stránce', exact: true }).click()
    assert.equal(await page.getByRole('checkbox', { name: 'Vybrat objednávku BR-PICKUP-2', exact: true }).isChecked(), true)
    assert.equal(await page.getByLabel('Nový stav', { exact: true }).locator('option[value=out_for_delivery]').innerText(), 'Připraveno k vyzvednutí')
    await page.getByRole('button', { name: 'Změnit stav vybraných', exact: true }).click()
    await page.getByText('Počet aktualizovaných objednávek: 2.', { exact: true }).waitFor()
    assert.deepEqual(bulk[0].orders.map(order => order.id), ['pickup-1', 'pickup-2'])
    assert.equal(bulk[0].status, 'out_for_delivery')
    assert(confirms.at(-1).includes('Připraveno k vyzvednutí'))
    checks.push('bulk pickup handling stays separate from delivery')

    await row('BR-PICKUP-1').getByRole('button', { name: 'Detail', exact: true }).click()
    await page.getByRole('heading', { name: 'BR-PICKUP-1', exact: true }).waitFor()
    const detail = page.getByRole('dialog')
    const text = await detail.innerText()
    assert(text.includes('Původní prodejna') && text.includes('Původní 12'))
    assert(!text.includes('Souběžná 40') && !text.includes('Jiná 1'))
    assert(text.includes('Parkování zdarma') && text.includes('Vchod ze dvora <img'))
    assert.equal(await detail.locator('img').count(), 0, 'Instructions are text, never executable HTML')
    assert(text.includes('při vyzvednutí') && !text.includes('při doručení') && !text.includes('Na cestě'))
    const map = new URL(await detail.getByRole('link', { name: 'Otevřít adresu v mapě ↗', exact: true }).getAttribute('href'))
    assert(map.searchParams.get('query').includes('Původní 12'))
    await page.screenshot({ path: path.join(output, 'pickup-order-desktop.png') })
    await page.emulateMedia({ media: 'print' })
    await page.screenshot({ path: path.join(output, 'pickup-order-print.png'), fullPage: true })
    await page.emulateMedia({ media: 'screen' })
    await detail.getByLabel('Stav', { exact: true }).selectOption('delivered')
    assert.equal(await detail.getByLabel('Platba', { exact: true }).inputValue(), 'paid')
    await detail.getByRole('button', { name: 'Uložit změny', exact: true }).click()
    await detail.getByLabel('Stav', { exact: true }).locator('option[value=delivered]').waitFor({ state: 'attached' })
    assert.equal(patches.at(-1).status, 'delivered')
    assert.equal(patches.at(-1).payment_status, 'paid')
    assert((await detail.innerText()).includes('Vyzvednuto'))
    assert(!(await detail.innerText()).includes('Doručená'))
    checks.push('pickup snapshot, free parking, map, safe instructions, print and collection status')
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await row('BR-PICKUP-2').getByRole('button', { name: 'Detail', exact: true }).click()
    await page.getByRole('heading', { name: 'BR-PICKUP-2', exact: true }).waitFor()
    assert(!(await detail.innerText()).includes('Parkování zdarma'))
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await row('BR-DELIVERY-1').getByRole('button', { name: 'Detail', exact: true }).click()
    await page.getByRole('heading', { name: 'BR-DELIVERY-1', exact: true }).waitFor()
    assert((await detail.innerText()).includes('Jiná 1'))
    assert((await detail.innerText()).includes('při doručení'))
    assert.equal(await detail.getByLabel('Stav', { exact: true }).locator('option[value=out_for_delivery]').innerText(), 'Na cestě')
    checks.push('legacy delivery wording and parking flag remain accurate')
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ result: 'passed', checks, screenshots: output }, null, 2))
  } finally {
    if (releaseSave) releaseSave()
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
