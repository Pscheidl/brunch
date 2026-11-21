const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const output = path.resolve('output/storefront-features-browser')
const products = [
  { id: 1, name: 'Sendviče pro tým', minimum_quantity: 5 },
  { id: 2, name: 'Malá svačina', minimum_quantity: 0 }
].map(product => ({
  description: 'Čerstvě připravené pečivo.', category: 'Svačiny', tag: '', weight: '200 g',
  price_cents: 10000, vegan: false, ingredients: 'Pečivo', allergens: 'Lepek',
  image: '/images/brunch-spread.webp', active: true, stock: 20, version: 1, ...product
}))

async function main() {
  await fs.mkdir(output, { recursive: true })
  const root = path.resolve('dist')
  await fs.access(path.join(root, 'index.html'))
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const type = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.webp': 'image/webp' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': type })
      response.end(body)
    } catch {
      response.writeHead(404)
      response.end('Not found')
    }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const errors = []
  const checks = []
  try {
    const context = await browser.newContext({ viewport: { width: 1280, height: 1000 } })
    const page = await context.newPage()
    await page.clock.setFixedTime(new Date('2026-11-21T12:00:00Z'))
    page.on('pageerror', error => errors.push(error.message))
    await page.route('**/api/**', async route => {
      const url = new URL(route.request().url())
      const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
      if (url.pathname === '/api/terms') return json({ version: 1, title: 'Obchodní podmínky', content: '## Testovací podmínky\n\nPodmínky pro místní test.', updated_at: '2026-11-21T12:00:00Z' })
      if (url.pathname === '/api/products') return json(products)
      if (url.pathname === '/api/settings') return json({ delivery_fee_cents: 5000, free_delivery_from_cents: null, delivery_postal_codes: ['11000', '12000'], online_payment_enabled: false })
      if (url.pathname === '/api/delivery-options') return json({ dates: [{ value: '2026-11-23', label: 'Pondělí 23. 11.' }], slots: ['09:00–10:00', '10:00–11:00'], postal_slots: { '12000': ['12:00–13:00'] } })
      if (url.pathname === '/api/auth/me') return json({ error: 'Nepřihlášen' }, 401)
      if (url.pathname === '/api/address-suggestions') return json([{ label: 'Dlouhá 12, Praha, 110 00', street: 'Dlouhá 12', city: 'Praha', postal_code: '11000' }])
      errors.push(`Unexpected API request: ${url.pathname}`)
      return json({ error: 'Unexpected request' }, 500)
    })
    await page.goto(base)
    const card = page.locator('.product-card').filter({ hasText: 'Sendviče pro tým' })
    await card.waitFor()
    const photo = card.locator('.product-photo')
    const bounds = await photo.boundingBox()
    assert(bounds.height <= 221 && bounds.width / bounds.height > 1.5, 'product photo is compact and landscape')
    assert.equal(await photo.locator('img').evaluate(img => getComputedStyle(img).objectFit), 'scale-down')
    await card.getByRole('button', { name: 'Přidat 5 ks do košíku: Sendviče pro tým', exact: true }).click()
    assert.equal(await card.locator('.quantity-control input').inputValue(), '5')
    await card.getByRole('button', { name: 'Přidat jeden kus: Sendviče pro tým', exact: true }).click()
    assert.equal(await card.locator('.quantity-control input').inputValue(), '6')
    await card.getByRole('button', { name: 'Odebrat jeden kus: Sendviče pro tým', exact: true }).click()
    await card.getByRole('button', { name: 'Odebrat z košíku: Sendviče pro tým', exact: true }).click()
    await card.getByRole('button', { name: 'Přidat 5 ks do košíku: Sendviče pro tým', exact: true }).click()
    await page.reload()
    await card.waitFor()
    assert.equal(await card.locator('.quantity-control input').inputValue(), '5')
    checks.push('Compact photographs and minimum quantities survive removal and reload')
    await page.screenshot({ path: path.join(output, 'storefront-desktop.png'), fullPage: true })

    await page.locator('.header-cart').click()
    await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
    const checkout = page.locator('.checkout-form')
    const calendar = checkout.getByRole('group', { name: 'Kalendář doručení', exact: true })
    assert.equal(await calendar.getByRole('button', { name: 'Pondělí 23. 11. 2026', exact: true }).getAttribute('aria-pressed'), 'true')
    assert(await calendar.getByRole('button', { name: 'Sobota 28. 11. 2026', exact: true }).isDisabled())
    assert(await calendar.getByRole('button', { name: 'Neděle 29. 11. 2026', exact: true }).isDisabled())
    assert(await calendar.getByRole('button', { name: 'Pátek 20. 11. 2026', exact: true }).isDisabled())
    assert(await calendar.getByRole('button', { name: 'Předchozí měsíc', exact: true }).isDisabled())
    await calendar.getByRole('button', { name: 'Následující měsíc', exact: true }).click()
    assert(await calendar.getByRole('button', { name: 'Čtvrtek 24. 12. 2026', exact: true }).isDisabled())
    await calendar.getByRole('button', { name: 'Následující měsíc', exact: true }).click()
    await calendar.getByRole('button', { name: 'Následující měsíc', exact: true }).click()
    await calendar.getByRole('button', { name: 'Úterý 9. 2. 2027', exact: true }).click()
    assert.equal(await calendar.getByRole('button', { name: 'Úterý 9. 2. 2027', exact: true }).getAttribute('aria-pressed'), 'true')
    checks.push('Calendar navigates future months and excludes past dates, weekends and holidays')

    await checkout.locator('[name=email]').fill('test@example.test')
    await checkout.locator('[name=street-address]').fill('Dlouhá')
    await checkout.getByRole('listbox', { name: 'Návrhy adres', exact: true }).getByRole('option').waitFor()
    await checkout.locator('[name=street-address]').press('ArrowDown')
    await checkout.locator('[name=street-address]').press('Enter')
    assert.equal(await checkout.locator('[name=street-address]').inputValue(), 'Dlouhá 12')
    assert.equal(await checkout.locator('[name=city]').inputValue(), 'Praha')
    assert.equal(await checkout.locator('[name=postal-code]').inputValue(), '11000')
    assert.equal(await checkout.locator('[name=checkout-time]').inputValue(), '09:00–10:00')
    await checkout.locator('[name=postal-code]').fill('120 00')
    await page.waitForFunction(() => document.querySelector('[name=checkout-time]').value === '12:00–13:00')
    assert.equal(await checkout.locator('[name=checkout-time] option').count(), 1)
    await checkout.locator('[name=postal-code]').fill('11000')
    await page.waitForFunction(() => document.querySelector('[name=checkout-time]').value === '09:00–10:00')
    await checkout.locator('[name=street-address]').fill('Ručně zadaná 99')
    await checkout.locator('[name=city]').fill('Praha')
    assert.equal(await checkout.locator('[name=street-address]').inputValue(), 'Ručně zadaná 99')
    checks.push('Address suggestions fill editable fields and postal code changes delivery hours')

    await checkout.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    assert(await page.locator('[data-checkout-step=delivery]').isVisible())
    assert.equal(await checkout.locator('[name=name]').evaluate(input => input.validity.valueMissing), true)
    await checkout.locator('[name=name]').fill('   ')
    assert.equal(await checkout.locator('[name=name]').evaluate(input => input.validity.patternMismatch), true)
    await checkout.locator('[name=name]').fill('Jana Testovací')
    await checkout.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    assert(await page.locator('[data-checkout-step=delivery]').isVisible())
    assert.equal(await checkout.locator('[name=tel]').evaluate(input => input.validity.valueMissing), true)
    await checkout.locator('[name=tel]').fill('+420 777 123 456')
    await page.setViewportSize({ width: 390, height: 844 })
    await calendar.scrollIntoViewIfNeeded()
    const mobileCalendar = await calendar.boundingBox()
    assert(mobileCalendar.x >= 0 && mobileCalendar.x + mobileCalendar.width <= 390, 'calendar fits mobile dialog')
    await page.screenshot({ path: path.join(output, 'calendar-mobile.png') })
    await checkout.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
    await page.getByRole('heading', { name: 'Už jen poslední krok.', exact: true }).waitFor()
    assert((await page.locator('[data-checkout-step=payment]').innerText()).includes('Ručně zadaná 99'))
    assert((await page.locator('[data-checkout-step=payment]').innerText()).includes('9. 2. 2027'))
    checks.push('Required name and phone gate checkout, manual address and selected future date reach review')
    assert.deepEqual(errors, [], 'No unhandled JavaScript errors or unexpected API requests')
    console.log(JSON.stringify({ checks, screenshots: output }, null, 2))
  } finally {
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors }, null, 2))
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
