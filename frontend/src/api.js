// Слой данных. Пока backend не подключён, данные читаются из заглушек в /public/data.
// Поля совпадают с контрактом API (backend/openapi.yaml), поэтому после подключения
// достаточно поменять API_BASE и пути ниже — разметку и остальной код не трогать.

const API_BASE = '/data' // TODO(API): адрес backend, например '/api/v1'

async function getJson(path) {
  const res = await fetch(`${API_BASE}${path}`)
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json()
}

export const getProducts = () => getJson('/products.json')     // TODO(API): '/products'
export const getCategories = () => getJson('/categories.json') // TODO(API): '/categories'

// Отправка заказа. Пока backend нет — ничего не отправляем, возвращаем номер-заглушку.
// TODO(API): POST `${API_BASE}/orders` с телом заказа, вернуть номер из ответа.
export async function createOrder(/* order */) {
  return { number: `DEMO-${String(Date.now()).slice(-6)}` }
}
