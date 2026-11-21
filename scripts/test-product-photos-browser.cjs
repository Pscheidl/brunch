const { chromium } = require('playwright')
const assert = require('node:assert/strict')
const fs = require('node:fs/promises')
const http = require('node:http')

const html = `<!doctype html><html><head><meta charset="utf-8"><style>
.photo-frame{display:grid;grid-template:minmax(0,1fr)/minmax(0,1fr);place-items:center;overflow:hidden;width:300px;height:200px;background:white}
.photo-frame img{width:100%;height:100%;object-fit:scale-down;transform:translate(var(--photo-x,0px),var(--photo-y,0px)) scale(var(--photo-zoom,1))}
#detail{width:400px;height:300px}#hidden{display:none;width:240px;height:180px}
</style></head><body><div id="list" class="photo-frame"></div><div id="detail" class="photo-frame"></div><div id="hidden" class="photo-frame"></div>
<script>
window.samples=[]
const getPixels=CanvasRenderingContext2D.prototype.getImageData
CanvasRenderingContext2D.prototype.getImageData=function(...args){
  if(window.throwCanvasRead)throw new Error('Canvas is unavailable')
  window.samples.push({width:this.canvas.width,height:this.canvas.height})
  return getPixels.apply(this,args)
}
window.fixture=function(background='white',color='#854b16'){
  const canvas=document.createElement('canvas');canvas.width=512;canvas.height=768
  const ctx=canvas.getContext('2d');ctx.fillStyle=background;ctx.fillRect(0,0,512,768)
  ctx.fillStyle=color;ctx.fillRect(40,200,160,260)
  return canvas.toDataURL('image/png')
}
window.photo=fixture()
for(const id of ['list','detail','hidden']){
  const image=new Image();image.className='catalog-photo';image.src=photo
  document.getElementById(id).append(image)
}
</script><script defer src="/product-photos.js"></script></body></html>`

async function main() {
  const script = await fs.readFile('public/product-photos.js')
  const server = http.createServer((request, response) => {
    response.writeHead(200, { 'Content-Type': request.url === '/product-photos.js' ? 'application/javascript' : 'text/html' })
    response.end(request.url === '/product-photos.js' ? script : html)
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
  const errors = []
  const requests = []
  const checks = []
  try {
    const page = await browser.newPage()
    page.on('pageerror', error => errors.push(error.message))
    page.on('request', request => requests.push({ method: request.method(), url: request.url() }))
    await page.goto(`http://127.0.0.1:${server.address().port}`)
    const zoomed = id => page.waitForFunction(id => Number(document.querySelector(id + ' img').style.getPropertyValue('--photo-zoom')) >= 1.15, id)
    await zoomed('#list')
    await zoomed('#detail')
    const values = await page.evaluate(() => ['list', 'detail'].map(id => {
      const image = document.querySelector('#' + id + ' img')
      return { zoom: Number(image.style.getPropertyValue('--photo-zoom')), x: parseFloat(image.style.getPropertyValue('--photo-x')) }
    }))
    assert(values.every(value => value.zoom <= 3 && value.x > 0))
    assert.notEqual(values[0].x, values[1].x, 'source-space bounds produce separate geometry for list/detail')
    assert.equal(await page.evaluate(() => samples.length), 1, 'one sampled analysis for the same source in all frames')
    assert(await page.evaluate(() => samples.every(sample => Math.max(sample.width, sample.height) <= 256)))
    checks.push('Cached initial loads share bounded source analysis and use per-frame scale-down geometry')

    await page.evaluate(() => { document.querySelector('#list').style.width = '120px'; document.querySelector('#list').style.height = '160px' })
    await page.waitForFunction(previous => parseFloat(document.querySelector('#list img').style.getPropertyValue('--photo-x')) !== previous, values[0].x)
    await page.evaluate(() => { document.querySelector('#hidden').style.display = 'grid' })
    await zoomed('#hidden')
    checks.push('ResizeObserver recomputes a resized frame and an opened hidden detail')

    await page.evaluate(() => { document.querySelector('#list img').src = fixture('#6c8673') })
    await page.waitForFunction(() => {
      const image = document.querySelector('#list img')
      return image.complete && image.naturalWidth > 0 && image.style.getPropertyValue('--photo-zoom') === ''
    })
    assert.equal(await page.locator('#list img').evaluate(image => getComputedStyle(image).transform), 'matrix(1, 0, 0, 1, 0, 0)')
    checks.push('Replacing src with a background photograph removes the previous transform')

    await page.evaluate(() => { window.detached = document.querySelector('#detail img'); detached.remove() })
    await page.waitForFunction(() => detached.style.getPropertyValue('--photo-zoom') === '')
    await page.evaluate(() => document.querySelector('#detail').append(detached))
    await zoomed('#detail')
    checks.push('Detached images release observers and reinserted images are processed again')

    await page.evaluate(() => {
      window.throwCanvasRead = true
      const frame = document.createElement('div')
      frame.id = 'unreadable'
      frame.className = 'photo-frame'
      const image = new Image()
      image.className = 'catalog-photo'
      image.src = fixture('white', '#4422aa')
      frame.append(image)
      document.body.append(frame)
    })
    await page.waitForFunction(() => {
      const image = document.querySelector('#unreadable img')
      return image.complete && image.naturalWidth > 0
    })
    assert.equal(await page.locator('#unreadable img').evaluate(image => image.style.getPropertyValue('--photo-zoom')), '')
    await page.evaluate(() => { window.throwCanvasRead = false })
    checks.push('Unreadable canvas gracefully keeps the original framing')
    assert.deepEqual(errors, [])
    assert(requests.every(request => request.method === 'GET' && new URL(request.url).hostname === '127.0.0.1'), 'analysis makes no external network calls')
    console.log(JSON.stringify({ passed: checks, errors }, null, 2))
  } finally {
    await browser.close()
    await new Promise(resolve => server.close(resolve))
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
