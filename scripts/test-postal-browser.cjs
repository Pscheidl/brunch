// Local browser smoke test for delivery areas. Requires Playwright and a database administrator.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const { adminTestPassword, loginAdministrator } = require('./admin-test-login.cjs')

async function main() {
  const base = process.env.TEST_BASE_URL || 'http://127.0.0.1:3100'
  const adminEmail = process.env.TEST_ADMIN_EMAIL || 'admin@example.test'
  const adminPassword = adminTestPassword()
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(new URL(base).hostname), 'Run this test against localhost only')
  const output = path.resolve('output/postal-browser')
  await fs.mkdir(output, { recursive: true })
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const admin = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
  const guest = await browser.newContext({ viewport: { width: 390, height: 844 }, locale: 'cs-CZ' })
  const page = await admin.newPage()
  const shop = await guest.newPage()
  const errors = []
  for (const target of [page, shop]) target.on('pageerror', error => errors.push(error.message))
  const run = Date.now()
  const email = `postal-browser-${run}@example.test`
  const label = `Test PSČ ${run}`
  let area
  let originalArea
  let acceptedOrder
  let initialAreas = []
  const checks = []
  const api = async (request, method, endpoint, data) => {
    const response = await request[method](`${base}${endpoint}`, { data, headers: { 'X-Brunch-Request': '1' } })
    assert.equal(response.status(), 200, `${method} ${endpoint}: ${await response.text()}`)
    return response.json()
  }
  const readAreas = () => api(admin.request, 'get', '/api/admin/delivery-areas')
  const writeArea = async active => {
    const current = (await readAreas()).find(candidate => candidate.id === area.id)
    assert.ok(current, 'Test area must still exist')
    area = await api(admin.request, 'put', `/api/admin/delivery-areas/${area.id}`, { postal_code: current.postal_code, label: current.label, active, version: current.version })
    return area
  }
  const waitAreaResponse = method => page.waitForResponse(response => response.request().method() === method && new URL(response.url()).pathname.startsWith('/api/admin/delivery-areas'))
  const submitArea = async method => {
    const pending = waitAreaResponse(method)
    await page.getByRole('button', { name: 'Uložit oblast', exact: true }).click()
    return pending
  }
  const areaRow = () => page.getByRole('row').filter({ hasText: area.postal_code.replace(/^(.{3})(.{2})$/, '$1 $2') })
  const uiToggle = async active => {
    const pending = waitAreaResponse('PUT')
    await areaRow().getByRole('button', { name: active ? 'Zapnout' : 'Pozastavit', exact: true }).click()
    const response = await pending
    assert.equal(response.status(), 200, await response.text())
    area = await response.json()
    assert.equal(area.active, active)
    await areaRow().getByRole('button', { name: active ? 'Pozastavit' : 'Zapnout', exact: true }).waitFor()
  }
  const fillCheckout = async postal => {
    await shop.locator('[name=email]').fill(email)
    await shop.locator('[name=name]').fill('Anna Testerová')
    await shop.locator('[name=tel]').fill('+420 777 123 456')
    await shop.locator('[name=street-address]').fill('Testovací 12')
    await shop.locator('[name=city]').fill('Test rozvozu')
    await shop.locator('[name=postal-code]').fill(postal)
    await shop.locator('[name=notes]').fill('Test PSČ, nevyrábět ani nerozvážet.')
  }
  try {
    await loginAdministrator(page, base, adminEmail, adminPassword)
    initialAreas = await readAreas()
    assert.ok(initialAreas.every(candidate => !candidate.active), 'Start this test with no active delivery areas. Existing active areas are not changed.')
    const usedCodes = new Set(initialAreas.map(candidate => candidate.postal_code))
    let postalCode = '99998'
    for (let candidate = 99998; usedCodes.has(postalCode) && candidate >= 99000; candidate--) postalCode = String(candidate)
    assert.ok(!usedCodes.has(postalCode), 'No unused test postal code available')
    originalArea = initialAreas.find(candidate => candidate.postal_code === postalCode)
    const formatted = `${postalCode.slice(0, 3)} ${postalCode.slice(3)}`
    const unicodePostal = `\uFEFF${postalCode.slice(0, 1)}\u0085${postalCode.slice(1, 3)}\u00a0\u2003${postalCode.slice(3)}\u3000`
    const disallowed = postalCode === '88888' ? '88887' : '88888'
    await page.goto(`${base}/admin/#delivery-areas`)
    await page.getByRole('heading', { name: 'Oblasti rozvozu', exact: true }).waitFor()
    assert.match(await page.locator('.delivery-area-guidance').innerText(), /Rozvoz není povolen do žádného PSČ/)
    await shop.goto(base)
    await shop.locator('.product-card').first().waitFor()
    assert.ok(await shop.locator('.product-card').count() > 0, 'Catalog is visible without a postal code or active delivery areas')
    await shop.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
    await shop.locator('.header-cart').click()
    await shop.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
    assert.equal(await shop.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).isDisabled(), true)
    await shop.getByRole('button', { name: 'Zavřít košík', exact: true }).click()
    checks.push('catalog and cart available without postal code or active areas')
    await shop.locator('.delivery-selector').first().click()
    await shop.locator('[name=delivery-postal]').fill(unicodePostal)
    assert.equal(await shop.locator('[name=delivery-postal]').inputValue(), postalCode)
    assert.equal(await shop.getByRole('button', { name: 'Uložit a vybírat', exact: true }).isDisabled(), true)
    assert.match(await shop.locator('#delivery-postal-status').innerText(), /Rozvoz zatím není k dispozici/)
    await shop.getByRole('button', { name: 'Zavřít nastavení doručení' }).click()
    checks.push('empty allowed list blocks delivery')

    await page.getByRole('button', { name: '+ Přidat oblast', exact: true }).click()
    await page.getByLabel('PSČ', { exact: true }).fill(unicodePostal)
    assert.equal(await page.getByLabel('PSČ', { exact: true }).inputValue(), postalCode)
    await page.getByLabel('PSČ', { exact: true }).evaluate((input, text) => {
      input.select()
      const clipboard = new DataTransfer()
      clipboard.setData('text/plain', text)
      input.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: clipboard }))
    }, unicodePostal)
    assert.equal(await page.getByLabel('PSČ', { exact: true }).inputValue(), postalCode)
    await page.getByLabel('Název oblasti', { exact: true }).fill(label)
    let response = await submitArea('POST')
    assert.equal(response.status(), 200, await response.text())
    area = await response.json()
    assert.equal(area.postal_code, postalCode)
    assert.equal(area.active, true)
    await areaRow().waitFor()
    await page.getByLabel('Vyhledat PSČ nebo název oblasti').fill(unicodePostal)
    assert.equal(await page.locator('.delivery-area-results tbody tr').count(), 1)
    await areaRow().getByText(label, { exact: true }).waitFor()
    checks.push('admin input and paste normalize Unicode whitespace', 'search ignores whitespace')

    await page.getByRole('button', { name: '+ Přidat oblast', exact: true }).click()
    await page.getByLabel('PSČ', { exact: true }).fill(formatted)
    await page.getByLabel('Název oblasti', { exact: true }).fill('Duplicitní oblast')
    response = await submitArea('POST')
    assert.equal(response.status(), 409)
    await page.getByRole('dialog').getByRole('alert').waitFor()
    assert.match(await page.getByRole('dialog').getByRole('alert').innerText(), /PSČ/)
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await areaRow().getByRole('button', { name: 'Upravit', exact: true }).click()
    await page.getByLabel('Název oblasti', { exact: true }).fill(`${label} upraveno`)
    response = await submitArea('PUT')
    assert.equal(response.status(), 200, await response.text())
    area = await response.json()
    await areaRow().getByText(`${label} upraveno`, { exact: true }).waitFor()
    await uiToggle(false)
    await uiToggle(true)
    checks.push('duplicate shows Czech error', 'edit label', 'deactivate and reactivate')
    await page.setViewportSize({ width: 390, height: 844 })
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1), 'Admin page fits a 390 px viewport')
    await page.screenshot({ path: path.join(output, 'delivery-areas-mobile.png'), fullPage: true })

    await shop.reload()
    await shop.locator('.product-card').first().waitFor()
    await shop.locator('.delivery-selector').first().click()
    await shop.locator('[name=delivery-postal]').fill(disallowed)
    assert.equal(await shop.getByRole('button', { name: 'Uložit a vybírat', exact: true }).isDisabled(), true)
    assert.match(await shop.locator('#delivery-postal-status').innerText(), /Na toto PSČ zatím nerozvážíme/)
    await shop.locator('[name=delivery-postal]').fill(unicodePostal)
    assert.equal(await shop.locator('[name=delivery-postal]').inputValue(), postalCode)
    assert.equal(await shop.getByRole('button', { name: 'Uložit a vybírat', exact: true }).isEnabled(), true)
    await shop.getByRole('button', { name: 'Uložit a vybírat', exact: true }).click()
    await shop.locator('.header-cart').click()
    await shop.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
    await fillCheckout(disallowed)
    assert.equal(await shop.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).isDisabled(), true)
    assert.match(await shop.locator('#checkout-postal-status').innerText(), /Na toto PSČ zatím nerozvážíme/)
    await fillCheckout(unicodePostal)
    assert.equal(await shop.locator('[name=postal-code]').inputValue(), postalCode)
    assert.equal(await shop.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).isEnabled(), true)
    await shop.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    await shop.locator('[name=terms-accepted]').check()
    await shop.getByRole('heading', { name: 'Už jen poslední krok.', exact: true }).waitFor()
    await shop.locator('[name=payment-method][value=cash_on_delivery]').check()
    assert.equal(await shop.locator('#review-postal-status').innerText(), 'Sem rozvážíme.')
    await shop.screenshot({ path: path.join(output, 'checkout-postal-mobile.png'), fullPage: false })
    checks.push('delivery selector rejects disallowed and accepts whitespace', 'checkout rejects disallowed and accepts Unicode whitespace')

    const query = `/api/admin/orders?q=${encodeURIComponent(email)}&page=1`
    assert.equal((await api(admin.request, 'get', query)).total, 0)
    await uiToggle(false)
    const refused = shop.waitForResponse(result => new URL(result.url()).pathname === '/api/orders' && result.request().method() === 'POST')
    await shop.getByRole('button', { name: /Objednat s povinností platby/ }).click()
    response = await refused
    assert.equal(response.status(), 400, await response.text())
    const failedRequest = response.request().postDataJSON()
    assert.equal(failedRequest.postal_code, postalCode)
    assert.equal((await api(admin.request, 'get', query)).total, 0, 'A deactivated area must not create an order')
    const mail = await api(admin.request, 'get', '/api/admin/mail')
    assert.ok(!mail.some(message => message.to_email === email), 'Rejected checkout must not enqueue customer mail')
    await shop.waitForFunction(() => document.querySelector('.payment-submit')?.disabled)
    await shop.screenshot({ path: path.join(output, 'deactivated-checkout-mobile.png'), fullPage: false })
    checks.push('server rejects area deactivated after review without order or email')

    await uiToggle(true)
    // Reload refreshes availability, while the saved cart and idempotency attempt remain.
    await shop.reload()
    await shop.locator('.product-card').first().waitFor()
    await shop.locator('.header-cart').click()
    await shop.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
    await fillCheckout(formatted)
    await shop.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    await shop.locator('[name=terms-accepted]').check()
    await shop.locator('[name=payment-method][value=cash_on_delivery]').check()
    const accepted = shop.waitForResponse(result => new URL(result.url()).pathname === '/api/orders' && result.request().method() === 'POST')
    await shop.getByRole('button', { name: /Objednat s povinností platby/ }).click()
    response = await accepted
    assert.equal(response.status(), 200, await response.text())
    acceptedOrder = await response.json()
    assert.equal(acceptedOrder.postal_code, postalCode)
    assert.equal(response.request().postDataJSON().idempotency_key, failedRequest.idempotency_key, 'Retry with canonical equivalent postal code keeps its idempotency key')
    assert.equal((await api(admin.request, 'get', query)).total, 1)
    await shop.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
    checks.push('reactivation and same-key retry create exactly one order')
    assert.deepEqual(errors, [], 'No uncaught browser exceptions')
    console.log(JSON.stringify({ result: 'passed', checks, postal_code: postalCode, order: acceptedOrder.number, screenshots: output }, null, 2))
  } catch (error) {
    await page.screenshot({ path: path.join(output, 'admin-failure.png'), fullPage: true }).catch(() => {})
    await shop.screenshot({ path: path.join(output, 'checkout-failure.png'), fullPage: false }).catch(() => {})
    console.error('Browser exceptions:', errors)
    throw error
  } finally {
    let cleanupError
    try {
      if (acceptedOrder) {
        const order = await api(admin.request, 'get', `/api/admin/orders/${acceptedOrder.id}`)
        if (order.status === 'new' && order.payment_status === 'unpaid') await api(admin.request, 'patch', `/api/admin/orders/${order.id}`, { status: 'cancelled', payment_status: 'unpaid', version: order.version })
      }
      if (area) {
        if (originalArea) {
          const current = (await readAreas()).find(candidate => candidate.id === area.id)
          await api(admin.request, 'put', `/api/admin/delivery-areas/${area.id}`, { postal_code: originalArea.postal_code, label: originalArea.label, active: originalArea.active, version: current.version })
        } else await writeArea(false)
        assert.ok((await readAreas()).every(candidate => !candidate.active), 'Cleanup leaves no active delivery areas')
      }
    } catch (error) { cleanupError = error }
    await browser.close()
    if (cleanupError) throw cleanupError
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
