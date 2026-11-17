// Клиент backend API (контракт — backend/openapi.yaml в ветке feat/backend-api).
// В разработке Vite проксирует /api на backend (vite.config.js, по умолчанию http://localhost:8080),
// поэтому сайт и CRM ходят на свой же адрес — и с других компьютеров в сети тоже.
//
// Вход: POST /api/auth/login (сотрудник) или /api/auth/client/login (покупатель) отдаёт токен,
// дальше он идёт в заголовке Authorization: Bearer <токен>. Права проверяет сервер.
// У CRM и сайта токены разные: сотрудник, открывший сайт, видит его как гость.

const BASE = import.meta.env.VITE_API_BASE ?? '/api'

const IS_CRM = location.pathname.startsWith('/crm')
export const TOKEN_KEY = IS_CRM ? 'bakeloom.crm.session' : 'bakeloom.client.session'

export class ApiError extends Error {
  constructor(message, status) {
    super(message)
    this.status = status
  }
}

// ---------------------------------------------------------------- сессия
// { token, expires_at, kind, user: { id, name, role } }
export const session = {
  get() {
    try {
      const s = JSON.parse(localStorage.getItem(TOKEN_KEY))
      if (!s?.token) return null
      if (s.expires_at && new Date(s.expires_at) < new Date()) { this.clear(); return null }
      return s
    } catch { return null }
  },
  set(s) { try { localStorage.setItem(TOKEN_KEY, JSON.stringify(s)) } catch { /* приватный режим */ } },
  clear() { try { localStorage.removeItem(TOKEN_KEY) } catch { /* приватный режим */ } },
}

// Сессия истекла или токен отозван (сменили пароль, отключили сотрудника)
const unauthorized = new Set()
export const onUnauthorized = fn => unauthorized.add(fn)

const OFFLINE = 'Сервер недоступен. Проверьте, что backend запущен (cd backend && cargo run).'

async function send(method, path, body, { auth = true, raw = false } = {}) {
  const headers = {}
  const form = body instanceof FormData   // файл: Content-Type с границей ставит браузер
  if (body !== undefined && !form) headers['Content-Type'] = 'application/json'
  const s = auth ? session.get() : null
  if (s) headers.Authorization = `Bearer ${s.token}`
  let res
  try {
    res = await fetch(`${BASE}${path}`, { method, headers, body: body === undefined || form ? body : JSON.stringify(body) })
  } catch {
    throw new ApiError(OFFLINE, 0)
  }
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`
    try { message = (await res.json()).error ?? message } catch { /* не JSON */ }
    if (res.status === 502 || res.status === 504) message = OFFLINE
    if (res.status === 401 && s) {
      session.clear()
      unauthorized.forEach(fn => fn(message))
    }
    throw new ApiError(message, res.status)
  }
  if (raw) return res.text()
  const text = await res.text()
  return text ? JSON.parse(text) : null
}

const request = (method, path, body, opts) => send(method, path, body, opts)

const query = params => {
  const q = new URLSearchParams()
  Object.entries(params ?? {}).forEach(([k, v]) => { if (v !== undefined && v !== null && v !== '') q.set(k, v) })
  const s = q.toString()
  return s ? `?${s}` : ''
}

// Выдача ограничена 200 записями за запрос — забираем страницами до конца
async function all(path, params = {}) {
  const limit = 200
  const out = []
  for (let offset = 0; ; offset += limit) {
    const page = await request('GET', `${path}${query({ ...params, limit, offset })}`)
    out.push(...page)
    if (page.length < limit) return out
  }
}

const id = v => encodeURIComponent(v)

export const api = {
  // Вход
  login: async (login, password) => {
    const s = await request('POST', '/auth/login', { login, password }, { auth: false })
    session.set(s)
    return s
  },
  clientLogin: async (phone, password) => {
    const s = await request('POST', '/auth/client/login', { phone, password }, { auth: false })
    session.set(s)
    return s
  },
  clientRegister: async body => {
    const s = await request('POST', '/auth/client/register', body, { auth: false })
    session.set(s)
    return s
  },
  logout: async () => {
    try { await request('POST', '/auth/logout') } catch { /* токен мог уже истечь */ }
    session.clear()
  },
  whoami: () => request('GET', '/auth/me'),

  // Каталог (персональная цена приходит вошедшему покупателю в personal_price)
  categories: () => request('GET', '/catalog/categories'),
  products: params => all('/catalog/products', params),
  product: pid => request('GET', `/catalog/products/${id(pid)}`),
  attributeNames: () => request('GET', '/catalog/attributes'),
  // Управление каталогом — только владелец (admin)
  createProduct: body => request('POST', '/catalog/products', body),
  updateProduct: (pid, body) => request('PATCH', `/catalog/products/${id(pid)}`, body),
  setAttributes: (pid, list) => request('PUT', `/catalog/products/${id(pid)}/attributes`, list),
  archiveProduct: pid => request('POST', `/catalog/products/${id(pid)}/archive`),
  restoreProduct: pid => request('POST', `/catalog/products/${id(pid)}/restore`),
  // Фото товара (#43): 6 вариантов с Pexels по запросу; выбранный вариант или свой файл —
  // multipart: pexels_id | file. Сервер обрабатывает в общем стиле и отдаёт товар с новым image_url
  photoSuggestions: q => request('GET', `/catalog/photo-suggestions${query({ q })}`),
  uploadPhoto: (pid, form) => request('POST', `/catalog/products/${id(pid)}/photo`, form),

  // Заказы
  orders: params => request('GET', `/orders${query(params)}`),
  allOrders: params => all('/orders', params),
  order: oid => request('GET', `/orders/${id(oid)}`),
  createOrder: body => request('POST', '/orders', body),
  track: (order_no, phone) => request('GET', `/orders/track${query({ order_no, phone })}`, undefined, { auth: false }),
  // { status?, cancelled_reason?, assigned_to?, is_urgent?, is_problem?, problem_note?, delivery?, ... } — переходы проверяет сервер (409)
  updateOrder: (oid, body) => request('PATCH', `/orders/${id(oid)}`, body),
  // { payment_status: unpaid|partial|paid|refunded, payment_method?, paid? }
  payment: (oid, body) => request('POST', `/orders/${id(oid)}/payment`, body),
  addItem: (oid, body) => request('POST', `/orders/${id(oid)}/items`, body),
  updateItem: (oid, itemId, body) => request('PATCH', `/orders/${id(oid)}/items/${id(itemId)}`, body),
  removeItem: (oid, itemId) => request('DELETE', `/orders/${id(oid)}/items/${id(itemId)}`),
  // { picked, short_qty? } — нехватка делает заказ проблемным
  pick: (oid, itemId, body) => request('PATCH', `/orders/${id(oid)}/items/${id(itemId)}/pick`, body),
  comment: (oid, text) => request('POST', `/orders/${id(oid)}/comments`, { text }),
  invoice: oid => request('GET', `/orders/${id(oid)}/invoice`, undefined, { raw: true }),

  // Склад
  stock: params => request('GET', `/warehouse/stock${query(params)}`),
  updateStock: (pid, body) => request('PATCH', `/warehouse/stock/${id(pid)}`, body),
  movements: params => request('GET', `/warehouse/movements${query(params)}`),
  // { product_id, movement_type: in|out|adjustment, quantity, reason?, storage_location? }
  addMovement: body => request('POST', '/warehouse/movements', body),
  assembly: () => request('GET', '/warehouse/assembly'),

  // Клиенты и сотрудники
  clients: params => all('/clients', params),
  client: cid => request('GET', `/clients/${id(cid)}`),
  employees: () => request('GET', '/employees'),

  // Профиль покупателя
  me: () => request('GET', '/me'),
  updateMe: body => request('PATCH', '/me', body),
  myOrders: () => request('GET', '/me/orders'),

  // Сводка
  dashboard: period => request('GET', `/dashboard${query({ period })}`),
  visit: body => request('POST', '/analytics/visit', body).catch(() => null),
}
