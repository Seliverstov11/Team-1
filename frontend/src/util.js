// Общие утилиты сайта и CRM: форматирование и работа с localStorage

const money = new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 })
const dateFmt = new Intl.DateTimeFormat('ru-RU', { day: '2-digit', month: '2-digit' })
const dateLongFmt = new Intl.DateTimeFormat('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' })
const timeFmt = new Intl.DateTimeFormat('ru-RU', { hour: '2-digit', minute: '2-digit' })
const weekdayFmt = new Intl.DateTimeFormat('ru-RU', { weekday: 'short' })

export const fmt = {
  price: value => `${money.format(Number(value))} ₸`,
  num: value => money.format(Number(value)),
  plural(n, one, few, many) {
    const m10 = n % 10, m100 = n % 100
    if (m10 === 1 && m100 !== 11) return one
    if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few
    return many
  },
  count(n, one, few, many) { return `${money.format(n)} ${fmt.plural(n, one, few, many)}` },
  date: iso => (iso ? dateFmt.format(new Date(iso)) : ''),
  dateLong: iso => (iso ? dateLongFmt.format(new Date(iso)) : ''),
  time: iso => (iso ? timeFmt.format(new Date(iso)) : ''),
  dateTime: iso => (iso ? `${dateFmt.format(new Date(iso))}, ${timeFmt.format(new Date(iso))}` : ''),
  weekday: iso => weekdayFmt.format(new Date(iso)).replace('.', ''),
  // «+12%», «−5%»; null — сравнивать не с чем
  delta(cur, prev) {
    if (!prev) return null
    return Math.round(((cur - prev) / prev) * 100)
  },
  initials: name => name.replace(/[«»"]/g, '').split(/\s+/).filter(Boolean).slice(0, 2).map(w => w[0]).join('').toUpperCase(),
}

// Мягкая маска казахстанского номера: +7 (7XX) XXX-XX-XX.
// «+7» — фиксированный префикс: всё, что набрано после него, — местный номер из 10 цифр.
// Вставка понимает форматы 8 701…, +7 701…, 7 701… и просто 701….
export function formatPhone(value) {
  const raw = String(value ?? '')
  let d = raw.replace(/\D/g, '')
  if (raw === '+') return '+'
  if (raw === '+7' || raw === '8') return '+7 (' // начали набирать «+7 701…» или «8 701…»
  if (raw.trimStart().startsWith('+7')) d = d.slice(1)
  else if (d.length === 11 && (d[0] === '7' || d[0] === '8')) d = d.slice(1)
  d = d.slice(0, 10)
  if (!d) return ''
  let out = `+7 (${d.slice(0, 3)}`
  if (d.length > 3) out += `) ${d.slice(3, 6)}`
  if (d.length > 6) out += `-${d.slice(6, 8)}`
  if (d.length > 8) out += `-${d.slice(8, 10)}`
  return out
}
export const phoneDigits = value => String(value ?? '').replace(/\D/g, '').replace(/^8/, '7')
export const isPhone = value => /^7\d{10}$/.test(phoneDigits(value))

export function persisted(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback } catch { return fallback }
}
export const save = (key, value) => localStorage.setItem(key, JSON.stringify(value))

export const startOfDay = (d = new Date()) => { const x = new Date(d); x.setHours(0, 0, 0, 0); return x }
export const addDays = (d, n) => { const x = new Date(d); x.setDate(x.getDate() + n); return x }
export const sameDay = (a, b) => startOfDay(a).getTime() === startOfDay(b).getTime()
