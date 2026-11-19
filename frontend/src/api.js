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

// Отслеживание активности API и таймер долгого ответа (> 4 сек, для Render Free)
let activeCount = 0
let slowTimer = null
const loadingListeners = new Set()

export const loadingState = {
  busy: false,
  slow: false,
}

function notifyLoading() {
  loadingListeners.forEach(fn => fn({ ...loadingState }))
}

export const onLoadingChange = fn => {
  loadingListeners.add(fn)
  fn({ ...loadingState })
  return () => loadingListeners.delete(fn)
}

function startRequest() {
  activeCount++
  if (activeCount === 1) {
    loadingState.busy = true
    notifyLoading()
    slowTimer = setTimeout(() => {
      if (activeCount > 0) {
        loadingState.slow = true
        notifyLoading()
      }
    }, 4000)
  }
}

function endRequest() {
  activeCount = Math.max(0, activeCount - 1)
  if (activeCount === 0) {
    clearTimeout(slowTimer)
    slowTimer = null
    loadingState.busy = false
    loadingState.slow = false
    notifyLoading()
  }
}

const OFFLINE = 'Сервер недоступен. Проверьте, что backend запущен (cd backend && cargo run).'

async function send(method, path, body, { auth = true, raw = false } = {}) {
  startRequest()
  try {
    const headers = {}
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    const s = auth ? session.get() : null
    if (s) headers.Authorization = `Bearer ${s.token}`
    let res
    try {
      res = await fetch(`${BASE}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
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
  } finally {
    endRequest()
  }
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
  // { name, phone?, city?, client_type?, note? } — менеджер и владелец
  createClient: body => request('POST', '/clients', body),
  updateClient: (cid, body) => request('PATCH', `/clients/${id(cid)}`, body),
  // Вход на сайт для клиента: пароль задаёт сотрудник (клиент потом меняет сам)
  setClientPassword: (cid, password) => request('POST', `/clients/${id(cid)}/password`, { password }),
  // Персональные цены (ФТ-Ц-09) — только владелец
  clientPrices: cid => request('GET', `/clients/${id(cid)}/prices`),
  setClientPrice: (cid, pid, price) => request('PUT', `/clients/${id(cid)}/prices/${id(pid)}`, { price }),
  deleteClientPrice: (cid, pid) => request('DELETE', `/clients/${id(cid)}/prices/${id(pid)}`),

  employees: () => request('GET', '/employees'),
  // { name, login, role: admin|manager|warehouse, password, phone? } — только владелец
  createEmployee: body => request('POST', '/employees', body),
  // { name?, login?, role?, password?, phone?, is_active? }; последнего владельца сервер отключить не даст
  updateEmployee: (eid, body) => request('PATCH', `/employees/${id(eid)}`, body),

  // Категории — только владелец; удалить можно только пустую
  createCategory: body => request('POST', '/catalog/categories', body),
  updateCategory: (cid, body) => request('PATCH', `/catalog/categories/${id(cid)}`, body),
  deleteCategory: cid => request('DELETE', `/catalog/categories/${id(cid)}`),

  // Профиль покупателя
  me: () => request('GET', '/me'),
  updateMe: body => request('PATCH', '/me', body),
  myOrders: () => request('GET', '/me/orders'),
  // Избранное и корзина вошедшего покупателя — общие для всех его устройств
  favorites: () => request('GET', '/me/favorites'),
  setFavorites: keys => request('PUT', '/me/favorites', keys),
  myCart: () => request('GET', '/me/cart'),
  setCart: items => request('PUT', '/me/cart', items),
  // «Сообщить о поступлении»: { product_id, name?, phone?, qty? }; список и отметки — CRM
  restockRequest: body => request('POST', '/restock', body),
  restockList: status => request('GET', `/restock${query({ status })}`),
  restockUpdate: (rid, status) => request('PATCH', `/restock/${id(rid)}`, { status }),

  // Сводка
  dashboard: period => request('GET', `/dashboard${query({ period })}`),
  // Раздел аналитики владельца (ФТ-В-04): каналы, города, отмены, топы, устройства, последний вход
  analytics: period => request('GET', `/analytics${query({ period })}`),
  visit: body => request('POST', '/analytics/visit', body).catch(() => null),
}
