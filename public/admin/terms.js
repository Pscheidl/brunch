'use strict'

async function termsPage() {
  const terms = await api('/api/admin/terms')
  const title = field('Název podmínek', 'title', terms.title, { required: true, maxlength: 160 })
  const text = field('Znění obchodních podmínek', 'content', terms.content, {
    kind: 'textarea', rows: 20, required: true, maxlength: 100000,
    hint: 'Pište běžný text. Prázdný řádek oddělí odstavce. HTML se zobrazí jako text.'
  })
  const form = el('form', { class: 'panel terms-editor', onsubmit: event => saveForm(event, async data => {
    const payload = { version: terms.version, title: String(data.get('title')).trim(), content: String(data.get('content')).trim() }
    if (!payload.title || !payload.content) throw new Error('Vyplňte název i znění obchodních podmínek.')
    const saved = await api('/api/admin/terms', { method: 'PUT', body: JSON.stringify(payload) })
    notify(`Obchodní podmínky byly uloženy jako verze ${saved.version}.`)
    await renderPage()
  }) },
  el('p', { class: 'section-note' }, terms.version > 0 ? `Aktuální verze: ${terms.version}${terms.updated_at ? `. Uloženo ${dateTime(terms.updated_at)}` : ''}.` : 'Obchodní podmínky zatím nejsou zveřejněné.'),
  el('p', { class: 'section-note' }, 'Uložením zveřejníte novou verzi pro nové objednávky. U starších objednávek zůstane zachováno znění, které zákazník přijal.'),
  title, text,
  el('div', { class: 'actions' }, button('Náhled', () => {
    const body = openDialog('Náhled obchodních podmínek')
    body.append(el('article', { class: 'terms-preview' },
      el('h3', {}, title.querySelector('input').value.trim() || 'Obchodní podmínky'),
      el('div', { class: 'terms-text' }, text.querySelector('textarea').value.trim() || 'Znění obchodních podmínek zatím není vyplněné.')),
      el('div', { class: 'actions' }, button('Zpět k úpravám', () => dialog.close())))
  }), el('button', { type: 'submit', class: 'button primary' }, 'Uložit a zveřejnit novou verzi')))
  return [header('Obchodní podmínky', 'Znění dostupné zákazníkům před odesláním objednávky.', button('Obnovit', renderPage)), form]
}
