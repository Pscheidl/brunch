/* Display-only white margin detection. Original files and image URLs stay unchanged. */
;(function (root) {
  'use strict'

  const MAX_SAMPLE_EDGE = 256
  const MAX_ZOOM = 3
  const SELECTOR = '.photo-frame img.catalog-photo'

  function analyzePixels(pixels, width, height) {
    if (width < 32 || height < 32 || pixels.length !== width * height * 4) return null
    const count = width * height
    const white = new Uint8Array(count)
    for (let i = 0; i < count; i++) {
      const at = i * 4
      const alpha = pixels[at + 3] / 255
      const red = pixels[at] * alpha + 255 * (1 - alpha)
      const green = pixels[at + 1] * alpha + 255 * (1 - alpha)
      const blue = pixels[at + 2] * alpha + 255 * (1 - alpha)
      const low = Math.min(red, green, blue)
      const high = Math.max(red, green, blue)
      white[i] = pixels[at + 3] <= 16 || (low >= 245 && high - low <= 10) ? 1 : 0
    }

    // Check every edge independently. A white sky or wall along just one side
    // does not make a lifestyle photograph a cutout.
    const edges = [[], [], [], []]
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const index = y * width + x
        if (y < 2) edges[0].push(index)
        if (y >= height - 2) edges[1].push(index)
        if (x < 2) edges[2].push(index)
        if (x >= width - 2) edges[3].push(index)
      }
    }
    if (edges.some(edge => edge.reduce((sum, i) => sum + white[i], 0) / edge.length < 0.98)) return null

    // Only white connected to the outer border is background. White enclosed
    // inside food, crockery or packaging remains part of the subject.
    const background = new Uint8Array(count)
    const queue = new Uint32Array(count)
    let head = 0
    let tail = 0
    const visit = index => {
      if (white[index] && !background[index]) {
        background[index] = 1
        queue[tail++] = index
      }
    }
    for (let x = 0; x < width; x++) {
      visit(x)
      visit((height - 1) * width + x)
    }
    for (let y = 0; y < height; y++) {
      visit(y * width)
      visit(y * width + width - 1)
    }
    while (head < tail) {
      const index = queue[head++]
      const x = index % width
      const y = Math.floor(index / width)
      if (x > 0) visit(index - 1)
      if (x + 1 < width) visit(index + 1)
      if (y > 0) visit(index - width)
      if (y + 1 < height) visit(index + width)
    }

    let left = width
    let right = -1
    let top = height
    let bottom = -1
    let foreground = 0
    for (let index = 0; index < count; index++) {
      if (background[index]) continue
      const x = index % width
      const y = Math.floor(index / width)
      left = Math.min(left, x)
      right = Math.max(right, x)
      top = Math.min(top, y)
      bottom = Math.max(bottom, y)
      foreground++
    }
    // Blank, nearly blank and border-touching subjects are too ambiguous.
    // Never discard small disconnected parts such as skewers or garnish.
    const subjectWidth = right - left + 1
    const subjectHeight = bottom - top + 1
    if (foreground < count * 0.01 || foreground > count * 0.7
      || subjectWidth < 8 || subjectHeight < 8
      || left < 2 || top < 2 || right >= width - 2 || bottom >= height - 2) return null
    const paddingX = Math.max(3, Math.ceil(subjectWidth * 0.1))
    const paddingY = Math.max(3, Math.ceil(subjectHeight * 0.1))
    const x = Math.max(0, left - paddingX)
    const y = Math.max(0, top - paddingY)
    const endX = Math.min(width, right + 1 + paddingX)
    const endY = Math.min(height, bottom + 1 + paddingY)
    if ((endX - x) * (endY - y) >= count * 0.8) return null
    return { x: x / width, y: y / height, width: (endX - x) / width, height: (endY - y) / height }
  }

  function computeTransform(bounds, naturalWidth, naturalHeight, frameWidth, frameHeight) {
    if (!bounds || ![naturalWidth, naturalHeight, frameWidth, frameHeight].every(value => Number.isFinite(value) && value > 0)) return null
    const fit = Math.min(frameWidth / naturalWidth, frameHeight / naturalHeight, 1)
    const cropWidth = bounds.width * naturalWidth
    const cropHeight = bounds.height * naturalHeight
    const target = Math.min(frameWidth / cropWidth, frameHeight / cropHeight, 1)
    const zoom = Math.min(MAX_ZOOM, target / fit)
    if (!Number.isFinite(zoom) || zoom < 1.15) return null
    return {
      zoom,
      x: (0.5 - bounds.x - bounds.width / 2) * naturalWidth * fit * zoom,
      y: (0.5 - bounds.y - bounds.height / 2) * naturalHeight * fit * zoom
    }
  }

  function install(document) {
    const cache = new Map()
    const tracked = new Map()
    let scheduled = false
    let stopped = false
    const reset = image => {
      image.style.removeProperty('--photo-zoom')
      image.style.removeProperty('--photo-x')
      image.style.removeProperty('--photo-y')
    }
    const paint = image => {
      const item = tracked.get(image)
      if (!item || !image.isConnected) return
      const transform = computeTransform(item.bounds, image.naturalWidth, image.naturalHeight, image.clientWidth, image.clientHeight)
      if (!transform) return reset(image)
      image.style.setProperty('--photo-zoom', transform.zoom.toFixed(5))
      image.style.setProperty('--photo-x', `${transform.x.toFixed(3)}px`)
      image.style.setProperty('--photo-y', `${transform.y.toFixed(3)}px`)
    }
    const analyze = image => {
      const naturalWidth = image.naturalWidth
      const naturalHeight = image.naturalHeight
      if (naturalWidth < 32 || naturalHeight < 32) return null
      const ratio = Math.min(MAX_SAMPLE_EDGE / Math.max(naturalWidth, naturalHeight), 1)
      const canvas = document.createElement('canvas')
      canvas.width = Math.max(1, Math.round(naturalWidth * ratio))
      canvas.height = Math.max(1, Math.round(naturalHeight * ratio))
      try {
        const context = canvas.getContext('2d', { willReadFrequently: true })
        if (!context) return null
        context.drawImage(image, 0, 0, canvas.width, canvas.height)
        return analyzePixels(context.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height)
      } catch {
        // A cross-origin or unsupported image remains in its ordinary frame.
        return null
      } finally {
        canvas.width = canvas.height = 0
      }
    }
    const refresh = image => {
      const item = tracked.get(image)
      if (!item) return
      const source = image.currentSrc || image.src
      const identity = `${source}\n${image.getAttribute('src') || ''}\n${image.getAttribute('srcset') || ''}`
      if (item.identity !== identity) {
        item.identity = identity
        item.bounds = null
        reset(image)
      }
      if (!image.complete || !image.naturalWidth) return
      if (!cache.has(source)) {
        cache.set(source, analyze(image))
        if (cache.size > 128) cache.delete(cache.keys().next().value)
      }
      item.bounds = cache.get(source)
      paint(image)
    }
    const resize = typeof root.ResizeObserver === 'function'
      ? new root.ResizeObserver(entries => entries.forEach(entry => refresh(entry.target)))
      : null
    const detach = image => {
      const item = tracked.get(image)
      if (!item) return
      image.removeEventListener('load', item.load)
      image.removeEventListener('error', item.error)
      resize?.unobserve(image)
      reset(image)
      tracked.delete(image)
    }
    const reconcile = () => {
      scheduled = false
      if (stopped) return
      for (const image of tracked.keys()) {
        if (!image.isConnected || !image.matches(SELECTOR)) detach(image)
      }
      for (const image of document.querySelectorAll(SELECTOR)) {
        if (!tracked.has(image)) {
          const item = { identity: '', bounds: null, load: () => refresh(image), error: () => reset(image) }
          tracked.set(image, item)
          image.addEventListener('load', item.load)
          image.addEventListener('error', item.error)
          resize?.observe(image)
        }
        refresh(image)
      }
    }
    const schedule = () => {
      if (!scheduled) {
        scheduled = true
        root.queueMicrotask(reconcile)
      }
    }
    const mutation = new root.MutationObserver(schedule)
    mutation.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['src', 'srcset', 'class'] })
    root.addEventListener('resize', schedule)
    reconcile()
    return () => {
      stopped = true
      mutation.disconnect()
      resize?.disconnect()
      root.removeEventListener('resize', schedule)
      for (const image of [...tracked.keys()]) detach(image)
      cache.clear()
    }
  }

  if (typeof module === 'object' && module.exports) module.exports = { analyzePixels, computeTransform }
  if (root.document && typeof root.MutationObserver === 'function') install(root.document)
})(typeof window === 'object' ? window : globalThis)
