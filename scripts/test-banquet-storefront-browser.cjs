// Real WASM storefront against controlled local APIs. No production orders are created.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')
const fixedNow = new Date('2026-11-21T12:00:00Z')
const pickup = { enabled: true, name: 'Testovací pekárna', street: 'Testovací 12', city: 'Praha', postal_code: '16000', instructions: '', parking_free: true, slots: ['08:00–17:00'], version: 1 }
const product = (id, name, banquet, minimum_quantity) => ({ id, name, banquet, minimum_quantity, description: 'Testovací pečivo', category: 'Slané', tag: '', weight: '30 g', ingredients: 'Pečivo', allergens: '1', image: '/images/brunch-spread.webp', active: true, stock: null, variants: [], vegan: false, version: 1, price_cents: 1000 })
const products = [product(1, 'Banketní houska se sýrem', true, 10), product(2, 'Banketní houska s lososem', true, 10), product(3, 'Banketní paštika', true, 10), product(4, 'Běžná houska', false, 0), product(5, 'Koláč s vlastním minimem', false, 5)]
async function main() {
  const root = path.resolve('dist'), output = path.resolve('output/banquet-storefront-browser')
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
  async function fixture({ loseResponse = false, minimum = 30, omitMinimum = false } = {}) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
    const page = await context.newPage()
    await page.clock.setFixedTime(fixedNow)
    const model = { minimum, omitMinimum, attempts: [], orders: new Map(), loseResponse }
    page.on('pageerror', e => errors.push(e.message))
    await context.route('**/api/**', async route => {
      const request = route.request(), url = new URL(request.url()), method = request.method()
      const reply = (json, status = 200) => route.fulfill({ json, status })
      if (url.pathname === '/api/products') return reply(products)
      if (url.pathname === '/api/settings') return reply({ ...(model.omitMinimum ? {} : { banquet_minimum_quantity: model.minimum }), pickup, delivery_fee_cents: 4900, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], online_payment_enabled: false, hero_image: '', hero_image_alt: '' })
      if (url.pathname === '/api/delivery-options') return reply({ dates: [{ value: '2026-11-23', label: '23. 11. 2026' }], slots: ['07:00–09:00'], postal_slots: {} })
      if (url.pathname === '/api/terms') return reply({ version: 1, title: 'Testovací podmínky', content: 'Podmínky místního testu', updated_at: fixedNow.toISOString() })
      if (url.pathname === '/api/auth/me') return reply({ error: 'Nepřihlášen' }, 401)
      if (url.pathname === '/api/orders/mine') return reply([])
      if (url.pathname === '/api/address-suggestions') return reply([])
      if (url.pathname === '/api/orders' && method === 'POST') {
        const payload = request.postDataJSON()
        model.attempts.push(payload)
        const saved = model.orders.get(payload.idempotency_key)
        if (saved) {
          assert.deepEqual(payload, saved.request, 'Saved order retry must stay exact when the shared minimum changes')
          return reply(saved.order)
        }
        const quantity = payload.items.reduce((sum, item) => sum + (products.find(p => p.id === item.product_id).banquet ? item.quantity : 0), 0)
        if (quantity > 0 && quantity < model.minimum) return reply({ error: `Minimální odběr banketního pečiva je ${model.minimum} ks. V košíku je ${quantity} ks.` }, 409)
        for (const item of payload.items) assert(item.quantity >= products.find(p => p.id === item.product_id).minimum_quantity)
        const items = payload.items.map(item => { const p = products.find(p => p.id === item.product_id); return { ...item, name: p.name, unit_price_cents: p.price_cents, total_cents: p.price_cents * item.quantity } })
        const isPickup = payload.fulfillment_method === 'pickup'
        const subtotal = items.reduce((sum, item) => sum + item.total_cents, 0), fee = isPickup ? 0 : 4900
        assert.equal(payload.expected_total_cents, subtotal + fee)
        const order = { ...payload, id: `banquet-order-${model.orders.size + 1}`, number: `TEST-BANQUET-${model.orders.size + 1}`, fulfillment_method: isPickup ? 'pickup' : 'delivery', pickup_location: isPickup ? pickup : null, street: isPickup ? pickup.street : payload.street, city: isPickup ? pickup.city : payload.city, postal_code: isPickup ? pickup.postal_code : payload.postal_code, status: 'new', payment_status: 'unpaid', subtotal_cents: subtotal, delivery_fee_cents: fee, total_cents: subtotal + fee, items, created_at: fixedNow.toISOString() }
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
  const add = async (page, id) => {
    const p = products.find(product => product.id === id)
    const label = p.minimum_quantity > 1 ? `Přidat ${p.minimum_quantity} ks do košíku: ${p.name}` : `Přidat ${p.name} do košíku`
    await page.locator('.product-card').getByRole('button', { name: label, exact: true }).click()
  }
  const cart = page => page.locator('[data-checkout-step=cart]')
  const next = page => page.getByRole('button', { name: /Pokračovat k (doručení|osobnímu odběru)/ })
  const quantity = (page, id) => cart(page).getByRole('spinbutton', { name: `Počet kusů: ${products.find(p => p.id === id).name}`, exact: true })
  async function setQuantity(page, id, count) { const input = quantity(page, id); await input.fill(String(count)); await input.blur() }
  async function assertProgress(page, count, minimum) {
    const banner = page.locator('.banquet-minimum')
    await banner.waitFor()
    await page.waitForFunction(({ count, minimum }) => { const banner = document.querySelector('.banquet-minimum'); return banner?.dataset.banquetQuantity === String(count) && banner?.dataset.banquetMinimum === String(minimum) }, { count, minimum })
    assert.equal(await banner.getAttribute('data-banquet-quantity'), String(count))
    assert.equal(await banner.getAttribute('data-banquet-minimum'), String(minimum))
    assert((await banner.innerText()).includes(`Banketní pečivo: ${count} z ${minimum} ks`))
  }
  async function addMixed(page) { for (const id of [1, 2, 3]) await add(page, id); await page.locator('.header-cart').click() }
  async function contact(page, isPickup = false) {
    await next(page).click()
    if (!isPickup) {
      await page.locator('[name=street-address]').fill('Zákaznická 15')
      await page.locator('[name=city]').fill('Praha')
      await page.locator('[name=postal-code]').fill('11000')
    }
    await page.locator('[name=name]').fill('Jana Testovací')
    await page.locator('[name=tel]').fill('+420 777 123 456')
    await page.locator('[name=email]').fill('jana@example.test')
    await page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
  }
  async function submit(page) {
    await page.locator('[name=terms-accepted]').check()
    await page.locator('[name=payment-method][value=cash_on_delivery]').check()
    await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
  }
  const confirmed = page => page.getByRole('heading', { name: 'Dobrá pauza je na dohled.', exact: true }).waitFor()
  try {
    const basic = await fixture({ omitMinimum: true })
    assert.equal(await basic.page.locator('.product-card .product-meta').getByText('Banketní pečivo', { exact: true }).count(), 3)
    await add(basic.page, 1)
    await add(basic.page, 4)
    await basic.page.locator('.header-cart').click()
    await assertProgress(basic.page, 10, 30)
    assert(await next(basic.page).isDisabled())
    await setQuantity(basic.page, 4, 99)
    await assertProgress(basic.page, 10, 30)
    assert(await next(basic.page).isDisabled(), 'Ordinary goods must not make up the banquet minimum')
    await setQuantity(basic.page, 1, 9)
    assert.equal(await quantity(basic.page, 1).inputValue(), '10', 'Individual banquet minimum is retained')
    await setQuantity(basic.page, 1, 29)
    await assertProgress(basic.page, 29, 30)
    assert(await next(basic.page).isDisabled())
    await setQuantity(basic.page, 1, 30)
    await assertProgress(basic.page, 30, 30)
    assert(await next(basic.page).isEnabled())
    await cart(basic.page).getByRole('button', { name: `Odebrat celou položku: ${products[0].name}`, exact: true }).click()
    assert.equal(await basic.page.locator('.banquet-minimum').count(), 0)
    assert(await next(basic.page).isEnabled(), 'Normal goods alone have no shared banquet minimum')
    checks.push('default 30, marked products, individual minimum, ordinary quantities excluded, 29/30 boundary and removing last banquet item')

    const normal = await fixture()
    await add(normal.page, 5)
    await normal.page.locator('.header-cart').click()
    assert.equal(await quantity(normal.page, 5).inputValue(), '5')
    assert.equal(await normal.page.locator('.banquet-minimum').count(), 0)
    assert(await next(normal.page).isEnabled())
    checks.push('ordinary product can retain its own minimum without a shared banquet requirement')

    const mixed = await fixture()
    await addMixed(mixed.page)
    await assertProgress(mixed.page, 30, 30)
    assert((await mixed.page.locator('.banquet-minimum').innerText()).includes('Společné minimum je splněno.'))
    await contact(mixed.page)
    await submit(mixed.page)
    await confirmed(mixed.page)
    assert.deepEqual(mixed.model.attempts[0].items.map(item => [item.product_id, item.quantity]), [[1, 10], [2, 10], [3, 10]])
    assert.equal(mixed.model.orders.size, 1)
    checks.push('10 cheese rolls + 10 salmon rolls + 10 pâté satisfy 30 and submit as delivery')

    const collect = await fixture()
    await add(collect.page, 1)
    await collect.page.locator('.header-cart').click()
    await collect.page.locator('[name=cart-fulfillment][value=pickup]').check()
    assert(await next(collect.page).isDisabled(), 'Pickup also requires the banquet minimum')
    for (const width of [390, 320]) {
      await collect.page.setViewportSize({ width, height: 844 })
      assert(await collect.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth))
      assert(await collect.page.locator('.checkout-body').evaluate(node => node.scrollWidth <= node.clientWidth))
      await collect.page.locator('.checkout-body').evaluate(node => { node.scrollTop = 0 })
      await collect.page.screenshot({ path: path.join(output, `banquet-cart-${width}.png`) })
    }
    await collect.page.setViewportSize({ width: 1440, height: 1000 })
    await collect.page.getByRole('button', { name: 'Zavřít košík', exact: true }).click()
    await add(collect.page, 2)
    await add(collect.page, 3)
    await collect.page.locator('.header-cart').click()
    await assertProgress(collect.page, 30, 30)
    await contact(collect.page, true)
    await submit(collect.page)
    await confirmed(collect.page)
    assert.equal(collect.model.attempts[0].fulfillment_method, 'pickup')
    assert.equal(collect.model.attempts[0].expected_total_cents, 30000)
    checks.push('pickup enforces same total and submits mixed quantities at zero delivery fee')
    checks.push('minimum guidance and cart fit 320 and 390 px')

    const restored = await fixture()
    await addMixed(restored.page)
    restored.model.minimum = 40
    await restored.page.reload()
    await restored.page.locator('.product-card').first().waitFor()
    await restored.page.locator('.header-cart').click()
    await assertProgress(restored.page, 30, 40)
    assert(await next(restored.page).isDisabled())
    await setQuantity(restored.page, 1, 20)
    await assertProgress(restored.page, 40, 40)
    assert(await next(restored.page).isEnabled())
    checks.push('restored cart adopts changed shared minimum and remains editable')

    const conflict = await fixture()
    await addMixed(conflict.page)
    await contact(conflict.page)
    conflict.model.minimum = 40
    await submit(conflict.page)
    await assertProgress(conflict.page, 30, 40)
    assert.equal(conflict.model.orders.size, 0)
    await conflict.page.locator('.checkout-stepper button').first().click()
    await setQuantity(conflict.page, 1, 20)
    await contact(conflict.page)
    await submit(conflict.page)
    await confirmed(conflict.page)
    assert.equal(conflict.model.attempts.length, 2)
    assert.notEqual(conflict.model.attempts[0].idempotency_key, conflict.model.attempts[1].idempotency_key)
    checks.push('changed shared minimum at submission refreshes settings and requires review before another attempt')

    const lost = await fixture({ loseResponse: true })
    await addMixed(lost.page)
    await contact(lost.page)
    await submit(lost.page)
    await lost.page.locator('[data-checkout-step=payment] [role=alert]').waitFor()
    assert.equal(lost.model.orders.size, 1)
    lost.model.minimum = 50
    await lost.page.reload()
    await lost.page.locator('.product-card').first().waitFor()
    await lost.page.locator('.header-cart').click()
    await lost.page.locator('[name=terms-accepted]').waitFor()
    await lost.page.locator('.checkout-stepper button').first().click()
    await setQuantity(lost.page, 1, 11)
    await contact(lost.page)
    await submit(lost.page)
    await cart(lost.page).waitFor({ state: 'visible' })
    assert.equal(lost.model.attempts.length, 1, 'Pending retry does not permit a changed order below the new minimum')
    await setQuantity(lost.page, 1, 10)
    await contact(lost.page)
    await submit(lost.page)
    await confirmed(lost.page)
    assert.equal(lost.model.attempts.length, 2)
    assert.deepEqual(lost.model.attempts[1], lost.model.attempts[0])
    assert.equal(lost.model.orders.size, 1)
    checks.push('lost-response retry retains exact accepted order after shared minimum is raised and rejects changed under-minimum content')
    assert.deepEqual(errors, [])
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ result: 'passed', checks, errors }, null, 2) + '\n')
    console.log(JSON.stringify({ result: 'passed', checks, errors, screenshots: output }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
