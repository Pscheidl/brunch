const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')
const fs = require('node:fs')

const base = process.env.BRUNCH_TEST_URL || 'http://127.0.0.1:3100'
const database = process.env.BRUNCH_TEST_DATABASE_URL || process.env.TEST_DATABASE_URL
const psql = process.env.BRUNCH_PSQL_BIN || 'psql'
assert(['127.0.0.1', 'localhost'].includes(new URL(base).hostname), 'Browser tests only run against localhost')
assert(database, 'Set BRUNCH_TEST_DATABASE_URL to the local fixture database used by the running server')
assert(['127.0.0.1', 'localhost'].includes(new URL(database).hostname), 'Database checks only run against localhost')
const out = 'output/browser'
fs.mkdirSync(out, { recursive: true })
const run = Date.now()
const errors = []
const results = []

function orderCount(email) {
  assert(/^[a-z0-9@.\-]+$/.test(email), 'Only generated fixture emails are queried')
  return Number(execFileSync(psql, [database, '-At', '-c', `SELECT count(*) FROM orders WHERE email = '${email}'`], { encoding: 'utf8' }).trim())
}
async function pageFor(browser) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
  const page = await context.newPage()
  page.setDefaultTimeout(15000)
  page.on('pageerror', error => errors.push(error.message))
  return { context, page }
}
async function reviewOrder(page, email) {
  await page.locator('.product-card').first().waitFor()
  await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
  await page.locator('.header-cart').click()
  await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
  await page.locator('[name=email]').fill(email)
  await page.locator('[name=name]').fill('Anna Testerová')
  await page.locator('[name=tel]').fill('+420 777 123 456')
  await page.locator('[name=street-address]').fill('Testovací 25')
  await page.locator('[name=city]').fill('Praha')
  await page.locator('[name=postal-code]').fill('110 00')
  await page.locator('[name=notes]').fill('Test zotavení objednávky. 2. patro.')
  await page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
  await page.locator('[name=terms-accepted]').check()
}
async function submit(page) {
  const response = page.waitForResponse(r => r.url() === `${base}/api/orders` && r.request().method() === 'POST')
  await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
  return response
}
async function unavailableApi(browser) {
  const { context, page } = await pageFor(browser)
  try {
    let calls = 0
    await page.route('**/api/products', async route => {
      calls++
      if (calls === 1) await route.abort('failed')
      else await route.continue()
    })
    await page.goto(base)
    await page.locator('#nabidka .api-message.error').waitFor()
    assert.equal(await page.locator('.product-card').count(), 0, 'Unavailable API does not display a fabricated catalog')
    assert.match(await page.locator('#nabidka .api-message.error').innerText(), /Spojení se nepodařilo/)
    await page.screenshot({ path: `${out}/catalog-api-unavailable.png`, fullPage: true })
    await page.getByRole('button', { name: 'Zkusit znovu', exact: true }).click()
    await page.locator('.product-card').first().waitFor()
    assert.equal(await page.locator('#nabidka .api-message.error').count(), 0)
    assert.equal(calls, 2)
    results.push({ scenario: 'unavailable catalog API and explicit retry', passed: true })
  } finally { await context.close() }
}
async function lostOrderResponse(browser) {
  const { context, page } = await pageFor(browser)
  const email = `test-recovery-lost-${run}@example.test`
  let accepted
  let firstPayload
  let retriedPayload
  let calls = 0
  try {
    await page.route('**/api/orders', async route => {
      if (route.request().method() !== 'POST') return route.continue()
      calls++
      if (calls === 1) {
        firstPayload = route.request().postDataJSON()
        const response = await route.fetch()
        assert.equal(response.status(), 200, await response.text())
        accepted = await response.json()
        await route.abort('failed')
      } else {
        retriedPayload = route.request().postDataJSON()
        await route.continue()
      }
    })
    await page.goto(base)
    await reviewOrder(page, email)
    await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
    await page.locator('[data-checkout-step=payment] .api-message.error').waitFor()
    assert(accepted?.id, 'The backend committed the first order before the response was discarded')
    assert.equal(orderCount(email), 1, 'Exactly one order exists after the discarded success response')
    assert.match(await page.locator('[data-checkout-step=payment] .api-message.error').innerText(), /nevytvoří podruhé/)
    assert.equal(await page.locator('.header-cart .cart-count').innerText(), '1', 'Unconfirmed cart remains intact')
    await page.screenshot({ path: `${out}/checkout-response-lost.png`, fullPage: false })
    const response = await submit(page)
    assert.equal(response.status(), 200, await response.text())
    const replayed = await response.json()
    assert.equal(replayed.id, accepted.id, 'The retry returns the original order')
    assert.deepEqual(retriedPayload, firstPayload, 'Retry sends identical payload and idempotency key')
    assert.equal(orderCount(email), 1, 'Retry did not create a second database order')
    await page.getByRole('heading', { name: 'Dobrá pauza je na dohled.', exact: true }).waitFor()
    assert.equal(await page.locator('.header-cart .cart-count').innerText(), '0')
    assert.equal(await page.evaluate(() => sessionStorage.getItem('brunch-order-attempt')), null)
    results.push({ scenario: 'lost success response and safe retry', passed: true, orderId: replayed.id, orderNumber: replayed.number, databaseOrderCount: 1 })
  } finally { await context.close() }
}
async function changedPrice(browser) {
  const { context, page } = await pageFor(browser)
  const email = `test-recovery-price-${run}@example.test`
  const payloads = []
  let catalogCalls = 0
  try {
    // Only the first catalog response is deliberately stale. The real server still
    // prices and accepts orders using its unchanged current PostgreSQL catalog.
    await page.route('**/api/products', async route => {
      catalogCalls++
      if (catalogCalls === 1) {
        const response = await route.fetch()
        const products = await response.json()
        assert(products[0].price_cents > 100)
        products[0].price_cents -= 100
        await route.fulfill({ response, json: products })
      } else await route.continue()
    })
    page.on('request', request => {
      if (request.url() === `${base}/api/orders` && request.method() === 'POST') payloads.push(request.postDataJSON())
    })
    await page.goto(base)
    await reviewOrder(page, email)
    const conflict = await submit(page)
    assert.equal(conflict.status(), 409, await conflict.text())
    assert.equal(orderCount(email), 0, 'Stale total did not create an order')
    await page.locator('[data-checkout-step=payment] .api-message.error').waitFor()
    await page.waitForFunction(() => {
      const button = document.querySelector('.payment-submit')
      return button && !button.disabled
    })
    assert(catalogCalls >= 2, 'The conflict reloads the authoritative catalog')
    assert.match(await page.locator('[data-checkout-step=payment] .api-message.error').innerText(), /znovu potvrďte/)
    assert.equal(payloads.length, 1, 'No automatic purchase after changed pricing')
    await page.screenshot({ path: `${out}/checkout-price-reconfirmation.png`, fullPage: false })
    const confirmed = await submit(page)
    assert.equal(confirmed.status(), 200, await confirmed.text())
    const order = await confirmed.json()
    assert.equal(payloads.length, 2)
    assert.equal(payloads[1].expected_total_cents, payloads[0].expected_total_cents + 100)
    assert.notEqual(payloads[1].idempotency_key, payloads[0].idempotency_key, 'Reconfirmed changed payload gets a new attempt key')
    assert.equal(order.total_cents, payloads[1].expected_total_cents)
    assert.equal(orderCount(email), 1)
    await page.getByRole('heading', { name: 'Dobrá pauza je na dohled.', exact: true }).waitFor()
    results.push({ scenario: 'real 409 for stale price, refresh, explicit reconfirmation', passed: true, oldTotalCents: payloads[0].expected_total_cents, confirmedTotalCents: order.total_cents, databaseOrderCount: 1 })
  } finally { await context.close() }
}
;(async () => {
  const browser = await chromium.launch({ headless: true })
  try {
    await unavailableApi(browser)
    await lostOrderResponse(browser)
    await changedPrice(browser)
    assert.deepEqual(errors, [], 'No JavaScript runtime errors')
    const report = { results, screenshots: out }
    fs.writeFileSync(`${out}/checkout-recovery-results.json`, `${JSON.stringify(report, null, 2)}\n`)
    console.log(JSON.stringify(report, null, 2))
  } finally { await browser.close() }
})().catch(error => { console.error(error); process.exit(1) })
