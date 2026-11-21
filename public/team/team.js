'use strict'
const content = document.querySelector('#content')
const params = new URLSearchParams(location.hash.slice(1))
const invite = params.get('join')
let products = []
let banquetMinimum = 30
let selectedId = params.get('team')
let loadSequence = 0
let editing = false
let refreshTimer
let contributionSecret = null
let storageWarning = false
const money = value => new Intl.NumberFormat('cs-CZ', { style: 'currency', currency: 'CZK', maximumFractionDigits: 0 }).format(value / 100)
const teamCaption = team => `${team.name} · ${team.order_id ? 'Objednáno' : team.closed_at ? 'Uzavřeno' : new Date(team.deadline) < new Date() ? 'Po uzávěrce' : 'Výběr otevřen'} · ${date(team.deadline)}`
const date = value => new Intl.DateTimeFormat('cs-CZ', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'Europe/Prague' }).format(new Date(value))
function node(tag, attributes = {}, ...children) {
  const element = document.createElement(tag)
  for (const [key, value] of Object.entries(attributes)) {
    if (key.startsWith('on')) element.addEventListener(key.slice(2), value)
    else if (key === 'class') element.className = value
    else if (value !== null && value !== false) element.setAttribute(key, value === true ? '' : String(value))
  }
  for (const child of children.flat()) if (child != null) element.append(child instanceof Node ? child : document.createTextNode(String(child)))
  return element
}
function button(text, action, secondary = false) { return node('button', { type: 'button', class: secondary ? 'secondary' : '', onclick: action }, text) }
function field(label, input, hint) { return node('label', { class: 'field' }, node('span', {}, label), input, hint ? node('small', {}, hint) : null) }
function notice(text, kind = '', role = 'status') { return node('p', { class: `notice ${kind}`, role }, text) }
function error(target, e) {
  target.querySelector('[data-error]')?.remove()
  const message = notice(e.message || 'Požadavek se nepodařil. Zkuste to znovu.', 'error', 'alert')
  message.dataset.error = '1'
  target.prepend(message)
  message.scrollIntoView({ block: 'nearest' })
}
async function api(path, method = 'GET', body, extraHeaders = {}) {
  const response = await fetch(path, { method, credentials: 'same-origin', headers: { 'x-brunch-request': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }), ...extraHeaders }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) })
  const data = await response.json()
  if (!response.ok) { const e = new Error(data.error || 'Požadavek se nepodařil.'); e.status = response.status; throw e }
  return data
}
function defaultDeadline() {
  const parts = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Europe/Prague', year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date(Date.now() + 86400000))
  const get = type => parts.find(p => p.type === type).value
  return `${get('year')}-${get('month')}-${get('day')}T14:00`
}
function ownToken() { if (contributionSecret) return contributionSecret; try { return localStorage.getItem(`brunch-team-contribution:${invite}`) } catch { return null } }
function freshToken() { return [...crypto.getRandomValues(new Uint8Array(32))].map(b => b.toString(16).padStart(2, '0')).join('') }
function login() {
  content.replaceChildren(node('h1', {}, 'Objednávka pro celý tým'), node('p', {}, 'Založte společný košík, pošlete kolegům odkaz a každý si vybere svou svačinu. Vy pak výběr zkontrolujete a objednáte společně.'), notice('Pro správu týmového košíku se přihlaste svým e-mailem. Kolegové účet nepotřebují.'), node('a', { href: '/?signin=team', class: 'button' }, 'Přihlásit se a pokračovat'))
}
async function organizer() {
  const teams = await api('/api/team-orders')
  content.replaceChildren(node('h1', {}, 'Společné objednávky'), node('p', {}, 'Kolegové si vyberou přes sdílený odkaz. Vy společný košík uzavřete, zkontrolujete a odešlete v obchodě. Výběr zatím nerezervuje zboží.'))
  const form = node('form', { class: 'card create-card stack' })
  const name = node('input', { name: 'team-name', required: true, maxlength: 100, placeholder: 'Např. Páteční snídaně pro kancelář' })
  const cutoff = node('input', { name: 'team-deadline', type: 'datetime-local', required: true, value: defaultDeadline() })
  const create = node('button', { type: 'submit' }, 'Vytvořit společný košík')
  form.append(node('h2', {}, 'Nová společná objednávka'), field('Název košíku', name), field('Uzávěrka výběru', cutoff, 'Čas v České republice. Za 5 minut až 31 dní. Termín doručení zvolíte později v objednávce.'), create)
  form.addEventListener('submit', async event => {
    event.preventDefault(); create.disabled = true
    try { const team = await api('/api/team-orders', 'POST', { name: name.value, deadline: cutoff.value }); selectedId = team.id; history.replaceState(null, '', `#team=${team.id}`); await organizer() }
    catch (e) { error(form, e) }
    finally { create.disabled = false }
  })
  content.append(form)
  const list = node('ul', { class: 'team-list' })
  for (const team of teams) {
    const teamButton = button(teamCaption(team), async () => {
      if (editing && !confirm('Opustit rozepsanou úpravu výběru?')) return
      editing = false; selectedId = team.id; history.replaceState(null, '', `#team=${team.id}`); await loadTeam()
    }, true)
    teamButton.dataset.teamId = team.id
    list.append(node('li', {}, teamButton))
  }
  content.append(node('h2', {}, 'Vaše týmové košíky'), teams.length ? list : node('p', { class: 'empty' }, 'Zatím nemáte žádný týmový košík.'), node('section', { id: 'team-detail', 'aria-live': 'polite' }))
  if (selectedId) await loadTeam()
}
async function loadTeam() {
  const target = document.querySelector('#team-detail')
  if (!target || !selectedId || editing) return
  const requestedId = selectedId
  const sequence = ++loadSequence
  try {
    const [data, settings] = await Promise.all([api(`/api/team-orders/${requestedId}`), api('/api/settings')])
    if (editing || requestedId !== selectedId || sequence !== loadSequence) return
    banquetMinimum = settings.banquet_minimum_quantity ?? 30
    for (const product of data.products || []) { const index = products.findIndex(p => p.id === product.id); if (index < 0) products.push(product); else products[index] = product }
    const { team, contributions } = data
    document.querySelectorAll('[data-team-id]').forEach(button => { if (button.dataset.teamId === team.id) button.textContent = teamCaption(team) })
    const card = node('div', { class: 'card' }, node('div', { class: 'row' }, node('h2', {}, team.name), node('span', { class: 'status' }, team.order_id ? 'Objednávka odeslána' : team.closed_at ? 'Uzavřený výběr' : new Date(team.deadline) < new Date() ? 'Uzávěrka uplynula' : 'Kolegové mohou vybírat')), node('p', {}, `Uzávěrka: ${date(team.deadline)} českého času`))
    const link = `${location.origin}/team/#join=${team.invite_token}`
    if (!team.closed_at) {
      const input = node('input', { readonly: true, value: link, 'aria-label': 'Odkaz pro kolegy', class: 'link-field' })
      card.append(node('div', { class: 'actions' }, input, button('Kopírovat odkaz', async () => {
        try { await navigator.clipboard.writeText(link); card.querySelector('[data-copy]')?.remove(); const message = notice('Odkaz je zkopírovaný. Sdílejte jej pouze s kolegy, kteří mají přispět.'); message.dataset.copy = '1'; card.append(message) }
        catch { input.focus(); input.select(); error(card, new Error('Odkaz označte a zkopírujte ručně.')) }
      }, true), node('a', { href: link, class: 'button secondary' }, 'Přidat vlastní výběr')))
    }
    if (!team.order_id) card.append(notice('Zboží zatím není rezervované. Ceny a dostupnost se znovu ověří při dokončení objednávky.'))
    let total = 0
    let banquetQuantity = 0
    for (const entry of contributions) {
      const items = node('ul')
      for (const item of entry.items) {
        const product = products.find(p => p.id === item.product_id)
        total += (product?.price_cents || 0) * item.quantity
        if (product?.banquet) banquetQuantity += item.quantity
        items.append(node('li', {}, `${item.quantity}× ${product?.name || 'Nedostupná položka'}${item.variant ? ` (${item.variant})` : ''}`))
      }
      const allocation = node('div', { class: 'allocation' }, node('div', { class: 'row' }, node('h3', {}, entry.name), !team.closed_at ? button('Upravit výběr', () => editEntry(card, team, entry), true) : null), entry.items.length ? items : node('p', { class: 'muted' }, 'Výběr odebrán.'))
      card.append(allocation)
    }
    if (banquetQuantity > 0 && !team.order_id) card.append(notice(`Banketní pečivo za celý tým: ${banquetQuantity} z ${banquetMinimum} ks. ${banquetQuantity < banquetMinimum ? `Do společného minima přidejte ještě ${banquetMinimum - banquetQuantity} ks banketního pečiva. Druhy můžete kombinovat.` : 'Společné minimum je splněno.'}`, banquetQuantity < banquetMinimum ? '' : 'success'))
    if (!contributions.length) card.append(node('p', { class: 'empty' }, 'Zatím nikdo neodeslal svůj výběr. Pošlete kolegům odkaz.'))
    card.append(node('p', { class: 'total' }, `Zboží podle aktuálních cen: ${money(total)}`), node('p', { class: 'muted' }, 'Cenu dopravy uvidíte při dokončení objednávky.'))
    const actions = node('div', { class: 'actions' }, button('Obnovit výběry', loadTeam, true), button('Vytisknout rozpis', () => window.print(), true))
    if (team.order_id) {
      card.append(notice('Týmový košík již byl objednán. Stav doručení najdete ve svém zákaznickém účtu. Pro další objednávku založte nový týmový košík.'))
    } else if (team.closed_at) {
      actions.append(button('Znovu otevřít výběr', () => reopenForm(card, team), true))
      actions.append(button('Přenést do objednávky', async event => {
        const trigger = event.currentTarget
        trigger.disabled = true
        try {
          const exported = await api(`/api/team-orders/${team.id}/export`)
          sessionStorage.setItem('brunch-team-import', JSON.stringify(exported))
          location.assign('/#team-checkout')
        } catch (e) { error(card, e); trigger.disabled = false }
      }))
    } else {
      actions.append(button('Uzavřít výběr kolegů', async event => {
        if (!confirm('Uzavřít výběr? Kolegové už nebudou moci měnit položky. Objednávku ještě samostatně zkontrolujete a odešlete.')) return
        const trigger = event.currentTarget
        trigger.disabled = true
        try { await api(`/api/team-orders/${team.id}/close`, 'POST', {}); await loadTeam() }
        catch (e) { error(card, e); trigger.disabled = false }
      }))
    }
    card.append(actions)
    target.replaceChildren(card)
  } catch (e) { if (requestedId === selectedId && sequence === loadSequence && !editing) error(target, e) }
}
function reopenForm(card, team) {
  editing = true
  card.querySelector('.editor')?.remove()
  const form = node('form', { class: 'stack editor' })
  const cutoff = node('input', { type: 'datetime-local', required: true, value: defaultDeadline() })
  const submit = node('button', { type: 'submit' }, 'Otevřít výběr s novou uzávěrkou')
  form.append(node('h3', {}, 'Nová uzávěrka'), notice('Tímto nezměníte ani nezrušíte případnou odeslanou objednávku. Pokud jste výběr už přenesli do košíku obchodu, před novým přenosem jeho položky odeberte.'), field('Uzávěrka v českém čase', cutoff), node('div', { class: 'actions' }, submit, button('Zrušit', async () => { editing = false; await loadTeam() }, true)))
  form.addEventListener('submit', async event => {
    event.preventDefault(); submit.disabled = true
    try { await api(`/api/team-orders/${team.id}/reopen`, 'POST', { deadline: cutoff.value }); editing = false; await loadTeam() }
    catch (e) { error(form, e) }
    finally { submit.disabled = false }
  })
  card.append(form)
  form.scrollIntoView({ block: 'start' })
}
function selectionForm(initial, save, cancel) {
  const form = node('form', { class: 'stack editor' })
  const name = node('input', { name: 'contributor-name', required: true, maxlength: 80, value: initial?.name || '', autocomplete: 'given-name' })
  const selected = new Map((initial?.items || []).map(item => [item.product_id, { ...item }]))
  const total = node('span', { class: 'total' })
  const updateTotal = () => { total.textContent = `Vaše svačiny: ${money([...selected.values()].reduce((sum, item) => sum + (products.find(p => p.id === item.product_id)?.price_cents || 0) * item.quantity, 0))}` }
  form.append(field('Vaše jméno pro organizátora', name), node('p', { class: 'muted' }, 'Počet 0 znamená, že položku nechcete. Minimální odběr se počítá za celý tým.'), node('div', { class: 'products' }))
  if (products.some(product => product.banquet)) form.insertBefore(notice(`Banketní pečivo má společný minimální odběr ${banquetMinimum} ks za celý tým. Druhy lze kombinovat. Svůj díl můžete uložit i v menším množství.`), form.querySelector('.products'))
  const grid = form.querySelector('.products')
  const available = [...products]
  for (const old of initial?.items || []) {
    if (!available.some(p => p.id === old.product_id)) available.push({ id: old.product_id, name: 'Nedostupná položka', price_cents: 0, active: false, stock: 0, variants: [] })
  }
  for (const product of available) {
    const max = product.active === false ? 0 : Math.min(product.stock ?? 99, 99)
    const input = node('input', { type: 'number', name: `quantity-${product.id}`, min: 0, max: Math.max(max, selected.get(product.id)?.quantity || 0), step: 1, inputmode: 'numeric', value: selected.get(product.id)?.quantity || 0, 'aria-label': `Počet: ${product.name}` })
    let variant = null
    if (product.variants?.length) {
      variant = node('select', { 'aria-label': `Varianta: ${product.name}`, name: `variant-${product.id}` }, node('option', { value: '' }, 'Vyberte variantu'), product.variants.map(v => node('option', { value: v }, v)))
      variant.value = selected.get(product.id)?.variant || ''
    }
    const change = () => {
      const count = Number(input.value)
      if (Number.isInteger(count) && count > 0) selected.set(product.id, { product_id: product.id, quantity: count, ...(variant?.value ? { variant: variant.value } : {}) })
      else selected.delete(product.id)
      updateTotal()
    }
    input.addEventListener('input', change)
    variant?.addEventListener('change', change)
    const info = node('div', {}, node('h3', {}, product.name), product.banquet ? node('small', { class: 'banquet-label' }, 'Banketní pečivo · ') : null, node('span', {}, `${money(product.price_cents)} / ks`), node('small', {}, ` · ${product.stock == null ? `max. ${max} ks v objednávce` : `${max} ks dostupných`}${product.minimum_quantity > 1 ? ` · minimum za tým ${product.minimum_quantity} ks` : ''}`), field('Počet kusů', input), variant ? field('Varianta', variant) : null)
    grid.append(node('div', { class: 'product' }, product.image ? node('img', { src: product.image.startsWith('/') ? product.image : `/${product.image}`, alt: '', loading: 'lazy', width: 80, height: 80 }) : node('span'), info))
  }
  updateTotal()
  const submit = node('button', { type: 'submit' }, 'Uložit výběr')
  form.append(node('div', { class: 'form-actions' }, total, submit, cancel ? button('Zrušit úpravu', cancel, true) : null))
  form.addEventListener('submit', async event => {
    event.preventDefault()
    const items = [...selected.values()]
    const missing = items.find(item => products.find(p => p.id === item.product_id)?.variants?.length && !item.variant)
    if (missing) { error(form, new Error('U vybraných položek prosím zvolte variantu.')); return }
    submit.disabled = true
    try { await save({ name: name.value, items }) }
    catch (e) { error(form, e) }
    finally { submit.disabled = false }
  })
  return form
}
function editEntry(card, team, entry) {
  editing = true
  card.querySelector('.editor')?.remove()
  const form = selectionForm(entry, async input => {
    await api(`/api/team-orders/${team.id}/contributions/${entry.id}`, 'PUT', input)
    editing = false; await loadTeam()
  }, async () => { editing = false; await loadTeam() })
  card.append(form)
  form.scrollIntoView({ block: 'start' })
  form.querySelector('input').focus()
}
async function contributor() {
  const token = ownToken()
  const team = await api(`/api/team-joins/${encodeURIComponent(invite)}`, 'GET', undefined, token ? { 'x-team-contribution': token } : {})
  content.replaceChildren(node('h1', {}, team.name), node('p', {}, `Vyberte si do ${date(team.deadline)} českého času. Objednávku odešle a zaplatí organizátor.`), notice(`Počet uložených výběrů: ${team.contributor_count}. Vaše jméno a položky vidí organizátor. Kontaktní údaje zadá organizátor až při objednávání.`))
  if (!team.accepting) {
    if (team.ordered) content.append(notice('Organizátor už společnou objednávku odeslal.'))
    content.append(notice('Výběr je uzavřený. Případnou změnu domluvte přímo s organizátorem.'))
    if (team.own) content.append(node('h2', {}, `Váš uložený výběr: ${team.own.name}`), node('ul', {}, team.own.items.map(item => node('li', {}, `${item.quantity}× ${products.find(p => p.id === item.product_id)?.name || 'Položka již není v nabídce'}${item.variant ? ` (${item.variant})` : ''}`))))
    return
  }
  content.append(notice('Tento výběr ještě není objednávka. Zboží není rezervované. Pro pozdější úpravu použijte stejný prohlížeč.'))
  const form = selectionForm(team.own, async input => {
    const editToken = ownToken() || freshToken()
    contributionSecret = editToken
    try { localStorage.setItem(`brunch-team-contribution:${invite}`, editToken) } catch { storageWarning = true }
    if (team.own) input.contribution_token = editToken
    else input.client_key = editToken
    const saved = await api(`/api/team-joins/${encodeURIComponent(invite)}`, 'PUT', input)
    contributionSecret = saved.contribution_token
    await contributor()
    content.prepend(notice('Váš výběr je uložený. Do uzávěrky jej můžete upravit na této stránce.', 'success'))
    if (storageWarning) content.prepend(notice('Prohlížeč neumožnil uchovat klíč pro další úpravy. Nezavírejte stránku, nebo další změny domluvte s organizátorem.'))
  })
  content.append(form)
}
async function start() {
  try {
    const [catalog, settings] = await Promise.all([api('/api/products'), api('/api/settings')])
    products = catalog
    banquetMinimum = settings.banquet_minimum_quantity ?? 30
    if (invite) await contributor()
    else {
      try { await api('/api/auth/me') } catch (e) { if (e.status === 401) { login(); return } throw e }
      await organizer()
      clearInterval(refreshTimer)
      refreshTimer = setInterval(() => { if (!document.hidden && selectedId && !editing) loadTeam() }, 30000)
    }
  } catch (e) { content.replaceChildren(node('h1', {}, 'Společná objednávka')); error(content, e); content.append(button('Zkusit znovu', start, true)) }
}
start()
