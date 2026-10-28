// Shared password login for local browser fixtures. Never use production credentials.
const assert = require('node:assert/strict')

function adminTestPassword() {
  const password = process.env.TEST_ADMIN_PASSWORD
  assert.ok(password, 'Set TEST_ADMIN_PASSWORD to the password of an administrator created in the isolated local test database')
  return password
}

async function loginAdministrator(page, base, email, password) {
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(new URL(base).hostname), 'Administrator browser tests must use a local fixture only')
  assert.ok(password, 'The local test administrator password is required')
  await page.goto(`${base}/admin/`)
  await page.getByLabel('E-mail správce', { exact: true }).fill(email)
  await page.getByLabel('Heslo', { exact: true }).fill(password)
  const pending = page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === '/api/admin/auth/login')
  await page.getByRole('button', { name: 'Přihlásit se', exact: true }).click()
  const response = await pending
  assert.equal(response.status(), 200, `Administrator password login failed (${response.status()})`)
  const { user } = await response.json()
  assert.equal(user.is_admin, true, 'Password login must establish an administrator session')
  assert.equal(user.email, email.trim().toLowerCase())
  await page.getByRole('heading', { name: 'Objednávky', exact: true }).waitFor()
}

module.exports = { adminTestPassword, loginAdministrator }
