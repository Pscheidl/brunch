'use strict'

async function siteSettingsPage() {
  const [settings, shopSettings] = await Promise.all([api('/api/admin/site-settings'), api('/api/admin/settings')])
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
    form.querySelector('[data-form-error]')?.remove()
    const alt = String(data.get('hero_image_alt') || '').trim()
    if (!alt) throw new Error('Doplňte popis hlavní fotografie.')
    saving = true
    updateControls()
    try {
      Object.assign(settings, await api('/api/admin/site-settings', { method: 'PUT', body: JSON.stringify({
        hero_image: selectedImage, hero_image_alt: alt, version: settings.version,
        ...Object.fromEntries(['public_contact_email', 'public_contact_phone', 'seller_name', 'seller_address', 'seller_ico', 'delivery_area_label'].map(key => [key, String(data.get(key) || '').trim()]))
      }) }))
      uploadStatus.textContent = 'Fotografie a údaje webu jsou uložené.'
      notify('Nastavení webu bylo zveřejněno.')
    } finally {
      saving = false
      updateControls()
    }
  }) },
  el('h2', {}, 'Hlavní fotografie úvodní stránky'),
  el('p', { class: 'section-note' }, 'Vyberte fotografii na šířku, ideálně v poměru 3 : 2. Nová fotografie se na webu zobrazí po uložení.'),
  preview,
  el('div', { class: 'field' }, el('label', {}, el('span', {}, 'Nahrát hlavní fotografii'), upload), uploadStatus),
  description,
  el('section', { class: 'contact-fields' }, el('h2', {}, 'Veřejné kontakty a provozovatel'),
    el('p', { class: 'section-note' }, 'Tyto údaje se zobrazí zákazníkům na webu. Vyplňte skutečné ověřené údaje. Znění obchodních podmínek upravte zvlášť v jejich sekci, aby zůstala zachována historie přijatých verzí.'),
    el('div', { class: 'form-grid' },
      field('Název provozovatele', 'seller_name', settings.seller_name || '', { maxlength: 200 }),
      field('IČO provozovatele', 'seller_ico', settings.seller_ico || '', { maxlength: 8, inputmode: 'numeric', pattern: '[0-9]{8}' }),
      field('Sídlo provozovatele', 'seller_address', settings.seller_address || '', { kind: 'textarea', maxlength: 500, full: true }),
      field('Veřejný kontaktní e-mail', 'public_contact_email', settings.public_contact_email || '', { type: 'email', maxlength: 254 }),
      field('Veřejný kontaktní telefon', 'public_contact_phone', settings.public_contact_phone || '', { type: 'tel', maxlength: 40 }),
      field('Oblast rozvozu pro zákazníky', 'delivery_area_label', settings.delivery_area_label || '', { maxlength: 200, full: true, placeholder: 'Například Praha 1 až 5', hint: 'Krátký srozumitelný popis. Skutečnou dostupnost stále určují povolená PSČ v sekci Oblasti rozvozu.' })
    )), el('div', { class: 'actions' }, reset, submit))

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
  return [header('Obecné nastavení webu', 'Hlavní fotografie, veřejné kontakty a pravidla objednávek.', button('Obnovit', renderPage)), banquetMinimumForm(shopSettings), form]
}

function banquetMinimumForm(settings) {
  const minimum = field('Minimální množství banketního pečiva celkem', 'banquet_minimum_quantity', settings.banquet_minimum_quantity ?? 30, {
    type: 'number', min: 1, max: 4950, step: 1, required: true,
    hint: 'Počet kusů za celou objednávku. Sčítají se pouze výrobky označené jako banketní pečivo. Objednávky bez banketního pečiva tento limit nemají.'
  })
  const input = minimum.querySelector('input')
  const hint = minimum.querySelector('.hint')
  hint.id = 'banquet-minimum-hint'
  input.setAttribute('aria-describedby', hint.id)
  const form = el('form', { class: 'panel settings-panel banquet-settings', 'aria-labelledby': 'banquet-settings-title', onsubmit: event => saveForm(event, async data => {
    notice.hidden = true
    form.querySelector('[data-form-error]')?.remove()
    const quantity = Number(data.get('banquet_minimum_quantity'))
    if (!Number.isInteger(quantity) || quantity < 1 || quantity > 4950) throw new Error('Společné minimum banketního pečiva musí být celé číslo od 1 do 4950 kusů.')
    input.disabled = true
    refresh.disabled = true
    try {
      const payload = {
        delivery_fee_cents: settings.delivery_fee_cents, free_delivery_from_cents: settings.free_delivery_from_cents,
        order_email: settings.order_email, additional_order_emails: settings.additional_order_emails,
        delivery_slots: settings.delivery_slots, version: settings.version, banquet_minimum_quantity: quantity
      }
      Object.assign(settings, await api('/api/admin/settings', { method: 'PUT', body: JSON.stringify(payload) }))
      input.value = settings.banquet_minimum_quantity
      notify('Minimum banketního pečiva bylo uloženo.')
    } catch (error) {
      if (error.status === 409) error.message = 'Nastavení mezitím změnil jiný správce. Načtěte aktuální hodnotu tlačítkem Obnovit minimum. Rozepsané údaje zatím zůstávají ve formuláři.'
      throw error
    } finally { input.disabled = false; refresh.disabled = false }
  }) }, el('h2', { id: 'banquet-settings-title' }, 'Banketní pečivo'), minimum,
  el('p', { class: 'hint' }, 'Například 10 ks banketních housek se sýrem, 10 ks s lososem a 10 ks s paštikou splní společné minimum 30 kusů. Každý výrobek zároveň musí splnit vlastní minimální odběr.'),
  el('p', { class: 'hint' }, 'Změna se použije pro nové objednávky. Již přijaté objednávky zůstávají zachovány. Výchozí minimum je 30 kusů.'))
  const refresh = button('Obnovit minimum', async () => {
    refresh.disabled = true
    try {
      Object.assign(settings, await api('/api/admin/settings'))
      input.value = settings.banquet_minimum_quantity ?? 30
      form.querySelector('[data-form-error]')?.remove()
    } catch (error) { formError(form, error) }
    finally { refresh.disabled = false }
  })
  form.append(el('div', { class: 'actions' }, el('button', { type: 'submit', class: 'button primary' }, 'Uložit minimum banketního pečiva'), refresh))
  return form
}
