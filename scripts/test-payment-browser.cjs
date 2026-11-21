// Payment UX regression against the actual built storefront with mocked API responses.
// Build dist first. Requires Playwright. No payment provider or database is contacted.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const orderId = 'f542a8e9-61c2-4dfd-b389-0660605a191e'
const product = { id: 1, name: 'Testovací svačina', description: 'Test platby', category: 'Slané', tag: '', weight: '100 g', price_cents: 12000, vegan: false, ingredients: 'Mouka', allergens: 'Lepek', image: '', active: true, stock: 99, version: 1 }
const expiresAt = '2026-10-24T09:00:00Z'
const order = { id: orderId, number: 'TEST-001', status: 'awaiting_payment', payment_method: 'card', payment_status: 'unpaid', payment_expires_at: expiresAt, subtotal_cents: 12000, delivery_fee_cents: 0, total_cents: 12000, email: 'test@example.test', street: 'Testovací 1', city: 'Praha', postal_code: '11000', delivery_date: '2026-10-26', delivery_slot: '09:00–10:00', created_at: '2026-10-24T08:00:00Z', version: 1, items: [{ product_id: 1, name: product.name, quantity: 1, unit_price_cents: 12000, total_cents: 12000 }] }
const pending = { status: 'awaiting_payment', payment_status: 'unpaid', checkout_url: null, payment_state: 'pending', expires_at: expiresAt }
const ready = { ...pending, payment_state: 'ready', checkout_url: 'https://checkout.teya.com/mock-session' }
const paid = { status: 'new', payment_status: 'paid', checkout_url: null, payment_state: 'paid' }
const receipt = { transaction_id: 'teya-transaction-test-001', session_id: 'teya-session-test-001', amount_cents: 12000, currency: 'CZK', verified_at: '2026-10-24T08:45:00Z' }
const key = '4a131ccd-d786-416d-8859-242bf967e5de'
const recovery = { order, idempotency_key: key }
const results = []
const errors = []

async function staticServer() {
  const root = path.resolve('dist')
  await fs.access(path.join(root, 'index.html'))
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const type = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.webp': 'image/webp', '.svg': 'image/svg+xml' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': type })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  return { server, base: `http://127.0.0.1:${server.address().port}` }
}
async function setup(browser, base, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 1000 } })
  const page = await context.newPage()
  await page.clock.setFixedTime(new Date('2026-10-24T08:00:00Z'))
  page.setDefaultTimeout(15000)
  page.on('pageerror', error => errors.push(error.message))
  const requests = { orders: [], payments: [] }
  await page.route('**/api/**', async route => {
    const request = route.request()
    const pathname = new URL(request.url()).pathname
    const json = (value, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(value) })
    if (pathname === '/api/address-suggestions') {
      const text = request.postDataJSON().text
      return text === 'Pod sídlištěm 9'
        ? json([{ label: 'Pod sídlištěm 1800/9, Kobylisy, 18200 Praha 8', street: 'Pod sídlištěm 1800/9, Kobylisy', city: 'Praha 8', postal_code: '18200' }])
        : json({ error: 'Registry temporarily unavailable' }, 503)
    }
    if (pathname === '/api/products') return json([product])
    if (pathname === '/api/settings') return json({ delivery_fee_cents: 0, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], ...(options.enabled === false ? {} : { online_payment_enabled: true }) })
    if (pathname === '/api/delivery-options') return json({ dates: [{ value: '2026-10-26', label: 'Pondělí 26. října' }], slots: ['09:00–10:00'], postal_slots: options.postalSlots || {} })
    if (pathname === '/api/auth/me') return options.admin ? json({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } }) : json({ error: 'Nepřihlášen' }, 401)
    if (pathname === '/api/orders') {
      requests.orders.push(request.postDataJSON())
      if (options.lostCreateResponse && requests.orders.length === 1) return route.abort('failed')
      return json(order)
    }
    if (pathname === `/api/orders/${orderId}/payment`) {
      const payload = request.postDataJSON()
      requests.payments.push(payload)
      assert.equal(payload.idempotency_key, requests.orders.at(-1)?.idempotency_key || key)
      if (options.payment) return options.payment(route, json, requests.payments.length)
      return json(pending)
    }
    if (pathname === '/api/admin/orders') return json({ orders: [options.adminOrder || order], total: 1, page: 1 })
    if (pathname === `/api/admin/orders/${orderId}`) return json({ ...(options.adminOrder || order), history: [] })
    throw new Error(`Unexpected API request ${pathname}`)
  })
  return { context, page, requests }
}
async function review(page, base) {
  await page.goto(base)
  await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
  await page.locator('.header-cart').click()
  await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
  await page.locator('[name=email]').fill('test@example.test')
  await page.locator('[name=name]').fill('Anna Testerová')
  await page.locator('[name=tel]').fill('+420 777 123 456')
  await page.locator('[name=street-address]').fill(order.street)
  await page.locator('[name=city]').fill(order.city)
  await page.locator('[name=postal-code]').fill('11000')
  await page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
}
async function seed(page, base) {
  await page.goto(base)
  await page.evaluate(value => {
    sessionStorage.setItem('brunch-payment-recovery', JSON.stringify(value))
    sessionStorage.setItem(`brunch-payment-recovery-${value.order.id}`, JSON.stringify(value))
  }, recovery)
}
async function run(browser, base, name, work, options = {}) {
  const test = await setup(browser, base, options)
  try {
    await work(test)
    results.push({ scenario: name, passed: true })
  } finally { await test.page.unrouteAll({ behavior: 'ignoreErrors' }); await test.context.close() }
}
async function main() {
  const { server, base } = await staticServer()
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  try {
    await run(browser, base, 'Calendar and postcode hours stay in sync between shop and checkout', async ({ page }) => {
      await page.goto(base)
      await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
      await page.locator('.delivery-selector').first().click()
      const calendar = page.getByRole('dialog').getByRole('group', { name: 'Kalendář doručení' })
      assert(await calendar.getByRole('button', { name: 'Předchozí měsíc' }).isDisabled())
      assert(await calendar.getByRole('button', { name: 'Středa 28. 10. 2026' }).isDisabled())
      assert(await calendar.getByRole('button', { name: 'Sobota 31. 10. 2026' }).isDisabled())
      await calendar.getByRole('button', { name: 'Následující měsíc' }).click()
      await calendar.getByRole('button', { name: 'Čtvrtek 19. 11. 2026' }).click()
      await page.locator('[name=delivery-postal]').fill('11000')
      assert.equal(await page.locator('[name=delivery-time]').inputValue(), '14:30–16:00')
      await page.getByRole('button', { name: 'Uložit a vybírat' }).click()
      await page.locator('.header-cart').click()
      await page.getByRole('button', { name: 'Pokračovat k doručení' }).click()
      assert.equal(await page.getByRole('button', { name: 'Čtvrtek 19. 11. 2026', exact: true }).getAttribute('aria-pressed'), 'true')
      assert.equal(await page.locator('[name=checkout-time]').inputValue(), '14:30–16:00')
    }, { postalSlots: { '11000': ['14:30–16:00'] } })

    await run(browser, base, 'Online payment hidden unless explicitly enabled', async ({ page }) => {
      await review(page, base)
      assert.equal(await page.locator('input[value=card]').count(), 0)
      assert.equal(await page.locator('input[value=card_on_delivery]').count(), 1)
    }, { enabled: false })

    await run(browser, base, 'Picking an address fills editable fields; manual checkout works when the registry is offline', async ({ page }) => {
      await page.goto(base)
      await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
      await page.locator('.header-cart').click()
      await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
      const street = page.locator('[name=street-address]')
      await street.fill('Pod sídlištěm 9')
      await page.getByRole('option', { name: /Pod sídlištěm 1800/ }).waitFor()
      await street.press('ArrowDown')
      await street.press('Enter')
      assert.equal(await street.inputValue(), 'Pod sídlištěm 1800/9, Kobylisy')
      assert.equal(await page.locator('[name=city]').inputValue(), 'Praha 8')
      assert.equal(await page.locator('[name=postal-code]').inputValue(), '18200')

      const offline = page.waitForResponse(response => new URL(response.url()).pathname === '/api/address-suggestions' && response.status() === 503)
      await street.fill('Vlastní adresa 5')
      await offline
      await page.locator('[name=city]').fill('Praha')
      await page.locator('[name=postal-code]').fill('11000')
      await page.locator('[name=email]').fill('test@example.test')
      const next = page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true })
      await next.click()
      assert(await page.locator('[name=name]').evaluate(el => el.matches(':invalid')))
      await page.locator('[name=name]').fill('Anna Testerová')
      await next.click()
      assert(await page.locator('[name=tel]').evaluate(el => el.matches(':invalid')))
      await page.locator('[name=tel]').fill('+420 777 123 456')
      await next.click()
      await page.getByText('Vlastní adresa 5', { exact: true }).waitFor()
    })

    let storedAtRedirect
    await run(browser, base, 'Lost create response reuses the key, delayed checkout persists recovery before redirect', async ({ page, requests }) => {
      await page.route('https://checkout.teya.com/**', async route => {
        await route.fulfill({ contentType: 'text/html', body: '<h1>Mock Teya</h1>' })
      })
      await review(page, base)
      await page.locator('input[value=card]').check()
      await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await page.locator('[data-checkout-step=payment] .api-message.error').waitFor()
      await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await page.getByRole('heading', { name: 'Čekáme na zaplacení.', exact: true }).waitFor()
      await page.waitForURL('https://checkout.teya.com/mock-session')
      await page.goto(base)
      storedAtRedirect = await page.evaluate(() => JSON.parse(sessionStorage.getItem('brunch-payment-recovery')))
      assert.equal(requests.orders.length, 2)
      assert.deepEqual(requests.orders[0], requests.orders[1])
      assert.equal(requests.payments.length, 3)
      assert.equal(storedAtRedirect.order.id, orderId)
      assert.equal(storedAtRedirect.idempotency_key, requests.orders[0].idempotency_key)
    }, { lostCreateResponse: true, payment: (route, json, count) => json(count < 3 ? pending : ready) })

    let verified = false
    await run(browser, base, 'Return URL does not prove payment, persistent status confirms it after page reload', async ({ page, requests }) => {
      await seed(page, base)
      await page.goto(`${base}/?payment_return=${orderId}&paid=true`)
      await page.getByRole('heading', { name: 'Čekáme na zaplacení.', exact: true }).waitFor()
      assert.equal(await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).count(), 0)
      assert.equal(requests.orders.length, 0)
      await page.reload()
      await page.getByRole('button', { name: 'Otevřít uloženou objednávku' }).click()
      await page.getByRole('heading', { name: 'Čekáme na zaplacení.', exact: true }).waitFor()
      verified = true
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(await page.evaluate(() => sessionStorage.getItem('brunch-payment-recovery')), null)
      assert.equal(await page.evaluate(id => sessionStorage.getItem(`brunch-payment-recovery-${id}`), orderId), null)
      assert.equal(requests.orders.length, 0)
    }, { payment: (route, json) => json(verified ? paid : pending) })

    await run(browser, base, 'Forged return without recovery never queries or asserts successful payment', async ({ page, requests }) => {
      await page.goto(`${base}/?payment_return=${orderId}&paid=true`)
      await page.getByText('Stav této platby zde nelze ověřit.', { exact: false }).first().waitFor()
      assert.equal(requests.payments.length, 0)
      assert.equal(await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).count(), 0)
    })

    await run(browser, base, 'Payment API outage keeps recovery and explicit resume reaches verified paid state', async ({ page, requests }) => {
      await seed(page, base)
      await page.goto(`${base}/?payment_return=${orderId}`)
      await page.locator('[data-checkout-step=confirmation] .api-message.error').waitFor()
      assert(await page.evaluate(() => sessionStorage.getItem('brunch-payment-recovery')))
      await page.getByRole('button', { name: 'Ověřit stav platby', exact: true }).click()
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(requests.orders.length, 0)
      assert.equal(requests.payments.length, 2)
    }, { payment: (route, json, count) => count === 1 ? route.abort('failed') : json(paid) })

    await run(browser, base, 'Review keeps recovery and does not offer another payment', async ({ page }) => {
      await seed(page, base)
      await page.goto(`${base}/?payment_return=${orderId}`)
      await page.getByText('Platbu musí ověřit obsluha.', { exact: false }).waitFor()
      assert.equal(await page.getByRole('button', { name: 'Pokračovat k platbě', exact: true }).count(), 0)
      assert(await page.evaluate(() => sessionStorage.getItem('brunch-payment-recovery')))
    }, { payment: (route, json) => json({ ...pending, payment_state: 'review' }) })

    for (const paymentState of ['pending', 'ready', 'review']) {
      await run(browser, base, `Order expiry stops polling and checkout when provider state is ${paymentState}`, async ({ page, requests }) => {
        await seed(page, base)
        await page.goto(`${base}/?payment_return=${orderId}`)
        await page.locator('.payment-deadline').waitFor()
        assert.match(await page.locator('.payment-deadline').innerText(), /24\. 10\. 2026.*11:00/)
        await page.getByRole('heading', { name: 'Platnost objednávky vypršela.', exact: true }).waitFor()
        assert.equal(await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).count(), 0)
        assert.equal(await page.getByRole('button', { name: 'Pokračovat k platbě', exact: true }).count(), 0)
        if (paymentState === 'review') await page.getByText('Platbu musí ověřit obsluha.', { exact: false }).waitFor()
        assert.equal(requests.orders.length, 0)
        assert.equal(requests.payments.length, 2)
        await page.waitForTimeout(2300)
        assert.equal(requests.payments.length, 2, 'Terminal order must stop polling, regardless of gateway state')
        const saved = await page.evaluate(() => JSON.parse(sessionStorage.getItem('brunch-payment-recovery')))
        assert.equal(saved.order.status, 'expired')
      }, { payment: (route, json, count) => json(count === 1 ? pending : { ...ready, status: 'expired', payment_state: paymentState }) })
    }

    await run(browser, base, 'An explicit new checkout after expiry uses a new idempotency key', async ({ page, requests }) => {
      for (let checkout = 0; checkout < 2; checkout++) {
        await review(page, base)
        await page.locator('input[value=card]').check()
        await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
        await page.getByRole('heading', { name: 'Platnost objednávky vypršela.', exact: true }).waitFor()
        assert.equal(requests.orders.length, checkout + 1)
        assert.equal(await page.getByRole('button', { name: 'Pokračovat k platbě', exact: true }).count(), 0)
      }
      assert.notEqual(requests.orders[0].idempotency_key, requests.orders[1].idempotency_key)
    }, { payment: (route, json) => json({ ...ready, status: 'expired' }) })

    await run(browser, base, 'Admin cannot accept or manually mark pending online order paid', async ({ page }) => {
      await page.goto(`${base}/admin/`)
      await page.getByRole('button', { name: 'Detail', exact: true }).click()
      await page.getByRole('heading', { name: 'Průběh objednávky' }).waitFor()
      assert(await page.locator('#editor [name=status]').isDisabled())
      assert(await page.locator('[name=payment_status]').isDisabled())
      assert(await page.getByRole('button', { name: 'Uložit změny' }).isDisabled())
      assert.equal(await page.locator('#editor [name=status] option').count(), 1)
    }, { admin: true })

    await run(browser, base, 'Verified online payment is readonly while fulfillment can progress', async ({ page }) => {
      await page.goto(`${base}/admin/`)
      await page.getByRole('button', { name: 'Detail', exact: true }).click()
      await page.getByRole('heading', { name: 'Průběh objednávky' }).waitFor()
      assert(!(await page.locator('#editor [name=status]').isDisabled()))
      assert(await page.locator('[name=payment_status]').isDisabled())
      assert.equal(await page.locator('[name=payment_status]').inputValue(), 'paid')
      assert.deepEqual(await page.locator('#editor [name=status] option').evaluateAll(nodes => nodes.map(node => node.value)), ['new', 'confirmed'])
    }, { admin: true, adminOrder: { ...order, status: 'new', payment_status: 'paid' } })

    await run(browser, base, 'Only a verified failed online payment allows admin cancellation', async ({ page }) => {
      await page.goto(`${base}/admin/`)
      await page.getByRole('button', { name: 'Detail', exact: true }).click()
      await page.getByRole('heading', { name: 'Průběh objednávky' }).waitFor()
      await page.getByText('Teya: Platba neproběhla', { exact: true }).waitFor()
      assert(!(await page.locator('#editor [name=status]').isDisabled()))
      assert(await page.locator('[name=payment_status]').isDisabled())
      assert(!(await page.getByRole('button', { name: 'Uložit změny' }).isDisabled()))
      assert.deepEqual(await page.locator('#editor [name=status] option').evaluateAll(nodes => nodes.map(node => node.value)), ['awaiting_payment', 'cancelled'])
      assert.match(await page.locator('#editor').innerText(), /Poslední ověření/)
    }, { admin: true, adminOrder: { ...order, online_payment: { state: 'failed', checked_at: '2026-10-24T10:00:00Z', last_error: null, cancellable: true } } })

    await run(browser, base, 'Expired order remains terminal in admin even if provider permits cancellation', async ({ page }) => {
      await page.goto(`${base}/admin/`)
      await page.getByRole('button', { name: 'Detail', exact: true }).click()
      await page.getByRole('heading', { name: 'Průběh objednávky' }).waitFor()
      assert(await page.locator('#editor [name=status]').isDisabled())
      assert(await page.locator('[name=payment_status]').isDisabled())
      assert(await page.getByRole('button', { name: 'Uložit změny' }).isDisabled())
      assert.deepEqual(await page.locator('#editor [name=status] option').evaluateAll(nodes => nodes.map(node => node.value)), ['expired'])
      assert.match(await page.locator('#editor').innerText(), /Vypršela/)
      assert.match(await page.locator('#editor').innerText(), /Lhůta pro online platbu:.*11:00/)
      await page.getByText('Platba vyžaduje ruční kontrolu v portálu Teya.', { exact: false }).waitFor()
    }, { admin: true, adminOrder: { ...order, status: 'expired', online_payment: { state: 'review', checked_at: '2026-10-24T10:00:00Z', expires_at: expiresAt, last_error: null, cancellable: true } } })

    for (const latePayment of [false, true]) {
      const scenarioReceipt = { ...receipt, verified_at: latePayment ? '2026-10-24T09:15:00Z' : receipt.verified_at }
      await run(browser, base, latePayment ? 'Admin shows late payment receipt and review warning without reopening expired order' : 'Admin shows verified payment receipt for the paid order', async ({ page }) => {
        await page.goto(`${base}/admin/`)
        await page.getByRole('button', { name: 'Detail', exact: true }).click()
        const evidence = page.getByRole('region', { name: 'Přijatá online platba', exact: true })
        await evidence.waitFor()
        assert.match(await evidence.innerText(), /Částka: 120,00\sKč/)
        await evidence.getByText('Měna: CZK', { exact: true }).waitFor()
        await evidence.getByText(`Transakce Teya: ${receipt.transaction_id}`, { exact: true }).waitFor()
        await evidence.getByText(`Platební relace: ${receipt.session_id}`, { exact: true }).waitFor()
        assert.match(await evidence.innerText(), latePayment ? /Ověřeno: 24\. 10\. 2026.*11:15/ : /Ověřeno: 24\. 10\. 2026.*10:45/)
        assert.equal(await page.locator('[name=payment_status]').inputValue(), latePayment ? 'unpaid' : 'paid')
        assert(await page.locator('[name=payment_status]').isDisabled())
        assert.equal(await page.locator('#editor [name=status]').isDisabled(), latePayment)
        assert.equal(await page.getByRole('button', { name: 'Uložit změny' }).isDisabled(), latePayment)
        if (latePayment) {
          await page.getByText('Platba vyžaduje ruční kontrolu v portálu Teya.', { exact: false }).waitFor()
          await evidence.getByText('Objednávka zůstává uzavřená.', { exact: false }).waitFor()
          assert.deepEqual(await page.locator('#editor [name=status] option').evaluateAll(nodes => nodes.map(node => node.value)), ['expired'])
        }
      }, { admin: true, adminOrder: { ...order, status: latePayment ? 'expired' : 'new', payment_status: latePayment ? 'unpaid' : 'paid', online_payment: { state: latePayment ? 'review' : 'paid', checked_at: scenarioReceipt.verified_at, expires_at: expiresAt, last_error: null, cancellable: false, receipt: scenarioReceipt } } })
    }

    assert.deepEqual(errors, [], 'No browser runtime errors')
    await fs.mkdir('output/browser', { recursive: true })
    await fs.writeFile('output/browser/payment-results.json', `${JSON.stringify({ results }, null, 2)}\n`)
    console.log(JSON.stringify({ results }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
