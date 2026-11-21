;(function () {
  'use strict'
  class TermsError extends Error {}
  const title = document.getElementById('terms-title')
  const meta = document.getElementById('terms-meta')
  const content = document.getElementById('terms-content')
  const status = document.getElementById('terms-status')
  const error = document.getElementById('terms-error')
  const errorMessage = document.getElementById('terms-error-message')
  const retry = document.getElementById('terms-retry')
  const print = document.getElementById('terms-print')
  const version = new URLSearchParams(location.search).get('version')

  function render(text) {
    const fragment = document.createDocumentFragment()
    for (const block of text.replace(/\r\n?/g, '\n').split(/\n\s*\n/)) {
      if (!block.trim()) continue
      const heading = /^(#{1,3})\s+([^\n]+)$/.exec(block.trim())
      const node = document.createElement(heading ? (heading[1].length >= 3 ? 'h3' : 'h2') : 'p')
      node.textContent = heading ? heading[2] : block.trim()
      fragment.append(node)
    }
    content.replaceChildren(fragment)
  }

  async function load() {
    error.hidden = true
    status.hidden = false
    content.hidden = true
    meta.hidden = true
    print.hidden = true
    retry.disabled = true
    try {
      if (version !== null && !/^[1-9][0-9]*$/.test(version)) throw new TermsError('Neplatná verze obchodních podmínek.')
      const response = await fetch(`/api/terms${version === null ? '' : `?version=${encodeURIComponent(version)}`}`, { headers: { Accept: 'application/json' }, cache: 'no-store' })
      if (!response.ok) throw new TermsError(response.status === 404 ? 'Požadovaná verze obchodních podmínek není dostupná.' : 'Obchodní podmínky se nepodařilo načíst. Zkuste to prosím znovu.')
      const terms = await response.json()
      if (!Number.isSafeInteger(terms.version) || terms.version <= 0 || typeof terms.title !== 'string' || !terms.title.trim() || typeof terms.content !== 'string' || !terms.content.trim() || (version !== null && String(terms.version) !== version)) throw new TermsError('Obchodní podmínky se nepodařilo načíst v úplném znění. Zkuste to prosím znovu.')
      title.textContent = terms.title
      document.title = `${terms.title} | Brunch Éra`
      const updated = new Date(terms.updated_at)
      meta.textContent = `Verze ${terms.version}${Number.isNaN(updated.valueOf()) ? '' : ` · Aktualizováno ${new Intl.DateTimeFormat('cs-CZ', { dateStyle: 'long', timeZone: 'Europe/Prague' }).format(updated)}`}`
      render(terms.content)
      meta.hidden = false
      content.hidden = false
      print.hidden = false
    } catch (failure) {
      errorMessage.textContent = failure instanceof TermsError ? failure.message : 'Obchodní podmínky se nepodařilo načíst. Zkuste to prosím znovu.'
      error.hidden = false
    } finally {
      status.hidden = true
      retry.disabled = false
    }
  }
  retry.addEventListener('click', load)
  print.addEventListener('click', () => window.print())
  load()
})()
