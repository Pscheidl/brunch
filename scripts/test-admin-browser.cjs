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
    await page.getByRole('link', { name: 'Zboží a ceny' }).click()
    await page.getByRole('button', { name: '+ Přidat zboží' }).click()
    const name = `Test administrace ${Date.now()}`
    await page.getByLabel('Název', { exact: true }).fill(name)
    await page.getByLabel('Popis', { exact: true }).fill('Ověření administračního formuláře a dostupnosti.')
    await page.getByLabel('Kategorie', { exact: true }).fill('Sladké')
    await page.getByLabel('Cena v Kč', { exact: true }).fill('118,50')
    await page.getByLabel('Hmotnost / objem', { exact: true }).fill('150 g')
    await page.getByLabel('Počet dostupných kusů', { exact: false }).fill('3')
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

    await page.getByRole('link', { name: 'Doprava a nastavení' }).click()
    await page.getByLabel('Cena dopravy v Kč', { exact: true }).waitFor()
    const settingsBefore = await json(context.request, 'get', '/api/admin/settings')
    await page.getByRole('button', { name: 'Uložit nastavení', exact: true }).click()
    await page.getByText('Nastavení bylo uloženo.', { exact: true }).waitFor()
    const settingsAfter = await json(context.request, 'get', '/api/admin/settings')
    assert.equal(settingsBefore.delivery_fee_cents, settingsAfter.delivery_fee_cents)
    assert.equal(settingsBefore.free_delivery_from_cents, settingsAfter.free_delivery_from_cents)
    assert.equal(settingsBefore.order_email, settingsAfter.order_email)

    const delivery = await json(guest.request, 'get', '/api/delivery-options')
    const settings = await json(guest.request, 'get', '/api/settings')
    const fee = settings.free_delivery_from_cents !== null && product.price_cents >= settings.free_delivery_from_cents ? 0 : settings.delivery_fee_cents
    const orderEmail = `admin-ui-${Date.now()}@example.test`
    const order = await json(guest.request, 'post', '/api/orders', {
      idempotency_key: require('node:crypto').randomUUID(), email: orderEmail, name: 'Test rozvozu',
      phone: '+420 777 123 456', street: 'Testovací 12', city: 'Praha', postal_code: '110 00',
      notes: '3. patro, výtah vlevo. Zvoňte Novák.', delivery_date: delivery.dates[0].value,
      delivery_slot: delivery.slots[0], payment_method: 'card_on_delivery',
      items: [{ product_id: product.id, quantity: 1 }], expected_total_cents: product.price_cents + fee
    })
    await page.getByRole('link', { name: 'Objednávky', exact: true }).click()
    await page.getByLabel('Vyhledat objednávku').fill(orderEmail)
    await page.getByRole('button', { name: 'Vyhledat', exact: true }).click()
    await page.getByRole('row').filter({ hasText: order.number }).getByRole('button', { name: 'Detail' }).click()
    await page.getByText('3. patro, výtah vlevo. Zvoňte Novák.', { exact: false }).waitFor()
    const mapLink = page.getByRole('link', { name: 'Otevřít adresu v mapě ↗' })
    assert.ok(decodeURIComponent(await mapLink.getAttribute('href')).includes('Testovací 12'))
    assert.equal(await page.getByRole('link', { name: '+420 777 123 456' }).getAttribute('href'), 'tel:+420777123456')
    await page.screenshot({ path: path.join(output, 'order-desktop.png'), fullPage: true })
    await page.evaluate(() => { window.print = () => { window.__printInvoked = true } })
    await page.getByRole('button', { name: 'Vytisknout rozvozový list' }).click()
    assert.equal(await page.evaluate(() => window.__printInvoked), true)
    await page.emulateMedia({ media: 'print' })
    await page.pdf({ path: path.join(output, 'delivery-slip.pdf'), format: 'A4', printBackground: true })
    await page.emulateMedia({ media: 'screen' })
    for (const status of ['confirmed', 'preparing', 'out_for_delivery', 'delivered']) {
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
    await page.getByRole('link', { name: 'Zákazníci', exact: true }).click()
    await page.getByRole('heading', { name: 'Zákazníci', exact: true }).waitFor()
    await page.getByRole('link', { name: 'E-maily', exact: true }).click()
    await page.getByRole('heading', { name: 'E-maily', exact: true }).waitFor()
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
    console.log(JSON.stringify({ result: 'passed', product: product.id, order: order.number, screenshots: output, checks: ['administrator password login', 'create/edit/archive product', 'upload product photograph', 'price history', 'settings save', 'order search and detail', 'map and phone links', 'print slip', 'delivery and payment states', 'customers and mail', '390px mobile', 'no browser errors'] }, null, 2))
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
