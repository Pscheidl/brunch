// Real WASM checkout tests with local controlled APIs. Never submits production orders.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')
const fixedNow = new Date('2026-11-21T12:00:00Z')
const initialPickup = { enabled: true, name: 'Prodejna u pekárny', street: 'Původní 12', city: 'Praha', postal_code: '16000', instructions: 'Vchod ze dvora', parking_free: true, slots: ['08:00–10:00', '14:00–16:00'], version: 1 }
const products = [{ id: 1, name: 'Svačina', description: 'Testovací svačina', category: 'Slané', tag: '', weight: '150 g', ingredients: 'Pečivo, sýr', allergens: '1, 7', image: '/images/brunch-spread.webp', active: true, stock: null, minimum_quantity: 0, variants: [], vegan: false, version: 1, price_cents: 3500 }]
async function main() {
  const root = path.resolve('dist'), output = path.resolve('output/pickup-storefront-browser')
  await fs.mkdir(output, { recursive: true })
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const file = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!file.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.webp': 'image/webp' }[path.extname(file)] || 'application/octet-stream' })
      response.end(await fs.readFile(file))
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true })
  const errors = [], checks = []
  async function fixture({ enabled = true, loseResponse = false, customerEmail = null } = {}) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
    const page = await context.newPage()
    await page.clock.setFixedTime(fixedNow)
    const model = { pickup: { ...initialPickup, enabled }, date: '2026-11-23', attempts: [], orders: new Map(), loseResponse }
    page.on('pageerror', e => errors.push(e.message))
    await context.route('**/api/**', async route => {
      const request = route.request(), url = new URL(request.url()), method = request.method()
      const reply = (json, status = 200) => route.fulfill({ json, status })
      if (url.pathname === '/api/products') return reply(products)
      if (url.pathname === '/api/settings') return reply({ pickup: model.pickup, delivery_fee_cents: 4900, free_delivery_from_cents: 35000, delivery_postal_codes: ['11000'], online_payment_enabled: false, hero_image: '', hero_image_alt: '' })
      if (url.pathname === '/api/delivery-options') return reply({ dates: [{ value: model.date, label: model.date }], slots: ['07:00–09:00'], postal_slots: {} })
      if (url.pathname === '/api/terms') return reply({ version: 1, title: 'Testovací podmínky', content: 'Podmínky místního testu', updated_at: fixedNow.toISOString() })
      if (url.pathname === '/api/auth/me') return customerEmail ? reply({ user: { id: 'customer', email: customerEmail, is_admin: false } }) : reply({ error: 'Nepřihlášen' }, 401)
      if (url.pathname === '/api/orders/mine') return reply([])
      if (url.pathname === '/api/address-suggestions') return reply([])
      if (url.pathname === '/api/orders' && method === 'POST') {
        const payload = request.postDataJSON()
        model.attempts.push(payload)
        const saved = model.orders.get(payload.idempotency_key)
        if (saved) {
          assert.deepEqual(payload, saved.request, 'Retry payload must remain exact after config changes')
          return reply(saved.order)
        }
        const pickup = payload.fulfillment_method === 'pickup'
        if (pickup && (!model.pickup.enabled || payload.pickup_version !== model.pickup.version)) return reply({ error: 'Odběrné místo se změnilo. Zkontrolujte aktuální adresu a čas.' }, 409)
        const total = pickup ? 3500 : 8400
        assert.equal(payload.expected_total_cents, total)
        if (pickup) {
          assert.equal(payload.street, '')
          assert.equal(payload.city, '')
          assert.equal(payload.postal_code, '')
          assert(model.pickup.slots.includes(payload.delivery_slot))
        }
        const order = { ...payload, id: `test-order-${model.orders.size + 1}`, number: `TEST-PICKUP-${model.orders.size + 1}`, fulfillment_method: pickup ? 'pickup' : 'delivery', pickup_location: pickup ? { ...model.pickup } : null, street: pickup ? model.pickup.street : payload.street, city: pickup ? model.pickup.city : payload.city, postal_code: pickup ? model.pickup.postal_code : payload.postal_code, status: 'new', payment_status: 'unpaid', subtotal_cents: 3500, delivery_fee_cents: pickup ? 0 : 4900, total_cents: total, items: [{ product_id: 1, name: 'Svačina', quantity: 1, unit_price_cents: 3500, total_cents: 3500 }], created_at: fixedNow.toISOString() }
        model.orders.set(payload.idempotency_key, { request: payload, order })
        if (model.loseResponse) { model.loseResponse = false; return route.abort('connectionreset') }
        return reply(order)
      }
      errors.push(`Unexpected ${method} ${url.pathname}`)
      return reply({ error: 'Unexpected request' }, 500)
    })
    await page.goto(base)
    await page.locator('.product-card').first().waitFor()
    return { page, model, context }
  }
  const choose = (page, value, prefix = 'checkout') => page.locator(`[name=${prefix}-fulfillment][value=${value}]`).check()
  const openCart = async page => {
    await page.locator('.product-card').getByRole('button', { name: 'Přidat Svačina do košíku', exact: true }).click()
    await page.locator('.header-cart').click()
  }
  const next = page => page.getByRole('button', { name: /Pokračovat k (doručení|osobnímu odběru)/ }).click()
  const contact = async page => {
    await page.locator('[name=name]').fill('Jana Testovací')
    await page.locator('[name=tel]').fill('+420 777 123 456')
    await page.locator('[name=email]').fill('jana@example.test')
  }
  const review = page => page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
  const submit = async page => {
    await page.locator('[name=terms-accepted]').check()
    await page.locator('[name=payment-method][value=cash_on_delivery]').check()
    await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
  }
  const waitConfirmation = page => page.getByRole('heading', { name: 'Dobrá pauza je na dohled.', exact: true }).waitFor()
  try {
    const { page, model } = await fixture({ customerEmail: 'owner@example.test' })
    assert((await page.locator('.pickup-teaser').innerText()).includes('Parkování zdarma'))
    await openCart(page)
    await next(page)
    await page.locator('[name=street-address]').fill('Zákaznická 15')
    await page.locator('[name=city]').fill('Praha')
    await page.locator('[name=postal-code]').fill('11000')
    await contact(page)
    await choose(page, 'pickup')
    assert.equal(await page.locator('.checkout-form [name=street-address]').count(), 0)
    assert.equal(await page.locator('.checkout-form [name=postal-code]').count(), 0)
    assert.deepEqual(await page.locator('[name=checkout-time] option').allTextContents(), initialPickup.slots)
    await page.locator('[name=checkout-time]').selectOption('14:00–16:00')
    assert((await page.locator('.checkout-form .pickup-address').innerText()).includes('Původní 12'))
    await page.reload()
    await page.locator('.product-card').first().waitFor()
    await page.locator('.header-cart').click()
    assert.equal(await page.locator('[name=checkout-fulfillment][value=pickup]').isChecked(), true)
    assert.equal(await page.locator('[name=checkout-time]').inputValue(), '14:00–16:00')
    assert.equal(await page.locator('[name=email]').inputValue(), 'jana@example.test', 'Signed-in email must not overwrite a manually entered draft email')
    await choose(page, 'delivery')
    assert.equal(await page.locator('[name=street-address]').inputValue(), 'Zákaznická 15')
    assert.equal(await page.locator('[name=postal-code]').inputValue(), '11000')
    assert.deepEqual(await page.locator('[name=checkout-time] option').allTextContents(), ['07:00–09:00'])
    await review(page)
    assert((await page.locator('[data-checkout-step=payment] .cart-totals').innerText()).includes('84'))
    await page.getByRole('button', { name: 'Zpět k převzetí', exact: true }).click()
    await choose(page, 'pickup')
    await review(page)
    assert((await page.locator('[data-checkout-step=payment] .cart-totals').innerText()).includes('Zdarma'))
    await submit(page)
    await waitConfirmation(page)
    assert.equal(model.attempts[0].pickup_version, 1)
    assert.equal(model.attempts[0].fulfillment_method, 'pickup')
    assert.equal(model.attempts[0].expected_total_cents, 3500)
    assert((await page.locator('.confirmation-details').innerText()).includes('Parkování zdarma'))
    assert((await page.locator('.confirmation-details').innerText()).includes('Hotově při vyzvednutí'))
    checks.push('selection, zero fee, pickup slots, draft reload and preserved delivery address and manually entered email')
    checks.push('pickup request version and snapshot confirmation without customer address')

    const changed = await fixture()
    await openCart(changed.page)
    await choose(changed.page, 'pickup', 'cart')
    await next(changed.page)
    await contact(changed.page)
    for (const width of [390, 320]) {
      await changed.page.setViewportSize({ width, height: 844 })
      assert(await changed.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth))
      await changed.page.locator('.checkout-body').evaluate(node => { node.scrollTop = 0 })
      await changed.page.screenshot({ path: path.join(output, `pickup-checkout-${width}.png`) })
    }
    await changed.page.setViewportSize({ width: 1440, height: 1000 })
    await review(changed.page)
    changed.model.pickup = { ...initialPickup, street: 'Nová 44', version: 2, slots: ['11:00–12:00'] }
    await submit(changed.page)
    await changed.page.locator('[name=checkout-time]').waitFor()
    await changed.page.waitForFunction(() => document.querySelector('.checkout-form .pickup-address')?.textContent.includes('Nová 44'))
    assert.equal(changed.model.orders.size, 0)
    assert.equal(changed.model.attempts.length, 1)
    assert.equal(await changed.page.locator('[name=checkout-time]').inputValue(), '11:00–12:00')
    await review(changed.page)
    assert.equal(await changed.page.locator('[name=terms-accepted]').isChecked(), false)
    await submit(changed.page)
    await waitConfirmation(changed.page)
    assert.equal(changed.model.attempts[1].pickup_version, 2)
    assert.notEqual(changed.model.attempts[1].idempotency_key, changed.model.attempts[0].idempotency_key)
    assert((await changed.page.locator('.confirmation-details').innerText()).includes('Nová 44'))
    checks.push('409 address change returns to review with current address and slots')
    checks.push('pickup contact form fits 320 and 390 px')

    const lost = await fixture({ loseResponse: true })
    await openCart(lost.page)
    await choose(lost.page, 'pickup', 'cart')
    await next(lost.page)
    await contact(lost.page)
    await review(lost.page)
    await submit(lost.page)
    await lost.page.locator('[data-checkout-step=payment] [role=alert]').waitFor()
    assert.equal(lost.model.orders.size, 1)
    lost.model.pickup = { ...initialPickup, street: 'Přestěhovaná 80', enabled: false, version: 2 }
    lost.model.date = '2026-11-24'
    await lost.page.clock.setFixedTime(new Date('2026-11-23T10:00:00Z'))
    await lost.page.reload()
    await lost.page.locator('.product-card').first().waitFor()
    await lost.page.locator('.header-cart').click()
    await lost.page.locator('[name=terms-accepted]').waitFor()
    await submit(lost.page)
    await waitConfirmation(lost.page)
    assert.equal(lost.model.attempts.length, 2)
    assert.deepEqual(lost.model.attempts[1], lost.model.attempts[0])
    assert.equal(lost.model.orders.size, 1)
    assert((await lost.page.locator('.confirmation-details').innerText()).includes('Původní 12'))
    assert(!(await lost.page.locator('.confirmation-details').innerText()).includes('Přestěhovaná 80'))
    checks.push('lost response replay retains date, key, version and snapshot across midnight after pickup disabled')

    const disabled = await fixture({ enabled: false })
    assert.equal(await disabled.page.locator('.pickup-teaser').count(), 0)
    await openCart(disabled.page)
    assert.equal(await disabled.page.locator('[name=cart-fulfillment][value=pickup]').count(), 0)
    await next(disabled.page)
    assert.equal(await disabled.page.locator('[name=street-address]').count(), 1)
    checks.push('disabled pickup hidden for new orders and delivery remains available')
    assert.deepEqual(errors, [])
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ result: 'passed', checks, errors }, null, 2)+'\n')
    console.log(JSON.stringify({ result: 'passed', checks, errors, screenshots: output }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
