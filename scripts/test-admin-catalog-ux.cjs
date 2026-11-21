// Deterministic local browser tests for catalog, public contacts and customer requests.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/admin-catalog-ux')
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
  const errors = []
  const savedProducts = []
  let categories = [{ id: 1, name: 'Slané', product_count: 1, version: 1 }, { id: 2, name: 'Sladké', product_count: 1, version: 1 }]
  let products = [
    { id: 1, name: 'Šunka & sýr', description: 'Čerstvá svačina', category: 'Slané', weight: '66 g', ingredients: 'Pečivo, šunka, sýr', allergens: 'Lepek, mléko', price_cents: 3500, minimum_quantity: 0, stock: null, active: true, vegan: false, image: '', version: 1, variants: [] },
    { id: 2, name: 'Koláčky', description: 'Tvarohová náplň. Minimální odběr činí 30 kusů.', category: 'Sladké', weight: '40', ingredients: '', allergens: '', price_cents: 3500, minimum_quantity: 0, stock: null, active: true, vegan: false, image: '', version: 1, variants: [] },
    { id: 3, name: 'Archivované ovoce', description: 'Jablka a hrušky', category: '', weight: '100 g', ingredients: 'Jablka a hrušky', allergens: 'Bez alergenů', price_cents: 5500, minimum_quantity: 0, stock: 0, active: false, vegan: true, image: '', version: 1, variants: [] }
  ]
  let settings = { hero_image: '', hero_image_alt: '', version: 1, public_contact_email: '', public_contact_phone: '', seller_name: '', seller_address: '', seller_ico: '', delivery_area_label: '' }
  let requests = [{ id: 'request-1', order_id: 'order-1', order_number: 'BE-TEST', email: 'customer@example.test', kind: 'change', message: 'Prosím jiné patro <img src=x onerror=alert(1)>', status: 'pending', response: '', version: 1, created_at: '2026-11-21T10:00:00Z', resolved_at: null }]
  const patches = []
  page.on('pageerror', error => errors.push(error.message))
  page.on('dialog', prompt => prompt.accept())
  await page.route('**/api/**', async route => {
    const request = route.request()
    const url = new URL(request.url())
    const method = request.method()
    const reply = (json, status = 200) => route.fulfill({ json, status })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/products' && method === 'GET') return reply(products)
    if (/^\/api\/admin\/products\/\d+$/.test(url.pathname) && method === 'PUT') {
      const id = Number(url.pathname.split('/').pop())
      const payload = request.postDataJSON()
      savedProducts.push(payload)
      const next = { ...products.find(product => product.id === id), ...payload, version: payload.version + 1 }
      products = products.map(product => product.id === id ? next : product)
      return reply(next)
    }
    if (url.pathname === '/api/admin/categories') return reply(categories)
    if (/^\/api\/admin\/categories\/\d+$/.test(url.pathname)) {
      const id = Number(url.pathname.split('/').pop())
      const old = categories.find(category => category.id === id)
      if (method === 'PUT') {
        const payload = request.postDataJSON()
        const next = { ...old, name: payload.name, version: old.version + 1 }
        categories = categories.map(category => category.id === id ? next : category)
        products = products.map(product => product.category === old.name ? { ...product, category: next.name, version: product.version + 1 } : product)
        return reply(next)
      }
      if (method === 'DELETE') {
        categories = categories.filter(category => category.id !== id)
        products = products.map(product => product.category === old.name ? { ...product, category: '', version: product.version + 1 } : product)
        return reply({ ok: true })
      }
    }
    if (url.pathname === '/api/admin/site-settings') {
      if (method === 'GET') return reply(settings)
      settings = { ...request.postDataJSON(), version: settings.version + 1 }
      return reply(settings)
    }
    if (url.pathname === '/api/admin/orders/order-1') return reply({ id: 'order-1', number: 'BE-TEST', version: 1, name: 'Příjemce', organization: '', email: 'customer@example.test', phone: '+420 777 123 456', street: 'Doručovací 1', city: 'Praha', postal_code: '11000', delivery_date: '2026-11-23', delivery_slot: '07:00–09:00', status: 'new', payment_status: 'unpaid', payment_method: 'card_on_delivery', created_at: '2026-11-21T10:00:00Z', items: [{ product_id: 1, name: 'Šunka & sýr', quantity: 1, unit_price_cents: 3500, total_cents: 3500 }], subtotal_cents: 3500, delivery_fee_cents: 4900, total_cents: 8400 })
    if (url.pathname === '/api/admin/orders/order-1/invoice') return reply(null)
    if (url.pathname === '/api/admin/invoice-supplier') return reply({ name: 'Dodavatel', company_id: '12345678', address: 'Praha', vat_payer: false, version: 1 })
    if (url.pathname === '/api/admin/orders/order-1/billing') return reply({ name: 'Firma zákazníka', address: 'Fakturační 10\nBrno', company_id: '87654321', tax_id: 'CZ87654321', version: 1 })
    if (url.pathname === '/api/admin/customer-requests') return reply(requests)
    if (url.pathname === '/api/admin/customer-requests/request-1' && method === 'PATCH') {
      const payload = request.postDataJSON()
      patches.push(payload)
      requests = [{ ...requests[0], ...payload, version: payload.version + 1, resolved_at: '2026-11-21T11:00:00Z' }]
      return reply(requests[0])
    }
    errors.push(`Unexpected API request ${method} ${url.pathname}`)
    return route.abort()
  })
  const catalog = () => page.getByRole('link', { name: 'Zboží a ceny', exact: true }).click()
  const search = () => page.getByRole('searchbox', { name: 'Vyhledat zboží', exact: true })
  const row = name => page.getByRole('row').filter({ hasText: name })
  try {
    await page.goto(`${base}/admin/#products`)
    await search().waitFor()
    await page.getByText('Zobrazeno 3 z 3 položek', { exact: true }).waitFor()
    await search().fill('  SUNKA   syr  ')
    assert.equal(await page.locator('tbody tr').count(), 1)
    assert.ok((await page.locator('tbody').innerText()).includes('Šunka & sýr'))
    await search().fill('sladke')
    assert.equal(await page.locator('tbody tr').count(), 1)
    assert.ok((await page.locator('tbody').innerText()).includes('Koláčky'))
    await search().fill('tvarohova')
    assert.equal(await page.locator('tbody tr').count(), 1)
    await search().fill('neexistuje')
    await page.getByText('Žádné odpovídající zboží', { exact: true }).waitFor()
    await page.getByRole('button', { name: 'Vymazat hledání', exact: true }).click()
    assert.equal(await page.locator('tbody tr').count(), 3)
    await row('Koláčky').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await page.getByLabel('Kategorie', { exact: true }).evaluate(node => node.tagName), 'SELECT')
    assert.deepEqual(await page.getByLabel('Kategorie', { exact: true }).locator('option').allTextContents(), ['Bez kategorie', 'Sladké', 'Slané'])
    assert.equal(await page.getByLabel('Kategorie', { exact: true }).inputValue(), 'Sladké')
    assert.equal(await page.locator('.product-completeness li').count(), 4)
    await page.getByLabel('Složení', { exact: true }).fill('Mouka, tvaroh, vejce')
    await page.getByLabel('Alergeny', { exact: true }).fill('Lepek, mléko, vejce')
    await page.getByLabel('Hmotnost / objem', { exact: true }).fill('40 g')
    await page.getByLabel('Minimální odběr v kusech', { exact: true }).fill('30')
    await page.getByLabel('Varianty (jedna na řádek)', { exact: true }).fill('Tvarohové\nMakové')
    assert.equal(await page.locator('.product-completeness').isHidden(), true)
    await page.getByLabel('Kategorie', { exact: true }).selectOption('Slané')
    await page.getByRole('button', { name: 'Uložit zboží', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(savedProducts.at(-1).category, 'Slané')
    assert.equal(savedProducts.at(-1).minimum_quantity, 30)
    assert.deepEqual(savedProducts.at(-1).variants, ['Tvarohové', 'Makové'])
    await row('Koláčky').getByRole('button', { name: 'Upravit', exact: true }).click()
    await page.getByLabel('Kategorie', { exact: true }).selectOption('Sladké')
    await page.getByRole('button', { name: 'Uložit zboží', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })

    await page.getByRole('link', { name: 'Kategorie', exact: true }).click()
    await row('Sladké').getByRole('button', { name: 'Přejmenovat', exact: true }).click()
    await page.getByLabel('Název kategorie', { exact: true }).fill('Sladké pečivo')
    await page.getByRole('button', { name: 'Uložit kategorii', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    await catalog()
    await row('Koláčky').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await page.getByLabel('Kategorie', { exact: true }).inputValue(), 'Sladké pečivo')
    assert.equal(await page.getByLabel('Kategorie', { exact: true }).locator('option[value="Sladké"]').count(), 0)
    await page.getByRole('button', { name: 'Zrušit', exact: true }).click()
    await page.getByRole('link', { name: 'Kategorie', exact: true }).click()
    await row('Sladké pečivo').getByRole('button', { name: 'Odstranit', exact: true }).click()
    await page.getByText('Kategorie byla odstraněna.', { exact: true }).waitFor()
    await catalog()
    await row('Koláčky').getByRole('button', { name: 'Upravit', exact: true }).click()
    assert.equal(await page.getByLabel('Kategorie', { exact: true }).inputValue(), '')
    assert.deepEqual(await page.getByLabel('Kategorie', { exact: true }).locator('option').allTextContents(), ['Bez kategorie', 'Slané'])
    await page.getByRole('button', { name: 'Uložit zboží', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.equal(savedProducts.at(-1).category, '')
    await search().fill('sunka')
    await page.getByRole('button', { name: 'Obnovit', exact: true }).click()
    await search().waitFor()
    assert.equal(await search().inputValue(), 'sunka')
    assert.equal(await page.locator('tbody tr').count(), 1)
    await page.screenshot({ path: path.join(output, 'catalog-desktop.png'), fullPage: true })

    await page.getByRole('link', { name: 'Obecné nastavení webu', exact: true }).click()
    await page.getByLabel('Název provozovatele', { exact: true }).fill('Testovací pekárna')
    await page.getByLabel('IČO provozovatele', { exact: true }).fill('12345678')
    await page.getByLabel('Sídlo provozovatele', { exact: true }).fill('Testovací 1, Praha')
    await page.getByLabel('Veřejný kontaktní e-mail', { exact: true }).fill('info@example.test')
    await page.getByLabel('Veřejný kontaktní telefon', { exact: true }).fill('+420 777 123 456')
    await page.getByLabel('Oblast rozvozu pro zákazníky', { exact: true }).fill('Praha 1 až 5')
    await page.getByRole('button', { name: 'Uložit a zveřejnit', exact: true }).click()
    await page.getByText('Nastavení webu bylo zveřejněno.', { exact: true }).waitFor()
    assert.equal(settings.public_contact_email, 'info@example.test')
    assert.equal(settings.seller_name, 'Testovací pekárna')
    assert.equal(settings.delivery_area_label, 'Praha 1 až 5')

    await page.getByRole('link', { name: 'Žádosti zákazníků', exact: true }).click()
    await page.getByRole('heading', { name: 'Změna objednávky BE-TEST', exact: true }).waitFor()
    assert.equal(await page.locator('.request-message img').count(), 0)
    await page.getByRole('button', { name: 'Otevřít objednávku', exact: true }).click()
    await page.getByRole('button', { name: 'Faktura', exact: true }).click()
    await page.getByLabel('Odběratel', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Odběratel', { exact: true }).inputValue(), 'Firma zákazníka')
    assert.equal(await page.getByLabel('Adresa odběratele', { exact: true }).inputValue(), 'Fakturační 10\nBrno')
    assert.equal(await page.getByLabel('IČO odběratele', { exact: true }).inputValue(), '87654321')
    assert.equal(await page.getByLabel('DIČ odběratele', { exact: true }).inputValue(), 'CZ87654321')
    await page.getByRole('button', { name: 'Zavřít okno', exact: true }).click()
    await page.getByRole('button', { name: 'Vyřídit žádost', exact: true }).click()
    await page.getByLabel('Výsledek žádosti', { exact: true }).selectOption('rejected')
    await page.getByLabel('Odpověď zákazníkovi', { exact: true }).fill('Kurýr už vyjel. Domluvili jsme předání na recepci.')
    await page.getByRole('button', { name: 'Uložit a odeslat odpověď', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    assert.deepEqual(patches, [{ status: 'rejected', response: 'Kurýr už vyjel. Domluvili jsme předání na recepci.', version: 1 }])
    await page.getByLabel('Stav žádosti', { exact: true }).selectOption('rejected')
    await page.getByText('Kurýr už vyjel. Domluvili jsme předání na recepci.', { exact: false }).waitFor()
    assert.equal(await page.getByRole('button', { name: 'Vyřídit žádost', exact: true }).count(), 0)
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `No overflow at ${width}px`)
      await page.screenshot({ path: path.join(output, `requests-${width}.png`), fullPage: true })
    }
    await catalog()
    await page.getByRole('button', { name: 'Vymazat hledání', exact: true }).click()
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Catalog fits 320px')
    await page.screenshot({ path: path.join(output, 'catalog-320.png'), fullPage: true })
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ result: 'passed', checks: ['search names/categories/descriptions without diacritics', 'empty and clear state', 'search retained after reload', 'category select with blank value', 'category rename and delete reflected in select', 'live completeness checks', 'variants serialized', 'public contact save', 'customer billing prefilled in invoice', 'request escaped and resolved with response', '320/390px without overflow'], screenshots: output }, null, 2))
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)) }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
