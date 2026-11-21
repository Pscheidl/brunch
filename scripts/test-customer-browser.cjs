const assert = require('node:assert/strict')
const fs = require('node:fs')
const { chromium } = require('playwright')
const base = process.env.BRUNCH_TEST_URL || 'http://127.0.0.1:3100'
assert(['localhost', '127.0.0.1'].includes(new URL(base).hostname), 'Customer browser tests only run against localhost')
const out = 'output/customer-browser'
fs.mkdirSync(out, { recursive: true })
const orderId = '11111111-1111-4111-8111-111111111111'
const order = { id: orderId, number: 'BR-TEST-1', status: 'confirmed', payment_method: 'cash_on_delivery', email: 'customer@example.test', name: 'Test Customer', organization: 'Test firma', phone: '+420777123456', street: 'Testovací 12', postal_code: '11000', city: 'Praha', notes: 'Recepce', delivery_date: '2026-11-23', delivery_slot: '07:00–09:00', total_cents: 16800, items: [{ product_id: 1, name: 'Sendvič', quantity: 1, total_cents: 11900 }] }
const invoice = { number: 'FA-TEST-1', order_number: order.number, issue_date: '2026-11-21', supply_date: order.delivery_date, due_date: order.delivery_date, supplier: { name: 'Test pekárna', address: 'Pekárenská 1', company_id: '12345678', tax_id: 'CZ12345678', vat_payer: true }, customer: { name: '<script>unsafe</script>', address: 'Testovací 12', company_id: '87654321' }, lines: [{ name: 'Sendvič', quantity: 1, unit_price_cents: 11900, total_cents: 11900, vat_rate: 1200, net_cents: 10625, vat_cents: 1275 }, { name: 'Doprava', quantity: 1, unit_price_cents: 4900, total_cents: 4900, vat_rate: 2100, net_cents: 4050, vat_cents: 850 }], total_cents: 16800, payment_method: 'cash_on_delivery', payment_status: 'unpaid' }
;(async () => {
  const browser = await chromium.launch({ headless: true })
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
    const errors = []
    let addresses = []
    let profile = null
    let billing = null
    let issued = false
    let requests = []
    let authorized = true
    let failRequest = true
    let failDelete = true
    const submitted = []
    await context.route('**/api/**', async route => {
      const request = route.request()
      const path = new URL(request.url()).pathname
      const method = request.method()
      const data = request.postDataJSON()
      let value = null
      let status = 200
      if (!authorized) { value = { error: 'Přihlášení vypršelo.' }; status = 401 }
      else if (path === '/api/auth/me') {
        value = authorized ? { user: { id: 'customer', email: 'customer@example.test' } } : { error: 'Nejprve se přihlaste.' }
        status = authorized ? 200 : 401
      } else if (path === '/api/customer/orders') value = { items: [order], active: [order], total: 1, page: 1 }
      else if (path === '/api/customer/addresses' && method === 'GET') value = addresses
      else if (path === '/api/customer/addresses' && method === 'POST') {
        value = { ...data, id: '22222222-2222-4222-8222-222222222222', version: 1 }
        addresses.push(value)
      } else if (path.startsWith('/api/customer/addresses/') && method === 'DELETE') {
        if (failDelete) { failDelete = false; status = 503; value = { error: 'Adresu se nepodařilo odstranit. Zkuste to znovu.' } }
        else { addresses = []; value = { deleted: true } }
      }
      else if (path === '/api/customer/billing') {
        if (method === 'PUT') profile = { ...data, version: (profile?.version || 0) + 1 }
        value = profile
      } else if (path === `/api/customer/orders/${orderId}/invoice`) value = issued ? invoice : null
      else if (path === `/api/customer/orders/${orderId}/billing`) {
        if (method === 'PUT') billing = { ...data, version: 1 }
        value = billing
      } else if (path === `/api/customer/orders/${orderId}/requests`) {
        if (method === 'POST') {
          submitted.push(data)
          if (failRequest) { failRequest = false; status = 500; value = { error: 'Zkuste žádost odeslat znovu.' } }
          else { value = { ...data, id: 'req1', created_at: '2026-11-21T12:00:00Z', status: 'pending', response: '' }; requests.push(value) }
        } else value = requests
      } else { status = 404; value = { error: `Unexpected ${method} ${path}` } }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(value) })
    })
    await context.route(`${base}/?checkout=1`, route => route.fulfill({ contentType: 'text/html', body: '<p>Checkout handoff</p>' }))
    const page = await context.newPage()
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(`${base}/customer/`)
    await page.getByRole('heading', { name: 'Můj účet', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Detail a možnosti', exact: true }).waitFor()
    assert.equal(await page.locator('[aria-current=step]').first().innerText(), 'Termín potvrzen')
    await page.getByRole('button', { name: 'Přidat místo', exact: true }).click()
    await page.getByRole('textbox', { name: 'Název místa', exact: true }).fill('Kancelář')
    await page.getByRole('textbox', { name: 'Jméno příjemce', exact: true }).fill('Test Customer')
    await page.getByRole('textbox', { name: 'Telefon pro kurýra', exact: true }).fill('+420777123456')
    await page.getByRole('textbox', { name: 'Ulice a číslo domu', exact: true }).fill('Testovací 12')
    await page.getByRole('textbox', { name: 'Město', exact: true }).fill('Praha')
    await page.getByRole('textbox', { name: 'PSČ', exact: true }).fill('11000')
    await page.getByRole('button', { name: 'Uložit místo', exact: true }).click()
    await page.getByRole('heading', { name: 'Kancelář', exact: true }).waitFor()
    assert.equal(addresses.length, 1)
    await page.getByRole('button', { name: 'Použít pro nákup', exact: true }).click()
    await page.waitForURL(`${base}/?checkout=1`)
    assert.equal(await page.evaluate(() => JSON.parse(sessionStorage.getItem('brunch-selected-address')).label), 'Kancelář', 'Checkout receives the selected address')
    await page.goto(`${base}/customer/`)
    await page.getByRole('button', { name: 'Odstranit', exact: true }).click()
    await page.getByRole('button', { name: 'Ponechat', exact: true }).click()
    assert.equal(addresses.length, 1)
    await page.getByRole('button', { name: 'Odstranit', exact: true }).click()
    await page.getByRole('button', { name: 'Odstranit místo', exact: true }).click()
    await page.getByRole('alert').filter({ hasText: 'Adresu se nepodařilo odstranit' }).waitFor()
    assert(await page.getByRole('button', { name: 'Odstranit místo', exact: true }).isEnabled(), 'Failed deletion remains retryable')
    assert.equal(addresses.length, 1, 'Failed deletion keeps the address')
    await page.getByRole('button', { name: 'Odstranit místo', exact: true }).click()
    await page.getByText('Uložte si kancelář nebo jiné místo, kam objednáváte.', { exact: true }).waitFor()
    const profileForm = page.locator('#billing-form')
    await profileForm.getByRole('textbox', { name: 'Odběratel', exact: true }).fill('Test firma')
    await profileForm.getByRole('textbox', { name: 'Fakturační adresa', exact: true }).fill('Testovací 12, Praha')
    await profileForm.getByRole('button', { name: 'Uložit fakturační údaje', exact: true }).click()
    await page.getByText('Výchozí fakturační údaje byly uloženy.', { exact: true }).waitFor()
    assert.equal(profile.name, 'Test firma')
    await page.getByRole('button', { name: 'Detail a možnosti', exact: true }).click()
    await page.getByRole('button', { name: 'Změnit doručení nebo údaje', exact: true }).click()
    await page.getByRole('textbox', { name: 'Vaše žádost', exact: true }).fill('Prosím předat na recepci.')
    await page.getByRole('button', { name: 'Odeslat žádost', exact: true }).click()
    await page.getByRole('alert').filter({ hasText: 'Zkuste žádost' }).waitFor()
    await page.getByRole('button', { name: 'Odeslat žádost', exact: true }).click()
    await page.getByText('Změna · Čeká na vyřízení', { exact: true }).waitFor()
    assert.equal(submitted.length, 2)
    assert.equal(submitted[0].idempotency_key, submitted[1].idempotency_key, 'Retry preserves request identity')
    assert.equal(await page.getByRole('button', { name: 'Požádat o zrušení', exact: true }).count(), 0)
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await page.screenshot({ path: `${out}/account-desktop.png`, fullPage: true })
    await page.setViewportSize({ width: 390, height: 844 })
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Customer page fits mobile')
    await page.screenshot({ path: `${out}/account-mobile.png`, fullPage: true })
    await page.setViewportSize({ width: 320, height: 740 })
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Customer page fits 320 px')
    await page.screenshot({ path: `${out}/account-320.png`, fullPage: true })
    issued = true
    await page.goto(`${base}/customer/?invoice=${orderId}`)
    await page.getByRole('heading', { name: 'Faktura FA-TEST-1', exact: true }).waitFor()
    assert.equal(await page.locator('.invoice script').count(), 0)
    await page.getByText('<script>unsafe</script>', { exact: true }).waitFor()
    await page.getByRole('heading', { name: 'Rekapitulace DPH', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Vytisknout / uložit PDF', exact: true }).waitFor()
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Invoice page fits 320 px')
    await page.screenshot({ path: `${out}/invoice-mobile.png`, fullPage: true })
    await page.setViewportSize({ width: 794, height: 1123 })
    await page.emulateMedia({ media: 'print' })
    assert.equal(await page.locator('.site-head').isVisible(), false, 'Print hides navigation')
    assert.equal(await page.getByRole('button', { name: 'Vytisknout / uložit PDF', exact: true }).isVisible(), false, 'Print hides actions')
    assert(await page.locator('.invoice').isVisible(), 'Invoice remains visible in print')
    await page.screenshot({ path: `${out}/invoice-print.png`, fullPage: true })
    await page.emulateMedia({ media: 'screen' })
    issued = false
    requests = []
    order.fulfillment_method = 'pickup'
    order.pickup_location = { name: 'Pekárna při objednání', street: 'Odběrná 88', postal_code: '46001', city: 'Liberec', parking_free: true, instructions: 'Vchod ze dvora.\nZazvoňte u výdeje.' }
    order.street = order.pickup_location.street
    order.postal_code = order.pickup_location.postal_code
    order.city = order.pickup_location.city
    order.status = 'out_for_delivery'
    order.payment_method = 'card_on_delivery'
    await page.goto(`${base}/customer/`)
    await page.locator('#active-list .status').getByText('Připraveno k vyzvednutí', { exact: true }).waitFor()
    assert.equal(await page.locator('#active-list [aria-current=step]').innerText(), 'Připraveno k vyzvednutí')
    await page.locator('#active-list').getByText('Parkování zdarma', { exact: true }).waitFor()
    assert(!(await page.locator('#active-list').innerText()).includes('Na cestě'), 'Pickup timeline never implies a courier')
    assert((await page.locator('#active-list a').getAttribute('href')).includes(encodeURIComponent('Odběrná 88')))
    await page.getByRole('button', { name: 'Detail a možnosti', exact: true }).click()
    const details = page.getByRole('dialog')
    await details.getByText('Způsob úhrady: Kartou při vyzvednutí.', { exact: true }).waitFor()
    await details.getByRole('button', { name: 'Změnit odběr nebo údaje', exact: true }).waitFor()
    await details.locator('summary').filter({ hasText: 'Fakturační údaje této objednávky' }).click()
    assert.equal(await details.getByLabel('Fakturační adresa', { exact: true }).inputValue(), '', 'Pickup location is not used as customer billing address')
    await details.getByLabel('Fakturační adresa', { exact: true }).fill('Adresa zákazníka 15, Praha')
    await details.getByRole('button', { name: 'Uložit fakturační údaje', exact: true }).click()
    await details.locator('summary').filter({ hasText: 'Fakturační údaje této objednávky' }).click()
    assert.equal(await details.getByLabel('Fakturační adresa', { exact: true }).inputValue(), 'Adresa zákazníka 15, Praha')
    assert.equal(billing.address, 'Adresa zákazníka 15, Praha')
    await details.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Pickup account fits ${width} px`)
      await page.screenshot({ path: `${out}/pickup-account-${width}.png`, fullPage: true })
    }
    order.status = 'delivered'
    issued = true
    invoice.fulfillment_method = 'pickup'
    invoice.pickup_location = structuredClone(order.pickup_location)
    invoice.payment_method = 'card_on_delivery'
    invoice.customer.address = billing.address
    invoice.lines = invoice.lines.filter(line => line.name !== 'Doprava')
    invoice.total_cents = 11900
    await page.goto(`${base}/customer/?invoice=${orderId}`)
    await page.getByText('Způsob úhrady: Kartou při vyzvednutí.', { exact: true }).waitFor()
    await page.locator('.invoice-parties').getByText('Adresa zákazníka 15, Praha', { exact: true }).waitFor()
    await page.locator('.fulfillment-details').getByText('Odběrná 88, 46001 Liberec', { exact: true }).waitFor()
    assert.equal(await page.locator('.invoice-parties').getByText('Odběrná 88', { exact: false }).count(), 0)
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Pickup invoice fits 320 px')
    await page.setViewportSize({ width: 794, height: 1123 })
    await page.emulateMedia({ media: 'print' })
    await page.screenshot({ path: `${out}/pickup-invoice-print.png`, fullPage: true })
    await page.emulateMedia({ media: 'screen' })
    await page.goto(`${base}/customer/`)
    await page.getByRole('button', { name: 'Detail a možnosti', exact: true }).waitFor()
    authorized = false
    await page.getByRole('button', { name: 'Detail a možnosti', exact: true }).click()
    await page.getByRole('heading', { name: 'Přihlaste se ke svému účtu', exact: true }).waitFor()
    assert.equal(await page.locator('#detail').getAttribute('open'), null, 'Expired session closes private dialog')
    assert.equal(await page.locator('#detail').innerText(), '', 'Expired session clears private dialog content')
    assert(!(await page.locator('body').innerText()).includes('BR-TEST-1'), 'Expired session clears order data')
    assert(!(await page.locator('body').innerText()).includes('customer@example.test'), 'Expired session clears account identity')
    assert.equal(await page.locator('#billing-form').count(), 0, 'Expired session clears billing form')
    authorized = true
    await page.goto(`${base}/customer/`)
    await page.getByRole('button', { name: 'Obnovit stav', exact: true }).waitFor()
    authorized = false
    await page.getByRole('button', { name: 'Obnovit stav', exact: true }).click()
    await page.getByRole('heading', { name: 'Přihlaste se ke svému účtu', exact: true }).waitFor()
    assert.equal(await page.locator('#active-list').count(), 0, 'Unauthorized refresh clears active orders')
    await page.goto(`${base}/customer/`)
    await page.getByRole('heading', { name: 'Přihlaste se ke svému účtu', exact: true }).waitFor()
    assert.equal(await page.getByRole('link', { name: 'Přihlásit se e-mailem', exact: true }).getAttribute('href'), '/?signin=customer')
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ passed: ['active order timeline', 'address create/use/delete and failed-delete retry', 'saved billing', 'change request retry', 'pending request prevents duplicates', '390px and 320px layout', 'invoice print view', 'session expiry clears private DOM', 'invoice VAT and safe rendering', 'login handoff', 'pickup status, address, parking and map', 'pickup customer billing stays distinct', 'pickup invoice print and mobile'], screenshots: out }, null, 2))
  } finally { await browser.close() }
})().catch(error => { console.error(error); process.exit(1) })
