const assert = require('node:assert/strict')
const { analyzePixels, computeTransform } = require('../public/product-photos.js')

function fixture(width = 128, height = 128, color = [255, 255, 255, 255]) {
  const pixels = new Uint8ClampedArray(width * height * 4)
  for (let i = 0; i < width * height; i++) pixels.set(color, i * 4)
  const fill = (x, y, w, h, value = [120, 70, 20, 255]) => {
    for (let yy = y; yy < y + h; yy++) for (let xx = x; xx < x + w; xx++) pixels.set(value, (yy * width + xx) * 4)
  }
  return { pixels, width, height, fill, bounds: () => analyzePixels(pixels, width, height) }
}

const centered = fixture()
centered.fill(44, 40, 40, 48)
const bounds = centered.bounds()
assert(bounds.x < 44 / 128 && bounds.y < 40 / 128)
assert(bounds.x + bounds.width > 84 / 128 && bounds.y + bounds.height > 88 / 128)
assert.deepEqual(computeTransform(bounds, 1280, 1280, 220, 220), { zoom: 220 / (bounds.height * 1280) / (220 / 1280), x: 0, y: 0 })

const offset = fixture()
offset.fill(15, 25, 30, 40)
offset.fill(18, 8, 2, 19) // Preserve a thin skewer, rather than deleting small parts.
const shifted = computeTransform(offset.bounds(), 1280, 1280, 400, 220)
assert(shifted.x > 0 && shifted.y > 0)
assert(offset.bounds().y < 8 / 128)
assert(shifted.zoom <= 3)
assert.notEqual(shifted.x, computeTransform(offset.bounds(), 1280, 1280, 100, 180).x)

for (const color of [[250, 249, 248, 255], [0, 0, 0, 0]]) {
  const image = fixture(128, 128, color)
  image.fill(44, 40, 40, 48)
  assert(image.bounds(), 'near-white and transparent outer whitespace can be detected')
}
const noisy = fixture()
for (let i = 0; i < noisy.pixels.length; i += 4) {
  const value = 245 + ((i / 4) % 11)
  noisy.pixels.set([value, value, value, 255], i)
}
noisy.fill(44, 40, 40, 48)
assert(noisy.bounds(), 'near-white JPEG noise remains background')

for (const color of [[230, 230, 230, 255], [240, 250, 220, 255], [80, 120, 60, 255]]) {
  const image = fixture(128, 128, color)
  image.fill(44, 40, 40, 48)
  assert.equal(image.bounds(), null, 'gray, colored and lifestyle backgrounds remain unchanged')
}
const touches = fixture()
touches.fill(0, 40, 40, 48)
assert.equal(touches.bounds(), null)
assert.equal(fixture().bounds(), null, 'all white is ambiguous')
const garnishOnly = fixture()
garnishOnly.fill(60, 60, 4, 8)
assert.equal(garnishOnly.bounds(), null, 'tiny dark garnish on otherwise pure white food is ambiguous')
const tiny = fixture(20, 20)
tiny.fill(7, 7, 6, 6)
assert.equal(tiny.bounds(), null)
const plate = fixture()
plate.fill(30, 30, 68, 68, [244, 244, 244, 255])
plate.fill(32, 32, 64, 64, [255, 255, 255, 255])
plate.fill(55, 55, 18, 18)
assert(plate.bounds().x < 30 / 128 && plate.bounds().width > 68 / 128, 'white interior enclosed by a subtle rim is preserved')
assert.equal(computeTransform(bounds, 128, 128, 500, 500), null, 'small originals are never enlarged above native resolution')
assert.equal(computeTransform(bounds, 1280, 1280, 0, 0), null, 'hidden frames wait until visible')
assert.equal(computeTransform({ x: 0.05, y: 0.05, width: 0.9, height: 0.9 }, 1000, 1000, 220, 220), null, 'insignificant margins remain unchanged')
console.log('Passed product photo analysis and geometry tests')
