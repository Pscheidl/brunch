// End-to-end pickup checkout against an isolated local server, PostgreSQL and Mailpit.
const { chromium, request } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { adminTestPassword } = require('./admin-test-login.cjs')

const base = process.env.BRUNCH_TEST_URL || 'http://127.0.0.1:3100'
const inbox = process.env.BRUNCH_MAILPIT_URL || 'http://127.0.0.1:18026'
for (const address of [base, inbox]) assert(['127.0.0.1', 'localhost', '[::1]'].includes(new URL(address).hostname), 'Pickup browser tests run against local fixtures only')
const output = path.resolve('output/pickup-flow-browser')

async function capturedMail(address, number) {
  for (let attempt = 0; attempt < 60; attempt++) {
    const list = await fetch(`${inbox}/api/v1/messages?limit=100`).then(response => response.json())
    const message = list.messages.find(message => message.To.some(to => to.Address === address) && message.Subject.includes(number))
    if (message) return fetch(`${inbox}/api/v1/message/${message.ID}`).then(response => response.json())
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  throw new Error('Pickup confirmation was not captured by local Mailpit')
}

async function main() {
  await fs.mkdir(output, { recursive: true })
  const admin = await request.newContext({ baseURL: base, extraHTTPHeaders: { 'X-Brunch-Request': '1' } })
  const browser = await chromium.launch({ headless: true })
  const errors = []
  const passed = []
  let original
  let order
  async function api(method, endpoint, data) {
    const response = await admin.fetch(endpoint, { method, data })
    const text = await response.text()
    assert.equal(response.status(), 200, `${method} ${endpoint}: ${text}`)
    return JSON.parse(text)
  }
  try {
    await api('POST', '/api/admin/auth/login', { email: process.env.TEST_ADMIN_EMAIL || 'admin@example.test', password: adminTestPassword() })
    original = await api('GET', '/api/admin/pickup-settings')
    const location = { enabled: true, name: 'Mysterium Gaudi', street: 'Zátopkova 100/2', city: 'Praha 6', postal_code: '16900', instructions: 'Vchod u parkoviště.', parking_free: true, slots: ['08:00–17:00'], version: original.version }
    const settings = await api('PUT', '/api/admin/pickup-settings', location)
    const products = await api('GET', '/api/products')
    const product = products.find(product => product.active && !product.variants.length && (product.stock === null || product.stock >= Math.max(1, product.minimum_quantity)) && product.price_cents > 0)
    assert(product, 'The fixture needs an available product without variants')
    const quantity = Math.max(1, product.minimum_quantity)
    const expectedTotal = quantity * product.price_cents
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
    const page = await context.newPage()
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(base)
    const card = page.locator('.product-card').filter({ has: page.getByRole('heading', { name: product.name, exact: true }) })
    await card.waitFor()
    assert.equal(await page.evaluate(async () => (await fetch('/api/auth/me')).status), 401, 'Customer context must be anonymous')
    await card.getByRole('button', { name: new RegExp('Přidat .* do košíku') }).click()
    await page.locator('.header-cart').click()
    await page.locator('[name=cart-fulfillment][value=pickup]').check()
    await page.getByRole('button', { name: 'Pokračovat k osobnímu odběru', exact: true }).click()
    const form = page.locator('.checkout-form')
    assert.equal(await form.locator('[name=street-address]:visible').count(), 0, 'Pickup does not ask for a delivery street')
    assert.equal(await form.locator('[name=city]:visible').count(), 0, 'Pickup does not ask for a delivery city')
    assert.equal(await form.locator('[name=postal-code]:visible').count(), 0, 'Pickup does not restrict the customer postcode')
    assert((await form.innerText()).includes(location.street))
    assert((await form.innerText()).includes('Parkování zdarma'))
    const email = `pickup-browser-${Date.now()}@example.test`
    const notes = 'Při vyzvednutí prosím připravit tašku.'
    await form.locator('[name=name]').fill('Jana Odběratelová')
    await form.locator('[name=tel]').fill('+420 777 123 456')
    await form.locator('[name=email]').fill(email)
    await form.locator('[name=notes]').fill(notes)
    await form.locator('[name=checkout-time]').selectOption('08:00–17:00')
    await page.screenshot({ path: path.join(output, 'pickup-contact.png'), fullPage: false })
    await page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    await page.locator('[data-checkout-step=payment]').waitFor({ state: 'visible' })
    const review = page.locator('.cart-dialog')
    assert((await review.innerText()).includes('Hotově při vyzvednutí'))
    assert((await review.innerText()).includes(notes))
    await page.locator('[name=terms-accepted]').check()
    await page.locator('[name=payment-method][value=cash_on_delivery]').check()
    await page.screenshot({ path: path.join(output, 'pickup-review.png'), fullPage: false })
    await page.setViewportSize({ width: 390, height: 844 })
    assert(await review.evaluate(dialog => dialog.getBoundingClientRect().width <= innerWidth), 'Pickup review fits mobile viewport')
    await page.screenshot({ path: path.join(output, 'pickup-review-mobile.png'), fullPage: false })
    await page.setViewportSize({ width: 1440, height: 1000 })
    const pending = page.waitForResponse(response => response.url() === `${base}/api/orders` && response.request().method() === 'POST')
    await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
    const response = await pending
    assert.equal(response.status(), 200, await response.text())
    const payload = response.request().postDataJSON()
    assert.equal(payload.fulfillment_method, 'pickup')
    assert.equal(payload.pickup_version, settings.version)
    assert.equal(payload.expected_total_cents, expectedTotal)
    assert(!payload.street && !payload.city && !payload.postal_code, 'No customer delivery address is sent for pickup')
    order = await response.json()
    assert.equal(order.email, email)
    assert.equal(order.fulfillment_method, 'pickup')
    assert.equal(order.delivery_fee_cents, 0)
    assert.equal(order.subtotal_cents, expectedTotal)
    assert.equal(order.total_cents, expectedTotal)
    assert.equal(order.delivery_slot, '08:00–17:00')
    assert.equal(order.notes, notes)
    assert.equal(order.pickup_location.name, location.name)
    assert.equal(order.pickup_location.street, location.street)
    assert.equal(order.pickup_location.city, location.city)
    assert.equal(order.pickup_location.postal_code, location.postal_code)
    assert.equal(order.pickup_location.parking_free, true)
    await page.getByRole('heading', { name: 'Dobrá pauza je na dohled.', exact: true }).waitFor()
    const confirmation = page.locator('.confirmation-details')
    assert((await confirmation.innerText()).includes(location.street))
    assert((await confirmation.innerText()).includes('Parkování zdarma'))
    assert((await confirmation.innerText()).includes(notes))
    assert((await confirmation.innerText()).includes('Hotově při vyzvednutí'))
    assert.equal(await page.evaluate(async () => (await fetch('/api/auth/me')).status), 401, 'Guest checkout does not create a session')
    passed.push('anonymous pickup without address', 'pickup slot and contact validation', 'real order POST with zero fee and immutable pickup snapshot', 'confirmation address, free parking and customer note')
    const mail = await capturedMail(email, order.number)
    for (const text of [order.number, location.street, location.name, 'Parkování zdarma', 'Hotově při vyzvednutí', notes]) assert(mail.Text.includes(text), `Captured email must include ${text}`)
    passed.push('real pickup confirmation captured by Mailpit')
    await api('PUT', '/api/admin/pickup-settings', { ...settings, street: 'Změněná testovací 9', parking_free: false })
    const stored = await api('GET', `/api/admin/orders/${order.id}`)
    assert.deepEqual(stored.pickup_location, order.pickup_location, 'Changing pickup settings must not change prior order address or parking')
    assert.equal(stored.street, location.street)
    assert.equal(stored.delivery_fee_cents, 0)
    assert((await confirmation.innerText()).includes(location.street))
    assert(!(await confirmation.innerText()).includes('Změněná testovací'))
    await page.screenshot({ path: path.join(output, 'pickup-confirmation.png'), fullPage: false })
    await page.setViewportSize({ width: 390, height: 844 })
    assert(await page.locator('.cart-dialog').evaluate(dialog => dialog.getBoundingClientRect().width <= innerWidth), 'Pickup confirmation fits mobile viewport')
    await page.screenshot({ path: path.join(output, 'pickup-confirmation-mobile.png'), fullPage: false })
    passed.push('desktop and mobile review and confirmation')
    passed.push('admin address edit preserves prior order and confirmation')
    assert.deepEqual(errors, [], 'No browser runtime errors')
    const report = { passed, order: order.number, errors, screenshots: output }
    await fs.writeFile(path.join(output, 'results.json'), `${JSON.stringify(report, null, 2)}\n`)
    console.log(JSON.stringify(report, null, 2))
  } catch (error) {
    await fs.writeFile(path.join(output, 'results.json'), `${JSON.stringify({ passed, errors, error: error.message }, null, 2)}\n`)
    throw error
  } finally {
    if (original) {
      const current = await api('GET', '/api/admin/pickup-settings')
      await api('PUT', '/api/admin/pickup-settings', { ...original, version: current.version })
    }
    await admin.dispose()
    await browser.close()
  }
}
main().catch(error => { console.error(error); process.exit(1) })
