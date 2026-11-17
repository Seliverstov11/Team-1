// Деньги и количества — целыми числами.
//
// API отдаёт их строками («9800.00», «40.000»), чтобы JSON-парсер не превратил их во float.
// Поэтому строку разбираем руками в целые тиыны (1 ₸ = 100 тиын) и тысячные доли количества,
// считаем в целых и обратно в строку переводим тоже без float — так суммы в корзине
// совпадают с суммами backend до тиына (backend считает так же: price_tiyn * qty_milli / 1000).

const SCALE = { 2: 100, 3: 1000 }

function parseScaled(value, decimals) {
  const s = String(value ?? '').trim()
  const m = /^(-?)(\d*)(?:\.(\d*))?$/.exec(s)
  if (!m || (!m[2] && !m[3])) return 0
  const frac = (m[3] ?? '').padEnd(decimals, '0').slice(0, decimals)
  const v = Number(m[2] || 0) * SCALE[decimals] + Number(frac || 0)
  return m[1] ? -v : v
}

export const tiyn = value => parseScaled(value, 2)
export const milli = value => parseScaled(value, 3)

const pad = (n, len) => String(n).padStart(len, '0')
export const tiynToString = t => `${t < 0 ? '-' : ''}${Math.trunc(Math.abs(t) / 100)}.${pad(Math.abs(t) % 100, 2)}`

// Количество для поля ввода и отправки: целое — без дробной части
export const qtyToString = q => String(Math.trunc(q))

// Строка заказа: та же формула, что на backend (целочисленное деление)
export const lineTotal = (priceTiyn, qtyMilli) => Math.trunc((priceTiyn * qtyMilli) / 1000)

// 980000 → «9 800 ₸», 95050 → «950,50 ₸» — разряды и копейки без перевода во float
const groupDigits = n => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ' ')
export function formatTiyn(t, { currency = true } = {}) {
  const abs = Math.abs(t)
  const whole = groupDigits(Math.trunc(abs / 100))
  const frac = abs % 100
  const s = `${t < 0 ? '−' : ''}${whole}${frac ? `,${pad(frac, 2)}` : ''}`
  return currency ? `${s} ₸` : s
}

// «40.000» → «40», «2.500» → «2,5»
export function formatQty(value) {
  const m = typeof value === 'number' ? value * 1000 : milli(value)
  const whole = Math.trunc(Math.abs(m) / 1000)
  const frac = String(Math.abs(m) % 1000).padStart(3, '0').replace(/0+$/, '')
  return `${m < 0 ? '−' : ''}${groupDigits(whole)}${frac ? `,${frac}` : ''}`
}
