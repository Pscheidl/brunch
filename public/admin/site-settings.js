'use strict'

async function siteSettingsPage() {
  const settings = await api('/api/admin/site-settings')
  const defaultImage = '/images/brunch-spread.webp'
  const defaultAlt = 'Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole'
  let selectedImage = settings.hero_image || ''
  let uploading = false
  let saving = false
  const description = field('Popis hlavní fotografie', 'hero_image_alt', settings.hero_image_alt || defaultAlt, {
    required: true, maxlength: 500,
    hint: 'Krátce popište, co je na fotografii. Text pomůže lidem, kteří obrázek nevidí.'
  })
  const altInput = description.querySelector('input')
  altInput.readOnly = !selectedImage
  const descriptionHint = description.querySelector('.hint')
  if (!selectedImage) descriptionHint.textContent = 'Výchozí fotografie má vlastní popis. Po nahrání jiné fotografie můžete popis upravit.'
  const image = el('img', { src: selectedImage || defaultImage, alt: altInput.value, width: 1536, height: 1024 })
  const preview = el('figure', { class: 'hero-settings-preview' }, image,
    el('figcaption', { class: 'hint' }, 'Náhled hlavní fotografie. Výřez se přizpůsobí velikosti obrazovky.'))
  const uploadStatus = el('p', { class: 'hint', role: 'status', id: 'hero-upload-status' }, 'JPG, PNG nebo WebP do 40 MB. Fotografii při nahrání automaticky zmenšíme a zkomprimujeme.')
  const upload = el('input', { type: 'file', accept: 'image/jpeg,image/png,image/webp', 'aria-label': 'Nahrát hlavní fotografii', 'aria-describedby': 'hero-upload-status' })
  const submit = el('button', { type: 'submit', class: 'button primary' }, 'Uložit a zveřejnit')
  const reset = button('Použít výchozí fotografii', () => {
    selectedImage = ''
    altInput.value = defaultAlt
    updatePreview()
    clearError()
    uploadStatus.textContent = 'Výchozí fotografie je vybraná. Změnu zveřejníte uložením.'
  })
  const form = el('form', { class: 'panel site-settings-editor', onsubmit: event => saveForm(event, async data => {
    notice.hidden = true
    const alt = String(data.get('hero_image_alt') || '').trim()
    if (!alt) throw new Error('Doplňte popis hlavní fotografie.')
    saving = true
    updateControls()
    try {
      await api('/api/admin/site-settings', { method: 'PUT', body: JSON.stringify({
        hero_image: selectedImage, hero_image_alt: alt, version: settings.version
      }) })
      notify('Hlavní fotografie webu byla zveřejněna.')
      await renderPage()
    } finally {
      saving = false
      updateControls()
    }
  }) },
  el('h2', {}, 'Hlavní fotografie úvodní stránky'),
  el('p', { class: 'section-note' }, 'Vyberte fotografii na šířku, ideálně v poměru 3 : 2. Nová fotografie se na webu zobrazí po uložení.'),
  preview,
  el('div', { class: 'field' }, el('label', {}, el('span', {}, 'Nahrát hlavní fotografii'), upload), uploadStatus),
  description, el('div', { class: 'actions' }, reset, submit))

  function updatePreview() {
    image.src = selectedImage || defaultImage
    image.alt = altInput.value.trim()
    altInput.readOnly = !selectedImage
    descriptionHint.textContent = selectedImage ? 'Krátce popište, co je na fotografii. Text pomůže lidem, kteří obrázek nevidí.' : 'Výchozí fotografie má vlastní popis. Po nahrání jiné fotografie můžete popis upravit.'
  }
  function clearError() {
    notice.hidden = true
    form.querySelector('[data-form-error]')?.remove()
    uploadStatus.className = 'hint'
    uploadStatus.setAttribute('role', 'status')
  }
  function updateControls() {
    submit.disabled = uploading || saving
    upload.disabled = uploading || saving
    reset.disabled = uploading || saving
    altInput.disabled = saving
  }
  altInput.addEventListener('input', updatePreview)
  upload.addEventListener('change', async () => {
    const file = upload.files[0]
    if (!file) return
    clearError()
    if (file.size > 40 * 1024 * 1024) {
      uploadStatus.textContent = 'Fotografie je příliš velká. Vyberte soubor do 40 MB.'
      uploadStatus.className = 'error'
      uploadStatus.setAttribute('role', 'alert')
      upload.value = ''
      return
    }
    uploading = true
    updateControls()
    uploadStatus.textContent = 'Nahrávám a upravuji fotografii…'
    try {
      const result = await api('/api/admin/images', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: file })
      if (!form.isConnected) return
      selectedImage = result.url
      if (altInput.value.trim() === defaultAlt) altInput.value = 'Svačiny z pekárny Mysterium Gaudi'
      updatePreview()
      uploadStatus.textContent = 'Fotografie je nahraná. Zkontrolujte její popis a pro zveřejnění ji uložte.'
    } catch (error) {
      uploadStatus.textContent = error.message
      uploadStatus.className = 'error'
      uploadStatus.setAttribute('role', 'alert')
    } finally {
      uploading = false
      updateControls()
      upload.value = ''
    }
  })
  return [header('Obecné nastavení webu', 'Vzhled hlavní části úvodní stránky.', button('Obnovit', renderPage)), form]
}
