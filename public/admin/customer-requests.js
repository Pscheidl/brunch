'use strict'

const customerRequestStatuses = { pending: 'Čeká na vyřízení', resolved: 'Vyřízeno', rejected: 'Zamítnuto' }

async function customerRequestsPage() {
  const requests = await api('/api/admin/customer-requests')
  const results = el('div', { class: 'request-list' })
  const count = el('p', { class: 'hint product-result-count', role: 'status', 'aria-live': 'polite' })
  const filter = selectField('Stav žádosti', 'request-status', [['pending', 'Čeká na vyřízení'], ['resolved', 'Vyřízeno'], ['rejected', 'Zamítnuto'], ['', 'Všechny žádosti']], state.customerRequestStatus)
  function renderRequests() {
    const visible = requests.filter(request => !state.customerRequestStatus || request.status === state.customerRequestStatus)
    count.textContent = `Zobrazeno ${visible.length} z ${requests.length} načtených žádostí`
    results.replaceChildren(...(visible.length ? visible.map(request => el('article', { class: 'panel customer-request', 'aria-label': `Žádost k objednávce ${request.order_number}` },
      el('div', { class: 'request-heading' }, el('h2', {}, `${request.kind === 'cancel' ? 'Zrušení' : 'Změna'} objednávky ${request.order_number}`), badge(customerRequestStatuses[request.status] || request.status, request.status === 'pending' ? 'pending' : request.status === 'rejected' ? 'cancelled' : 'sent')),
      el('p', { class: 'section-note' }, `${request.email} · ${dateTime(request.created_at)}`),
      el('p', { class: 'request-message' }, request.message),
      request.response ? el('p', { class: 'request-response' }, el('strong', {}, 'Odpověď zákazníkovi: '), request.response) : null,
      request.resolved_at ? el('p', { class: 'section-note' }, `Vyřízeno ${dateTime(request.resolved_at)}`) : null,
      el('div', { class: 'actions' }, button('Otevřít objednávku', () => orderDialog(request.order_id)), request.status === 'pending' ? button('Vyřídit žádost', () => customerRequestDialog(request), 'primary') : null)
    )) : [empty('Žádné žádosti v tomto stavu', 'Nové žádosti zákazníků se zobrazí po obnovení seznamu.')]))
  }
  filter.querySelector('select').addEventListener('change', event => { state.customerRequestStatus = event.target.value; renderRequests() })
  renderRequests()
  return [header('Žádosti zákazníků', 'Požadavky na změnu nebo zrušení objednávky. Zákazník dostane odpověď e-mailem.'), el('p', { class: 'note' }, 'Nejdříve vyřešte změnu nebo storno v detailu objednávky a případné vrácení platby. Označení žádosti za vyřízenou samo objednávku nemění ani nevrací peníze.'), el('div', { class: 'filters' }, filter, button('Obnovit', renderPage)), count, results]
}

function customerRequestDialog(request) {
  const body = openDialog(`Žádost k objednávce ${request.order_number}`)
  const form = el('form', { onsubmit: event => saveForm(event, async values => {
    await api(`/api/admin/customer-requests/${request.id}`, { method: 'PATCH', body: JSON.stringify({
      status: values.get('status'), response: String(values.get('response') || '').trim(), version: request.version
    }) })
    dialog.close()
    notify('Žádost byla vyřízena. Odpověď je zařazena k odeslání zákazníkovi.')
    await renderPage()
  }, 'Ukládám odpověď…') },
  el('p', { class: 'request-message' }, request.message),
  el('p', { class: 'note' }, 'Tento krok uloží odpověď a zařadí e-mail zákazníkovi. Objednávku ani její platbu nemění. Před označením za vyřízenou dokončete domluvenou změnu v detailu objednávky.'),
  selectField('Výsledek žádosti', 'status', [['resolved', 'Vyřízeno'], ['rejected', 'Zamítnuto']], 'resolved'),
  field('Odpověď zákazníkovi', 'response', '', { kind: 'textarea', required: true, maxlength: 2000, hint: 'Popište provedenou změnu, nebo vysvětlete důvod zamítnutí. Tuto odpověď zákazník uvidí v účtu i e-mailu.' }),
  el('div', { class: 'actions' }, button('Zrušit', () => dialog.close()), el('button', { type: 'submit', class: 'button primary' }, 'Uložit a odeslat odpověď')))
  body.append(form)
}
