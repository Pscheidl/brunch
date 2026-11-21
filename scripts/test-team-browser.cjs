// Browser regressions run against an isolated static server and mocked APIs.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const http = require('node:http')
const fs = require('node:fs')
const path = require('node:path')
const root = path.resolve(__dirname, '../public')
const output = path.resolve(__dirname, '../output/team-browser')
fs.mkdirSync(output, { recursive: true })
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, 'http://127.0.0.1').pathname
  if (pathname === '/') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><html lang="cs"><title>Checkout test</title><h1>Kontrola objednávky</h1></html>'); return }
  const file = path.resolve(root, `.${pathname}`, pathname.endsWith('/') ? 'index.html' : '')
  if (!file.startsWith(root + path.sep)) { res.writeHead(403); res.end(); return }
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }
  res.setHeader('content-type', types[path.extname(file)] || 'application/octet-stream')
  fs.readFile(file, (error, bytes) => { if (error) { res.writeHead(404); res.end() } else res.end(bytes) })
})
const products = [{ id: 1, name: 'Sendvič', active: true, variants: ['Sýr', 'Šunka'], stock: null, minimum_quantity: 120, banquet: true, price_cents: 3500, image: '' }, { id: 2, name: 'Fresh', active: true, variants: [], stock: null, minimum_quantity: 0, price_cents: 9000, image: '' }, { id: 3, name: 'Banketní rohlík', active: true, variants: [], stock: 150, minimum_quantity: 1, banquet: true, price_cents: 2000, image: '' }]
const state = { team: null, contributions: [], token: null, savedKeys: [], banquetMinimum: 30 }
async function mock(context, authenticated) {
  await context.route('**/api/**', async route => {
    const request = route.request()
    const pathname = new URL(request.url()).pathname
    const method = request.method()
    let body
    if (method !== 'GET') body = request.postDataJSON()
    let result
    if (pathname === '/api/products') result = products
    else if (pathname === '/api/settings') result = { banquet_minimum_quantity: state.banquetMinimum }
    else if (pathname === '/api/auth/me') {
      if (!authenticated) { await route.fulfill({ status: 401, json: { error: 'Přihlaste se.' } }); return }
      result = { user: { id: 'owner', email: 'owner@example.test' } }
    } else if (pathname === '/api/team-orders' && method === 'GET') result = state.team ? [state.team] : []
    else if (pathname === '/api/team-orders' && method === 'POST') {
      assert.equal(request.headers()['x-brunch-request'], '1')
      state.team = { id: 'team-test', name: body.name, deadline: new Date(Date.now() + 3600000).toISOString(), invite_token: 'a'.repeat(64), closed_at: null, order_id: null }
      result = state.team
    } else if (pathname === '/api/team-orders/team-test') result = { team: state.team, contributions: state.contributions, products }
    else if (pathname === `/api/team-joins/${'a'.repeat(64)}` && method === 'GET') result = { id: state.team.id, name: state.team.name, deadline: state.team.deadline, closed: !!state.team.closed_at, ordered: !!state.team.order_id, accepting: !state.team.closed_at, contributor_count: state.contributions.length, own: request.headers()['x-team-contribution'] === state.token ? state.contributions[0] : null }
    else if (pathname === `/api/team-joins/${'a'.repeat(64)}` && method === 'PUT') {
      state.token = body.contribution_token || body.client_key
      assert.match(state.token, /^[a-f0-9]{64}$/)
      state.savedKeys.push(state.token)
      state.contributions = [{ id: 'contribution-test', name: body.name, items: body.items }]
      result = { id: 'contribution-test', contribution_token: state.token }
    } else if (pathname === '/api/team-orders/team-test/contributions/contribution-test') {
      state.contributions[0] = { ...state.contributions[0], ...body }
      result = { id: 'contribution-test' }
    } else if (pathname === '/api/team-orders/team-test/close') { state.team.closed_at = new Date().toISOString(); result = { closed: true } }
    else if (pathname === '/api/team-orders/team-test/reopen') { state.team.closed_at = null; result = { closed: false } }
    else if (pathname === '/api/team-orders/team-test/export') result = { team_id: state.team.id, items: state.contributions.flatMap(c => c.items), notes: 'Společná objednávka: tým.' }
    else { await route.fulfill({ status: 404, json: { error: `Unexpected API: ${pathname}` } }); return }
    await route.fulfill({ json: result })
  })
}
;(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true })
  const errors = []
  try {
    const owner = await browser.newContext({ viewport: { width: 1280, height: 900 } })
    await mock(owner, true)
    const organizer = await owner.newPage()
    organizer.on('pageerror', e => errors.push(e.message))
    await organizer.goto(`${base}/team/`)
    await organizer.getByLabel('Název košíku', { exact: true }).fill('Snídaně pro kolegy')
    await organizer.getByRole('button', { name: 'Vytvořit společný košík', exact: true }).click()
    await organizer.getByRole('heading', { name: 'Snídaně pro kolegy', exact: true }).waitFor()
    const joinUrl = await organizer.getByRole('textbox', { name: 'Odkaz pro kolegy', exact: true }).inputValue()
    assert.equal(joinUrl, `${base}/team/#join=${'a'.repeat(64)}`)
    const guest = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true })
    await mock(guest, false)
    const page = await guest.newPage()
    page.on('pageerror', e => errors.push(e.message))
    await page.goto(joinUrl)
    await page.getByLabel('Vaše jméno pro organizátora', { exact: true }).fill('Jana <test>')
    await page.getByText('Banketní pečivo má společný minimální odběr 30 ks za celý tým. Druhy lze kombinovat. Svůj díl můžete uložit i v menším množství.', { exact: true }).waitFor()
    assert.equal(await page.locator('.banquet-label').count(), 2)
    await page.getByText('· Množství bez omezení · minimum za tým 120 ks', { exact: true }).waitFor()
    const banquetQuantity = page.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true })
    assert.equal(await banquetQuantity.getAttribute('max'), '2147483647')
    await banquetQuantity.fill('2147483648')
    assert.equal(await banquetQuantity.evaluate(input => input.checkValidity()), false)
    const finiteQuantity = page.getByRole('spinbutton', { name: 'Počet: Banketní rohlík', exact: true })
    assert.equal(await finiteQuantity.getAttribute('max'), '150')
    await finiteQuantity.fill('151')
    assert.equal(await finiteQuantity.evaluate(input => input.checkValidity()), false)
    await finiteQuantity.fill('150')
    assert.equal(await finiteQuantity.evaluate(input => input.checkValidity()), true)
    await finiteQuantity.fill('0')
    const ordinaryQuantity = page.getByRole('spinbutton', { name: 'Počet: Fresh', exact: true })
    assert.equal(await ordinaryQuantity.getAttribute('max'), '99')
    await ordinaryQuantity.fill('100')
    assert.equal(await ordinaryQuantity.evaluate(input => input.checkValidity()), false)
    await ordinaryQuantity.fill('0')
    await page.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true }).fill('2')
    await page.getByRole('button', { name: 'Uložit výběr', exact: true }).click()
    await page.getByRole('alert').getByText('U vybraných položek prosím zvolte variantu.', { exact: true }).waitFor()
    await page.getByRole('combobox', { name: 'Varianta: Sendvič', exact: true }).selectOption('Sýr')
    await page.getByRole('button', { name: 'Uložit výběr', exact: true }).click()
    await page.getByText('Váš výběr je uložený. Do uzávěrky jej můžete upravit na této stránce.', { exact: true }).waitFor()
    assert.equal(state.contributions.length, 1)
    await page.reload()
    await page.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true }).waitFor()
    assert.equal(await page.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true }).inputValue(), '2')
    await page.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true }).fill('3')
    await page.getByRole('button', { name: 'Uložit výběr', exact: true }).click()
    await page.getByText('Váš výběr je uložený. Do uzávěrky jej můžete upravit na této stránce.', { exact: true }).waitFor()
    assert.equal(state.savedKeys[0], state.savedKeys[1], 'Editing reuses the private contribution key')
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Contributor mobile page fits the viewport')
    await page.screenshot({ path: `${output}/contributor-mobile.png`, fullPage: true })
    await organizer.getByRole('button', { name: 'Obnovit výběry', exact: true }).click()
    await organizer.getByRole('heading', { name: 'Jana <test>', exact: true }).waitFor()
    await organizer.getByText('Banketní pečivo za celý tým: 3 z 30 ks. Do společného minima přidejte ještě 27 ks banketního pečiva. Druhy můžete kombinovat.', { exact: true }).waitFor()
    state.banquetMinimum = 120
    await organizer.getByRole('button', { name: 'Obnovit výběry', exact: true }).click()
    await organizer.getByText('Banketní pečivo za celý tým: 3 z 120 ks. Do společného minima přidejte ještě 117 ks banketního pečiva. Druhy můžete kombinovat.', { exact: true }).waitFor()
    assert.equal(await organizer.locator('test').count(), 0, 'Participant name is rendered as text')
    await organizer.getByRole('button', { name: 'Upravit výběr', exact: true }).click()
    await organizer.getByRole('spinbutton', { name: 'Počet: Sendvič', exact: true }).fill('70000')
    await organizer.getByRole('button', { name: 'Uložit výběr', exact: true }).click()
    await organizer.getByText('70000× Sendvič (Sýr)', { exact: true }).waitFor()
    await organizer.getByText('Banketní pečivo za celý tým: 70000 z 120 ks. Společné minimum je splněno.', { exact: true }).waitFor()
    organizer.once('dialog', dialog => dialog.accept())
    await organizer.getByRole('button', { name: 'Uzavřít výběr kolegů', exact: true }).click()
    await organizer.getByRole('button', { name: 'Přenést do objednávky', exact: true }).waitFor()
    await organizer.screenshot({ path: `${output}/organizer-desktop.png`, fullPage: true })
    await page.reload()
    await page.getByText('Výběr je uzavřený. Případnou změnu domluvte přímo s organizátorem.', { exact: true }).waitFor()
    assert.equal(await page.getByRole('button', { name: 'Uložit výběr', exact: true }).count(), 0)
    await organizer.getByRole('button', { name: 'Přenést do objednávky', exact: true }).click()
    await organizer.waitForURL(`${base}/#team-checkout`)
    const imported = JSON.parse(await organizer.evaluate(() => sessionStorage.getItem('brunch-team-import')))
    assert.deepEqual(imported.items, [{ product_id: 1, quantity: 70000, variant: 'Sýr' }])
    assert.equal(imported.team_id, 'team-test')
    await page.goto(`${base}/team/`)
    await page.getByRole('link', { name: 'Přihlásit se a pokračovat', exact: true }).waitFor()
    assert.equal(await page.getByRole('link', { name: 'Přihlásit se a pokračovat', exact: true }).getAttribute('href'), '/?signin=team')
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ passed: ['organizer creates and shares cart', 'guest selects and validates variant', 'contribution persists and updates', 'private edit key reused', 'mobile without overflow', 'organizer edits named allocation', 'safe text rendering', 'closing locks participant UI', 'transfer to checkout with variant and 70000 units', 'unauthenticated organizer sign-in', 'banquet labels and partial participant selections', 'organizer banquet total and refreshed threshold', 'unlimited banquet and finite stock above 99', 'ordinary quantity still capped at 99', 'technical integer limit enforced'], screenshots: output }, null, 2))
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)) }
})().catch(error => { console.error(error); server.close(); process.exitCode = 1 })
