'use strict'

async function invoiceSettingsPage() {
  const supplier = await api('/api/admin/invoice-supplier')
  const vat = field('Dodavatel je plátce DPH', 'vat_payer', supplier.vat_payer, { type: 'checkbox' })
  const taxId = field('DIČ dodavatele', 'tax_id', supplier.tax_id, { maxlength: 30, required: supplier.vat_payer })
  vat.querySelector('input').addEventListener('change', event => { taxId.querySelector('input').required = event.target.checked })
  const form = el('form', { class: 'panel', onsubmit: event => saveForm(event, async data => {
    const payload = Object.fromEntries(data)
    payload.vat_payer = data.has('vat_payer')
    payload.version = supplier.version
    await api('/api/admin/invoice-supplier', { method: 'PUT', body: JSON.stringify(payload) })
    notify('Fakturační údaje byly uloženy.')
    await renderPage()
  }) }, el('div', { class: 'form-grid' },
    field('Název nebo jméno dodavatele', 'name', supplier.name, { maxlength: 200, required: true }),
    field('IČO dodavatele', 'company_id', supplier.company_id, { maxlength: 24, required: true }),
    field('Sídlo dodavatele', 'address', supplier.address, { kind: 'textarea', maxlength: 500, required: true, full: true }),
    field('Zápis v rejstříku', 'registration', supplier.registration, { maxlength: 300, full: true }),
    vat, taxId), el('button', { type: 'submit', class: 'button primary' }, 'Uložit fakturační údaje'))
  return [header('Fakturační údaje', 'Tyto údaje se použijí při vystavení nové faktury.'), form]
}

async function invoiceDialog(order) {
  const path = `/api/admin/orders/${order.id}/invoice`
  const body = openDialog('Faktura')
  body.append(el('p', { class: 'loading' }, 'Načítám fakturu…'))
  try {
    const existing = await api(path)
    if (!body.isConnected) return
    if (existing) { showInvoice(existing, order.id); return }
    const [supplier, billing] = await Promise.all([api('/api/admin/invoice-supplier'), api(`/api/admin/orders/${order.id}/billing`)])
    if (!body.isConnected) return
    if (!supplier.name) {
      body.replaceChildren(el('p', {}, 'Před vystavením první faktury vyplňte údaje dodavatele.'),
        el('a', { class: 'button', href: '#invoice-settings' }, 'Nastavit fakturační údaje'))
      return
    }
    const taxItems = order.items.map(item => [String(item.product_id), item.name])
    if (order.delivery_fee_cents > 0) taxItems.push(['delivery', 'Doprava'])
    const today = pragueCalendarDate()
    const form = el('form', { onsubmit: event => saveForm(event, async data => {
      const rates = {}
      if (supplier.vat_payer) for (const [key] of taxItems) rates[key] = Math.round(Number(data.get(`rate_${key}`)) * 100)
      const document = await api(path, { method: 'POST', body: JSON.stringify({
        supplier_version: supplier.version, customer_name: data.get('customer_name'),
        customer_address: data.get('customer_address'), customer_company_id: data.get('customer_company_id'),
        customer_tax_id: data.get('customer_tax_id'), supply_date: data.get('supply_date'),
        due_date: data.get('due_date'), vat_rates: rates
      }) })
      if (body.isConnected) showInvoice(document, order.id)
    }, 'Vystavuji…') },
    el('p', { class: 'section-note' }, `Dodavatel: ${supplier.name} · IČO ${supplier.company_id}. Objednávka ${order.number} · ${money(order.total_cents)}.`),
    el('div', { class: 'form-grid' },
      field('Odběratel', 'customer_name', billing?.name || order.organization || order.name, { maxlength: 250, required: true, full: true }),
      field('Adresa odběratele', 'customer_address', billing?.address || (order.fulfillment_method === 'pickup' ? '' : `${order.street}\n${order.postal_code} ${order.city}`), { kind: 'textarea', maxlength: 500, required: true, full: true, hint: order.fulfillment_method === 'pickup' && !billing?.address ? 'Doplňte fakturační adresu zákazníka. Adresa odběrného místa není adresou odběratele.' : null }),
      field('IČO odběratele', 'customer_company_id', billing?.company_id || '', { maxlength: 24 }),
      field('DIČ odběratele', 'customer_tax_id', billing?.tax_id || '', { maxlength: 30 }),
      field('Datum plnění', 'supply_date', order.delivery_date, { type: 'date', required: true }),
      field('Datum splatnosti', 'due_date', order.delivery_date, { type: 'date', required: true })),
    supplier.vat_payer ? el('section', {}, el('h3', {}, 'Sazby DPH'), el('div', { class: 'form-grid' }, taxItems.map(([key, name]) =>
      field(`${name} · DPH (%)`, `rate_${key}`, '', { type: 'number', min: 0, max: 100, step: 0.01, required: true })))) : null,
    el('p', { class: 'hint' }, `Datum vystavení: ${invoiceDate(today)}. Vystavená faktura dostane vlastní číslo a uloží se k objednávce.`),
    el('div', { class: 'actions' }, button('Zpět k objednávce', () => orderDialog(order.id)), el('button', { type: 'submit', class: 'button primary' }, 'Vystavit fakturu')))
    body.replaceChildren(form)
  } catch (error) { if (body.isConnected) body.replaceChildren(errorBox(error)) }
}

function invoiceDate(value) {
  const [year, month, day] = value.split('-')
  return `${Number(day)}. ${Number(month)}. ${year}`
}

function showInvoice(invoice, orderId) {
  const body = openDialog(`Faktura ${invoice.number}`)
  const issuer = invoice.supplier
  const vat = issuer.vat_payer
  const party = (label, person) => el('section', {}, el('h3', {}, label), el('strong', {}, person.name),
    el('p', {}, person.address), person.company_id ? el('p', {}, `IČO ${person.company_id}`) : null,
    person.tax_id ? el('p', {}, `DIČ ${person.tax_id}`) : null, person.registration ? el('p', {}, person.registration) : null)
  const taxes = new Map()
  const rows = invoice.lines.map(line => {
    if (vat) {
      const group = taxes.get(line.vat_rate) || { net: 0, tax: 0 }
      group.net += line.net_cents
      group.tax += line.vat_cents
      taxes.set(line.vat_rate, group)
    }
    return el('tr', {}, el('td', {}, line.name), el('td', {}, `${line.quantity} ks`),
      el('td', { class: 'number' }, money(line.unit_price_cents)),
      vat ? el('td', {}, `${line.vat_rate / 100} %`) : null,
      el('td', { class: 'number' }, money(line.total_cents)))
  })
  const taxRows = [...taxes.entries()].sort(([a], [b]) => a - b).map(([rate, group]) =>
    el('tr', {}, el('td', {}, `${rate / 100} %`), el('td', {}, money(group.net)), el('td', {}, money(group.tax)), el('td', {}, money(group.net + group.tax))))
  const pickup = invoice.fulfillment_method === 'pickup'
  const method = { card: 'Online kartou', cash_on_delivery: `Hotově při ${pickup ? 'vyzvednutí' : 'doručení'}`, card_on_delivery: `Kartou při ${pickup ? 'vyzvednutí' : 'doručení'}` }
  body.append(el('article', { class: 'invoice' },
    el('p', { class: 'section-note' }, `Objednávka ${invoice.order_number}`),
    el('div', { class: 'summary-grid' }, party('Dodavatel', issuer), party('Odběratel', invoice.customer)),
    el('div', { class: 'invoice-dates' },
      el('p', {}, `Datum vystavení: ${invoiceDate(invoice.issue_date)}`),
      el('p', {}, `Datum plnění: ${invoiceDate(invoice.supply_date)}`),
      el('p', {}, `Datum splatnosti: ${invoiceDate(invoice.due_date)}`)),
    table(['Položka', 'Množství', vat ? 'Cena za kus s DPH' : 'Cena za kus', ...(vat ? ['DPH'] : []), 'Celkem'], rows, true),
    vat ? el('section', {}, el('h3', {}, 'Rekapitulace DPH'), table(['Sazba', 'Základ', 'DPH', 'Celkem'], taxRows, true)) : el('p', {}, 'Dodavatel není plátcem DPH.'),
    el('p', { class: 'invoice-total' }, `Celkem ${money(invoice.total_cents)}`),
    el('p', {}, `Způsob úhrady: ${method[invoice.payment_method]}.`),
    pickup ? el('section', { class: 'invoice-pickup' }, el('h3', {}, 'Osobní odběr'),
      invoice.pickup_location ? el('p', {}, `${invoice.pickup_location.name}\n${invoice.pickup_location.street}\n${invoice.pickup_location.postal_code} ${invoice.pickup_location.city}`) : null,
      invoice.pickup_location?.parking_free ? el('p', {}, 'Parkování zdarma') : null,
      invoice.pickup_location?.instructions ? el('p', {}, invoice.pickup_location.instructions) : null) : null,
    el('p', {}, `Stav při vystavení: ${payments[invoice.payment_status]}.`)),
    el('div', { class: 'actions no-print' }, button('Zpět k objednávce', () => orderDialog(orderId)),
      button('Vytisknout / uložit PDF', () => window.print(), 'primary')))
}
