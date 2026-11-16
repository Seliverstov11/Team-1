// Клиент backend API (контракт — backend/openapi.yaml).
// В разработке Vite проксирует /api на backend (vite.config.js, по умолчанию http://localhost:8080),
// поэтому сайт и CRM ходят на свой же адрес — и с других компьютеров в сети тоже.
//
// Авторизации и ролей на backend пока нет, эндпоинты открыты. Разделение экранов
// менеджера и кладовщика в CRM — только интерфейс, не защита.

const BASE = import.meta.env.VITE_API_BASE ?? '/api'

export class ApiError extends Error {
  constructor(message, status) {
    super(message)
    this.status = status
  }
}

async function request(method, path, body) {
  let res
  try {
    res = await fetch(`${BASE}${path}`, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch {
    throw new ApiError('Сервер недоступен. Проверьте, что backend запущен (cd backend && cargo run).', 0)
  }
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`
    try { message = (await res.json()).error ?? message } catch { /* не JSON */ }
    if (res.status === 502 || res.status === 504) message = 'Сервер недоступен. Проверьте, что backend запущен (cd backend && cargo run).'
    throw new ApiError(message, res.status)
  }
  return res.json()
}

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

export const api = {
  // Каталог
  categories: () => request('GET', '/catalog/categories'),
  products: params => all('/catalog/products', params),
  product: id => request('GET', `/catalog/products/${encodeURIComponent(id)}`),

  // Заказы
  orders: params => request('GET', `/orders${query(params)}`),
  allOrders: params => all('/orders', params),
  order: id => request('GET', `/orders/${encodeURIComponent(id)}`),
  createOrder: body => request('POST', '/orders', body),
  // { status?, cancelled_reason?, assigned_to?, payment_status?, paid?, payment_method?, delivery? }
  updateOrder: (id, body) => request('PATCH', `/orders/${encodeURIComponent(id)}`, body),

  // Склад
  stock: () => request('GET', '/warehouse/stock'),
  movements: () => request('GET', '/warehouse/movements'),
  // { product_id, movement_type: in|out|adjustment, quantity, reason?, storage_location?, employee_id? }
  addMovement: body => request('POST', '/warehouse/movements', body),

  employees: () => request('GET', '/employees'),
}
