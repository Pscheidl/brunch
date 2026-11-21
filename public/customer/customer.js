'use strict'
const main = document.querySelector('#main')
const dialog = document.querySelector('#detail')
const state = { user: null, addresses: [], page: 1, query: '', generation: 0 }
const statuses = { awaiting_payment: 'Čeká na platbu', new: 'Přijata', confirmed: 'Termín potvrzen', out_for_delivery: 'Na cestě', delivered: 'Doručena', cancelled: 'Zrušena', expired: 'Vypršela' }
const requestStatuses = { pending: 'Čeká na vyřízení', resolved: 'Vyřízeno', rejected: 'Zamítnuto' }
const money = value => new Intl.NumberFormat('cs-CZ', { style: 'currency', currency: 'CZK' }).format(value / 100)
const date = value => new Intl.DateTimeFormat('cs-CZ', { dateStyle: 'long', timeZone: 'Europe/Prague' }).format(new Date(`${value}T12:00:00Z`))
const datetime = value => new Intl.DateTimeFormat('cs-CZ', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'Europe/Prague' }).format(new Date(value))
function el(tag, attributes = {}, ...children) {
  const element = document.createElement(tag)
  for (const [key, value] of Object.entries(attributes)) {
    if (key.startsWith('on')) element.addEventListener(key.slice(2), value)
    else if (value !== null && value !== false && value !== undefined) element.setAttribute(key, value === true ? '' : String(value))
  }
  element.append(...children.flat(Infinity).filter(child => child !== null && child !== undefined).map(child => child instanceof Node ? child : document.createTextNode(String(child))))
  return element
}
function button(text, action, type = '') { return el('button', { type: 'button', class: type, onclick: action }, text) }
function message(text, error = false) { return el('p', { class: error ? 'error' : 'notice', role: error ? 'alert' : 'status' }, text) }
function loginView(expired = false) {
  state.user = null
  state.addresses = []
  state.query = ''
  state.generation++
  if (dialog.open) dialog.close()
  dialog.replaceChildren()
  document.title = 'Přihlášení | Brunch Éra'
  main.replaceChildren(el('h1', {}, 'Přihlaste se ke svému účtu'),
    el('p', {}, expired ? 'Přihlášení vypršelo. Pro zobrazení objednávek a uložených údajů se znovu přihlaste.' : 'Pro objednávky, adresy a doklady ověřte svůj e-mail jednorázovým odkazem.'),
    el('a', { class: 'button primary', href: '/?signin=customer' }, 'Přihlásit se e-mailem'), el('a', { class: 'button', href: '/' }, 'Zpět na nabídku'))
  const heading = main.querySelector('h1')
  heading.tabIndex = -1
  heading.focus()
}
async function api(path, options = {}) {
  const response = await fetch(path, { credentials: 'same-origin', ...options, headers: { 'content-type': 'application/json', 'x-brunch-request': '1', ...options.headers } })
  const data = await response.json()
  if (!response.ok) {
    const error = new Error(data.error || 'Požadavek se nepodařil.')
    error.status = response.status
    if (response.status === 401) loginView(Boolean(state.user))
    throw error
  }
  return data
}
function field(label, name, value = '', options = {}) {
  const { multiline, full, ...attributes } = options
  const input = el(multiline ? 'textarea' : 'input', { name, ...attributes })
  input.value = value || ''
  return el('label', { class: full ? 'full' : '' }, label, input)
}
function openDialog(title) {
  const body = el('div', { class: 'dialog-body' })
  dialog.replaceChildren(el('div', { class: 'dialog-head' }, el('h2', { id: 'dialog-title' }, title), button('×', () => dialog.close(), 'dialog-close')), body)
  dialog.querySelector('.dialog-close').setAttribute('aria-label', 'Zavřít okno')
  if (!dialog.open) dialog.showModal()
  const heading = dialog.querySelector('#dialog-title')
  heading.tabIndex = -1
  heading.focus()
  return body
}
async function save(event, work) {
  event.preventDefault()
  const form = event.currentTarget
  const submit = form.querySelector('[type=submit]')
  if (submit.disabled) return
  submit.disabled = true
  form.querySelector('[data-error]')?.remove()
  try { await work(new FormData(form)) }
  catch (error) {
    const box = message(error.message, true)
    box.dataset.error = 'true'
    form.prepend(box)
  } finally { submit.disabled = false }
}
function setSession(key, value) {
  try { sessionStorage.setItem(key, JSON.stringify(value)) }
  catch { throw new Error('Pro použití uložených údajů povolte ukládání dat pro tento web.') }
}
function timeline(order) {
  const steps = ['new', 'confirmed', 'out_for_delivery', 'delivered']
  const position = steps.indexOf(order.status)
  return el('ol', { class: 'timeline', 'aria-label': 'Průběh objednávky' }, steps.map((step, index) => el('li', { class: index <= position ? 'done' : '', 'aria-current': step === order.status ? 'step' : null }, statuses[step])))
}
function activeCard(order) {
  return el('article', { class: 'card' }, el('div', { class: 'order-head' }, el('h3', {}, `Objednávka ${order.number}`), el('span', { class: 'status' }, statuses[order.status])),
    el('p', {}, `${date(order.delivery_date)} · ${order.delivery_slot}`),
    el('p', { class: 'muted' }, order.status === 'new' || order.status === 'awaiting_payment' ? 'Požadovaný termín čeká na potvrzení obsluhou.' : 'Termín doručení je potvrzený.'),
    el('p', {}, `${order.street}, ${order.postal_code} ${order.city}`), timeline(order),
    el('div', { class: 'actions' }, button('Detail a možnosti', () => orderDetail(order))))
}
async function loadOrders() {
  const target = document.querySelector('#order-list')
  const active = document.querySelector('#active-list')
  if (!target) return
  const generation = ++state.generation
  try {
    const data = await api(`/api/customer/orders?page=${state.page}&q=${encodeURIComponent(state.query)}`)
    if (generation !== state.generation || !state.user || !target.isConnected) return
    active.replaceChildren(...(data.active.length ? data.active.map(activeCard) : [el('p', { class: 'muted' }, 'Žádná objednávka nyní nečeká na doručení.')]))
    target.replaceChildren(...data.items.map(order => el('article', { class: 'card' },
      el('div', { class: 'order-head' }, el('strong', {}, order.number), el('span', { class: 'status' }, statuses[order.status]), el('strong', {}, money(order.total_cents))),
      el('p', {}, `${date(order.delivery_date)} · ${order.delivery_slot}`), el('p', { class: 'muted' }, `${order.street}, ${order.city}`),
      el('div', { class: 'actions' }, button('Detail, doklad a změny', () => orderDetail(order))))))
    if (!data.items.length) target.append(el('p', {}, state.query ? 'Žádná objednávka neodpovídá hledání.' : 'Zatím nemáte žádné objednávky.'))
    const previous = button('Předchozí', () => { state.page--; loadOrders() })
    previous.disabled = state.page <= 1
    const next = button('Další', () => { state.page++; loadOrders() })
    next.disabled = state.page * 20 >= data.total
    target.append(el('div', { class: 'pagination' }, previous, el('span', {}, `Strana ${state.page} z ${Math.max(1, Math.ceil(data.total / 20))} · ${data.total} objednávek`), next))
  } catch (error) { target.replaceChildren(message(error.message, true), button('Zkusit znovu', loadOrders)) }
}
async function loadAddresses() {
  const target = document.querySelector('#address-list')
  if (!target) return
  try {
    const addresses = await api('/api/customer/addresses')
    if (!state.user || !target.isConnected) return
    state.addresses = addresses
    target.replaceChildren(...state.addresses.map(address => el('article', { class: 'card' }, el('h3', {}, address.label),
      el('p', {}, `${address.name}${address.organization ? ` · ${address.organization}` : ''}`),
      el('p', {}, `${address.street}, ${address.postal_code} ${address.city}`), el('p', { class: 'muted' }, address.notes),
      el('div', { class: 'actions' }, button('Použít pro nákup', () => {
        try { setSession('brunch-selected-address', address); location.assign('/?checkout=1') }
        catch (error) { target.prepend(message(error.message, true)) }
      }, 'primary'), button('Upravit', () => addressForm(address)), button('Odstranit', () => deleteAddress(address), 'danger')))))
    if (!state.addresses.length) target.append(el('p', { class: 'muted' }, 'Uložte si kancelář nebo jiné místo, kam objednáváte.'))
  } catch (error) { target.replaceChildren(message(error.message, true), button('Zkusit znovu', loadAddresses)) }
}
function deleteAddress(address) {
  const body = openDialog(`Odstranit místo ${address.label}?`)
  body.append(el('p', {}, 'Dřívější objednávky zůstanou zachované.'), el('div', { class: 'actions' }, button('Ponechat', () => dialog.close()), button('Odstranit místo', async event => {
    const trigger = event.currentTarget
    trigger.disabled = true
    try { await api(`/api/customer/addresses/${address.id}`, { method: 'DELETE', body: JSON.stringify({ version: address.version }) }); dialog.close(); await loadAddresses() }
    catch (error) { if (body.isConnected) body.prepend(message(error.message, true)); trigger.disabled = false }
  }, 'danger')))
}
function addressForm(address = {}) {
  const body = openDialog(address.id ? 'Upravit místo doručení' : 'Nové místo doručení')
  body.append(el('form', { onsubmit: event => save(event, async form => {
    const payload = Object.fromEntries(form)
    if (address.id) payload.version = address.version
    await api(`/api/customer/addresses${address.id ? `/${address.id}` : ''}`, { method: address.id ? 'PUT' : 'POST', body: JSON.stringify(payload) })
    dialog.close()
    await loadAddresses()
  }) }, el('div', { class: 'grid' },
    field('Název místa', 'label', address.label, { maxlength: 80, required: true, placeholder: 'Kancelář', full: true }),
    field('Jméno příjemce', 'name', address.name, { maxlength: 200, required: true, autocomplete: 'name' }),
    field('Telefon pro kurýra', 'phone', address.phone, { maxlength: 40, required: true, type: 'tel', autocomplete: 'tel' }),
    field('Firma (nepovinné)', 'organization', address.organization, { maxlength: 200, autocomplete: 'organization', full: true }),
    field('Ulice a číslo domu', 'street', address.street, { maxlength: 250, required: true, autocomplete: 'street-address', full: true }),
    field('Město', 'city', address.city, { maxlength: 100, required: true, autocomplete: 'address-level2' }),
    field('PSČ', 'postal_code', address.postal_code, { maxlength: 8, required: true, inputmode: 'numeric', autocomplete: 'postal-code', pattern: '[0-9 ]{5,8}' }),
    field('Pokyny pro doručení', 'notes', address.notes, { maxlength: 2000, multiline: true, full: true })),
    el('p', { class: 'hint' }, 'Dostupnost rozvozu ověříme při nové objednávce.'), el('button', { type: 'submit', class: 'primary' }, 'Uložit místo')))
}
function billingForm(value = {}, onSave, buttonText = 'Uložit fakturační údaje') {
  return el('form', { onsubmit: event => save(event, async form => { await onSave({ ...Object.fromEntries(form), version: value.version || 0 }) }) },
    el('div', { class: 'grid' }, field('Odběratel', 'name', value.name, { maxlength: 250, required: true, full: true }),
      field('Fakturační adresa', 'address', value.address, { maxlength: 500, required: true, multiline: true, full: true }),
      field('IČO (nepovinné)', 'company_id', value.company_id, { maxlength: 24 }), field('DIČ (nepovinné)', 'tax_id', value.tax_id, { maxlength: 30 })),
    el('div', { class: 'actions section-gap' }, el('button', { type: 'submit', class: 'primary' }, buttonText)))
}
async function loadProfileBilling() {
  const target = document.querySelector('#billing-form')
  if (!target) return
  try {
    const value = await api('/api/customer/billing') || {}
    if (!state.user || !target.isConnected) return
    target.replaceChildren(billingForm(value, async data => {
      await api('/api/customer/billing', { method: 'PUT', body: JSON.stringify(data) })
      await loadProfileBilling()
      target.prepend(message('Výchozí fakturační údaje byly uloženy.'))
    }))
    if (value.version) target.append(button('Použít pro další objednávku', () => {
      try { setSession('brunch-selected-billing', value); location.assign('/?checkout=1') }
      catch (error) { target.prepend(message(error.message, true)) }
    }))
  } catch (error) { target.replaceChildren(message(error.message, true), button('Zkusit znovu', loadProfileBilling)) }
}
async function orderDetail(order) {
  const body = openDialog(`Objednávka ${order.number}`)
  body.append(el('p', { role: 'status' }, 'Načítáme podrobnosti…'))
  try {
    const [invoice, billing, requests] = await Promise.all([
      api(`/api/customer/orders/${order.id}/invoice`), api(`/api/customer/orders/${order.id}/billing`), api(`/api/customer/orders/${order.id}/requests`)
    ])
    if (!body.isConnected) return
    body.replaceChildren(el('span', { class: 'status' }, statuses[order.status]),
      el('p', {}, `${date(order.delivery_date)} · ${order.delivery_slot}`), el('p', {}, `${order.street}, ${order.postal_code} ${order.city}`),
      el('p', { class: 'address-lines' }, order.notes), timeline(order),
      order.items?.length ? el('ul', { class: 'order-items' }, order.items.map(item => el('li', {}, `${item.quantity}× ${item.name} · ${money(item.total_cents)}`))) : null,
      el('p', {}, `Celkem ${money(order.total_cents)}`),
      el('section', { class: 'section-gap' }, el('h3', {}, 'Faktura'), invoice ? el('a', { class: 'button primary', href: `/customer/?invoice=${encodeURIComponent(order.id)}` }, `Otevřít fakturu ${invoice.number}`) : el('p', { class: 'muted' }, 'Faktura zatím nebyla vystavena. Zde můžete doplnit její údaje.')))
    if (!invoice) {
      const details = el('details', {}, el('summary', {}, 'Fakturační údaje této objednávky'))
      const value = billing || { name: order.organization || order.name, address: `${order.street}\n${order.postal_code} ${order.city}` }
      details.append(billingForm(value, async data => { await api(`/api/customer/orders/${order.id}/billing`, { method: 'PUT', body: JSON.stringify(data) }); await orderDetail(order) }))
      body.append(details)
    }
    body.append(el('h3', { class: 'section-gap' }, 'Žádosti o změnu'))
    for (const request of requests) body.append(el('article', { class: 'request' }, el('strong', {}, `${request.kind === 'cancel' ? 'Zrušení' : 'Změna'} · ${requestStatuses[request.status]}`), el('p', { class: 'muted' }, datetime(request.created_at)), el('p', {}, request.message), request.response ? el('p', {}, `Odpověď: ${request.response}`) : null))
    if (!['delivered', 'cancelled', 'expired'].includes(order.status) && !requests.some(request => request.status === 'pending')) {
      body.append(el('div', { class: 'actions' }, button('Změnit doručení nebo údaje', () => requestForm(order, 'change')), button('Požádat o zrušení', () => requestForm(order, 'cancel'))))
    }
    body.append(el('p', { class: 'hint' }, 'Odeslaná žádost sama nemění objednávku. Výsledek vám obsluha potvrdí e-mailem.'))
  } catch (error) { if (body.isConnected) body.replaceChildren(message(error.message, true), button('Zkusit znovu', () => orderDetail(order))) }
}
function requestForm(order, kind) {
  const body = openDialog(kind === 'cancel' ? 'Žádost o zrušení' : 'Žádost o změnu')
  const key = crypto.randomUUID()
  body.append(el('p', {}, `Objednávka ${order.number}. Popište prosím požadovanou změnu. Obsluha potvrdí, zda ji lze provést.`),
    el('form', { onsubmit: event => save(event, async form => {
      await api(`/api/customer/orders/${order.id}/requests`, { method: 'POST', body: JSON.stringify({ idempotency_key: key, kind, message: form.get('message') }) })
      await orderDetail(order)
    }) }, field('Vaše žádost', 'message', '', { required: true, maxlength: 2000, multiline: true }), el('div', { class: 'actions section-gap' }, button('Zpět', () => orderDetail(order)), el('button', { type: 'submit', class: 'primary' }, 'Odeslat žádost'))))
}
function invoiceView(invoice) {
  const party = (label, person) => el('section', {}, el('h2', {}, label), el('strong', {}, person.name), el('p', { class: 'address-lines' }, person.address), person.company_id ? el('p', {}, `IČO ${person.company_id}`) : null, person.tax_id ? el('p', {}, `DIČ ${person.tax_id}`) : null, person.registration ? el('p', {}, person.registration) : null)
  const vat = invoice.supplier.vat_payer
  const taxes = new Map()
  const rows = invoice.lines.map(line => {
    if (vat) { const group = taxes.get(line.vat_rate) || { net: 0, tax: 0 }; group.net += line.net_cents; group.tax += line.vat_cents; taxes.set(line.vat_rate, group) }
    return el('tr', {}, el('td', {}, line.name), el('td', {}, `${line.quantity} ks`), el('td', { class: 'number' }, money(line.unit_price_cents)), vat ? el('td', {}, `${line.vat_rate / 100} %`) : null, el('td', { class: 'number' }, money(line.total_cents)))
  })
  const table = (heads, body) => el('div', { class: 'table-wrap' }, el('table', {}, el('thead', {}, el('tr', {}, heads.map(text => el('th', { scope: 'col' }, text)))), el('tbody', {}, body)))
  return el('article', { class: 'invoice panel' }, el('header', {}, el('h1', {}, `Faktura ${invoice.number}`), el('p', {}, `Objednávka ${invoice.order_number}`)),
    el('div', { class: 'invoice-parties' }, party('Dodavatel', invoice.supplier), party('Odběratel', invoice.customer)),
    el('p', {}, `Vystaveno ${date(invoice.issue_date)} · Plnění ${date(invoice.supply_date)} · Splatnost ${date(invoice.due_date)}`),
    table(['Položka', 'Množství', vat ? 'Cena za kus s DPH' : 'Cena za kus', ...(vat ? ['DPH'] : []), 'Celkem'], rows),
    vat ? el('section', {}, el('h2', {}, 'Rekapitulace DPH'), table(['Sazba', 'Základ', 'DPH', 'Celkem'], [...taxes.entries()].sort(([a], [b]) => a - b).map(([rate, group]) => el('tr', {}, el('td', {}, `${rate / 100} %`), el('td', {}, money(group.net)), el('td', {}, money(group.tax)), el('td', {}, money(group.net + group.tax)))))) : el('p', {}, 'Dodavatel není plátcem DPH.'),
    el('p', { class: 'invoice-total' }, `Celkem ${money(invoice.total_cents)}`),
    el('p', {}, `Způsob úhrady: ${{ card: 'Online kartou', card_on_delivery: 'Kartou při doručení', cash_on_delivery: 'Hotově při doručení' }[invoice.payment_method]}.`),
    el('p', {}, `Stav při vystavení: ${invoice.payment_status === 'paid' ? 'Zaplaceno' : 'Nezaplaceno'}.`),
    el('div', { class: 'actions no-print' }, el('a', { class: 'button', href: '/customer/' }, 'Zpět do účtu'), button('Vytisknout / uložit PDF', () => window.print(), 'primary')))
}
async function start() {
  try {
    const auth = await api('/api/auth/me')
    state.user = auth.user
    const invoiceId = new URL(location.href).searchParams.get('invoice')
    if (invoiceId) {
      if (!/^[0-9a-f-]{36}$/i.test(invoiceId)) throw new Error('Neplatný odkaz na fakturu.')
      const invoice = await api(`/api/customer/orders/${invoiceId}/invoice`)
      if (!invoice) throw new Error('Tato faktura zatím nebyla vystavena.')
      document.title = `Faktura ${invoice.number} | Brunch Éra`
      main.replaceChildren(invoiceView(invoice))
      return
    }
    main.replaceChildren(el('h1', {}, 'Můj účet'), el('p', { class: 'muted' }, state.user.email),
      el('nav', { class: 'tabs', 'aria-label': 'Sekce zákaznického účtu' }, el('a', { href: '#orders' }, 'Objednávky'), el('a', { href: '#addresses' }, 'Místa doručení'), el('a', { href: '#billing' }, 'Fakturační údaje'), el('a', { href: '/team/' }, 'Společná objednávka')),
      el('section', { id: 'orders' }, el('div', { class: 'heading-row' }, el('h2', {}, 'Nejbližší doručení'), button('Obnovit stav', loadOrders)), el('div', { id: 'active-list' }), el('h2', {}, 'Historie a doklady'),
        el('form', { class: 'search', onsubmit: event => { event.preventDefault(); state.query = new FormData(event.currentTarget).get('q').trim(); state.page = 1; loadOrders() } }, field('Hledat podle čísla, adresy nebo firmy', 'q', '', { maxlength: 100, type: 'search' }), el('button', { type: 'submit' }, 'Vyhledat')), el('div', { id: 'order-list' })),
      el('section', { id: 'addresses' }, el('div', { class: 'heading-row' }, el('h2', {}, 'Místa doručení'), button('Přidat místo', () => addressForm(), 'primary')), el('div', { id: 'address-list' })),
      el('section', { id: 'billing', class: 'panel' }, el('h2', {}, 'Výchozí fakturační údaje'), el('p', { class: 'hint' }, 'Použijte je pro další nákup. Údaje již vystavených faktur se nemění.'), el('div', { id: 'billing-form' })))
    await Promise.all([loadOrders(), loadAddresses(), loadProfileBilling()])
  } catch (error) {
    if (error.status === 401) return
    main.replaceChildren(el('h1', {}, 'Účet se nepodařilo otevřít'),
      el('p', {}, error.status === 401 ? 'Pro objednávky, adresy a doklady ověřte svůj e-mail jednorázovým odkazem.' : error.message),
      el('a', { class: 'button primary', href: '/?signin=customer' }, 'Přihlásit se e-mailem'), el('a', { class: 'button', href: '/' }, 'Zpět na nabídku'))
  }
}
setInterval(() => { if (!document.hidden && !dialog.open && document.querySelector('#order-list') && !document.activeElement?.closest('form')) loadOrders() }, 30000)
start()
