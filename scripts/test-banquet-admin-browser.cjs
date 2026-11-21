// Local browser checks for banquet product validation and the shared minimum. No production requests.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/banquet-admin-browser')
  await fs.mkdir(output, { recursive: true })
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const file = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!file.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const data = await fs.readFile(file)
      response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.webp': 'image/webp' }[path.extname(file)] || 'application/octet-stream' })
      response.end(data)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, locale: 'cs-CZ' })
  const errors = [], productSaves = [], settingsSaves = [], checks = []
  let products = [{ id: 1, name: 'Běžná svačina', description: 'Svačina', category: '', weight: '100 g', ingredients: 'Mouka', allergens: 'Lepek', price_cents: 3500, minimum_quantity: 0, banquet: false, stock: null, active: true, vegan: false, image: '', version: 1, variants: [] }]
  let settings = { delivery_fee_cents: 4900, free_delivery_from_cents: 35000, order_email: 'orders@example.test', additional_order_emails: ['bakery@example.test'], delivery_slots: ['07:00–09:00'], banquet_minimum_quantity: 30, version: 1 }
  let site = { hero_image: '', hero_image_alt: '', version: 1, public_contact_email: '', public_contact_phone: '', seller_name: '', seller_address: '', seller_ico: '', delivery_area_label: '' }
  let failProduct = 0, failSettings = 0
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/**', async route => {
    const request = route.request(), url = new URL(request.url()), method = request.method()
    const reply = (json, status = 200) => route.fulfill({ json, status })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/products' && method === 'GET') return reply(products)
    if ((url.pathname === '/api/admin/products' && method === 'POST') || (/^\/api\/admin\/products\/\d+$/.test(url.pathname) && method === 'PUT')) {
      const payload = request.postDataJSON()
      productSaves.push(payload)
      if (failProduct) { const status = failProduct; failProduct = 0; return reply({ error: 'Zboží mezitím změnil jiný správce.' }, status) }
      const id = method === 'POST' ? products.length + 1 : Number(url.pathname.split('/').pop())
      const next = { ...products.find(product => product.id === id), ...payload, id, version: (payload.version || 0) + 1 }
      products = [...products.filter(product => product.id !== id), next]
      return reply(next)
    }
    if (url.pathname === '/api/admin/categories') return reply([])
    if (url.pathname === '/api/admin/settings') {
      if (method === 'GET') return reply(settings)
      const payload = request.postDataJSON()
      settingsSaves.push(payload)
      if (failSettings) { const status = failSettings; failSettings = 0; return reply({ error: 'Nastavení nelze uložit.' }, status) }
      if (payload.version !== settings.version) return reply({ error: 'Nastavení mezitím změnil jiný správce.' }, 409)
      settings = { ...payload, version: settings.version + 1 }
      return reply(settings)
    }
    if (url.pathname === '/api/admin/site-settings') {
      if (method === 'GET') return reply(site)
      site = { ...request.postDataJSON(), version: site.version + 1 }
      return reply(site)
    }
    errors.push(`Unexpected API request ${method} ${url.pathname}`)
    return route.abort()
  })
  const minimum = () => page.getByLabel('Minimální odběr v kusech', { exact: true })
  const banquet = () => page.getByLabel('Banketní pečivo', { exact: true })
  const total = () => page.getByLabel('Minimální množství banketního pečiva celkem', { exact: true })
  const saveProduct = () => page.getByRole('button', { name: 'Uložit zboží', exact: true }).click()
  const saveTotal = async () => {
    const response = page.waitForResponse(r => r.url().endsWith('/api/admin/settings') && r.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit minimum banketního pečiva', exact: true }).click()
    return response
  }
  const row = name => page.getByRole('row').filter({ hasText: name })
  try {
    await page.goto(`${base}/admin/#products`)
    await row('Běžná svačina').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await banquet().isChecked(), false)
    assert.equal(await minimum().inputValue(), '')
    assert.equal(await minimum().evaluate(node => node.required), false)
    await saveProduct()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(productSaves.at(-1).minimum_quantity, 0)
    assert.equal(productSaves.at(-1).banquet, false)
    await row('Běžná svačina').getByRole('button', { name: 'Upravit', exact: true }).click()
    await minimum().fill('6')
    await saveProduct()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(productSaves.at(-1).minimum_quantity, 6)
    checks.push('ordinary product accepts blank or positive optional minimum')

    await page.getByRole('button', { name: '+ Přidat zboží', exact: true }).click()
    await page.getByLabel('Název', { exact: true }).fill('Banketní houska se sýrem')
    await page.getByLabel('Cena v Kč', { exact: true }).fill('25')
    await banquet().check()
    assert.equal(await minimum().evaluate(node => node.required), true)
    assert.equal(await minimum().getAttribute('min'), '1')
    assert.equal(await minimum().getAttribute('max'), null)
    const beforeInvalid = productSaves.length
    for (const value of ['', '0', '-1', '1.5']) {
      await minimum().fill(value)
      await saveProduct()
      assert.equal(await minimum().evaluate(node => node.checkValidity()), false, `reject minimum ${value}`)
      assert.equal(productSaves.length, beforeInvalid, `invalid banquet minimum ${value} is not sent`)
    }
    await minimum().fill('2147483648')
    await saveProduct()
    await page.locator('[data-form-error]').getByText('Zadaný minimální odběr je příliš vysoký.', { exact: true }).waitFor()
    assert.equal(productSaves.length, beforeInvalid)
    await minimum().fill('120')
    await saveProduct()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(productSaves.at(-1).minimum_quantity, 120)
    assert.equal(productSaves.at(-1).banquet, true)
    await row('Banketní houska se sýrem').getByText('Banketní pečivo', { exact: true }).waitFor()
    await page.getByRole('searchbox', { name: 'Vyhledat zboží', exact: true }).fill('banketni pecivo')
    assert.equal(await page.locator('tbody tr').count(), 1)
    await row('Banketní houska se sýrem').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await banquet().isChecked(), true)
    assert.equal(await minimum().inputValue(), '120')
    assert.equal(await minimum().getAttribute('max'), null)
    await minimum().fill('70000')
    await saveProduct()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(productSaves.at(-1).minimum_quantity, 70000)
    await row('Banketní houska se sýrem').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await minimum().inputValue(), '70000')
    checks.push('banquet minimum has no 99 cap, requires positive integers and saves and restores 120 and 70000')

    await minimum().fill('70001')
    failProduct = 409
    await saveProduct()
    await page.locator('[data-form-error]').waitFor()
    assert.equal(await banquet().isChecked(), true)
    assert.equal(await minimum().inputValue(), '70001')
    assert.equal(await page.getByRole('button', { name: 'Uložit zboží', exact: true }).isEnabled(), true)
    await banquet().uncheck()
    assert.equal(await minimum().evaluate(node => node.required), false)
    assert.equal(await minimum().getAttribute('min'), '0')
    assert.equal(await minimum().getAttribute('max'), '99')
    assert.equal(await minimum().inputValue(), '70001')
    const beforeOrdinarySave = productSaves.length
    await saveProduct()
    assert.equal(await minimum().evaluate(node => node.checkValidity()), false)
    assert.equal(productSaves.length, beforeOrdinarySave)
    await minimum().fill('')
    await saveProduct()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(productSaves.at(-1).banquet, false)
    assert.equal(productSaves.at(-1).minimum_quantity, 0)
    checks.push('product conflict keeps draft and unchecking makes minimum optional')

    await page.getByRole('link', { name: 'Obecné nastavení webu', exact: true }).click()
    await total().waitFor()
    assert.equal(await total().inputValue(), '30')
    assert.equal(await total().getAttribute('min'), '1')
    assert.equal(await total().getAttribute('step'), '1')
    assert.equal(await total().evaluate(node => node.required), true)
    assert.deepEqual(await page.locator('.banquet-settings .hint').allTextContents(), ['Počet kusů za celou objednávku. Sčítají se pouze výrobky označené jako banketní pečivo. Objednávky bez banketního pečiva tento limit nemají.'])
    assert.equal(await page.getByRole('button', { name: 'Obnovit minimum', exact: true }).count(), 0)
    await page.getByLabel('Název provozovatele', { exact: true }).fill('Rozepsaný provozovatel')
    const originalSettings = structuredClone(settings)
    await total().fill('40')
    assert.equal((await saveTotal()).status(), 200)
    assert.equal(settings.banquet_minimum_quantity, 40)
    assert.deepEqual(settingsSaves.at(-1), { ...originalSettings, banquet_minimum_quantity: 40 })
    assert.equal(await page.getByLabel('Název provozovatele', { exact: true }).inputValue(), 'Rozepsaný provozovatel')
    await total().fill('1')
    assert.equal((await saveTotal()).status(), 200)
    assert.equal(settings.banquet_minimum_quantity, 1)
    const beforeInvalidTotal = settingsSaves.length
    for (const value of ['', '0', '-1', '1.5', '4951']) {
      await total().fill(value)
      await page.getByRole('button', { name: 'Uložit minimum banketního pečiva', exact: true }).click()
      assert.equal(await total().evaluate(node => node.checkValidity()), false)
      assert.equal(settingsSaves.length, beforeInvalidTotal)
      await page.locator('.banquet-settings').evaluate(form => form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })))
      await page.locator('.banquet-settings [data-form-error]').getByText('Společné minimum banketního pečiva musí být celé číslo od 1 do 4950 kusů.', { exact: true }).waitFor()
      assert.equal(settingsSaves.length, beforeInvalidTotal, `JavaScript validation also rejects ${value} when browser validation is bypassed`)
      assert.equal(settings.banquet_minimum_quantity, 1)
    }
    await total().fill('4950')
    assert.equal((await saveTotal()).status(), 200)
    assert.equal(settings.banquet_minimum_quantity, 4950)
    checks.push('single description paragraph and no minimum refresh button, HTML and JavaScript validate integer 1..4950 with both boundaries accepted')
    checks.push('saving the minimum preserves all delivery settings and the other form draft')

    await total().fill('50')
    failSettings = 500
    assert.equal((await saveTotal()).status(), 500)
    assert.equal(await total().inputValue(), '50')
    assert.equal(await total().isEnabled(), true)
    settings = { ...settings, version: settings.version + 1, banquet_minimum_quantity: 60, delivery_fee_cents: 5900 }
    assert.equal((await saveTotal()).status(), 409)
    await page.locator('.banquet-settings [data-form-error]').getByText('Obnovit v záhlaví stránky.', { exact: false }).waitFor()
    assert.equal(await total().inputValue(), '50')
    assert.equal(await page.getByLabel('Název provozovatele', { exact: true }).inputValue(), 'Rozepsaný provozovatel')
    await page.locator('.page-head').getByRole('button', { name: 'Obnovit', exact: true }).click()
    await page.waitForFunction(() => document.querySelector('[name=banquet_minimum_quantity]')?.value === '60')
    assert.equal(await page.locator('.banquet-settings [data-form-error]').count(), 0)
    await page.getByLabel('Název provozovatele', { exact: true }).fill('Rozepsaný provozovatel')
    await total().fill('70')
    assert.equal((await saveTotal()).status(), 200)
    assert.equal(settings.delivery_fee_cents, 5900)
    checks.push('settings errors preserve drafts and stale versions recover through the existing page refresh action')

    await total().fill('80')
    await page.getByRole('button', { name: 'Uložit a zveřejnit', exact: true }).click()
    await page.getByText('Nastavení webu bylo zveřejněno.', { exact: true }).waitFor()
    assert.equal(await total().inputValue(), '80')
    assert.equal(settings.banquet_minimum_quantity, 70)
    assert.equal(site.seller_name, 'Rozepsaný provozovatel')
    await page.getByLabel('Název provozovatele', { exact: true }).fill('Podruhé uložený provozovatel')
    await page.getByRole('button', { name: 'Uložit a zveřejnit', exact: true }).click()
    await page.waitForFunction(() => !document.querySelector('.site-settings-editor [type=submit]').disabled)
    assert.equal(site.version, 3)
    assert.equal(site.seller_name, 'Podruhé uložený provozovatel')
    checks.push('site photo and contacts save preserves unsaved banquet minimum and advances site version')

    await page.screenshot({ path: path.join(output, 'settings-desktop.png'), fullPage: true })
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Settings fit ${width}px`)
      await page.screenshot({ path: path.join(output, `settings-${width}.png`), fullPage: true })
    }
    await page.getByRole('link', { name: 'Zboží a ceny', exact: true }).click()
    await page.getByRole('button', { name: 'Vymazat hledání', exact: true }).click()
    await row('Banketní houska se sýrem').getByRole('button', { name: 'Upravit', exact: true }).click()
    await banquet().check()
    await minimum().fill('120')
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Product dialog fits320px')
    await page.screenshot({ path: path.join(output, 'product-320.png'), fullPage: true })
    assert.deepEqual(errors, [])
    checks.push('320px and390px responsive settings and product dialog with no unhandled JavaScript errors')
    const result = { result: 'passed', checks, screenshots: output }
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2))
    console.log(JSON.stringify(result, null, 2))
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)) }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
