// Local, mocked browser checks for homepage photograph settings. No production requests.
const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const path = require('node:path')
const http = require('node:http')

async function main() {
  const root = path.resolve('public')
  const output = path.resolve('output/site-settings-admin-browser')
  await fs.mkdir(output, { recursive: true })
  const server = http.createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
      const filename = path.resolve(root, `.${pathname.endsWith('/') ? `${pathname}index.html` : pathname}`)
      if (!filename.startsWith(`${root}${path.sep}`)) throw new Error('Invalid path')
      const body = await fs.readFile(filename)
      const contentType = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.webp': 'image/webp' }[path.extname(filename)] || 'application/octet-stream'
      response.writeHead(200, { 'Content-Type': contentType })
      response.end(body)
    } catch { response.writeHead(404); response.end('Not found') }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, locale: 'cs-CZ' })
  const errors = []
  const saves = []
  const uploads = []
  const defaultImage = '/images/brunch-spread.webp'
  const defaultAlt = 'Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole'
  const customImage = '/api/images/12345678-1234-4234-9234-123456789012'
  let settings = { hero_image: '', hero_image_alt: defaultAlt, version: 1 }
  let uploadStarted
  const uploadedPhoto = await fs.readFile(path.join(root, 'images/brunch-spread.webp'))
  page.on('pageerror', error => errors.push(error.message))
  await page.route('**/api/**', async route => {
    const request = route.request()
    const url = new URL(request.url())
    const reply = (json, status = 200) => route.fulfill({ status, json })
    if (url.pathname === '/api/auth/me') return reply({ user: { id: 'admin', email: 'admin@example.test', is_admin: true } })
    if (url.pathname === '/api/admin/site-settings') {
      if (request.method() === 'GET') return reply(settings)
      const payload = request.postDataJSON()
      saves.push(payload)
      if (payload.version !== settings.version) return reply({ error: 'Nastavení mezitím změnil jiný správce.' }, 409)
      settings = { ...payload, version: settings.version + 1 }
      return reply(settings)
    }
    if (url.pathname === '/api/admin/images' && request.method() === 'POST') {
      uploads.push({ type: request.headers()['content-type'], size: request.postDataBuffer().length })
      return new Promise(resolve => uploadStarted(async options => {
        await route.fulfill(options)
        resolve()
      }))
    }
    if (url.pathname === customImage) return route.fulfill({ contentType: 'image/webp', body: uploadedPhoto })
    errors.push(`Unexpected API request ${request.method()} ${url.pathname}`)
    return route.abort()
  })
  const input = () => page.getByLabel('Nahrát hlavní fotografii', { exact: true })
  const preview = () => page.locator('.hero-settings-preview img')
  const description = () => page.getByLabel('Popis hlavní fotografie', { exact: true })
  const beginUpload = async () => {
    const started = new Promise(resolve => { uploadStarted = resolve })
    await input().setInputFiles(path.join(root, 'images/brunch-spread.webp'))
    return started
  }
  const save = async () => {
    const response = page.waitForResponse(item => item.url().endsWith('/api/admin/site-settings') && item.request().method() === 'PUT')
    await page.getByRole('button', { name: 'Uložit a zveřejnit', exact: true }).click()
    return response
  }
  try {
    await page.goto(`${base}/admin/#site-settings`)
    await page.getByRole('heading', { name: 'Obecné nastavení webu', exact: true }).waitFor()
    assert.equal(await preview().getAttribute('src'), defaultImage)
    assert.equal(await description().getAttribute('maxlength'), '500')
    assert.equal(await description().evaluate(node => node.readOnly), true)
    assert.equal(saves.length, 0)

    await input().evaluate(node => {
      const files = new DataTransfer()
      files.items.add(new File([new Uint8Array(40 * 1024 * 1024 + 1)], 'too-large.jpg', { type: 'image/jpeg' }))
      node.files = files.files
      node.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await page.getByText('Fotografie je příliš velká. Vyberte soubor do 40 MB.', { exact: true }).waitFor()
    assert.equal(uploads.length, 0, 'Oversized photos never leave the browser')
    assert.equal(await preview().getAttribute('src'), defaultImage)

    const failUpload = await beginUpload()
    await page.getByText('Nahrávám a upravuji fotografii…', { exact: true }).waitFor()
    assert.equal(await page.getByText('Fotografie je příliš velká. Vyberte soubor do 40 MB.', { exact: true }).count(), 0)
    assert.equal(await page.getByRole('button', { name: 'Uložit a zveřejnit', exact: true }).isDisabled(), true)
    assert.equal(await page.getByRole('button', { name: 'Použít výchozí fotografii', exact: true }).isDisabled(), true)
    assert.equal(await input().isDisabled(), true)
    await failUpload({ status: 503, json: { error: 'Testovací výpadek nahrávání.' } })
    await page.getByText('Testovací výpadek nahrávání.', { exact: true }).waitFor()
    assert.equal(await preview().getAttribute('src'), defaultImage)
    assert.equal(await input().isEnabled(), true)

    const finishUpload = await beginUpload()
    await finishUpload({ json: { url: customImage } })
    await page.getByText('Fotografie je nahraná.', { exact: false }).waitFor()
    assert.equal(await preview().getAttribute('src'), customImage)
    assert.equal(await preview().evaluate(node => getComputedStyle(node).objectFit), 'cover')
    assert.equal(await description().evaluate(node => node.readOnly), false)
    assert.equal(await description().inputValue(), 'Svačiny z pekárny Mysterium Gaudi')
    assert.equal(saves.length, 0, 'Upload is only a draft until saved')
    assert.ok(uploads.every(upload => upload.type === 'application/octet-stream' && upload.size === uploadedPhoto.length))
    await description().fill('  Čerstvé pečivo a ovoce na stole  ')
    assert.equal(await preview().getAttribute('alt'), 'Čerstvé pečivo a ovoce na stole')
    assert.equal((await save()).status(), 200)
    await page.getByText('Hlavní fotografie webu byla zveřejněna.', { exact: true }).waitFor()
    assert.deepEqual(saves[0], { hero_image: customImage, hero_image_alt: 'Čerstvé pečivo a ovoce na stole', version: 1 })

    await page.getByRole('button', { name: 'Použít výchozí fotografii', exact: true }).click()
    assert.equal(await preview().getAttribute('src'), defaultImage)
    assert.equal(await description().inputValue(), defaultAlt)
    assert.equal(saves.length, 1, 'Reset is only a draft until saved')
    assert.equal((await save()).status(), 200)
    await page.getByText('Hlavní fotografie webu byla zveřejněna.', { exact: true }).waitFor()
    assert.deepEqual(saves[1], { hero_image: '', hero_image_alt: defaultAlt, version: 2 })

    const conflictUpload = await beginUpload()
    await conflictUpload({ json: { url: customImage } })
    await page.getByText('Fotografie je nahraná.', { exact: false }).waitFor()
    await description().fill('Rozpracovaný popis fotografie')
    const replacementUpload = await beginUpload()
    await replacementUpload({ json: { url: customImage } })
    await page.getByText('Fotografie je nahraná.', { exact: false }).waitFor()
    assert.equal(await description().inputValue(), 'Rozpracovaný popis fotografie', 'A new upload preserves a custom draft description')
    settings = { ...settings, version: 4 }
    assert.equal((await save()).status(), 409)
    await page.locator('[data-form-error]').getByText('Nastavení mezitím změnil jiný správce.', { exact: false }).waitFor()
    assert.equal(await preview().getAttribute('src'), customImage)
    assert.equal(await description().inputValue(), 'Rozpracovaný popis fotografie')
    assert.equal(await page.locator('#notice').isHidden(), true, 'A stale success notice must not obscure the current conflict')
    assert.equal(settings.hero_image, '', 'A stale save must not change the published image')
    assert.equal(saves[2].version, 3)
    await page.screenshot({ path: path.join(output, 'editor-desktop.png'), fullPage: true })
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Editor fits ${width}px`)
      await page.screenshot({ path: path.join(output, `editor-${width}.png`), fullPage: true })
    }
    assert.deepEqual(errors, [])
    console.log(JSON.stringify({ result: 'passed', checks: ['default photo and description', '40MB local limit', 'clear old upload error', 'pending upload controls', 'failed upload preserves photo', 'binary upload and cropped preview', 'explicit save', 'draft reset and save', 'custom description preserved on replacement', 'version conflict preserves draft', '390px and 320px layouts', 'no browser errors or deletion requests'], screenshots: output }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
