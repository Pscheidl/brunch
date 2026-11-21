const assert = require('node:assert/strict')
const fs = require('node:fs')
const { chromium } = require('playwright')
const base = process.env.BRUNCH_TEST_URL || 'http://127.0.0.1:3181'
assert(['127.0.0.1', 'localhost'].includes(new URL(base).hostname), 'Local tests only')
const out = 'output/pickup-invoice-browser'
fs.mkdirSync(out, { recursive: true })
const id = '11111111-1111-4111-8111-111111111111'
const place = { name: 'Odběrná pekárna', street: 'U výdeje 88', postal_code: '46001', city: 'Liberec', instructions: 'Vchod ze dvora', parking_free: true }
const order = { id, number: 'BR-PICKUP-1', version: 1, status: 'confirmed', payment_status: 'unpaid', payment_method: 'cash_on_delivery', fulfillment_method: 'pickup', pickup_location: place, name: 'Test zákazník', email: 'buyer@example.test', phone: '+420777123456', organization: '', street: place.street, postal_code: place.postal_code, city: place.city, notes: '', delivery_date: '2026-11-23', delivery_slot: '10:00–12:00', subtotal_cents: 11900, delivery_fee_cents: 0, total_cents: 11900, created_at: '2026-11-21T12:00:00Z', items: [{ product_id: 1, name: 'Sendvič', quantity: 1, unit_price_cents: 11900, total_cents: 11900 }], history: [] }
const supplier = { name: 'Dodavatel', address: 'Sídlo 1', company_id: '12345678', tax_id: '', vat_payer: false, version: 2 }
;(async () => {
  const browser = await chromium.launch({ headless: true })
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
    let invoice = null
    let billing = null
    let issuedPayload = null
    const errors = []
    await context.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname
      const method = route.request().method()
      let data
      if (path === '/api/auth/me') data = { user: { id: 'admin', email: 'admin@example.test', is_admin: true } }
      else if (path === '/api/admin/orders') data = { orders: [order], total: 1, page: 1 }
      else if (path === `/api/admin/orders/${id}`) data = order
      else if (path === '/api/admin/invoice-supplier') data = supplier
      else if (path === `/api/admin/orders/${id}/billing`) data = billing
      else if (path === `/api/admin/orders/${id}/invoice`) {
        if (method === 'POST') {
          issuedPayload = route.request().postDataJSON()
          invoice = { number: 'FA-PICKUP-1', order_number: order.number, supplier, customer: { name: issuedPayload.customer_name, address: issuedPayload.customer_address, company_id: '', tax_id: '' }, issue_date: '2026-11-21', supply_date: issuedPayload.supply_date, due_date: issuedPayload.due_date, lines: order.items, payment_method: order.payment_method, payment_status: order.payment_status, fulfillment_method: order.fulfillment_method, pickup_location: structuredClone(place), total_cents: order.total_cents }
        }
        data = invoice
      } else throw new Error(`Unexpected ${method} ${path}`)
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify(data) })
    })
    const page = await context.newPage()
    page.on('pageerror', error => errors.push(error.message))
    let visit = 0
    const openInvoice = async () => {
      await page.goto(`${base}/admin/?invoice-test=${++visit}#orders`)
      await page.getByRole('button', { name: 'Detail', exact: true }).click()
      await page.getByRole('button', { name: 'Faktura', exact: true }).click()
    }
    await openInvoice()
    await page.getByLabel('Adresa odběratele', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Adresa odběratele', { exact: true }).inputValue(), '', 'Pickup location is not used as invoice recipient address')
    assert(await page.getByLabel('Adresa odběratele', { exact: true }).evaluate(el => el.required))
    await page.getByLabel('Adresa odběratele', { exact: true }).fill('Fakturační 15, Praha')
    await page.getByRole('button', { name: 'Vystavit fakturu', exact: true }).click()
    await page.getByRole('heading', { name: 'Faktura FA-PICKUP-1', exact: true }).waitFor()
    assert.equal(issuedPayload.customer_address, 'Fakturační 15, Praha')
    await page.getByText('Způsob úhrady: Hotově při vyzvednutí.', { exact: true }).waitFor()
    await page.locator('.invoice-pickup').getByText('Parkování zdarma', { exact: true }).waitFor()
    assert(!(await page.locator('.summary-grid').innerText()).includes(place.street), 'Pickup address is not invoice recipient address')
    await page.screenshot({ path: `${out}/pickup-invoice-desktop.png`, fullPage: false })
    await page.setViewportSize({ width: 390, height: 844 })
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth))
    await page.screenshot({ path: `${out}/pickup-invoice-mobile.png`, fullPage: false })
    invoice = null
    billing = { name: 'Uložený odběratel', address: 'Uložená fakturační 99', company_id: '87654321', tax_id: '' }
    await openInvoice()
    assert.equal(await page.getByLabel('Adresa odběratele', { exact: true }).inputValue(), billing.address, 'Explicit billing remains preferred for pickup')
    billing = null
    order.fulfillment_method = 'delivery'
    order.pickup_location = null
    order.street = 'Doručovací 7'
    order.postal_code = '11000'
    order.city = 'Praha'
    await openInvoice()
    assert.equal(await page.getByLabel('Adresa odběratele', { exact: true }).inputValue(), 'Doručovací 7\n11000 Praha', 'Delivery retains existing address default')
    invoice = { number: 'FA-LEGACY-1', order_number: order.number, supplier, customer: { name: 'Dřívější odběratel', address: 'Historická 1' }, issue_date: '2026-11-21', supply_date: order.delivery_date, due_date: order.delivery_date, lines: order.items, payment_method: 'card_on_delivery', payment_status: 'paid', total_cents: 11900 }
    await openInvoice()
    await page.getByText('Způsob úhrady: Kartou při doručení.', { exact: true }).waitFor()
    assert.equal(await page.locator('.invoice-pickup').count(), 0, 'Legacy invoice without fulfillment fields still renders as delivery')
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ passed: ['pickup invoice empty recipient address', 'explicit pickup billing', 'pickup invoice rendering and mobile', 'delivery address compatibility', 'legacy invoice snapshot compatibility'], screenshots: out }, null, 2))
  } finally { await browser.close() }
})().catch(error => { console.error(error); process.exit(1) })
