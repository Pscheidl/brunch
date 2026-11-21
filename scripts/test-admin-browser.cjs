// Browser smoke test against a local server with a database administrator. Requires Playwright.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { adminTestPassword, loginAdministrator } = require('./admin-test-login.cjs')

async function main() {
  const base = process.env.TEST_BASE_URL || 'http://127.0.0.1:3100'
  const adminEmail = process.env.TEST_ADMIN_EMAIL || 'admin@example.test'
  const adminPassword = adminTestPassword()
  if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(base).hostname)) throw new Error('Run this smoke test against a local server only')
  const output = path.resolve('output/admin-browser')
  await fs.mkdir(output, { recursive: true })
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const context = await browser.newContext({ viewport: { width: 1440, height: 1050 }, locale: 'cs-CZ' })
  const guest = await browser.newContext()
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  page.on('dialog', prompt => prompt.accept())
  const json = async (request, method, endpoint, data) => {
    const response = await request[method](`${base}${endpoint}`, { data, headers: { 'X-Brunch-Request': '1' } })
    assert.equal(response.status(), 200, `${method} ${endpoint}: ${await response.text()}`)
    return response.json()
  }
  let product
  try {
    await loginAdministrator(page, base, adminEmail, adminPassword)
    const categoryName = `Kategorie testu ${Date.now()}`
    await page.getByRole('link', { name: 'Kategorie', exact: true }).click()
    await page.getByRole('button', { name: '+ Přidat kategorii' }).click()
    await page.getByLabel('Název kategorie', { exact: true }).fill(categoryName)
    await page.getByRole('button', { name: 'Uložit kategorii' }).click()
    await page.getByRole('cell', { name: categoryName, exact: true }).waitFor()
    await page.getByRole('link', { name: 'Zboží a ceny' }).click()
    await page.getByRole('button', { name: '+ Přidat zboží' }).click()
    const name = `Test administrace ${Date.now()}`
    await page.getByLabel('Název', { exact: true }).fill(name)
    await page.getByLabel('Popis', { exact: true }).fill('Ověření administračního formuláře a dostupnosti.')
    await page.getByLabel('Kategorie', { exact: true }).fill(categoryName)
    await page.getByLabel('Cena v Kč', { exact: true }).fill('118,50')
    await page.getByLabel('Hmotnost / objem', { exact: true }).fill('150 g')
    await page.getByLabel('Minimální odběr v kusech', { exact: true }).fill('2')
    await page.getByLabel('Počet dostupných kusů', { exact: false }).fill('12')
    await page.getByLabel('Složení', { exact: true }).fill('Oves, ovoce')
    await page.getByLabel('Alergeny', { exact: true }).fill('Lepek')
    await page.getByLabel('Vegan', { exact: true }).check()
    await page.getByLabel('Nahrát fotografii', { exact: true }).setInputFiles(path.resolve('public/images/brunch-spread.webp'))
    await page.getByText('Fotografie je nahraná.', { exact: false }).waitFor()
    assert.ok((await page.getByLabel('Obrázek', { exact: true }).inputValue()).startsWith('/api/images/'))
    const created = page.waitForResponse(response => response.url().endsWith('/api/admin/products') && response.request().method() === 'POST')
    await page.getByRole('button', { name: 'Uložit zboží' }).click()
    product = await (await created).json()
    assert.equal(product.price_cents, 11850)
    assert.equal(product.minimum_quantity, 2)
    assert.equal((await guest.request.get(`${base}${product.image}`)).status(), 200)
    let row = page.getByRole('row').filter({ hasText: name })
    await row.getByRole('button', { name: 'Upravit', exact: true }).click()
    await page.getByLabel('Cena v Kč', { exact: true }).fill('125')
    const edited = page.waitForResponse(response => response.url().endsWith(`/api/admin/products/${product.id}`) && response.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit zboží' }).click()
    product = await (await edited).json()
    assert.equal(product.price_cents, 12500)
    await row.getByRole('button', { name: 'Ceny', exact: true }).click()
    await page.getByRole('dialog').getByRole('cell', { name: /118,50/ }).waitFor()
    await page.getByRole('dialog').getByRole('cell', { name: /125,00/ }).waitFor()
    await page.getByRole('button', { name: 'Zavřít okno' }).click()

    await page.getByRole('link', { name: 'Kategorie', exact: true }).click()
    await page.getByRole('row').filter({ hasText: categoryName }).getByRole('button', { name: 'Přejmenovat' }).click()
    await page.getByLabel('Název kategorie', { exact: true }).fill(`${categoryName} upravená`)
    await page.getByRole('button', { name: 'Uložit kategorii' }).click()
    await page.getByRole('cell', { name: `${categoryName} upravená`, exact: true }).waitFor()
    assert.equal((await json(context.request, 'get', '/api/admin/products')).find(item => item.id === product.id).category, `${categoryName} upravená`)
    await page.getByRole('row').filter({ hasText: `${categoryName} upravená` }).getByRole('button', { name: 'Odstranit' }).click()
    await page.getByText('Kategorie byla odstraněna.', { exact: true }).waitFor()
    const retained = (await json(context.request, 'get', '/api/admin/products')).find(item => item.id === product.id)
    assert.equal(retained.category, '')
    for (const key of ['name', 'image', 'price_cents', 'minimum_quantity', 'stock', 'active']) assert.equal(retained[key], product[key], `Category removal preserves ${key}`)

    await page.getByRole('link', { name: 'Doprava a nastavení' }).click()
    await page.getByLabel('Cena dopravy v Kč', { exact: true }).waitFor()
    const settingsBefore = await json(context.request, 'get', '/api/admin/settings')
    await page.getByLabel('Výchozí rozvozová okna', { exact: true }).fill('08:00–10:00\n10:00–12:00')
    await page.getByRole('button', { name: 'Uložit nastavení', exact: true }).click()
    await page.getByText('Nastavení bylo uloženo.', { exact: true }).waitFor()
    const settingsAfter = await json(context.request, 'get', '/api/admin/settings')
    assert.equal(settingsBefore.delivery_fee_cents, settingsAfter.delivery_fee_cents)
    assert.equal(settingsBefore.free_delivery_from_cents, settingsAfter.free_delivery_from_cents)
    assert.equal(settingsBefore.order_email, settingsAfter.order_email)
    assert.deepEqual(settingsAfter.delivery_slots, ['08:00–10:00', '10:00–12:00'])
    await page.getByLabel('Výchozí rozvozová okna', { exact: true }).fill(settingsBefore.delivery_slots.join('\n'))
    const restoredSettings = page.waitForResponse(response => response.url().endsWith('/api/admin/settings') && response.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit nastavení', exact: true }).click()
    assert.equal((await restoredSettings).status(), 200)

    const delivery = await json(guest.request, 'get', '/api/delivery-options')
    const settings = await json(guest.request, 'get', '/api/settings')
    const fee = settings.free_delivery_from_cents !== null && product.price_cents * 2 >= settings.free_delivery_from_cents ? 0 : settings.delivery_fee_cents
    const orderEmail = `admin-ui-${Date.now()}@example.test`
    const orderInput = {
      idempotency_key: require('node:crypto').randomUUID(), email: orderEmail, name: 'Test rozvozu',
      phone: '+420 777 123 456', street: 'Testovací 12', city: 'Praha', postal_code: '110 00',
      notes: '3. patro, výtah vlevo. Zvoňte Novák.', delivery_date: delivery.dates[0].value,
      delivery_slot: delivery.slots[0], payment_method: 'card_on_delivery',
      items: [{ product_id: product.id, quantity: 2 }], expected_total_cents: product.price_cents * 2 + fee
    }
    const order = await json(guest.request, 'post', '/api/orders', orderInput)
    await page.getByRole('link', { name: 'Objednávky', exact: true }).click()
    await page.getByLabel('Vyhledat objednávku').fill(orderEmail)
    await page.getByRole('button', { name: 'Vyhledat', exact: true }).click()
    await page.getByRole('row').filter({ hasText: order.number }).getByRole('button', { name: 'Detail' }).click()
    await page.getByText('3. patro, výtah vlevo. Zvoňte Novák.', { exact: false }).waitFor()
    const mapLink = page.getByRole('link', { name: 'Otevřít adresu v mapě ↗' })
    assert.ok(decodeURIComponent(await mapLink.getAttribute('href')).includes('Testovací 12'))
    assert.equal(await page.getByRole('link', { name: '+420 777 123 456' }).getAttribute('href'), 'tel:+420777123456')
    await page.screenshot({ path: path.join(output, 'order-desktop.png'), fullPage: true })
    const supplier = await json(context.request, 'get', '/api/admin/invoice-supplier')
    if (!supplier.name) await json(context.request, 'put', '/api/admin/invoice-supplier', {
      ...supplier, name: 'Testovací dodavatel', address: 'Praha', company_id: '12345678'
    })
    await page.getByRole('button', { name: 'Faktura', exact: true }).click()
    await page.getByRole('button', { name: 'Vystavit fakturu' }).waitFor()
    if (supplier.vat_payer) {
      for (const field of await page.getByRole('spinbutton').all()) await field.fill('12')
    }
    await page.getByRole('button', { name: 'Vystavit fakturu' }).click()
    await page.getByRole('heading', { name: /Faktura FA-/ }).waitFor()
    await page.evaluate(() => { window.print = () => { window.__printInvoked = true } })
    await page.getByRole('button', { name: 'Vytisknout / uložit PDF' }).click()
    assert.equal(await page.evaluate(() => window.__printInvoked), true)
    await page.emulateMedia({ media: 'print' })
    await page.pdf({ path: path.join(output, 'invoice.pdf'), format: 'A4', printBackground: true })
    await page.emulateMedia({ media: 'screen' })
    await page.getByRole('button', { name: 'Zpět k objednávce' }).click()
    assert.equal(await page.getByLabel('Stav', { exact: true }).locator('option[value=preparing]').count(), 0)
    for (const status of ['confirmed', 'out_for_delivery', 'delivered']) {
      await page.getByLabel('Stav', { exact: true }).selectOption(status)
      if (status === 'delivered') assert.equal(await page.getByLabel('Platba', { exact: true }).inputValue(), 'paid')
      const saved = page.waitForResponse(response => response.url().endsWith(`/api/admin/orders/${order.id}`) && response.request().method() === 'PATCH')
      await page.getByRole('button', { name: 'Uložit změny' }).click()
      const response = await saved
      assert.equal(response.status(), 200, await response.text())
      const savedOrder = await response.json()
      await page.getByLabel('Stav', { exact: true }).waitFor()
      await page.waitForFunction(version => document.querySelector('#editor .dialog-body')?.dataset.orderVersion === String(version), savedOrder.version)
    }
    assert.equal(await page.getByRole('button', { name: 'Uložit změny' }).isDisabled(), true)
    await page.getByRole('button', { name: 'Zavřít okno' }).click()
    const selectedOrders = []
    for (let i = 0; i < 3; i++) selectedOrders.push(await json(guest.request, 'post', '/api/orders', {
      ...orderInput, idempotency_key: require('node:crypto').randomUUID()
    }))
    const otherStatus = selectedOrders.pop()
    await json(context.request, 'patch', `/api/admin/orders/${otherStatus.id}`, { status: 'confirmed', payment_status: 'unpaid', version: otherStatus.version })
    await page.getByRole('button', { name: 'Obnovit', exact: true }).click()
    await page.getByLabel(`Vybrat objednávku ${selectedOrders[0].number}`, { exact: true }).check()
    assert.equal(await page.getByLabel(`Vybrat objednávku ${otherStatus.number}`, { exact: true }).isDisabled(), true)
    await page.getByRole('button', { name: 'Vybrat všechny ve stejném stavu na stránce' }).click()
    for (const selected of selectedOrders) assert.equal(await page.getByLabel(`Vybrat objednávku ${selected.number}`, { exact: true }).isChecked(), true)
    await page.getByLabel('Nový stav', { exact: true }).selectOption('confirmed')
    const bulkResult = page.waitForResponse(response => response.url().endsWith('/api/admin/orders/bulk-status') && response.request().method() === 'POST')
    await page.getByRole('button', { name: 'Změnit stav vybraných' }).click()
    assert.equal((await bulkResult).status(), 200)
    for (const selected of selectedOrders) assert.equal((await json(context.request, 'get', `/api/admin/orders/${selected.id}`)).status, 'confirmed')
    await page.getByRole('link', { name: 'Zákazníci', exact: true }).click()
    await page.getByRole('heading', { name: 'Zákazníci', exact: true }).waitFor()
    await page.getByRole('link', { name: 'E-maily', exact: true }).click()
    await page.getByRole('heading', { name: 'E-maily', exact: true }).waitFor()
    const broadcastRequests = []
    let failBroadcast = false
    await page.route('**/api/admin/mail/broadcast', async route => {
      if (route.request().method() !== 'POST') return route.continue()
      broadcastRequests.push(route.request().postDataJSON())
      await route.fulfill(failBroadcast ? { status: 503, json: { error: 'Testovací výpadek spojení.' } } : { json: { recipient_count: 1 } })
    })
    await page.getByRole('button', { name: 'Napsat hromadný e-mail' }).click()
    await page.getByLabel('Předmět', { exact: true }).fill('Test potvrzení rozesílky')
    await page.getByRole('button', { name: 'Zrušit', exact: true }).click()
    assert.equal(broadcastRequests.length, 0)
    await page.getByRole('button', { name: 'Napsat hromadný e-mail' }).click()
    await page.getByLabel('Předmět', { exact: true }).fill('Test potvrzení rozesílky')
    await page.getByLabel('Text e-mailu', { exact: true }).fill('Testovací text')
    await page.getByRole('button', { name: 'Odeslat všem zákazníkům' }).click()
    await page.getByText('Zpráva byla zařazena k odeslání.', { exact: false }).waitFor()
    assert.equal(broadcastRequests.length, 1)
    assert.equal(broadcastRequests[0].subject, 'Test potvrzení rozesílky')
    assert.equal(broadcastRequests[0].body, 'Testovací text')
    assert.match(broadcastRequests[0].idempotency_key, /^[0-9a-f-]{36}$/)
    await page.getByRole('button', { name: 'Napsat hromadný e-mail' }).click()
    await page.getByLabel('Předmět', { exact: true }).fill('Test opakování po výpadku')
    await page.getByLabel('Text e-mailu', { exact: true }).fill('Původní text')
    failBroadcast = true
    const tryBroadcast = async () => {
      const response = page.waitForResponse(item => item.url().endsWith('/api/admin/mail/broadcast') && item.request().method() === 'POST')
      await page.getByRole('button', { name: 'Odeslat všem zákazníkům' }).click()
      await response
    }
    await tryBroadcast()
    await page.getByRole('dialog').getByRole('alert').waitFor()
    await tryBroadcast()
    assert.equal(broadcastRequests[1].idempotency_key, broadcastRequests[2].idempotency_key, 'Retry of the same message retains its key')
    await page.getByLabel('Text e-mailu', { exact: true }).fill('Upravený text')
    await tryBroadcast()
    assert.notEqual(broadcastRequests[2].idempotency_key, broadcastRequests[3].idempotency_key, 'Changed message gets a new request key')
    failBroadcast = false
    await tryBroadcast()
    await page.getByText('Zpráva byla zařazena k odeslání.', { exact: false }).waitFor()
    assert.equal(broadcastRequests[3].idempotency_key, broadcastRequests[4].idempotency_key)
    await page.unroute('**/api/admin/mail/broadcast')
    await page.getByRole('link', { name: 'Zboží a ceny' }).click()
    await page.setViewportSize({ width: 390, height: 844 })
    await page.getByRole('heading', { name: 'Zboží a ceny', exact: true }).waitFor()
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1), 'Mobile page must not overflow horizontally')
    await page.screenshot({ path: path.join(output, 'products-mobile.png'), fullPage: true })
    row = page.getByRole('row').filter({ hasText: name })
    await row.getByRole('button', { name: 'Upravit', exact: true }).click()
    await page.getByLabel('Zobrazovat v nabídce', { exact: true }).uncheck()
    await page.screenshot({ path: path.join(output, 'product-editor-mobile.png'), fullPage: true })
    const archived = page.waitForResponse(response => response.url().endsWith(`/api/admin/products/${product.id}`) && response.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit zboží' }).click()
    assert.equal((await (await archived).json()).active, false)
    const publicProducts = await json(guest.request, 'get', '/api/products')
    assert.ok(!publicProducts.some(item => item.id === product.id))
    assert.deepEqual(errors, [], 'No uncaught browser errors')
    console.log(JSON.stringify({ result: 'passed', product: product.id, order: order.number, screenshots: output, checks: ['administrator password login', 'create/edit/archive product with minimum quantity', 'create/rename/delete occupied category while retaining product', 'upload product photograph', 'price history', 'delivery hours and settings save', 'order search and detail', 'map and phone links', 'issue and print invoice', 'delivery and payment states', 'bulk change with same-state selection', 'customers and mail', 'broadcast retry and edited-message keys', '390px mobile', 'no browser errors'] }, null, 2))
  } catch (error) {
    await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true }).catch(() => {})
    console.error('Browser page errors:', errors)
    throw error
  } finally {
    // Retain the test order for inspection, hide the test product from the public catalog.
    if (product?.id) {
      try {
        const products = await json(context.request, 'get', '/api/admin/products')
        const current = products.find(item => item.id === product.id)
        if (current?.active) {
          const { id, ...payload } = current
          await json(context.request, 'put', `/api/admin/products/${id}`, { ...payload, active: false })
        }
      } catch (error) { console.error(`Test product cleanup: ${error.message}`) }
    }
    await browser.close()
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
