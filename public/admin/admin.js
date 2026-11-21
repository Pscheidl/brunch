'use strict'

const content = document.querySelector('#content')
const nav = document.querySelector('#navigation')
const identity = document.querySelector('#identity')
const dialog = document.querySelector('#editor')
const notice = document.querySelector('#notice')
const state = { user: null, page: 'orders', generation: 0, orderPage: 1, customerPage: 1, orderStatus: '', orderQuery: '', orderDeliveryDate: '', deliveryAreaQuery: '', productQuery: '', customerRequestStatus: 'pending' }
const money = value => new Intl.NumberFormat('cs-CZ', { style: 'currency', currency: 'CZK' }).format(value / 100)
const dateTime = value => value ? new Intl.DateTimeFormat('cs-CZ', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'Europe/Prague' }).format(new Date(value)) : 'Dosud neodesláno'
const statuses = { awaiting_payment: 'Čeká na zaplacení', new: 'Přijatá', confirmed: 'Potvrzená', out_for_delivery: 'Na cestě', delivered: 'Doručená', cancelled: 'Zrušená', expired: 'Vypršela' }
const isPickup = order => order.fulfillment_method === 'pickup'
const fulfillmentLabel = order => isPickup(order) ? 'Osobní odběr' : 'Rozvoz'
const orderStatusLabel = (order, status = order.status) => isPickup(order) && status === 'out_for_delivery' ? 'Připraveno k vyzvednutí' : isPickup(order) && status === 'delivered' ? 'Vyzvednuto' : statuses[status] || (status === 'preparing' ? 'Připravuje se (dřívější stav)' : status)
const orderAddress = order => isPickup(order) && order.pickup_location ? order.pickup_location : order
const payments = { unpaid: 'Nezaplaceno', paid: 'Zaplaceno' }
const onlinePaymentStates = { pending: 'Čeká na přípravu nebo ověření', ready: 'Platební brána je připravena, platba čeká', paid: 'Platba ověřena', failed: 'Platba neproběhla', expired: 'Platnost platby skončila', review: 'Vyžaduje kontrolu obsluhy' }
const mailStates = { pending: 'Čeká na odeslání', sent: 'Odesláno', failed: 'Odeslání selhalo' }
const routes = [['orders', 'Objednávky'], ['customer-requests', 'Žádosti zákazníků'], ['products', 'Zboží a ceny'], ['categories', 'Kategorie'], ['customers', 'Zákazníci'], ['delivery-areas', 'Oblasti rozvozu'], ['settings', 'Doprava a nastavení'], ['site-settings', 'Obecné nastavení webu'], ['invoice-settings', 'Fakturační údaje'], ['terms', 'Obchodní podmínky'], ['administrators', 'Administrátoři'], ['mail', 'E-maily']]
const normalizePostalCode = value => String(value ?? '').replace(/[\s\u0085]/g, '')
const formatPostalCode = value => normalizePostalCode(value).replace(/^([0-9]{3})([0-9]{2})$/, '$1 $2')

function pragueCalendarDate(offset = 0) {
  const parts = new Intl.DateTimeFormat('en', { timeZone: 'Europe/Prague', year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date())
  const part = type => Number(parts.find(value => value.type === type).value)
  // Add calendar days after finding the Prague date, so daylight saving changes do not shift the result.
  return new Date(Date.UTC(part('year'), part('month') - 1, part('day') + offset)).toISOString().slice(0, 10)
}

function el(tag, attributes = {}, ...children) {
  const node = document.createElement(tag)
  for (const [key, value] of Object.entries(attributes)) {
    if (key === 'class') node.className = value
    else if (key.startsWith('on')) node.addEventListener(key.slice(2), value)
    else if (value !== null && value !== false && value !== undefined) node.setAttribute(key, value === true ? '' : String(value))
  }
  for (const child of children.flat(Infinity)) {
    if (child !== null && child !== undefined) node.append(child instanceof Node ? child : document.createTextNode(String(child)))
  }
  return node
}
function button(label, click, type = '') {
  return el('button', { type: 'button', class: `button ${type}`, onclick: click }, label)
}
function badge(label, kind = '') { return el('span', { class: `badge ${kind}` }, label) }
function notify(message, error = false) {
  notice.textContent = message
  notice.className = error ? 'error' : ''
  notice.hidden = false
  clearTimeout(notify.timer)
  notify.timer = setTimeout(() => { notice.hidden = true }, error ? 12000 : 5000)
}
async function api(path, options = {}) {
  const response = await fetch(path, { credentials: 'same-origin', ...options, headers: { 'Content-Type': 'application/json', 'X-Brunch-Request': '1', ...options.headers } })
  const data = await response.json().catch(() => ({}))
  if (!response.ok) {
    const conflictHint = response.status === 409 && !path.startsWith('/api/admin/administrators')
    const conflictAction = {
      '/api/admin/terms': 'Načtěte aktuální znění tlačítkem Obnovit. Váš rozepsaný text zatím zůstává ve formuláři.',
      '/api/admin/site-settings': 'Načtěte aktuální nastavení tlačítkem Obnovit. Vybraná fotografie i rozepsaný popis zatím zůstávají ve formuláři.',
      '/api/admin/pickup-settings': 'Načtěte aktuální adresu a časy tlačítkem Obnovit osobní odběr. Rozepsané údaje zatím zůstávají ve formuláři.',
      '/api/admin/settings': 'Načtěte aktuální nastavení tlačítkem Obnovit. Rozepsané údaje zatím zůstávají ve formuláři.'
    }[path] || 'Zavřete okno a načtěte aktuální data tlačítkem Obnovit.'
    const error = new Error(conflictHint ? `${data.error || 'Záznam se mezitím změnil.'} ${conflictAction}` : data.error || `Požadavek se nepodařil (${response.status}).`)
    error.status = response.status
    if (path.startsWith('/api/admin/') && path !== '/api/admin/auth/login' && [401, 403].includes(response.status)) {
      state.generation++
      state.user = null
      nav.hidden = true
      identity.replaceChildren()
      if (dialog.open) dialog.close()
      content.replaceChildren(el('p', { class: 'loading', role: 'status' }, 'Ověřuji oprávnění…'))
      await checkAuth({ accessError: error })
    }
    throw error
  }
  return data
}
function field(label, name, value = '', options = {}) {
  const { kind = 'input', hint, full, ...attributes } = options
  const input = el(kind, { name, 'aria-label': label, ...attributes })
  if (kind === 'textarea') input.value = value ?? ''
  else if (attributes.type === 'checkbox') input.checked = Boolean(value)
  else input.value = value ?? ''
  return el('label', { class: `field${full ? ' full' : ''}` }, el('span', {}, label), input, hint ? el('small', { class: 'hint' }, hint) : null)
}
function selectField(label, name, values, selected, options = {}) {
  const select = el('select', { name, 'aria-label': label, ...options }, values.map(([value, text]) => el('option', { value }, text)))
  select.value = selected
  return el('label', { class: 'field' }, el('span', {}, label), select)
}
function errorBox(error) { return el('p', { class: 'error', role: 'alert' }, error.message || String(error)) }
function empty(title, text) { return el('div', { class: 'empty' }, el('strong', {}, title), text) }
function header(title, description, action = null) {
  return el('div', { class: 'page-head' }, el('div', {}, el('h1', {}, title), el('p', {}, description)), action)
}
function table(headings, rows, compact = false) {
  return el('div', { class: `table-wrap${compact ? ' compact' : ''}` }, el('table', {}, el('thead', {}, el('tr', {}, headings.map(h => el('th', { scope: 'col' }, h)))), el('tbody', {}, rows)))
}
function pager(page, total, change) {
  const prev = button('← Předchozí', () => change(page - 1), 'small')
  const next = button('Další →', () => change(page + 1), 'small')
  prev.disabled = page <= 1
  next.disabled = page * 50 >= total
  return el('div', { class: 'pagination' }, el('span', {}, `Celkem ${total} · Strana ${page} z ${Math.max(1, Math.ceil(total / 50))}`), el('div', {}, prev, next))
}
function openDialog(title) {
  dialog.replaceChildren(el('div', { class: 'dialog-head' }, el('h2', { id: 'dialog-title' }, title), el('button', { type: 'button', class: 'button link', 'aria-label': 'Zavřít okno', onclick: () => dialog.close() }, '×')))
  const body = el('div', { class: 'dialog-body' })
  dialog.append(body)
  if (!dialog.open) dialog.showModal()
  return body
}
function formError(form, error) {
  form.querySelector('[data-form-error]')?.remove()
  const box = errorBox(error)
  box.dataset.formError = 'true'
  form.prepend(box)
  box.scrollIntoView({ block: 'nearest' })
}
async function saveForm(event, work, pendingLabel = 'Ukládám…') {
  event.preventDefault()
  const form = event.currentTarget
  const submit = form.querySelector('[type=submit]')
  if (submit.disabled) return
  const label = submit.textContent
  submit.disabled = true
  submit.textContent = pendingLabel
  try { await work(new FormData(form)) }
  catch (error) { formError(form, error) }
  finally { submit.disabled = false; submit.textContent = label }
}
function cents(value, nullable = false) {
  const text = String(value ?? '').trim().replace(',', '.')
  if (!text && nullable) return null
  if (!/^\d+(\.\d{1,2})?$/.test(text)) throw new Error('Cenu zadejte v korunách, nejvýše se dvěma desetinnými místy.')
  const number = Math.round(Number(text) * 100)
  if (!Number.isSafeInteger(number)) throw new Error('Cena je příliš vysoká.')
  return number
}

async function checkAuth({ accessError = null } = {}) {
  try {
    const { user } = await api('/api/auth/me')
    state.user = user
    identity.replaceChildren(el('span', {}, user.email), button('Odhlásit', logout, 'small'))
    if (!user.is_admin) {
      nav.hidden = true
      content.replaceChildren(el('section', { class: 'panel login-panel' }, el('h1', {}, 'Přístup do administrace'), el('p', {}, 'Pro vstup do administrace se přihlaste e-mailem a heslem správce. Pokud ještě nemáte administrátorský účet, požádejte některého ze správců o přidání.'), button('Přihlásit se jako správce', logout), el('p', {}, el('a', { href: '/' }, 'Přejít do obchodu'))))
      return
    }
    nav.hidden = false
    nav.replaceChildren(...routes.map(([route, label]) => el('a', { href: `#${route}`, 'data-route': route }, label)))
    // A request-specific 403 must not cause an endless retry when the session
    // itself is still valid. Let the operator choose whether to try again.
    if (accessError) content.replaceChildren(header('Požadavek nelze dokončit', 'Přihlášení je platné, ale server tento požadavek odmítl.'), errorBox(accessError), button('Obnovit', renderPage))
    else renderPage()
  } catch (error) {
    if ([401, 403].includes(error.status)) showLogin()
    else content.replaceChildren(header('Administrace', 'Spojení se serverem se nepodařilo.'), errorBox(error), button('Zkusit znovu', checkAuth))
  }
}
function showLogin() {
  state.user = null
  nav.hidden = true
  identity.replaceChildren()
  const form = el('form', { onsubmit: event => saveForm(event, async data => {
    await api('/api/admin/auth/login', { method: 'POST', body: JSON.stringify({ email: String(data.get('email')).trim(), password: String(data.get('password')) }) })
    await checkAuth()
  }, 'Přihlašuji…') }, field('E-mail správce', 'email', '', { type: 'email', autocomplete: 'username', required: true, maxlength: 254 }), field('Heslo', 'password', '', { type: 'password', autocomplete: 'current-password', required: true, maxlength: 128 }), el('button', { class: 'button primary', type: 'submit' }, 'Přihlásit se'))
  content.replaceChildren(el('section', { class: 'panel login-panel' }, el('h1', {}, 'Vítejte v administraci'), el('p', {}, 'Přihlaste se e-mailem a heslem svého administrátorského účtu.'), form, el('p', {}, 'Pokud heslo neznáte, požádejte jiného správce o nastavení nového hesla.'), el('p', {}, el('a', { href: '/' }, 'Zpět do obchodu'))))
}

async function logout() {
  try {
    await api('/api/auth/logout', { method: 'POST', body: '{}' })
    if (dialog.open) dialog.close()
    state.generation++
    showLogin()
  } catch (error) { notify(error.message, true) }
}
async function renderPage() {
  if (!state.user?.is_admin) return
  const requested = location.hash.slice(1)
  state.page = routes.some(([key]) => key === requested) ? requested : 'orders'
  const generation = ++state.generation
  for (const link of nav.querySelectorAll('a')) {
    if (link.dataset.route === state.page) link.setAttribute('aria-current', 'page')
    else link.removeAttribute('aria-current')
  }
  content.replaceChildren(el('p', { class: 'loading', role: 'status' }, 'Načítám data…'))
  try {
    const page = await ({ orders: ordersPage, 'customer-requests': customerRequestsPage, products: productsPage, categories: categoriesPage, customers: customersPage, 'delivery-areas': deliveryAreasPage, settings: settingsPage, 'site-settings': siteSettingsPage, 'invoice-settings': invoiceSettingsPage, terms: termsPage, administrators: administratorsPage, mail: mailPage })[state.page]()
    if (generation === state.generation) content.replaceChildren(...page)
  } catch (error) {
    if (generation !== state.generation) return
    if (error.status === 401) { showLogin(); return }
    content.replaceChildren(header('Data nelze načíst', 'Zkuste požadavek opakovat.'), errorBox(error), button('Obnovit', renderPage))
  }
}

async function ordersPage() {
  const query = new URLSearchParams({ page: state.orderPage, status: state.orderStatus, q: state.orderQuery, delivery_date: state.orderDeliveryDate })
  const data = await api(`/api/admin/orders?${query}`)
  const applyFilters = form => {
    const data = new FormData(form)
    state.orderQuery = String(data.get('q')).trim()
    state.orderStatus = String(data.get('status'))
    state.orderDeliveryDate = String(data.get('delivery_date'))
    state.orderPage = 1
    renderPage()
  }
  const filter = el('form', { class: 'filters', onsubmit: event => {
    event.preventDefault()
    applyFilters(event.currentTarget)
  } }, field('Vyhledat objednávku', 'q', state.orderQuery, { type: 'search', placeholder: 'Číslo nebo e-mail', maxlength: 100 }), selectField('Stav objednávky', 'status', [['', 'Všechny stavy'], ...Object.entries(statuses).map(([key, label]) => [key, key === 'out_for_delivery' ? 'Na cestě / Připraveno k vyzvednutí' : key === 'delivered' ? 'Doručená / Vyzvednuto' : label])], state.orderStatus), el('button', { class: 'button', type: 'submit' }, 'Vyhledat'))
  filter.firstChild.classList.add('search')
  const deliveryDate = field('Den doručení / odběru', 'delivery_date', state.orderDeliveryDate, { type: 'date', 'aria-describedby': 'order-delivery-date-hint', onchange: () => applyFilters(filter) })
  const shortcuts = el('div', { class: 'date-shortcuts', role: 'group', 'aria-label': 'Rychlá volba dne doručení nebo odběru' }, [['Dnes', 0], ['Zítra', 1], ['Všechny dny', null]].map(([label, offset]) => {
    const value = offset === null ? '' : pragueCalendarDate(offset)
    const shortcut = button(label, () => {
      deliveryDate.querySelector('input').value = offset === null ? '' : pragueCalendarDate(offset)
      applyFilters(filter)
    })
    shortcut.setAttribute('aria-pressed', String(state.orderDeliveryDate === value))
    return shortcut
  }))
  filter.prepend(el('div', { class: 'order-delivery-filter' }, deliveryDate, shortcuts, el('p', { class: 'hint', id: 'order-delivery-date-hint' }, 'Vybírá den doručení nebo osobního odběru objednávky. Dnes a zítra se řídí českým časem.')))
  const bulk = orderSelection(data.orders)
  const rows = data.orders.map(order => el('tr', {}, el('td', {}, bulk.checkboxes.get(order.id)), el('td', {}, el('strong', {}, order.number), el('small', {}, dateTime(order.created_at))), el('td', {}, order.name || order.email, order.name ? el('small', {}, order.email) : null), el('td', {}, order.delivery_date, el('small', {}, order.delivery_slot), el('small', {}, fulfillmentLabel(order)), el('small', {}, orderAddress(order).city)), el('td', {}, badge(orderStatusLabel(order), order.status), el('small', {}, payments[order.payment_status] || order.payment_status)), el('td', { class: 'number' }, money(order.total_cents)), el('td', {}, button('Detail', () => orderDialog(order.id), 'small'))))
  return [header('Objednávky', 'Od potvrzení až po předání zákazníkovi.', button('Obnovit', renderPage)), filter, bulk.actions, rows.length ? table(['Výběr', 'Objednávka', 'Zákazník', 'Den doručení / odběru', 'Stav', 'Celkem', 'Akce'], rows) : empty('Žádné objednávky', state.orderStatus || state.orderQuery || state.orderDeliveryDate ? 'Zkuste změnit hledání nebo filtr.' : 'Nové objednávky se zobrazí na tomto místě.'), pager(data.page, data.total, page => { state.orderPage = page; renderPage() })]
}
function orderSelection(orders) {
  const selected = new Map()
  const checkboxes = new Map()
  const actions = el('form', { class: 'filters bulk-orders', hidden: true, onsubmit: event => saveForm(event, apply) })
  const next = { new: 'confirmed', confirmed: 'out_for_delivery', out_for_delivery: 'delivered' }
  const offers = order => {
    const choices = next[order.status] ? [next[order.status]] : []
    if (order.payment_status === 'unpaid' && (order.status in next || order.online_cancellable && order.status === 'awaiting_payment')) choices.push('cancelled')
    return choices
  }
  function refresh() {
    const group = [...selected.values()]
    const first = group[0]
    for (const order of orders) {
      const box = checkboxes.get(order.id)
      box.disabled = !offers(order).length || Boolean(first && (order.status !== first.status || isPickup(order) !== isPickup(first)))
      box.checked = selected.has(order.id)
    }
    actions.hidden = !first
    if (!first) return
    const choices = offers(first).filter(status => group.every(order => offers(order).includes(status)))
    const selectAll = button('Vybrat všechny ve stejném stavu na stránce', () => {
      for (const order of orders) if (order.status === first.status && isPickup(order) === isPickup(first) && offers(order).length) selected.set(order.id, order)
      refresh()
    }, 'small')
    actions.replaceChildren(el('span', { role: 'status' }, `Vybráno: ${group.length} · ${fulfillmentLabel(first)} · ${orderStatusLabel(first)}`),
      selectAll, button('Zrušit výběr', () => { selected.clear(); refresh() }, 'small'),
      selectField('Nový stav', 'status', choices.map(status => [status, orderStatusLabel(first, status)]), choices[0]),
      el('button', { class: 'button primary', type: 'submit', disabled: choices.length === 0 }, 'Změnit stav vybraných'))
  }
  async function apply(data) {
    const group = [...selected.values()]
    const status = data.get('status')
    const collect = status === 'delivered' ? group.filter(o => o.payment_status === 'unpaid' && o.payment_method !== 'card') : []
    const payment = collect.length ? ` Současně potvrzujete převzetí ${money(collect.reduce((sum, o) => sum + o.total_cents, 0))} za ${collect.length} objednávek.` : ''
    const stock = status === 'cancelled' ? ' Rezervované kusy se vrátí na sklad.' : ''
    if (!confirm(`Změnit stav ${group.length} objednávek na „${orderStatusLabel(group[0], status)}“?${payment}${stock}`)) return
    const result = await api('/api/admin/orders/bulk-status', { method: 'POST', body: JSON.stringify({
      orders: group.map(({ id, version }) => ({ id, version })), from_status: group[0].status,
      status, confirm_payments: collect.length > 0
    }) })
    notify(`Počet aktualizovaných objednávek: ${result.updated}.`)
    await renderPage()
  }
  for (const order of orders) {
    checkboxes.set(order.id, el('input', { type: 'checkbox', 'aria-label': `Vybrat objednávku ${order.number}`,
      onchange: event => {
        if (event.target.checked) selected.set(order.id, order)
        else selected.delete(order.id)
        refresh()
      }
    }))
  }
  refresh()
  return { actions, checkboxes }
}

async function orderDialog(id) {
  const body = openDialog('Detail objednávky')
  body.append(el('p', { class: 'loading' }, 'Načítám objednávku…'))
  try {
    const order = await api(`/api/admin/orders/${id}`)
    if (!body.isConnected) return
    body.dataset.orderVersion = String(order.version)
    dialog.querySelector('#dialog-title').textContent = order.number
    const contact = el('p', {}, [order.name, order.organization].filter(Boolean).join('\n'))
    contact.append(el('br'), el('a', { href: `mailto:${encodeURIComponent(order.email)}` }, order.email))
    if (order.phone) contact.append(el('br'), /^\+?[0-9 ()-]{5,40}$/.test(order.phone) ? el('a', { href: `tel:${order.phone.replace(/[^\d+]/g, '')}` }, order.phone) : order.phone)
    const pickup = isPickup(order)
    const address = orderAddress(order)
    const mapUrl = `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(`${address.street}, ${address.postal_code} ${address.city}, Česko`)}`
    body.replaceChildren(el('div', { class: 'summary-grid' }, el('section', {}, el('h3', {}, 'Zákazník'), contact), el('section', {}, el('h3', {}, pickup ? 'Osobní odběr' : 'Doručení'), el('p', {}, `${pickup && address.name ? address.name + '\n' : ''}${address.street}\n${address.postal_code} ${address.city}\n${order.delivery_date} · ${order.delivery_slot}`), pickup && address.parking_free ? el('p', { class: 'pickup-parking' }, 'Parkování zdarma') : null, pickup && address.instructions ? el('p', { class: 'pickup-instructions' }, address.instructions) : null)), el('div', { class: 'actions no-print' }, el('a', { class: 'button small', href: mapUrl, target: '_blank', rel: 'noopener noreferrer' }, 'Otevřít adresu v mapě ↗'), button('Faktura', () => invoiceDialog(order), 'small')))
    if (order.notes) body.append(el('div', { class: 'note' }, el('strong', {}, pickup ? 'Poznámka k osobnímu odběru\n' : 'Pokyny pro doručení\n'), order.notes))
    body.append(el('h3', {}, 'Položky objednávky'), table(['Zboží', 'Množství', 'Cena za kus', 'Celkem'], order.items.map(item => el('tr', {}, el('td', {}, item.name), el('td', {}, `${item.quantity} ks`), el('td', { class: 'number' }, money(item.unit_price_cents)), el('td', { class: 'number' }, money(item.total_cents)))), true), el('div', { class: 'totals' }, el('div', {}, el('span', {}, 'Zboží'), money(order.subtotal_cents)), el('div', {}, el('span', {}, pickup ? 'Osobní odběr' : 'Doprava'), money(order.delivery_fee_cents)), el('div', {}, el('span', {}, 'Celkem'), money(order.total_cents))))
    body.append(el('p', { class: 'hint' }, `${order.payment_method === 'card' ? 'Online platba kartou přes Teya' : `Platba ${order.payment_method === 'card_on_delivery' ? 'kartou' : 'hotově'} ${pickup ? 'při vyzvednutí' : 'při doručení'}`}. ${payments[order.payment_status]}. Stav: ${orderStatusLabel(order)}. Vytvořeno ${dateTime(order.created_at)}.`))
    body.append(el('p', { class: 'hint order-origin' }, el('strong', {}, 'IP adresa při objednání: '), order.created_from_ip || 'U starších objednávek neevidována'))
    if (Number.isSafeInteger(order.terms_version) && order.terms_version > 0) body.append(el('p', { class: 'hint order-origin' }, el('a', {
      href: `/obchodni-podminky/?version=${order.terms_version}`, target: '_blank', rel: 'noopener noreferrer'
    }, `Obchodní podmínky přijaté při objednání, verze ${order.terms_version}`)))
    const sequence = ['new', 'confirmed', 'out_for_delivery', 'delivered']
    const terminal = ['delivered', 'cancelled', 'expired'].includes(order.status)
    const online = order.payment_method === 'card'
    const awaitingPayment = online && order.payment_status !== 'paid'
    const onlineCancellable = online && order.online_payment?.cancellable === true
    if (online && order.online_payment) {
      const payment = order.online_payment
      if (payment.expires_at) body.append(el('p', { class: 'hint' }, `Lhůta pro online platbu: ${dateTime(payment.expires_at)}.`))
      if (payment.receipt) {
        const receipt = payment.receipt
        body.append(el('section', { class: 'note', 'aria-label': 'Přijatá online platba' }, el('h3', {}, 'Přijatá online platba'), el('p', {}, el('strong', {}, `Částka: ${money(receipt.amount_cents)}`)), el('p', {}, `Měna: ${receipt.currency}`), el('p', {}, `Transakce Teya: ${receipt.transaction_id}`), el('p', {}, `Platební relace: ${receipt.session_id}`), el('p', {}, `Ověřeno: ${dateTime(receipt.verified_at)}.`), terminal && payment.state === 'review' ? el('p', {}, 'Přijatá částka vyžaduje ruční vyřešení. Objednávka zůstává uzavřená.') : null))
      }
      body.append(el('section', { class: 'note', role: payment.state === 'review' ? 'alert' : 'status' }, el('strong', {}, `Teya: ${onlinePaymentStates[payment.state] || 'Čeká na ověření'}`), el('p', {}, payment.checked_at ? `Poslední ověření: ${dateTime(payment.checked_at)}.` : 'Platba ještě nebyla ověřena.'), payment.state === 'review' ? el('p', {}, 'Platba vyžaduje ruční kontrolu v portálu Teya. Ověřte číslo objednávky, částku a měnu. Dokud není výsledek vyřešen, objednávku nevyřizujte a zákazníka nežádejte o další platbu.') : null, payment.last_error ? el('small', {}, `Provozní informace: ${payment.last_error}`) : null))
    }
    const allowed = [order.status]
    if (!terminal && !awaitingPayment && sequence.includes(order.status)) allowed.push(sequence[sequence.indexOf(order.status) + 1])
    if (!terminal && ((!online && order.payment_status !== 'paid') || onlineCancellable)) allowed.push('cancelled')
    const form = el('form', { onsubmit: event => saveForm(event, async data => {
      const status = data.get('status')
      if (status === 'cancelled' && !confirm(`Opravdu zrušit objednávku ${order.number}? Rezervované kusy se vrátí na sklad.`)) return
      if (data.get('payment_status') === 'paid' && order.payment_status !== 'paid' && !confirm(`Potvrzujete přijetí platby ${money(order.total_cents)} ${order.payment_method === 'card_on_delivery' ? 'kartou' : 'v hotovosti'} za objednávku ${order.number}?`)) return
      await api(`/api/admin/orders/${order.id}`, { method: 'PATCH', body: JSON.stringify({ status, payment_status: online ? order.payment_status : data.get('payment_status'), version: order.version }) })
      notify('Objednávka byla uložena.')
      await orderDialog(order.id)
      renderPage()
    }) }, el('h3', {}, 'Průběh objednávky'), el('div', { class: 'form-grid' }, selectField('Stav', 'status', allowed.map(key => [key, orderStatusLabel(order, key)]), order.status), selectField('Platba', 'payment_status', Object.entries(payments), order.payment_status)), el('p', { class: 'hint' }, terminal ? 'Objednávka má konečný stav a nelze ji dále měnit.' : awaitingPayment && onlineCancellable ? 'Platební brána potvrdila, že platba skončila bez zaplacení. Objednávku můžete zrušit a vrátit rezervované kusy na sklad. Platbu nelze ručně označit jako přijatou.' : awaitingPayment ? 'Čekáme na ověření online platby. Do té doby nelze objednávku přijmout, připravovat ani ručně označit jako zaplacenou. Stav aktualizujte tlačítkem Obnovit stav platby.' : online ? 'Online platba byla ověřena platební bránou. Její stav nelze měnit ručně. Změnu stavu objednávky oznámíme zákazníkovi e-mailem.' : pickup ? 'Platbu lze přijmout při vyzvednutí. Označení Vyzvednuto vyžaduje potvrzení přijetí platby. Stav Připraveno k vyzvednutí oznámíme zákazníkovi e-mailem.' : 'Platbu lze přijmout při rozvozu. Označení Doručená vyžaduje potvrzení přijetí platby. Změnu stavu oznámíme zákazníkovi e-mailem.'), el('div', { class: 'actions' }, el('button', { class: 'button primary', type: 'submit' }, 'Uložit změny')))
    const orderStatus = form.querySelector('[name=status]')
    const paymentStatus = form.querySelector('[name=payment_status]')
    const updatePaymentOptions = () => {
      if (online) {
        paymentStatus.value = order.payment_status
        paymentStatus.disabled = true
        return
      }
      const paidOption = paymentStatus.querySelector('option[value=paid]')
      const canPay = ['out_for_delivery', 'delivered'].includes(orderStatus.value)
      paidOption.disabled = !canPay
      paymentStatus.querySelector('option[value=unpaid]').disabled = order.payment_status === 'paid' || orderStatus.value === 'delivered'
      if (orderStatus.value === 'delivered') paymentStatus.value = 'paid'
      else if (!canPay) paymentStatus.value = 'unpaid'
    }
    orderStatus.addEventListener('change', updatePaymentOptions)
    updatePaymentOptions()
    if (terminal || (awaitingPayment && !onlineCancellable)) {
      orderStatus.disabled = true
      paymentStatus.disabled = true
      form.querySelector('[type=submit]').disabled = true
    }
    body.append(form)
    if (online) body.append(button('Obnovit stav platby', () => orderDialog(id)))
    if (order.history?.length) body.append(el('h3', {}, 'Historie změn'), el('ol', { class: 'history' }, order.history.map(item => el('li', {}, `${orderStatusLabel(order, item.status)} · ${payments[item.payment_status] || item.payment_status}`, el('small', {}, `${dateTime(item.created_at)}${item.changed_by ? ` · ${item.changed_by}` : ''}`)))))
  } catch (error) { body.replaceChildren(errorBox(error), button('Načíst znovu', () => orderDialog(id))) }
}

function normalizedProductSearch(value) {
  return String(value ?? '').normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLocaleLowerCase('cs-CZ').trim().replace(/\s+/g, ' ')
}
function productWarnings(product) {
  const warnings = []
  if (!String(product.ingredients || '').trim()) warnings.push('Doplňte ověřené složení.')
  if (!String(product.allergens || '').trim()) warnings.push('Doplňte ověřené alergeny. Prázdné pole neznamená, že výrobek alergeny neobsahuje.')
  const weight = String(product.weight || '').trim()
  if (!weight) warnings.push('Doplňte hmotnost, objem nebo počet kusů v balení.')
  else if (/^[\d.,\s]+$/.test(weight)) warnings.push('U hmotnosti nebo objemu chybí jednotka, například g nebo ml.')
  const minimum = normalizedProductSearch(product.description).match(/minimalni\s+odber[^\d]{0,40}(\d+)\s*(?:kusu|ks)/)
  if (minimum && Number(minimum[1]) !== Math.max(1, Number(product.minimum_quantity || 0))) warnings.push(`Popis uvádí minimální odběr ${minimum[1]} ks, nastavení ale ${Math.max(1, Number(product.minimum_quantity || 0))} ks. Sjednoťte obě hodnoty.`)
  return warnings
}
async function productsPage() {
  const [products, categories] = await Promise.all([api('/api/admin/products'), api('/api/admin/categories')])
  const results = el('div', { class: 'product-results' })
  const count = el('p', { class: 'hint product-result-count', role: 'status', 'aria-live': 'polite' })
  const search = field('Vyhledat zboží', 'product-search', state.productQuery, { type: 'search', placeholder: 'Název, kategorie, popis nebo banketní pečivo', autocomplete: 'off' })
  search.classList.add('search')
  const clear = button('Vymazat hledání', () => {
    state.productQuery = ''
    search.querySelector('input').value = ''
    updateResults()
    search.querySelector('input').focus()
  }, 'small')
  function updateResults() {
    const query = normalizedProductSearch(state.productQuery)
    const words = query.split(' ').filter(Boolean)
    const filtered = products.filter(product => {
      const text = normalizedProductSearch([product.name, product.category, product.description, product.banquet ? 'banketní pečivo' : ''].join(' '))
      return words.every(word => text.includes(word))
    })
    count.textContent = `Zobrazeno ${filtered.length} z ${products.length} položek`
    clear.hidden = !state.productQuery
    const rows = filtered.map(product => {
      const warnings = productWarnings(product)
      return el('tr', {}, el('td', {}, el('strong', {}, product.name), el('small', {}, `${product.category || 'Bez kategorie'} · ${product.weight || 'Bez uvedené hmotnosti'}`), product.banquet ? el('small', { class: 'banquet-label' }, 'Banketní pečivo') : null, warnings.length ? el('small', { class: 'product-warning-label', title: warnings.join(' ') }, `Ke kontrole: ${warnings.length} ${warnings.length === 1 ? 'údaj' : warnings.length < 5 ? 'údaje' : 'údajů'}`) : null), el('td', { class: 'number' }, money(product.price_cents)), el('td', {}, product.stock === null ? 'Bez omezení' : `${product.stock} ks`, product.minimum_quantity ? el('small', {}, `Min. odběr ${product.minimum_quantity} ks`) : null), el('td', {}, badge(product.active ? 'V prodeji' : 'Archivováno', product.active ? '' : 'archived')), el('td', {}, el('div', { class: 'cell-actions' }, button('Upravit', () => productDialog(product, categories), 'small'), button('Ceny', () => priceDialog(product), 'small'))))
    })
    results.replaceChildren(rows.length ? table(['Zboží', 'Cena', 'Sklad', 'Dostupnost', 'Akce'], rows) : products.length ? empty('Žádné odpovídající zboží', 'Zkuste jiný název, kategorii nebo vymažte hledání.') : empty('Zatím žádné zboží', 'Přidejte první produkt do nabídky.'))
  }
  search.querySelector('input').addEventListener('input', event => { state.productQuery = event.target.value; updateResults() })
  updateResults()
  return [header('Zboží a ceny', 'Sortiment, dostupnost a přehled každé změny ceny.', button('+ Přidat zboží', () => productDialog(null, categories), 'primary')), el('div', { class: 'filters product-filters' }, search, clear, button('Obnovit', renderPage, 'small')), count, results, el('p', { class: 'section-note' }, 'Archivované zboží se nezobrazuje v obchodě. Objednávky uchovávají původní názvy a ceny.')]
}
function productDialog(product, categories) {
  const body = openDialog(product ? 'Upravit zboží' : 'Přidat zboží')
  const data = product || { name: '', description: '', category: '', minimum_quantity: 0, banquet: false, tag: '', weight: '', price_cents: 0, vegan: false, ingredients: '', allergens: '', image: '', active: true, stock: null }
  const form = el('form', { onsubmit: event => saveForm(event, async values => {
    const payload = Object.fromEntries(['name', 'description', 'category', 'tag', 'weight', 'ingredients', 'allergens', 'image'].map(key => [key, String(values.get(key)).trim()]))
    payload.variants = String(values.get('variants') || '').split('\n').map(value => value.trim()).filter(Boolean)
    if (payload.variants.length > 30 || payload.variants.some(value => [...value].length > 120)) throw new Error('Varianty mohou obsahovat nejvýše 30 hodnot, každou do 120 znaků.')
    payload.price_cents = cents(values.get('price'))
    payload.stock = String(values.get('stock')).trim() === '' ? null : Number(values.get('stock'))
    payload.minimum_quantity = Number(values.get('minimum_quantity') || 0)
    payload.banquet = values.has('banquet')
    if (!Number.isSafeInteger(payload.minimum_quantity) || payload.minimum_quantity < (payload.banquet ? 1 : 0) || (!payload.banquet && payload.minimum_quantity > 99)) throw new Error(payload.banquet ? 'U banketního pečiva vyplňte minimální odběr jako celé číslo od 1 kusu.' : 'Minimální odběr musí být celé číslo od 0 do 99 kusů.')
    if (payload.minimum_quantity > 2147483647) throw new Error('Zadaný minimální odběr je příliš vysoký.')
    payload.vegan = values.has('vegan')
    payload.active = values.has('active')
    if (product) payload.version = product.version
    await api(product ? `/api/admin/products/${product.id}` : '/api/admin/products', { method: product ? 'PUT' : 'POST', body: JSON.stringify(payload) })
    dialog.close()
    notify(product ? 'Zboží bylo upraveno.' : 'Nové zboží bylo přidáno.')
    renderPage()
  }) }, el('div', { class: 'form-grid' }, field('Název', 'name', data.name, { required: true, maxlength: 150, full: true }), field('Popis', 'description', data.description, { kind: 'textarea', maxlength: 2000, full: true }), selectField('Kategorie', 'category', [['', 'Bez kategorie'], ...categories.slice().sort((a, b) => a.name.localeCompare(b.name, 'cs')).map(category => [category.name, category.name])], data.category), field('Štítek', 'tag', data.tag, { maxlength: 80, placeholder: 'Např. Z naší pece' }), field('Cena v Kč', 'price', (data.price_cents / 100).toFixed(2), { required: true, inputmode: 'decimal', placeholder: '119,00' }), field('Hmotnost / objem', 'weight', data.weight, { maxlength: 80, placeholder: 'Např. 180 g' }), field('Banketní pečivo', 'banquet', data.banquet, { type: 'checkbox', hint: 'Kusy se započítávají do společného minima banketního pečiva v objednávce.' }), field('Minimální odběr v kusech', 'minimum_quantity', data.minimum_quantity || '', { type: 'number', min: 0, max: 99, step: 1, hint: 'Prázdné pole nebo nula znamená bez minima. Při prvním přidání do košíku se nastaví zadaný počet.' }), field('Počet dostupných kusů', 'stock', data.stock, { type: 'number', min: 0, max: 1000000, step: 1, hint: 'Prázdné pole znamená neomezené množství. Nula znamená vyprodáno.' }), field('Obrázek', 'image', data.image, { maxlength: 500, placeholder: '/images/produkt.webp', hint: 'Cesta k obrázku na tomto webu. Pro výchozí vzhled ponechte prázdné.' }), field('Složení', 'ingredients', data.ingredients, { kind: 'textarea', maxlength: 2000, full: true }), field('Alergeny', 'allergens', data.allergens, { kind: 'textarea', maxlength: 1000, full: true }), field('Varianty (jedna na řádek)', 'variants', (data.variants || []).join('\n'), { kind: 'textarea', maxlength: 3630, full: true, hint: 'Nejvýše 30 variant, každá do 120 znaků. Zákazník si musí jednu vybrat. Prázdné pole znamená pevný produkt. Všechny varianty mají stejnou cenu. Cenově odlišné varianty založte jako samostatné produkty.' }), field('Vegan', 'vegan', data.vegan, { type: 'checkbox' }), field('Zobrazovat v nabídce', 'active', data.active, { type: 'checkbox' })), el('p', { class: 'hint' }, 'Kategorie spravujete v sekci ', el('a', { href: '#categories', onclick: () => dialog.close() }, 'Kategorie'), '. Zde vybíráte z již vytvořených kategorií.'), el('p', { class: 'hint' }, 'Odškrtnutím nabídky zboží archivujete. Historie cen i dřívější objednávky zůstanou zachovány.'), el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { type: 'submit', class: 'button primary' }, 'Uložit zboží')))
  const banquet = form.elements.banquet
  const minimum = form.elements.minimum_quantity
  const minimumHint = minimum.closest('label').querySelector('.hint')
  minimumHint.id = 'product-minimum-hint'
  minimum.setAttribute('aria-describedby', minimumHint.id)
  function updateBanquetMinimum() {
    minimum.required = banquet.checked
    minimum.min = banquet.checked ? '1' : '0'
    if (banquet.checked) minimum.removeAttribute('max')
    else minimum.max = '99'
    minimumHint.textContent = banquet.checked
      ? 'Povinné pro banketní pečivo: celé číslo od 1 kusu tohoto výrobku. Navíc musí objednávka splnit společné minimum všech banketních výrobků nastavené v obecném nastavení.'
      : 'Nepovinné. Prázdné pole nebo nula znamená bez minima. Při prvním přidání do košíku se nastaví zadaný počet.'
  }
  banquet.addEventListener('change', updateBanquetMinimum)
  updateBanquetMinimum()
  const warnings = el('section', { class: 'product-completeness', role: 'status', 'aria-live': 'polite', 'aria-label': 'Kontrola údajů zboží' })
  function refreshWarnings() {
    const values = new FormData(form)
    const issues = productWarnings(Object.fromEntries(values))
    warnings.hidden = !issues.length
    warnings.replaceChildren(el('strong', {}, 'Před zveřejněním zkontrolujte'), el('ul', {}, issues.map(issue => el('li', {}, issue))), el('p', { class: 'hint' }, 'Upozornění nebrání uložení rozpracovaného zboží. Údaje ověřte podle skutečného výrobku.'))
  }
  form.prepend(warnings)
  form.addEventListener('input', refreshWarnings)
  refreshWarnings()
  const imageField = form.querySelector('[name=image]')
  const preview = el('div', { class: 'image-preview' })
  const showPreview = () => {
    const value = imageField.value.trim()
    preview.replaceChildren()
    if (value && !value.startsWith('//') && !value.includes(':')) preview.append(el('img', { src: value, alt: 'Náhled fotografie produktu', loading: 'lazy' }))
  }
  imageField.addEventListener('change', showPreview)
  showPreview()
  const uploadStatus = el('p', { class: 'hint', role: 'status' }, 'JPG, PNG nebo WebP do 40 MB. Delší stranu automaticky zmenšíme na nejvýše 1 440 px.')
  const upload = el('input', { type: 'file', accept: 'image/jpeg,image/png,image/webp', 'aria-label': 'Nahrát fotografii' })
  upload.addEventListener('change', async () => {
    const file = upload.files[0]
    if (!file) return
    if (file.size > 40 * 1024 * 1024) {
      uploadStatus.textContent = 'Fotografie je příliš velká. Vyberte soubor do 40 MB.'
      uploadStatus.className = 'error'
      upload.value = ''
      return
    }
    const submit = form.querySelector('[type=submit]')
    upload.disabled = true
    submit.disabled = true
    uploadStatus.className = 'hint'
    uploadStatus.textContent = 'Nahrávám fotografii…'
    try {
      const result = await api('/api/admin/images', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: file })
      imageField.value = result.url
      showPreview()
      uploadStatus.textContent = 'Fotografie je nahraná. Pro přiřazení produktu uložte zboží.'
    } catch (error) {
      uploadStatus.textContent = error.message
      uploadStatus.className = 'error'
    } finally {
      upload.disabled = false
      submit.disabled = false
      upload.value = ''
    }
  })
  imageField.closest('label').after(el('div', { class: 'field full' }, el('label', {}, el('span', {}, 'Nahrát fotografii'), upload), uploadStatus, preview))
  body.append(form)
}
async function categoriesPage() {
  const categories = await api('/api/admin/categories')
  const rows = categories.map(category => el('tr', {},
    el('td', {}, el('strong', {}, category.name)),
    el('td', {}, category.product_count),
    el('td', {}, el('div', { class: 'cell-actions' },
      button('Přejmenovat', () => categoryDialog(category), 'small'),
      button('Odstranit', async event => {
        if (!confirm(`Odstranit kategorii „${category.name}“? Přiřazené zboží zůstane v katalogu bez kategorie.`)) return
        const remove = event.currentTarget
        remove.disabled = true
        try {
          await api(`/api/admin/categories/${category.id}`, { method: 'DELETE', body: JSON.stringify({ version: category.version }) })
          notify('Kategorie byla odstraněna.')
          renderPage()
        } catch (error) { notify(error.message, true); remove.disabled = false }
      }, 'small danger')))))
  return [
    header('Kategorie', 'Kategorie se zákazníkům zobrazují jako filtry nabídky.', button('+ Přidat kategorii', () => categoryDialog(), 'primary')),
    el('div', { class: 'filters' }, button('Obnovit', renderPage, 'small')),
    rows.length ? table(['Název kategorie', 'Počet položek', 'Akce'], rows) : empty('Zatím žádné kategorie', 'Přidejte kategorii a přiřaďte ji zboží.')
  ]
}
function categoryDialog(category = null) {
  const body = openDialog(category ? 'Přejmenovat kategorii' : 'Přidat kategorii')
  const form = el('form', { onsubmit: event => saveForm(event, async values => {
    const payload = { name: String(values.get('name')).trim() }
    if (category) payload.version = category.version
    await api(category ? `/api/admin/categories/${category.id}` : '/api/admin/categories', {
      method: category ? 'PUT' : 'POST', body: JSON.stringify(payload)
    })
    dialog.close()
    notify(category ? 'Kategorie byla přejmenována.' : 'Kategorie byla přidána.')
    renderPage()
  }) }, field('Název kategorie', 'name', category?.name || '', { required: true, maxlength: 80 }),
    el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()),
      el('button', { type: 'submit', class: 'button primary' }, 'Uložit kategorii')))
  body.append(form)
}
async function priceDialog(product) {
  const body = openDialog(`Historie cen: ${product.name}`)
  body.append(el('p', { class: 'loading' }, 'Načítám historii cen…'))
  try {
    const history = await api(`/api/admin/products/${product.id}/prices`)
    if (!body.isConnected) return
    body.replaceChildren(history.length ? table(['Od kdy', 'Cena', 'Změnil'], history.map(item => el('tr', {}, el('td', {}, dateTime(item.created_at)), el('td', { class: 'number' }, money(item.price_cents)), el('td', {}, item.changed_by || 'Počáteční cena')))) : empty('Bez změn ceny', 'Historie ceny ještě není dostupná.'))
  } catch (error) { body.replaceChildren(errorBox(error)) }
}

function deliverySlotsField(slots, inherit = false) {
  return field(inherit ? 'Vlastní rozvozová okna pro toto PSČ' : 'Výchozí rozvozová okna', 'delivery_slots', (slots || []).join('\n'), {
    kind: 'textarea', rows: 3, required: !inherit, placeholder: '07:00–09:00\n09:00–11:00',
    hint: `Každé časové okno zadejte na samostatný řádek, například 07:00–09:00.${inherit ? ' Prázdné pole použije výchozí okna obchodu.' : ''}`
  })
}
function readDeliverySlots(data) {
  const slots = String(data.get('delivery_slots') || '').split(/\r?\n/).map(line => line.trim()).filter(Boolean)
  return slots.length ? slots : null
}

async function deliveryAreasPage() {
  const areas = await api('/api/admin/delivery-areas')
  const results = el('div', { class: 'delivery-area-results', 'aria-live': 'polite' })
  const search = field('Vyhledat PSČ nebo název oblasti', 'delivery_area_search', state.deliveryAreaQuery, { type: 'search', placeholder: 'Např. 110 00 nebo Praha', maxlength: 200 })
  const renderResults = () => {
    const query = normalizePostalCode(state.deliveryAreaQuery).toLocaleLowerCase('cs-CZ')
    const matching = areas.filter(area => normalizePostalCode(area.postal_code).includes(query) || normalizePostalCode(area.label).toLocaleLowerCase('cs-CZ').includes(query))
    if (!matching.length) {
      results.replaceChildren(areas.length ? empty('Žádná odpovídající oblast', 'Zkuste jiné PSČ nebo název oblasti.') : empty('Zatím žádná oblast rozvozu', 'Přidejte první povolené PSČ a nechte rozvoz zapnutý. Bez aktivních oblastí nelze přijímat objednávky.'))
      return
    }
    const rows = matching.map(area => el('tr', {}, el('td', { class: 'postal-code' }, el('strong', {}, formatPostalCode(area.postal_code))), el('td', {}, area.label || 'Bez názvu'), el('td', {}, area.delivery_slots?.join(', ') || 'Výchozí okna'), el('td', {}, badge(area.active ? 'Rozvážíme' : 'Pozastaveno', area.active ? '' : 'archived')), el('td', {}, el('div', { class: 'cell-actions' }, button('Upravit', () => deliveryAreaDialog(area), 'small'), button(area.active ? 'Pozastavit' : 'Zapnout', async event => {
      const toggle = event.currentTarget
      toggle.disabled = true
      try {
        await api(`/api/admin/delivery-areas/${area.id}`, { method: 'PUT', body: JSON.stringify({ postal_code: normalizePostalCode(area.postal_code), label: area.label || '', active: !area.active, version: area.version }) })
        notify(area.active ? `Rozvoz pro PSČ ${formatPostalCode(area.postal_code)} byl pozastaven.` : `Rozvoz pro PSČ ${formatPostalCode(area.postal_code)} byl zapnut.`)
        renderPage()
      } catch (error) { notify(error.message, true); toggle.disabled = false }
    }, 'small')))))
    results.replaceChildren(table(['PSČ', 'Název oblasti', 'Časová okna', 'Rozvoz', 'Akce'], rows), el('p', { class: 'section-note' }, `Zobrazeno ${matching.length} z ${areas.length} oblastí.`))
  }
  search.querySelector('input').addEventListener('input', event => {
    state.deliveryAreaQuery = event.currentTarget.value
    renderResults()
  })
  renderResults()
  const activeCount = areas.filter(area => area.active).length
  const guidance = el('p', { class: activeCount ? 'section-note delivery-area-guidance' : 'note delivery-area-guidance', role: activeCount ? 'note' : 'status' }, activeCount ? `Rozvoz je povolen pouze do aktivních PSČ. Aktivních oblastí: ${activeCount}. Pozastavení ovlivní nové objednávky, již přijaté objednávky zůstanou zachovány.` : 'Rozvoz není povolen do žádného PSČ. Pro příjem objednávek přidejte první oblast nebo zapněte některou z existujících.')
  return [header('Oblasti rozvozu', 'Určete přesná PSČ, na která přijímáte objednávky.', button('+ Přidat oblast', () => deliveryAreaDialog(), 'primary')), guidance, el('div', { class: 'filters delivery-area-filters' }, search, button('Obnovit', renderPage)), results]
}
function deliveryAreaDialog(area = null) {
  const body = openDialog(area ? 'Upravit oblast rozvozu' : 'Přidat oblast rozvozu')
  const form = el('form', { onsubmit: event => saveForm(event, async values => {
    const postalCode = normalizePostalCode(values.get('postal_code'))
    if (!/^[0-9]{5}$/.test(postalCode)) throw new Error('PSČ musí obsahovat právě pět číslic. Mezery můžete vynechat.')
    const payload = { postal_code: postalCode, label: String(values.get('label') || '').trim(), delivery_slots: readDeliverySlots(values), active: values.has('active') }
    if (area) payload.version = area.version
    await api(area ? `/api/admin/delivery-areas/${area.id}` : '/api/admin/delivery-areas', { method: area ? 'PUT' : 'POST', body: JSON.stringify(payload) })
    dialog.close()
    notify(area ? 'Oblast rozvozu byla upravena.' : 'Oblast rozvozu byla přidána.')
    renderPage()
  }) }, field('PSČ', 'postal_code', area?.postal_code || '', { required: true, inputmode: 'numeric', autocomplete: 'off', pattern: '[0-9]{5}', placeholder: '11000', hint: 'Pět číslic, například 110 00. Při psaní a vložení mezery odstraníme.' }), field('Název oblasti', 'label', area?.label || '', { maxlength: 100, placeholder: 'Např. Praha 1', hint: 'Nepovinné označení pro snadnou orientaci.' }), deliverySlotsField(area?.delivery_slots, true), field('Povolit rozvoz do tohoto PSČ', 'active', area ? area.active : true, { type: 'checkbox' }), el('p', { class: 'hint' }, 'Zákazník může objednat pouze na aktivní PSČ. Oblast můžete kdykoli pozastavit a později znovu zapnout.'), el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { class: 'button primary', type: 'submit' }, 'Uložit oblast')))
  const postal = form.querySelector('[name=postal_code]')
  const normalizeInput = () => {
    const start = postal.selectionStart ?? postal.value.length
    const normalizedStart = normalizePostalCode(postal.value.slice(0, start)).length
    const normalized = normalizePostalCode(postal.value)
    if (postal.value !== normalized) {
      postal.value = normalized
      postal.setSelectionRange(normalizedStart, normalizedStart)
    }
    postal.setCustomValidity(postal.value && !/^[0-9]{5}$/.test(postal.value) ? 'PSČ musí obsahovat právě pět číslic.' : '')
  }
  postal.addEventListener('input', normalizeInput)
  postal.addEventListener('paste', event => {
    if (!event.clipboardData) return
    event.preventDefault()
    postal.setRangeText(normalizePostalCode(event.clipboardData.getData('text')), postal.selectionStart ?? 0, postal.selectionEnd ?? postal.value.length, 'end')
    normalizeInput()
  })
  normalizeInput()
  body.append(form)
}

function orderEmailRecipients(primaryValue, additionalValue) {
  const primary = String(primaryValue || '').trim().toLowerCase()
  const addresses = [primary, ...String(additionalValue || '').split(/[\r\n,;]+/).map(value => value.trim().toLowerCase()).filter(Boolean)]
  const validator = document.createElement('input')
  validator.type = 'email'
  validator.required = true
  const unique = new Set()
  for (const address of addresses) {
    validator.value = address
    if (address.length > 254 || !validator.checkValidity()) throw new Error(`Zkontrolujte e-mailovou adresu „${address || 'prázdná adresa'}“.`)
    unique.add(address)
  }
  if (unique.size > 20) throw new Error('Objednávky lze posílat nejvýše na 20 různých e-mailů včetně hlavního příjemce.')
  unique.delete(primary)
  return { primary, additional: [...unique] }
}

function pickupSettingsForm(pickup) {
  const form = el('form', { class: 'panel settings-panel pickup-settings', 'aria-labelledby': 'pickup-settings-title', onsubmit: event => saveForm(event, async data => {
    form.querySelector('[data-form-error]')?.remove()
    const payload = {
      enabled: data.has('pickup_enabled'), name: String(data.get('pickup_name') || '').trim(),
      street: String(data.get('pickup_street') || '').trim(), city: String(data.get('pickup_city') || '').trim(),
      postal_code: normalizePostalCode(data.get('pickup_postal_code')),
      instructions: String(data.get('pickup_instructions') || '').trim(), parking_free: data.has('pickup_parking_free'),
      slots: String(data.get('pickup_slots') || '').split(/\r?\n/).map(value => value.trim()).filter(Boolean), version: pickup.version
    }
    if (payload.enabled && (!payload.name || !payload.street || !payload.city || !/^[0-9]{5}$/.test(payload.postal_code) || !payload.slots.length)) throw new Error('Pro zapnutí osobního odběru doplňte název místa, celou adresu a alespoň jedno časové okno.')
    if (payload.postal_code && !/^[0-9]{5}$/.test(payload.postal_code)) throw new Error('PSČ odběrného místa musí obsahovat pět číslic.')
    if (payload.slots.length > 24) throw new Error('Zadejte nejvýše 24 časových oken osobního odběru.')
    let previousEnd = ''
    payload.slots = payload.slots.map(value => {
      const match = value.match(/^([0-2][0-9]:[0-5][0-9])\s*[–-]\s*([0-2][0-9]:[0-5][0-9])$/)
      if (!match || match[1] >= '24:00' || match[2] > '24:00' || match[1] >= match[2]) throw new Error('Časové okno osobního odběru zadejte například jako 08:00–10:00. Konec musí být po začátku.')
      return `${match[1]}–${match[2]}`
    }).sort()
    for (const slot of payload.slots) {
      const [start, end] = slot.split('–')
      if (start < previousEnd) throw new Error('Časová okna osobního odběru se nesmějí překrývat.')
      previousEnd = end
    }
    const controls = [...form.querySelectorAll('input, textarea, button')]
    controls.forEach(control => { control.disabled = true })
    try {
      const updated = await api('/api/admin/pickup-settings', { method: 'PUT', body: JSON.stringify(payload) })
      Object.assign(pickup, updated)
      applySaved()
      notify('Nastavení osobního odběru bylo uloženo.')
    } finally { controls.forEach(control => { control.disabled = false }) }
  }) }, el('h2', { id: 'pickup-settings-title' }, 'Osobní odběr'),
  el('p', { class: 'hint' }, 'Zákazník si vybere osobní odběr bez poplatku za dopravu. Změny adresy a časů platí pro nové objednávky. Přijaté objednávky uchovají místo a pokyny platné při objednání.'),
  field('Povolit osobní odběr', 'pickup_enabled', pickup.enabled, { type: 'checkbox' }),
  field('Název odběrného místa', 'pickup_name', pickup.name, { maxlength: 200 }),
  field('Ulice a číslo odběrného místa', 'pickup_street', pickup.street, { maxlength: 250, autocomplete: 'off' }),
  el('div', { class: 'form-grid' }, field('Město odběrného místa', 'pickup_city', pickup.city, { maxlength: 100, autocomplete: 'off' }), field('PSČ odběrného místa', 'pickup_postal_code', formatPostalCode(pickup.postal_code), { maxlength: 8, inputmode: 'numeric', autocomplete: 'off', pattern: '[0-9 ]{5,8}' })),
  field('Časová okna osobního odběru', 'pickup_slots', (pickup.slots || []).join('\n'), { kind: 'textarea', rows: 3, maxlength: 1000, placeholder: '08:00–10:00\n14:00–16:00', hint: 'Jedno okno na řádek. Odběr se nabízí od nejbližšího pracovního dne. Zákazník zvolí den i čas a obsluha je potvrdí.' }),
  field('Pokyny k odběru', 'pickup_instructions', pickup.instructions, { kind: 'textarea', rows: 3, maxlength: 2000, placeholder: 'Například vstup do prodejny ze dvora.' }),
  field('Parkování zdarma', 'pickup_parking_free', pickup.parking_free !== false, { type: 'checkbox', hint: 'Informace se zobrazí u odběrného místa i v potvrzení objednávky.' }),
  el('div', { class: 'actions' }, el('button', { class: 'button primary', type: 'submit' }, 'Uložit osobní odběr'), button('Obnovit osobní odběr', async event => {
    if (!confirm('Načíst uloženou adresu a časy osobního odběru? Rozepsané změny v této části se ztratí.')) return
    const trigger = event.currentTarget
    trigger.disabled = true
    try {
      Object.assign(pickup, await api('/api/admin/pickup-settings'))
      applySaved()
      form.querySelector('[data-form-error]')?.remove()
    } catch (error) { formError(form, error) }
    finally { trigger.disabled = false }
  })))
  function updateRequired() {
    for (const name of ['name', 'street', 'city', 'postal_code', 'slots']) form.elements[`pickup_${name}`].required = form.elements.pickup_enabled.checked
  }
  function applySaved() {
    for (const name of ['name', 'street', 'city', 'instructions']) form.elements[`pickup_${name}`].value = pickup[name] || ''
    form.elements.pickup_postal_code.value = formatPostalCode(pickup.postal_code)
    form.elements.pickup_slots.value = (pickup.slots || []).join('\n')
    form.elements.pickup_enabled.checked = Boolean(pickup.enabled)
    form.elements.pickup_parking_free.checked = pickup.parking_free !== false
    updateRequired()
  }
  form.elements.pickup_enabled.addEventListener('change', updateRequired)
  updateRequired()
  return form
}

async function settingsPage() {
  const [settings, pickup] = await Promise.all([api('/api/admin/settings'), api('/api/admin/pickup-settings')])
  const form = el('form', { class: 'panel settings-panel', onsubmit: event => saveForm(event, async data => {
    notice.hidden = true
    form.querySelector('[data-form-error]')?.remove()
    const recipients = orderEmailRecipients(data.get('order_email'), data.get('additional_order_emails'))
    const payload = { delivery_fee_cents: cents(data.get('delivery_fee')), free_delivery_from_cents: cents(data.get('free_delivery_from'), true), order_email: recipients.primary, additional_order_emails: recipients.additional, delivery_slots: readDeliverySlots(data) || [], version: settings.version }
    const controls = [...form.querySelectorAll('input, textarea')]
    controls.forEach(control => { control.disabled = true })
    try {
      Object.assign(settings, await api('/api/admin/settings', { method: 'PUT', body: JSON.stringify(payload) }))
      form.elements.order_email.value = settings.order_email
      form.elements.additional_order_emails.value = (settings.additional_order_emails || []).join('\n')
      form.elements.delivery_slots.value = (settings.delivery_slots || []).join('\n')
      notify('Nastavení bylo uloženo.')
    } finally { controls.forEach(control => { control.disabled = false }) }
  }) }, el('p', { class: 'hint' }, 'Změny se použijí pro nové objednávky. Již přijaté objednávky uchovají původní dopravu a celkovou cenu.'), deliverySlotsField(settings.delivery_slots), field('Cena dopravy v Kč', 'delivery_fee', (settings.delivery_fee_cents / 100).toFixed(2), { required: true, inputmode: 'decimal' }), field('Doprava zdarma od částky v Kč', 'free_delivery_from', settings.free_delivery_from_cents === null ? '' : (settings.free_delivery_from_cents / 100).toFixed(2), { inputmode: 'decimal', hint: 'Prázdné pole vypne dopravu zdarma. Limit se vztahuje na cenu zboží.' }), field('Hlavní e-mail pro příjem objednávek', 'order_email', settings.order_email, { type: 'email', required: true, maxlength: 254, autocomplete: 'off', hint: 'Tento e-mail přijímá objednávky a zákazníkům se uvádí jako kontakt pro jejich vyřízení.' }), field('Další příjemci objednávek', 'additional_order_emails', (settings.additional_order_emails || []).join('\n'), { kind: 'textarea', rows: 4, maxlength: 10000, autocomplete: 'off', placeholder: 'kuchyne@example.com\nrozvoz@example.com', hint: 'Adresy oddělte novým řádkem, čárkou nebo středníkem. Nejvýše 20 různých příjemců včetně hlavního e-mailu.' }), el('p', { class: 'hint' }, 'Každý příjemce dostane samostatné oznámení. U platby při převzetí přijde po přijetí objednávky, u online platby po ověření úhrady. Zákazníkům se jako kontakt uvádí pouze hlavní e-mail.'), el('div', { class: 'actions' }, el('button', { type: 'submit', class: 'button primary' }, 'Uložit nastavení')))
  return [header('Doprava a nastavení', 'Rozvoz, osobní odběr a příjemci objednávek.', button('Obnovit', renderPage)), form, pickupSettingsForm(pickup)]
}
async function customersPage() {
  const data = await api(`/api/admin/customers?page=${state.customerPage}`)
  return [header('Zákazníci', 'Zákaznické účty a všechny objednávky na jejich e-mail.', button('Obnovit', renderPage)), data.customers.length ? table(['E-mail', 'Registrace', 'Objednávky', 'Oprávnění'], data.customers.map(customer => el('tr', {}, el('td', {}, customer.email), el('td', {}, dateTime(customer.created_at)), el('td', {}, customer.order_count), el('td', {}, badge(customer.is_admin ? 'Správce' : 'Zákazník'))))) : empty('Zatím žádné účty', 'Účet vznikne při první objednávce nebo prvním přihlášení e-mailem.'), pager(data.page, data.total, page => { state.customerPage = page; renderPage() }), el('p', { class: 'section-note' }, 'Objednávky se k účtu přiřazují podle e-mailu, včetně dřívějších nákupů bez přihlášení. Zákazník získá přístup k historii až po ověření odkazu z e-mailu.')]
}
async function administratorsPage() {
  const { administrators } = await api('/api/admin/administrators')
  const lastAdministrator = administrators.length === 1
  const rows = administrators.map(administrator => {
    const self = administrator.id === state.user?.id
    const remove = button('Odebrat oprávnění', () => removeAdministratorDialog(administrator), 'small danger')
    remove.disabled = lastAdministrator
    remove.setAttribute('aria-label', `Odebrat oprávnění správce ${administrator.email}`)
    if (lastAdministrator) remove.setAttribute('aria-describedby', 'last-administrator-note')
    const password = button('Nastavit nové heslo', () => administratorPasswordDialog(administrator), 'small')
    password.setAttribute('aria-label', `Nastavit nové heslo pro ${administrator.email}`)
    return el('tr', {}, el('td', { class: 'administrator-email' }, el('strong', {}, administrator.email), self ? el('small', {}, badge('Vy')) : null), el('td', {}, el('div', { class: 'cell-actions administrator-actions' }, password, remove)))
  })
  return [header('Administrátoři', 'Správci mají přístup ke všem částem administrace obchodu.', button('+ Přidat správce', addAdministratorDialog, 'primary')), el('div', { class: 'administrator-tools' }, button('Obnovit', renderPage, 'small')), table(['E-mail správce', 'Akce'], rows), el('p', { class: 'section-note', id: 'last-administrator-note' }, lastAdministrator ? 'Tento účet je poslední správce a nelze mu odebrat oprávnění. Nejprve přidejte dalšího správce.' : 'V obchodě musí zůstat alespoň jeden správce. Odebrání oprávnění nevymaže zákaznický účet ani jeho objednávky.')]
}
function addAdministratorDialog() {
  const body = openDialog('Přidat správce')
  const form = el('form', { onsubmit: event => saveForm(event, async data => {
    const { administrator } = await api('/api/admin/administrators', { method: 'POST', body: JSON.stringify({ email: String(data.get('email')).trim(), password: String(data.get('password')) }) })
    dialog.close()
    notify(`Správce ${administrator.email} byl přidán.`)
    renderPage()
  }) }, field('E-mail nového správce', 'email', '', { type: 'email', autocomplete: 'off', required: true, maxlength: 254, autofocus: true }), administratorPasswordField(), el('p', { class: 'hint' }, 'Nový správce získá přístup ke všem částem administrace. Přihlásí se tímto e-mailem a heslem. Přístupové údaje mu předejte bezpečně, e-mailem je automaticky neposíláme. Případný zákaznický účet i objednávky zůstanou zachované.'), el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { class: 'button primary', type: 'submit' }, 'Přidat správce')))
  body.append(form)
  form.querySelector('[name=email]').focus()
}
function administratorPasswordField() {
  return field('Nové heslo', 'password', '', { type: 'password', autocomplete: 'new-password', required: true, minlength: 12, maxlength: 128, hint: 'Použijte 12 až 128 znaků a heslo, které nepoužíváte jinde.' })
}
function administratorPasswordDialog(administrator) {
  const self = administrator.id === state.user?.id
  const body = openDialog('Nastavit nové heslo')
  const form = el('form', { onsubmit: event => saveForm(event, async data => {
    await api('/api/admin/administrators/password', { method: 'PUT', body: JSON.stringify({ email: administrator.email, password: String(data.get('password')) }) })
    dialog.close()
    notify(self ? 'Heslo bylo změněno. Přihlaste se novým heslem.' : `Nové heslo pro ${administrator.email} bylo nastaveno.`)
    if (self) {
      state.generation++
      state.user = null
      nav.hidden = true
      await checkAuth()
    } else {
      renderPage()
    }
  }) }, el('p', { class: 'administrator-email' }, 'Účet: ', el('strong', {}, administrator.email)), el('p', { class: 'section-note' }, self ? 'Změna hesla ukončí všechna vaše přihlášení. Potom se přihlaste novým heslem.' : 'Změna hesla ukončí všechna přihlášení tohoto správce. Nové heslo mu předejte bezpečně, e-mailem ho automaticky neposíláme.'), administratorPasswordField(), el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { class: 'button primary', type: 'submit' }, 'Uložit nové heslo')))
  body.append(form)
  form.querySelector('[name=password]').focus()
}
function removeAdministratorDialog(administrator) {
  const self = administrator.id === state.user?.id
  const body = openDialog('Odebrat oprávnění správce')
  const form = el('form', { onsubmit: event => saveForm(event, async () => {
    await api('/api/admin/administrators', { method: 'DELETE', body: JSON.stringify({ email: administrator.email }) })
    dialog.close()
    notify(self ? 'Vaše oprávnění správce bylo odebráno. Byli jste odhlášeni.' : `Oprávnění správce ${administrator.email} bylo odebráno.`)
    if (self) {
      state.generation++
      state.user = null
      nav.hidden = true
      await checkAuth()
    } else {
      renderPage()
    }
  }) }, el('p', { class: 'administrator-email' }, 'Odebrat oprávnění správce účtu ', el('strong', {}, administrator.email), '?'), el('p', { class: 'section-note' }, 'Účet ihned ztratí přístup do administrace a jeho přihlášení budou ukončena. Zákaznický účet i objednávky zůstanou zachované.'), self ? el('p', { class: 'note' }, 'Odebíráte vlastní oprávnění. Budete odhlášeni a do administrace se už nedostanete, dokud vás jiný správce znovu nepřidá.') : null, el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { class: 'button danger', type: 'submit' }, 'Odebrat oprávnění')))
  body.append(form)
  form.querySelector('button[type=button]').focus()
}
function composeBroadcast(recipientCount) {
  const body = openDialog('Hromadný e-mail zákazníkům')
  const storageKey = `brunch.mail.pending.${state.user.id}`
  let pendingRequest = null
  const form = el('form', { class: 'form-grid', onsubmit: event => saveForm(event, async data => {
    const subject = String(data.get('subject')).trim()
    const message = String(data.get('body')).trim()
    if (!subject || !message) throw new Error('Vyplňte předmět i text zprávy.')
    // Retain only a digest and request ID, so retrying after a lost response or reload is safe.
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify([subject, message])))
    const fingerprint = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('')
    let key = pendingRequest?.fingerprint === fingerprint ? pendingRequest.key : crypto.randomUUID()
    try {
      const pending = JSON.parse(sessionStorage.getItem(storageKey))
      if (pending?.fingerprint === fingerprint) key = pending.key
      sessionStorage.setItem(storageKey, JSON.stringify({ fingerprint, key }))
    } catch { /* Private browsing can disable session storage. The open form retains its key. */ }
    pendingRequest = { fingerprint, key }
    const result = await api('/api/admin/mail/broadcast', { method: 'POST', body: JSON.stringify({ idempotency_key: key, subject, body: message }) })
    try { sessionStorage.removeItem(storageKey) } catch { /* Storage may be unavailable. */ }
    dialog.close()
    notify(`Zpráva byla zařazena k odeslání. Počet příjemců: ${result.recipient_count}.`)
    renderPage()
  }, 'Zařazuji zprávy…') },
  el('p', { class: 'full section-note' }, `Počet příjemců podle dosavadních objednávek: ${recipientCount}. Každý dostane samostatný e-mail.`),
  field('Předmět', 'subject', '', { full: true, required: true, maxlength: 200 }),
  field('Text e-mailu', 'body', '', { kind: 'textarea', full: true, required: true, rows: 8, maxlength: 12000 }),
  el('div', { class: 'actions full' }, button('Zrušit', () => dialog.close()), el('button', { class: 'button primary', type: 'submit' }, 'Odeslat všem zákazníkům')))
  body.append(form)
  form.elements.subject.focus()
}

async function mailPage() {
  const [messages, audience] = await Promise.all([api('/api/admin/mail'), api('/api/admin/mail/broadcast')])
  const compose = button('Napsat hromadný e-mail', () => composeBroadcast(audience.recipient_count), 'primary')
  compose.disabled = audience.recipient_count === 0
  const rows = messages.map(message => el('tr', {}, el('td', { class: 'mail-subject' }, el('strong', {}, message.subject), el('small', {}, message.to_email)), el('td', {}, badge(mailStates[message.status] || message.status, message.status), el('small', {}, `Pokusy: ${message.attempts}`)), el('td', {}, dateTime(message.created_at), message.sent_at ? el('small', {}, `Odesláno ${dateTime(message.sent_at)}`) : null), el('td', {}, message.last_error ? el('small', { class: 'mail-error' }, message.last_error) : 'Bez chyby'), el('td', {}, message.can_retry ? button('Zkusit znovu', async event => {
    const retryButton = event.currentTarget
    retryButton.disabled = true
    try {
      await api(`/api/admin/mail/${message.id}/retry`, { method: 'POST', body: '{}' })
      notify('E-mail byl znovu zařazen k odeslání.')
      renderPage()
    } catch (error) { notify(error.message, true); retryButton.disabled = false }
  }, 'small') : message.status === 'failed' ? el('small', {}, 'Vyžádejte nový přihlašovací odkaz.') : null)))
  const tools = el('div', { class: 'actions' }, compose, button('Obnovit', renderPage))
  return [header('E-maily', `Příjemci hromadného e-mailu: ${audience.recipient_count}. Přehled zobrazuje posledních 100 zpráv.`, tools), rows.length ? table(['Zpráva / příjemce', 'Stav', 'Čas', 'Poslední chyba', 'Akce'], rows) : empty('Žádné zprávy', 'Zde se zobrazí přihlášení, potvrzení objednávek a zprávy zákazníkům i provozovateli.'), el('p', { class: 'section-note' }, 'Stav Odesláno potvrzuje převzetí poštovním serverem. Nezaručuje doručení do doručené pošty příjemce.')]
}

dialog.addEventListener('close', () => { for (const input of dialog.querySelectorAll('input[type=password]')) input.value = '' })
window.addEventListener('hashchange', () => { if (dialog.open) dialog.close(); renderPage() })
document.addEventListener('visibilitychange', () => { if (!document.hidden && !state.user) checkAuth() })
checkAuth()
