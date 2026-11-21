// Actual built storefront and public terms page with isolated API fixtures.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

const product = { id: 1, name: 'Testovací svačina', description: 'Test podmínek', category: 'Slané', tag: '', weight: '100 g', price_cents: 12000, vegan: false, ingredients: '', allergens: '', image: '', active: true, stock: 99, version: 1 }
const makeTerms = version => ({ version, title: 'Obchodní podmínky', content: '## 1. Objednávka\n\nObjednávka je závazná.\n\n## 2. Doručení\n\nRozvážíme v pracovní dny.', updated_at: '2026-11-21T12:00:00Z' })
const out = 'output/terms-browser'
const errors = []
const passed = []

async function staticServer() {
  const root = path.resolve('dist')
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const type = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.webp': 'image/webp', '.svg': 'image/svg+xml' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': type, 'Content-Security-Policy': "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self'; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'self'" })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  return { server, base: `http://127.0.0.1:${server.address().port}` }
}

async function setup(browser, base, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 1000 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(10000)
  page.on('pageerror', error => errors.push(error.message))
  await page.clock.setFixedTime(new Date('2026-11-21T12:00:00Z'))
  const api = { terms: [], orders: [], committed: new Map(), current: 1, failTerms: options.failTerms || false, stale: options.stale || false, lost: options.lost || false }
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/terms') {
      api.terms.push(url.search)
      if (api.failTerms === 'network') return route.abort('failed')
      if (api.failTerms) return json({ error: 'Nedostupné' }, 503)
      const version = url.searchParams.has('version') ? Number(url.searchParams.get('version')) : api.current
      const terms = makeTerms(version)
      if (options.literalContent) terms.content += '\n\n<img src=x onerror="window.injected=true">\n<script>window.injected=true</script>'
      return json(terms)
    }
    if (url.pathname === '/api/products') return json([product])
    if (url.pathname === '/api/settings') return json({ delivery_fee_cents: 0, free_delivery_from_cents: null, delivery_postal_codes: ['11000'], online_payment_enabled: false })
    if (url.pathname === '/api/delivery-options') return json({ dates: [{ value: '2026-11-23', label: 'Pondělí 23. listopadu' }], slots: ['09:00–10:00'] })
    if (url.pathname === '/api/auth/me') return json({ error: 'Nepřihlášen' }, 401)
    if (url.pathname === '/api/address-suggestions') return json([])
    if (url.pathname === '/api/orders') {
      const body = route.request().postDataJSON()
      api.orders.push(body)
      const confirmed = payload => json({ ...payload, id: 'e86f212a-0465-4c22-a2d8-1a6157d98f28', number: 'TEST-TERMS', status: 'new', payment_status: 'unpaid', subtotal_cents: 12000, delivery_fee_cents: 0, total_cents: 12000, items: [] })
      if (api.committed.has(body.idempotency_key)) {
        assert.deepEqual(body, api.committed.get(body.idempotency_key), 'recover the original committed payload')
        return confirmed(body)
      }
      if (api.lost && api.orders.length === 1) {
        if (!options.lostBeforeCommit) api.committed.set(body.idempotency_key, body)
        api.current = 2
        return route.abort('failed')
      }
      if (api.stale && api.orders.length === 1) {
        api.current = 2
        return json({ error: 'Obchodní podmínky se změnily. Přečtěte si prosím aktuální znění a potvrďte souhlas znovu.' }, 409)
      }
      if (body.terms_version !== api.current) return json({ error: 'Obchodní podmínky se změnily. Přečtěte si prosím aktuální znění a potvrďte souhlas znovu.' }, 409)
      api.committed.set(body.idempotency_key, body)
      return confirmed(body)
    }
    errors.push(`Unexpected request ${url.pathname}`)
    return json({ error: 'Unexpected' }, 500)
  })
  return { page, context, api }
}

async function review(page, base, addToCart = true) {
  await page.goto(base)
  await page.locator('.product-card').first().waitFor()
  if (addToCart) await page.locator('.product-card').first().getByRole('button', { name: /Přidat .* do košíku/ }).click()
  await page.locator('.header-cart').click()
  await page.getByRole('button', { name: 'Pokračovat k doručení', exact: true }).click()
  await page.locator('[name=name]').fill('Jana Testovací')
  await page.locator('[name=tel]').fill('+420 777 123 456')
  await page.locator('[name=email]').fill('test@example.test')
  await page.locator('[name=street-address]').fill('Testovací 12')
  await page.locator('[name=city]').fill('Praha')
  await page.locator('[name=postal-code]').fill('11000')
  await page.getByRole('button', { name: 'Zkontrolovat objednávku', exact: true }).click()
  await page.getByRole('heading', { name: 'Už jen poslední krok.', exact: true }).waitFor()
}

async function run(browser, base, name, options, work) {
  const test = await setup(browser, base, options)
  try {
    await work(test)
    passed.push(name)
  } finally {
    await test.page.unrouteAll({ behavior: 'ignoreErrors' })
    await test.context.close()
  }
}

async function main() {
  await fs.mkdir(out, { recursive: true })
  const { server, base } = await staticServer()
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  try {
    await run(browser, base, 'Terms outage preserves the cart and requires explicit acceptance after retry', { failTerms: true }, async ({ page, api }) => {
      await review(page, base)
      const submit = page.getByRole('button', { name: /Objednat s povinností platby/ })
      assert(await submit.isDisabled())
      assert.equal(await page.locator('.header-cart .cart-count').innerText(), '1')
      await submit.evaluate(button => { button.disabled = false; button.click() })
      assert.equal(api.orders.length, 0, 'JS submit guard also rejects bypassed disabled button')
      api.failTerms = false
      await page.getByRole('button', { name: 'Načíst obchodní podmínky znovu', exact: true }).click()
      const checkbox = page.locator('[name=terms-accepted]')
      await checkbox.waitFor()
      assert(!(await checkbox.isChecked()))
      await checkbox.check()
      assert(await submit.isEnabled())
      assert.equal(await page.locator('.terms-acceptance a').getAttribute('href'), '/obchodni-podminky/?version=1')
      await page.setViewportSize({ width: 390, height: 844 })
      await checkbox.scrollIntoViewIfNeeded()
      const box = await page.locator('.terms-acceptance').boundingBox()
      assert(box.x >= 0 && box.x + box.width <= 390)
      await page.screenshot({ path: `${out}/acceptance-mobile.png`, animations: 'disabled' })
      await submit.click()
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(api.orders[0].terms_version, 1)
    })

    await run(browser, base, 'Changed terms require renewed acceptance and link to the new exact version', { stale: true }, async ({ page, api }) => {
      await review(page, base)
      const checkbox = page.locator('[name=terms-accepted]')
      const submit = page.getByRole('button', { name: /Objednat s povinností platby/ })
      await checkbox.check()
      await submit.click()
      await page.waitForFunction(() => document.querySelector('.terms-acceptance a')?.getAttribute('href') === '/obchodni-podminky/?version=2')
      assert(!(await checkbox.isChecked()))
      assert(await submit.isDisabled())
      assert.equal(await page.locator('.header-cart .cart-count').innerText(), '1')
      await checkbox.check()
      await submit.click()
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.deepEqual(api.orders.map(order => order.terms_version), [1, 2])
      assert.notEqual(api.orders[0].idempotency_key, api.orders[1].idempotency_key)
    })

    await run(browser, base, 'Lost response keeps the accepted version and original idempotency key', { lost: true }, async ({ page, api }) => {
      await review(page, base)
      await page.locator('[name=terms-accepted]').check()
      const submit = page.getByRole('button', { name: /Objednat s povinností platby/ })
      await submit.click()
      await page.getByRole('alert').filter({ hasText: 'Spojení se nepodařilo' }).waitFor()
      await submit.click()
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(api.orders.length, 2)
      assert.deepEqual(api.orders[0], api.orders[1])
      assert.equal(api.orders[1].terms_version, 1)
    })

    for (const legacy of [false, true]) {
      await run(browser, base, `Reload after a lost committed response recovers one ${legacy ? 'legacy' : 'versioned'} order when terms change`, { lost: true }, async ({ page, api }) => {
        await review(page, base)
        await page.locator('[name=terms-accepted]').check()
        await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
        await page.getByRole('alert').filter({ hasText: 'Spojení se nepodařilo' }).waitFor()
        if (legacy) {
          // Represent an unresolved attempt written by the pre-terms storefront.
          await page.evaluate(() => {
            const [signature, key] = JSON.parse(sessionStorage.getItem('brunch-order-attempt'))
            const body = JSON.parse(signature)
            delete body.terms_version
            sessionStorage.setItem('brunch-order-attempt', JSON.stringify([JSON.stringify(body), key]))
          })
          const previous = { ...api.orders[0] }
          delete previous.terms_version
          api.committed.set(previous.idempotency_key, previous)
        }
        await review(page, base, false)
        assert.equal(await page.locator('.terms-acceptance a').getAttribute('href'), '/obchodni-podminky/?version=2')
        await page.locator('[name=terms-accepted]').check()
        await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
        await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
        assert.equal(api.orders.length, 2)
        assert.equal(api.orders[1].idempotency_key, api.orders[0].idempotency_key)
        assert.equal(api.orders[1].terms_version, legacy ? undefined : 1)
        assert.equal(api.committed.size, 1, 'no second order is committed after the reload')
      })
    }

    await run(browser, base, 'An unresolved uncommitted retry gets stale terms rejection before new explicit consent creates one order', { lost: true, lostBeforeCommit: true }, async ({ page, api }) => {
      await review(page, base)
      await page.locator('[name=terms-accepted]').check()
      await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await page.getByRole('alert').filter({ hasText: 'Spojení se nepodařilo' }).waitFor()
      await review(page, base, false)
      await page.locator('[name=terms-accepted]').check()
      await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await page.getByRole('alert').filter({ hasText: 'Obchodní podmínky se změnily' }).waitFor()
      await page.waitForFunction(() => document.querySelector('[name=terms-accepted]')?.checked === false)
      assert.equal(api.orders[1].idempotency_key, api.orders[0].idempotency_key)
      assert.equal(api.orders[1].terms_version, 1)
      assert.equal(api.committed.size, 0)
      await page.locator('[name=terms-accepted]').check()
      await page.getByRole('button', { name: /Objednat s povinností platby/ }).click()
      await page.getByRole('heading', { name: 'Objednávku jsme přijali.', exact: true }).waitFor()
      assert.equal(api.orders[2].terms_version, 2)
      assert.notEqual(api.orders[2].idempotency_key, api.orders[0].idempotency_key)
      assert.equal(api.committed.size, 1)
    })

    await run(browser, base, 'Public terms render literal safe content, exact versions, mobile and print layouts', { literalContent: true }, async ({ page, api }) => {
      await page.goto(`${base}/obchodni-podminky/?version=7`)
      await page.getByRole('heading', { name: '1. Objednávka', exact: true }).waitFor()
      assert.deepEqual(api.terms, ['?version=7'])
      assert.match(await page.locator('#terms-meta').innerText(), /Verze 7/)
      assert.match(await page.locator('#terms-content').innerText(), /<script>window.injected=true<\/script>/)
      assert.equal(await page.locator('#terms-content script, #terms-content img').count(), 0)
      assert.equal(await page.evaluate(() => window.injected), undefined)
      await page.screenshot({ path: `${out}/terms-desktop.png`, fullPage: true, animations: 'disabled' })
      await page.setViewportSize({ width: 390, height: 844 })
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth))
      await page.screenshot({ path: `${out}/terms-mobile.png`, fullPage: true, animations: 'disabled' })
      await page.emulateMedia({ media: 'print' })
      assert(await page.locator('.document-actions').isHidden())
      assert(await page.locator('#terms-content').isVisible())
    })

    await run(browser, base, 'Public terms recover from a network failure with a Czech message', { failTerms: 'network' }, async ({ page, api }) => {
      await page.goto(`${base}/obchodni-podminky/`)
      await page.getByRole('alert').waitFor()
      assert.match(await page.getByRole('alert').innerText(), /Obchodní podmínky se nepodařilo načíst/)
      assert(await page.locator('#terms-content').isHidden())
      api.failTerms = false
      await page.getByRole('button', { name: 'Zkusit načíst znovu', exact: true }).click()
      await page.getByRole('heading', { name: '1. Objednávka', exact: true }).waitFor()
      assert(await page.getByRole('alert').isHidden())
    })

    await run(browser, base, 'Homepage keeps the office panel and exposes terms without the removed top bar or navigation link', {}, async ({ page }) => {
      await page.goto(base)
      await page.locator('.product-card').waitFor()
      assert.equal(await page.locator('.top-note').count(), 0)
      assert.equal(await page.locator('.desktop-nav').getByRole('link', { name: 'Pro vaši kancelář', exact: true }).count(), 0)
      assert.equal(await page.locator('#do-kancelare').count(), 1)
      assert.equal(await page.locator('footer').getByRole('link', { name: 'Obchodní podmínky', exact: true }).getAttribute('href'), '/obchodni-podminky/')
    })
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ passed, errors }, null, 2))
  } finally {
    await fs.writeFile(`${out}/results.json`, JSON.stringify({ passed, errors }, null, 2))
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
