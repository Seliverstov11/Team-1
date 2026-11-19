// CRM Bakeloom: владелец, менеджеры заказов и кладовщик.
// Данные — backend API (src/api.js): те же заказы и остатки видит сайт.
//
// Вход — POST /api/auth/login, дальше каждый запрос с токеном. Права проверяет сервер:
// меню по роли только убирает лишнее, а запрос без прав всё равно получит 403.
// Кладовщику сервер не отдаёт поля оплаты — интерфейс их просто не показывает.

import Alpine from 'alpinejs'
import { api, onUnauthorized, session, onLoadingChange } from './api.js'
import { buildSections, groupOfCategory, lineSum, retail } from './catalog.js'
import { formatQty, formatTiyn, milli, qtyToString, tiyn, tiynToString } from './money.js'
import {
  ACTIVE, CHANNELS, DELIVERY, EDITABLE, FLOW, NEXT, PAYMENT, PAYMENT_METHODS, STATUSES, STATUS_HINT, STATUS_ORDER,
  paymentKey,
} from './orders.js'
import { passwordProblems } from './password.js'
import { fmt, formatPhone, isPhone, persisted, phoneDigits, save, initPageProgress } from './util.js'

const params = new URLSearchParams(location.search)
const LOCK_KEY = 'bakeloom.crm.lock'

// ---------------------------------------------------------------- приложение на рабочем столе (PWA)
// Service worker держит оболочку CRM (public/crm/sw.js). Данные всегда идут с сервера.
// Работает на https и на localhost; по адресу из локальной сети (http://192.168…) браузер установку не предложит.
if ('serviceWorker' in navigator && window.isSecureContext) {
  navigator.serviceWorker.register('/crm/sw.js', { scope: '/crm/' }).then(reg => {
    // CRM может быть открыта весь день — раз в час проверяем, не выложили ли новую версию
    setInterval(() => reg.update().catch(() => {}), 60 * 60 * 1000)
  }).catch(e => console.warn('Service worker не зарегистрирован:', e))
}
// Браузер готов предложить установку — событие может прийти раньше Alpine, поэтому ловим его сразу
let installEvent = null
window.addEventListener('beforeinstallprompt', e => {
  e.preventDefault()
  installEvent = e
  if (window.Alpine) Alpine.store('pwa').available = true
})
window.addEventListener('appinstalled', () => {
  installEvent = null
  if (window.Alpine) Object.assign(Alpine.store('pwa'), { available: false, installed: true })
})
// Телефон: CRM-таблицы показываются карточками — каждой ячейке нужна подпись из заголовка столбца.
// Строки рисует Alpine (x-for), поэтому подписываем при каждом изменении DOM
function labelTables() {
  document.querySelectorAll('table.crm-table').forEach(t => {
    const heads = [...(t.tHead?.rows[0]?.cells ?? [])].map(th => th.textContent.trim())
    t.querySelectorAll('tbody tr').forEach(tr => {
      let col = 0
      for (const td of tr.cells) {
        const label = td.colSpan > 1 ? '' : heads[col] ?? ''
        if (td.dataset.label !== label) td.dataset.label = label
        col += td.colSpan
      }
    })
  })
}
let labelQueued = false
new MutationObserver(() => {
  if (labelQueued) return
  labelQueued = true
  requestAnimationFrame(() => { labelQueued = false; labelTables() })
}).observe(document.documentElement, { childList: true, subtree: true })

const standalone = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true
// iPhone и iPad: Safari не показывает окно установки — ставится через «Поделиться → На экран „Домой“».
// iPadOS представляется Mac-ом, поэтому ещё проверяем сенсорный экран
const isIOS = () => /iphone|ipad|ipod/i.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)

// Роли сервера: admin — владелец, manager — менеджер, warehouse — кладовщик
const ROLES = {
  admin: { title: 'Владелец магазина', app: 'CRM · владелец' },
  manager: { title: 'Менеджер по заказам', app: 'CRM · менеджер' },
  warehouse: { title: 'Склад', app: 'CRM · склад' },
}
const OFFICE = ['admin', 'manager']
const CLIENT_TYPES = { home_baker: 'Домашний кондитер', bakery: 'Кондитерская', cafe: 'Кафе', shop: 'Магазин', other: 'Другое' }
const PERIODS = [['today', 'Сегодня'], ['week', 'Неделя'], ['month', 'Месяц'], ['all', 'Всё время']]
const PERIOD_SHORT = { today: 'сегодня', week: 'за неделю', month: 'за месяц', all: 'за всё время' }

const NAV = [
  { group: 'Продажи', page: 'home', href: '/crm/', label: 'Сводка', icon: 'home', roles: OFFICE },
  { group: 'Продажи', page: 'orders', href: '/crm/orders.html', label: 'Заказы', icon: 'orders', badge: 'new', roles: OFFICE },
  { group: 'Продажи', page: 'clients', href: '/crm/clients.html', label: 'Клиенты', icon: 'clients', roles: OFFICE },
  { group: 'Склад', page: 'stock', href: '/crm/stock.html', label: 'Остатки и полки', short: 'Остатки', icon: 'stock', badge: 'low', roles: ['admin', 'manager', 'warehouse'] },
  { group: 'Склад', page: 'picking', href: '/crm/picking.html', label: 'Сборка и выдача', short: 'Сборка', icon: 'picking', badge: 'queue', roles: ['admin', 'warehouse'] },
  { group: 'Магазин', page: 'catalog', href: '/crm/catalog.html', label: 'Каталог и цены', short: 'Каталог', icon: 'catalog', roles: ['admin'] },
  { group: 'Магазин', page: 'analytics', href: '/crm/analytics.html', label: 'Аналитика', icon: 'analytics', roles: ['admin'] },
  { group: 'Магазин', page: 'employees', href: '/crm/employees.html', label: 'Сотрудники', icon: 'employees', roles: ['admin'] },
]
// Нижняя панель на телефоне: первые 4 пункта роли, остальное — во «Ещё»
const TABS = {
  admin: ['home', 'orders', 'picking', 'analytics'],
  manager: ['home', 'orders', 'clients', 'stock'],
  warehouse: ['picking', 'stock'],
}
// Иконки разделов (24×24, одна линия) — общие для бокового меню и нижней панели
const ICONS = {
  home: 'M4 11 12 4l8 7v9h-5v-6H9v6H4z',
  orders: 'M6 3h12v18l-3-2-3 2-3-2-3 2zM9 8h6M9 12h6M9 16h3',
  clients: 'M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM2 21c.8-3.6 3.6-6 7-6s6.2 2.4 7 6M16 3.5a4 4 0 0 1 0 7M18.5 15c1.8.9 3 2.9 3.5 6',
  stock: 'M12 3 4 7v10l8 4 8-4V7zM4 7l8 4 8-4M12 11v10',
  catalog: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 16.5h7M16.5 13v7',
  picking: 'M9 4h6v3H9zM7 5.5H5V21h14V5.5h-2M8.5 13l2.5 2.5 4.5-5',
  analytics: 'M4 20V10M10 20V4M16 20v-7M22 20H2',
  employees: 'M15 8a3 3 0 1 1-6 0 3 3 0 0 1 6 0ZM5 20c.8-3.4 3.6-5.5 7-5.5s6.2 2.1 7 5.5M19 4.5l1.5 1.5 3-3',
  profile: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM4 21c1-4 4.5-6 8-6s7 2 8 6',
  more: 'M4.5 12a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M10.5 12a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M16.5 12a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0',
}
const homeFor = role => NAV.find(i => i.roles.includes(role))?.href ?? '/crm/stock.html'

const toLogin = () => {
  const next = location.pathname + location.search
  location.replace(`/crm/login.html${next.startsWith('/crm/login') ? '' : `?next=${encodeURIComponent(next)}`}`)
}

const csvCell = v => `"${String(v ?? '').replaceAll('"', '""')}"`
function downloadCsv(name, rows) {
  const blob = new Blob(['﻿' + rows.map(r => r.map(csvCell).join(';')).join('\r\n')], { type: 'text/csv;charset=utf-8' })
  const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: name })
  a.click()
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
}

// «1 200,50» → «1200.50» для API; null — не сумма
const moneyInput = v => {
  const s = String(v ?? '').replace(/\s/g, '').replace(',', '.')
  return /^\d+(\.\d{1,2})?$/.test(s) ? s : null
}

// Пароль, который проходит правила входа (8+ символов, строчная и заглавная, цифра); без похожих 0/O, 1/l
function genPassword() {
  const rnd = n => crypto.getRandomValues(new Uint32Array(1))[0] % n
  const parts = ['abcdefghkmnpqrstuvwxyz', 'ABCDEFGHKMNPQRSTUVWXYZ', '23456789']
  const all = parts.join('')
  const chars = [...parts.map(p => p[rnd(p.length)]), ...Array.from({ length: 7 }, () => all[rnd(all.length)])]
  for (let i = chars.length - 1; i > 0; i--) { const j = rnd(i + 1); [chars[i], chars[j]] = [chars[j], chars[i]] }
  return chars.join('')
}

// Остаток к закупке: сколько не хватает до минимума
const shortage = s => Math.max(0, milli(s.min_stock_qty) - milli(s.available_qty))
// К оплате считает сервер (due). Кладовщику поля нет — тогда и долга не видно
const remaining = o => (o?.due == null ? 0 : Math.max(0, tiyn(o.due)))
const isOutstanding = o => o.status !== 'cancelled' && o.payment_status !== 'refunded' && remaining(o) > 0
const byShelf = items => [...items].sort((a, b) => (a.storage_location ?? '').localeCompare(b.storage_location ?? '', 'ru', { numeric: true }))

document.addEventListener('alpine:init', () => {
  const pageProgress = initPageProgress()
  Alpine.store('loading', {
    busy: false,
    slow: false,
  })
  onLoadingChange(s => {
    const store = Alpine.store('loading')
    if (store) {
      store.busy = s.busy
      store.slow = s.slow
    }
    if (s.busy) pageProgress.start()
    else pageProgress.done()
  })

  Alpine.store('fmt', { ...fmt, phone: formatPhone, money: formatTiyn, qty: formatQty, tiyn })

  // «Установить приложение»: кнопка видна, пока браузер готов установить и CRM открыта не из значка
  Alpine.store('pwa', {
    available: Boolean(installEvent),
    installed: standalone(),
    ios: isIOS(),
    iosHint: false,
    get canInstall() { return (this.available || this.ios) && !this.installed },
    async install() {
      if (!installEvent) { if (this.ios) this.iosHint = true; return }
      installEvent.prompt()
      const { outcome } = await installEvent.userChoice
      if (outcome === 'accepted') { this.installed = true; installEvent = null }
      this.available = Boolean(installEvent)
    },
  })

  Alpine.store('crm', {
    page: document.body.dataset.page ?? '',
    ready: false,
    error: null,
    toast: '',
    drawer: false,
    statuses: STATUSES,
    payment: PAYMENT,
    channels: CHANNELS,
    delivery: DELIVERY,
    payMethods: PAYMENT_METHODS,

    session: session.get(),
    employees: [],
    orders: [],
    stock: [],
    products: [],
    categories: [],
    sections: [],
    stockById: new Map(),
    productById: new Map(),

    async init() {
      if (this.page === 'login') { this.ready = true; return }
      if (this.session?.kind !== 'employee') { toLogin(); return }
      const page = NAV.find(i => i.page === this.page)
      if (page && !page.roles.includes(this.role)) { location.replace(homeFor(this.role)); return }
      if (this.page === 'order' && !this.can('office')) { location.replace(`/crm/picking.html${params.get('id') ? `?id=${params.get('id')}` : ''}`); return }
      try {
        const [me, employees, orders, stock, products, categories] = await Promise.all([
          api.whoami(), api.employees(), api.allOrders(), api.stock(), api.products(), api.categories(),
        ])
        // Роль могли поменять, пока сессия жила
        if (me.role !== this.role) { this.session = { ...this.session, user: me }; session.set(this.session) }
        this.employees = employees
        this.categories = categories
        this.sections = buildSections(categories)
        this.orders = orders
        this.setStock(stock)
        this.setProducts(products)
      } catch (e) {
        console.error(e)
        if (e.status !== 401) this.error = `Не удалось загрузить данные: ${e.message}`
      } finally {
        this.ready = true
      }
    },
    setStock(list) {
      this.stock = list
      this.stockById = new Map(list.map(s => [s.product_id, s]))
    },
    setProducts(list) {
      this.products = list
      this.productById = new Map(list.map(p => [p.id, p]))
    },
    async refreshOrders() { this.orders = await api.allOrders() },
    async refreshStock() { this.setStock(await api.stock()) },
    // Заказ изменился (сервер вернул новую версию) — подменить в списке
    putOrder(o) {
      const { items, events, client_orders, ...brief } = o
      const i = this.orders.findIndex(x => x.id === o.id)
      if (i >= 0) this.orders.splice(i, 1, { ...this.orders[i], ...brief })
      else this.orders.unshift(brief)
    },

    // ---------------------------------------------------------------- пользователь
    get user() {
      const u = this.session?.user ?? { name: '', role: '' }
      return { ...u, ...(ROLES[u.role] ?? { title: '', app: 'CRM' }) }
    },
    get role() { return this.session?.user?.role ?? '' },
    // office — заказы, оплаты, клиенты; owner — каталог, цены, сотрудники; stock — приход и полки
    can(what) {
      const r = this.role
      if (what === 'office') return OFFICE.includes(r)
      if (what === 'owner') return r === 'admin'
      if (what === 'stock') return r === 'admin' || r === 'warehouse'
      return false
    },
    async logout() {
      await api.logout()
      location.href = '/crm/login.html'
    },
    employee(id) { return this.employees.find(e => e.id === id) },
    employeeName(id) { return id ? this.employee(id)?.name ?? id : 'не назначен' },
    get assignees() { return this.employees.filter(e => e.is_active !== false && OFFICE.includes(e.role)) },

    icon(name) { return ICONS[name] ?? '' },
    isActive(i) { return this.page === i.page || (this.page === 'order' && i.page === 'orders') },
    // Нижняя панель (телефон): до 4 разделов роли + «Ещё», если остались
    get tabs() {
      const ids = TABS[this.role] ?? []
      return ids.map(id => NAV.find(i => i.page === id)).filter(i => i && i.roles.includes(this.role))
    },
    get moreItems() {
      const tabs = this.tabs.map(i => i.page)
      return NAV.filter(i => i.roles.includes(this.role) && !tabs.includes(i.page))
    },
    get moreActive() { return this.moreItems.some(i => this.isActive(i)) },
    sheet: false,
    openSheet() { this.sheet = true; document.documentElement.classList.add('overflow-hidden') },
    closeSheet() { if (!this.sheet) return; this.sheet = false; document.documentElement.classList.remove('overflow-hidden') },

    get nav() {
      const items = NAV.filter(i => i.roles.includes(this.role))
      return [...new Set(items.map(i => i.group))].map(group => ({ group, items: items.filter(i => i.group === group) }))
    },
    badge(kind) {
      if (kind === 'new') return this.orders.filter(o => o.status === 'new').length
      if (kind === 'queue') return this.orders.filter(o => o.status === 'confirmed' || o.status === 'assembled').length
      if (kind === 'low') return this.stock.filter(s => s.low_stock).length
      return 0
    },

    // ---------------------------------------------------------------- справочники
    product(id) { return this.productById.get(id) },
    stockOf(id) { return this.stockById.get(id) },
    shelf(id) { return this.stockOf(id)?.storage_location ?? '—' },
    category(id) { return this.categories.find(c => c.id === id) ?? {} },
    productName(p) {
      if (!p) return ''
      return p.variant_name && !p.name.includes(p.variant_name) && p.group_id && this.products.filter(x => x.group_id === p.group_id).length > 1
        ? `${p.name}, ${p.variant_name}` : p.name
    },
    payKey: paymentKey,
    remaining,

    notify(text) {
      this.toast = text
      clearTimeout(this._t)
      this._t = setTimeout(() => (this.toast = ''), 3200)
    },
    openDrawer() {
      this.drawer = true
      document.documentElement.classList.add('overflow-hidden')
      requestAnimationFrame(() => document.querySelector('#crm-menu [data-close]')?.focus({ focusVisible: false }))
    },
    closeDrawer() {
      if (!this.drawer) return
      this.drawer = false
      document.documentElement.classList.remove('overflow-hidden')
    },
  })

  // Сессия истекла или пароль сменили — на вход
  onUnauthorized(() => { if (Alpine.store('crm').page !== 'login') toLogin() })

  matchMedia('(min-width: 64rem)').addEventListener('change', e => e.matches && Alpine.store('crm').closeDrawer())

  // ================================================================ Вход сотрудника
  Alpine.data('crmLogin', () => ({
    login: '',
    password: '',
    show: false,
    caps: false,
    busy: false,
    error: null,
    lock: persisted(LOCK_KEY, { fails: 0, until: 0 }),
    now: Date.now(),
    init() {
      if (session.get()?.kind === 'employee') location.replace(this.next(session.get().user.role))
      setInterval(() => (this.now = Date.now()), 1000)
    },
    next(role) {
      const n = params.get('next')
      return n && n.startsWith('/crm/') && !n.startsWith('/crm/login') ? n : homeFor(role)
    },
    get lockLeft() { return Math.max(0, Math.ceil((this.lock.until - this.now) / 1000)) },
    capsCheck(e) { this.caps = e.getModifierState?.('CapsLock') ?? false },
    async submit() {
      this.error = null
      if (!this.login.trim() || !this.password) { this.error = 'Введите логин и пароль'; return }
      if (this.lockLeft) return
      this.busy = true
      try {
        const s = await api.login(this.login.trim(), this.password)
        save(LOCK_KEY, { fails: 0, until: 0 })
        location.replace(this.next(s.user.role))
      } catch (e) {
        if (e.status === 401) {
          const fails = this.lock.fails + 1
          this.lock = fails >= 5 ? { fails: 0, until: Date.now() + 5 * 60_000 } : { fails, until: 0 }
          save(LOCK_KEY, this.lock)
          this.error = fails >= 5 ? 'Слишком много попыток. Попробуйте через 5 минут' : `Неверный логин или пароль. Осталось попыток: ${5 - fails}`
        } else this.error = e.message
      } finally {
        this.busy = false
      }
    },
  }))

  // ================================================================ Сводка (владелец, менеджер) — GET /api/dashboard
  Alpine.data('crmHome', () => ({
    period: 'week',
    d: null,
    loading: false,
    error: null,
    periods: PERIODS,
    get periodLabel() { return { today: 'Сегодня', week: 'За неделю', month: 'За месяц', all: 'За всё время' }[this.period] },
    get periodShort() { return PERIOD_SHORT[this.period] },
    init() {
      this.load()
      this.$watch('period', () => this.load())
    },
    async load() {
      this.loading = true
      try { this.d = await api.dashboard(this.period); this.error = null } catch (e) { this.error = e.message } finally { this.loading = false }
    },
    get unpaid() {
      return this.$store.crm.orders.filter(isOutstanding).sort((a, b) => remaining(b) - remaining(a)).slice(0, 8)
    },
    get fresh() { return this.$store.crm.orders.filter(o => o.status === 'new').slice(0, 6) },
    get attention() { return this.$store.crm.orders.filter(o => o.is_problem || (o.is_urgent && ACTIVE.includes(o.status))).slice(0, 6) },
    get byStatus() {
      const counts = new Map((this.d?.by_status ?? []).map(x => [x.key, x.count]))
      return STATUS_ORDER.map(s => ({ s, n: counts.get(s) ?? 0 }))
    },
    get byChannel() {
      const list = this.d?.by_channel ?? []
      const max = Math.max(1, ...list.map(c => c.count))
      return list.filter(c => c.count).map(c => ({ id: c.key, label: CHANNELS[c.key] ?? c.key, n: c.count, sum: tiyn(c.sum), pct: Math.round((c.count / max) * 100) }))
    },
    get purchase() {
      return this.$store.crm.stock.filter(s => s.low_stock).sort((a, b) => shortage(b) - shortage(a)).slice(0, 8)
    },
  }))

  // ================================================================ Аналитика (владелец) — GET /api/analytics (ФТ-В-04)
  // Откуда приходят заказы, где теряются клиенты, что популярно, с каких устройств заходят, последний вход
  Alpine.data('crmAnalytics', () => ({
    period: params.get('period') ?? 'month',
    a: null,
    loading: false,
    error: null,
    periods: PERIODS,
    devices: { mobile: 'Телефон', tablet: 'Планшет', desktop: 'Компьютер' },
    init() {
      this.load()
      this.$watch('period', () => { this.load(); history.replaceState(null, '', `?period=${this.period}`) })
    },
    async load() {
      this.loading = true
      try { this.a = await api.analytics(this.period); this.error = null } catch (e) { this.error = e.message } finally { this.loading = false }
    },
    get periodShort() { return PERIOD_SHORT[this.period] },
    // Полоски: доля от самого большого значения; by — count (штук) или sum (деньги)
    bars(list, names = {}, by = 'count') {
      const rows = (list ?? []).filter(x => x.count)
      const val = x => (by === 'sum' ? tiyn(x.sum) : x.count)
      const max = Math.max(1, ...rows.map(val))
      const total = rows.reduce((n, x) => n + x.count, 0) || 1
      return rows.map(x => ({
        key: x.key ?? '—', label: names[x.key] ?? (x.key || 'не указан'), n: x.count, sum: tiyn(x.sum),
        pct: Math.round((val(x) / max) * 100), share: Math.round((x.count / total) * 100),
      }))
    },
    get channels() { return this.bars(this.a?.orders_by_channel, CHANNELS, 'sum') },
    get cities() { return this.bars(this.a?.orders_by_city, {}, 'sum').slice(0, 8) },
    get clientTypes() { return this.bars(this.a?.orders_by_client_type, CLIENT_TYPES, 'sum') },
    get categories() { return this.bars(this.a?.top_categories, {}, 'sum') },
    get reasons() { return this.bars(this.a?.cancel_reasons) },
    get byDevice() { return this.bars(this.a?.by_device, this.devices) },
    get byBrowser() { return this.bars(this.a?.by_browser).slice(0, 6) },
    get byOs() { return this.bars(this.a?.by_os).slice(0, 6) },
    // Мало посещений — процент ничего не значит (а на демо-данных заказов с сайта больше, чем визитов)
    get conversion() {
      if ((this.a?.visitors ?? 0) < 10) return null
      return Math.min(100, this.a.site_conversion ?? 0).toLocaleString('ru-RU', { maximumFractionDigits: 1 })
    },
    get repeatShare() {
      const n = (this.a?.new_clients ?? 0) + (this.a?.repeat_clients ?? 0)
      return n ? Math.round(((this.a?.repeat_clients ?? 0) / n) * 100) : 0
    },
    ago(iso) {
      const min = Math.round((Date.now() - new Date(iso)) / 60000)
      if (min < 60) return min <= 1 ? 'только что' : `${min} мин назад`
      const h = Math.round(min / 60)
      if (h < 24) return `${h} ч назад`
      const d = Math.round(h / 24)
      return d === 1 ? 'вчера' : `${this.$store.fmt.count(d, 'день', 'дня', 'дней')} назад`
    },
  }))

  // ================================================================ Сотрудники (владелец) — /api/employees (ФТ-Р-01, ФТ-Р-04)
  // Личный вход у каждого; роль решает, что видно. Пароль сотрудника хранится только хешем — сменить можно, увидеть нельзя
  Alpine.data('crmEmployees', () => ({
    edit: null,
    saving: false,
    formError: null,
    showPw: false,
    roles: Object.entries(ROLES).map(([id, r]) => [id, r.title]),
    access: {
      admin: 'Всё: заказы, оплаты, клиенты, склад, каталог и цены, аналитика, сотрудники',
      manager: 'Заказы, оплаты, клиенты, остатки — без каталога, цен и сотрудников',
      warehouse: 'Остатки, приход, сборка и выдача — без оплат и цен',
    },
    get list() {
      const order = { admin: 0, manager: 1, warehouse: 2 }
      return [...this.$store.crm.employees].sort((a, b) => Number(b.is_active) - Number(a.is_active) || order[a.role] - order[b.role] || a.name.localeCompare(b.name, 'ru'))
    },
    open(e) {
      this.formError = null
      this.showPw = !e
      this.edit = e
        ? { id: e.id, name: e.name, login: e.login ?? '', role: e.role, phone: e.phone ?? '', is_active: e.is_active, password: '' }
        : { id: null, name: '', login: '', role: 'manager', phone: '', is_active: true, password: this.suggest() }
    },
    suggest: () => genPassword(),
    get pwProblems() { return this.edit?.password ? passwordProblems(this.edit.password) : [] },
    async save() {
      const e = this.edit
      this.formError = null
      if (e.name.trim().length < 2) { this.formError = 'Укажите имя'; return }
      if (!/^[a-z0-9_.]{3,}$/i.test(e.login.trim())) { this.formError = 'Логин — латиница, цифры, точка или _, от 3 символов'; return }
      if (!e.id && !e.password) { this.formError = 'Задайте пароль для первого входа'; return }
      if (e.password && this.pwProblems.length) { this.formError = `Пароль: не хватает — ${this.pwProblems.join(', ')}`; return }
      const body = { name: e.name.trim(), login: e.login.trim().toLowerCase(), role: e.role, phone: e.phone.trim() || null }
      if (e.password) body.password = e.password
      if (e.id) body.is_active = e.is_active
      this.saving = true
      try {
        const saved = e.id ? await api.updateEmployee(e.id, body) : await api.createEmployee(body)
        const list = this.$store.crm.employees
        const i = list.findIndex(x => x.id === saved.id)
        if (i >= 0) list.splice(i, 1, saved); else list.push(saved)
        this.$store.crm.notify(e.id ? `Сохранено: ${saved.name}` : `Добавлен сотрудник ${saved.name}. Передайте ему логин и пароль`)
        this.edit = null
      } catch (err) { this.formError = err.message } finally { this.saving = false }
    },
    copy(text) {
      navigator.clipboard?.writeText(text).then(() => this.$store.crm.notify('Скопировано'), () => {})
    },
  }))

  // ================================================================ Заказы: список
  Alpine.data('crmOrders', () => ({
    status: params.get('status') ?? '',
    pay: params.get('pay') ?? '',
    q: params.get('q') ?? '',
    channel: params.get('channel') ?? '',
    manager: '',
    flag: params.get('flag') ?? '',
    page: 1,
    perPage: 20,
    list: [],        // ответ API с серверными фильтрами status + payment_status
    loading: false,
    error: null,
    creating: params.get('new') === '1',
    payOptions: [['', 'Оплата: все'], ['unpaid', 'Не оплачен'], ['partial', 'Частично'], ['paid', 'Оплачен'], ['refunded', 'Возврат']],

    init() {
      this.fetch()
      ;['status', 'pay'].forEach(k => this.$watch(k, () => { this.page = 1; this.syncUrl(); this.fetch() }))
      ;['q', 'channel', 'manager', 'flag'].forEach(k => this.$watch(k, () => { this.page = 1; this.syncUrl() }))
      // Заказ поменялся в карточке или создан здесь — список обновить
      this.$watch('$store.crm.orders', () => this.fetch())
    },
    syncUrl() {
      const url = new URL(location.href)
      const set = (k, v) => (v ? url.searchParams.set(k, v) : url.searchParams.delete(k))
      set('status', this.status); set('pay', this.pay); set('q', this.q.trim()); set('channel', this.channel); set('flag', this.flag)
      url.searchParams.delete('new')
      history.replaceState(null, '', url)
    },
    // Фильтры по статусу и оплате — на сервере (?status=&payment_status=)
    async fetch() {
      this.loading = true
      try {
        this.list = await api.allOrders({ status: this.status, payment_status: this.pay })
        this.error = null
      } catch (e) {
        this.error = e.message
      } finally {
        this.loading = false
      }
    },
    get tabs() { return [['', 'Все'], ...STATUS_ORDER.map(s => [s, STATUSES[s].label])] },
    count(s) { return s ? this.$store.crm.orders.filter(o => o.status === s).length : this.$store.crm.orders.length },
    get filtered() {
      const q = this.q.trim().toLowerCase()
      const qd = phoneDigits(q)
      return this.list.filter(o => {
        if (this.channel && o.channel !== this.channel) return false
        if (this.manager && o.assigned_to !== this.manager) return false
        if (this.flag === 'problem' && !o.is_problem) return false
        if (this.flag === 'urgent' && !o.is_urgent) return false
        if (q) {
          const hay = `${o.order_no} ${o.client_name} ${o.city ?? ''} ${o.delivery_address ?? ''}`.toLowerCase()
          if (!hay.includes(q) && !(qd.length >= 4 && phoneDigits(o.client_phone).includes(qd))) return false
        }
        return true
      })
    },
    get pages() { return Math.max(1, Math.ceil(this.filtered.length / this.perPage)) },
    get rows() { return this.filtered.slice((this.page - 1) * this.perPage, this.page * this.perPage) },
    get hasFilters() { return Boolean(this.status || this.pay || this.q || this.channel || this.manager || this.flag) },
    reset() { Object.assign(this, { status: '', pay: '', q: '', channel: '', manager: '', flag: '' }) },

    get kpi() {
      const orders = this.$store.crm.orders
      const unpaid = orders.filter(isOutstanding)
      return {
        inWork: orders.filter(o => ACTIVE.includes(o.status)).length,
        waiting: orders.filter(o => o.status === 'new').length,
        unpaid: unpaid.length,
        unpaidSum: unpaid.reduce((s, o) => s + remaining(o), 0),
        toShip: orders.filter(o => o.status === 'assembled').length,
        problem: orders.filter(o => o.is_problem).length,
      }
    },
    exportCsv() {
      const crm = this.$store.crm
      const rows = [['№', 'Дата', 'Клиент', 'Телефон', 'Канал', 'Город', 'Статус', 'Товары', 'Доставка', 'Итого', 'Оплачено', 'Оплата', 'Менеджер']]
      this.filtered.forEach(o => rows.push([
        o.order_no, fmt.dateTime(o.created_at), o.client_name, o.client_phone, CHANNELS[o.channel], o.city, STATUSES[o.status].label,
        o.items_total.replace('.', ','), o.delivery.replace('.', ','), o.total.replace('.', ','), (o.paid ?? '').replace('.', ','),
        PAYMENT[paymentKey(o)]?.label ?? '', crm.employeeName(o.assigned_to),
      ]))
      downloadCsv(`zakazy-${new Date().toISOString().slice(0, 10)}.csv`, rows)
    },
  }))

  // ---------------------------------------------------------------- новый заказ (звонок, WhatsApp, Instagram)
  Alpine.data('newOrder', () => ({
    form: { phone: '', name: '', channel: 'whatsapp', city: 'Астана', delivery_method: 'pickup', address: '', delivery: '', note: '', urgent: false },
    client: null,   // найденная карточка клиента — заказ пойдёт с его персональными ценами
    lines: [],      // { productId, qty }
    add: '',
    search: '',
    errors: {},
    sending: false,
    serverError: null,
    channelOptions: Object.entries(CHANNELS).filter(([id]) => !id.startsWith('site')),
    init() {
      this.$watch('form.phone', async v => {
        this.client = null
        if (!isPhone(v)) return
        try {
          const found = await api.clients({ search: phoneDigits(v).slice(1) })
          this.client = found.find(c => phoneDigits(c.phone) === phoneDigits(v)) ?? null
          if (this.client && !this.form.name) this.form.name = this.client.name
          if (this.client?.city) this.form.city = this.client.city
        } catch { /* поиск — подсказка, без него заказ всё равно оформится */ }
      })
    },
    get choices() {
      const q = this.search.trim().toLowerCase()
      const crm = this.$store.crm
      return crm.products
        .filter(p => !this.lines.some(l => l.productId === p.id))
        .filter(p => !q || `${p.name} ${p.variant_name ?? ''} ${p.sku ?? ''}`.toLowerCase().includes(q))
        .slice(0, 60)
    },
    free(id) { return Math.max(0, Math.floor(milli(this.$store.crm.product(id)?.available_qty) / 1000)) },
    addLine() {
      if (!this.add) return
      if (this.free(this.add) > 0) this.lines.push({ productId: this.add, qty: 1 })
      else this.serverError = 'Этого товара нет в наличии'
      this.add = ''
      this.search = ''
    },
    fixQty(l) { l.qty = Math.max(1, Math.min(Math.floor(Number(l.qty) || 1), this.free(l.productId) || 1)) },
    price(l) { return retail(this.$store.crm.product(l.productId)) },
    sum(l) { return lineSum(this.$store.crm.product(l.productId), Math.max(0, Number(l.qty) || 0)) },
    get items() { return this.lines.reduce((s, l) => s + this.sum(l), 0) },
    get deliveryTiyn() { return this.form.delivery_method === 'pickup' ? 0 : tiyn(moneyInput(this.form.delivery) ?? '0') },
    get total() { return this.items + this.deliveryTiyn },
    validate() {
      const e = {}
      if (this.form.name.trim().length < 2) e.name = 'Укажите клиента'
      if (!isPhone(this.form.phone)) e.phone = 'Телефон обязателен: +7 (7XX) XXX-XX-XX'
      if (!this.lines.length) e.lines = 'Добавьте хотя бы один товар'
      if (this.form.delivery && moneyInput(this.form.delivery) === null) e.delivery = 'Сумма, например 1200'
      this.errors = e
      return !Object.keys(e).length
    },
    async submit() {
      if (!this.validate()) return
      const crm = this.$store.crm
      this.sending = true
      this.serverError = null
      try {
        const o = await api.createOrder({
          client_name: this.form.name.trim(),
          client_phone: formatPhone(this.form.phone),
          client_id: this.client?.id,
          source: this.form.channel,
          city: this.form.city.trim() || null,
          delivery_method: this.form.delivery_method,
          delivery_address: this.form.delivery_method === 'pickup' ? null : this.form.address.trim() || null,
          delivery: this.form.delivery_method === 'pickup' ? null : tiynToString(this.deliveryTiyn),
          is_urgent: this.form.urgent || undefined,
          assigned_to: crm.role === 'manager' ? crm.user.id : undefined,
          note: this.form.note.trim() || null,
          items: this.lines.map(l => ({ product_id: l.productId, quantity: qtyToString(l.qty) })),
        })
        crm.notify(`Заказ №${o.order_no} создан`)
        location.href = `/crm/order.html?id=${o.id}`
      } catch (e) {
        this.serverError = e.message
      } finally {
        this.sending = false
      }
    },
  }))

  // ================================================================ Карточка заказа (владелец, менеджер)
  Alpine.data('crmOrder', () => ({
    id: params.get('id'),
    o: null,
    loading: true,
    notFound: false,
    busy: false,
    error: null,
    // смена статуса
    modal: false,
    next: '',
    reason: '',
    refundOk: false,
    // оплата
    payModal: false,
    payForm: { method: 'kaspi_transfer', kind: 'full', amount: '' },
    deliveryEdit: false,
    deliveryValue: '',
    // состав
    addId: '',
    addSearch: '',
    comment: '',
    hints: STATUS_HINT,
    flow: FLOW,
    deliveryNames: DELIVERY,

    async init() {
      if (!this.id) { this.loading = false; this.notFound = true; return }
      try {
        this.o = await api.order(this.id)
      } catch (e) {
        if (e.status === 404) this.notFound = true
        else this.error = e.message
      } finally {
        this.loading = false
      }
    },
    // Переходы по графу; «Собран» сервер пропустит, только когда склад отметил все позиции
    get options() { return this.o ? NEXT[this.o.status] : [] },
    get step() { return FLOW.indexOf(this.o.status) },
    get closed() { return ['cancelled', 'completed'].includes(this.o.status) },
    get editable() { return EDITABLE.includes(this.o.status) },
    get paidT() { return tiyn(this.o.paid) },
    get totalT() { return tiyn(this.o.total) },
    get due() { return remaining(this.o) },
    get shortItems() { return this.o.items.filter(i => milli(i.short_qty) > 0) },
    free(i) { return this.$store.crm.stockOf(i.product_id) },

    // Любое действие: сервер возвращает заказ целиком — подменяем и показываем текст ошибки (409 и др.)
    async run(fn, message) {
      this.busy = true
      this.error = null
      try {
        const updated = await fn()
        this.o = updated
        this.$store.crm.putOrder(updated)
        if (message) this.$store.crm.notify(message)
        return true
      } catch (e) {
        this.error = e.message
        return false
      } finally {
        this.busy = false
      }
    },
    patch(body, message) { return this.run(() => api.updateOrder(this.o.id, body), message) },

    openStatus() {
      this.next = this.options[0] ?? ''
      this.reason = ''
      this.refundOk = false
      this.error = null
      this.modal = true
    },
    get needsRefund() { return this.next === 'cancelled' && ['paid', 'partial'].includes(this.o.payment_status) },
    async saveStatus() {
      if (!this.next) return
      const crm = this.$store.crm
      // Отмена оплаченного — сначала возврат (ФТ-П-05), потом отмена
      if (this.needsRefund) {
        if (!this.refundOk) return
        if (!await this.run(() => api.payment(this.o.id, { payment_status: 'refunded' }), null)) return
      }
      const body = { status: this.next }
      if (this.next === 'cancelled') body.cancelled_reason = this.reason.trim() || null
      if (crm.role === 'manager' && !this.o.assigned_to) body.assigned_to = crm.user.id
      if (await this.patch(body, `Заказ №${this.o.order_no}: ${STATUSES[this.next].label}`)) {
        this.modal = false
        crm.refreshStock()
      }
    },
    toggleUrgent() { return this.patch({ is_urgent: !this.o.is_urgent }, this.o.is_urgent ? 'Срочность снята' : 'Заказ отмечен срочным') },
    clearProblem() { return this.patch({ is_problem: false }, 'Проблема снята') },

    // ---- оплата: только вручную, система о платеже не узнаёт (POST /payment)
    openPay() {
      this.payForm = { method: this.o.payment_method !== 'none' ? this.o.payment_method : 'kaspi_transfer', kind: 'full', amount: '' }
      this.error = null
      this.payModal = true
    },
    get payAmount() { return tiyn(moneyInput(this.payForm.amount) ?? '0') },
    get payValid() { return this.payForm.kind === 'full' || (this.payAmount > 0 && this.payAmount < this.totalT) },
    async savePay() {
      if (!this.payValid) return
      const body = this.payForm.kind === 'full'
        ? { payment_status: 'paid', payment_method: this.payForm.method }
        : { payment_status: 'partial', paid: tiynToString(this.payAmount), payment_method: this.payForm.method }
      const msg = this.payForm.kind === 'full' ? `Заказ №${this.o.order_no} отмечен оплаченным` : `Предоплата ${formatTiyn(this.payAmount)} записана`
      if (await this.run(() => api.payment(this.o.id, body), msg)) this.payModal = false
    },
    unmarkPay() { return this.run(() => api.payment(this.o.id, { payment_status: 'unpaid' }), 'Отметка об оплате снята') },
    refund() { return this.run(() => api.payment(this.o.id, { payment_status: 'refunded' }), 'Возврат отмечен') },

    // ---- доставка: итог = товары + доставка
    editDelivery() { this.deliveryValue = this.o.delivery.replace(/\.00$/, ''); this.deliveryEdit = true },
    async saveDelivery() {
      const v = moneyInput(this.deliveryValue)
      if (v === null) { this.error = 'Стоимость доставки — число, например 1200'; return }
      if (await this.patch({ delivery: v }, 'Стоимость доставки сохранена')) this.deliveryEdit = false
    },
    assign(id) { return this.patch({ assigned_to: id }, `Менеджер: ${this.$store.crm.employeeName(id)}`) },

    // ---- состав: правка в статусах «Новый» и «Подтверждён» (ФТ-М-07), резерв пересчитывает сервер
    async setQty(i, value) {
      const q = String(value).replace(',', '.').trim()
      if (!/^\d+(\.\d{1,3})?$/.test(q) || !(Number(q) > 0)) { this.error = 'Количество — число больше нуля'; return }
      if (milli(q) === milli(i.quantity)) return
      if (await this.run(() => api.updateItem(this.o.id, i.id, { quantity: q }), `${i.product_name}: ${formatQty(q)} ${i.unit}`)) this.$store.crm.refreshStock()
    },
    async setDiscount(i, value) {
      const d = Math.round(Number(value))
      if (!(d >= 0 && d <= 100)) { this.error = 'Скидка — от 0 до 100 %'; return }
      if (d === i.discount_percent) return
      await this.run(() => api.updateItem(this.o.id, i.id, { discount_percent: d }), `Скидка ${d} % на ${i.product_name}`)
    },
    async removeItem(i) {
      if (await this.run(() => api.removeItem(this.o.id, i.id), `Убрано: ${i.product_name}`)) this.$store.crm.refreshStock()
    },
    get addChoices() {
      const q = this.addSearch.trim().toLowerCase()
      const crm = this.$store.crm
      return crm.products
        .filter(p => !this.o.items.some(i => i.product_id === p.id))
        .filter(p => !q || `${p.name} ${p.variant_name ?? ''} ${p.sku ?? ''}`.toLowerCase().includes(q))
        .slice(0, 60)
    },
    async addItem() {
      if (!this.addId) return
      const p = this.$store.crm.product(this.addId)
      if (await this.run(() => api.addItem(this.o.id, { product_id: this.addId, quantity: '1' }), `Добавлено: ${this.$store.crm.productName(p)}`)) {
        this.addId = ''
        this.addSearch = ''
        this.$store.crm.refreshStock()
      }
    },

    // ---- журнал
    async sendComment() {
      const text = this.comment.trim()
      if (!text) return
      try {
        this.o.events = await api.comment(this.o.id, text)
        this.comment = ''
      } catch (e) { this.error = e.message }
    },
    // Накладная З-2: сервер отдаёт HTML только с токеном, поэтому открываем её из ответа
    async printInvoice() {
      const w = window.open('', '_blank')
      try {
        const html = await api.invoice(this.o.id)
        w.document.open()
        w.document.write(html)
        w.document.close()
      } catch (e) {
        w?.close()
        this.error = e.message
      }
    },
  }))

  // ================================================================ Клиенты — GET /api/clients
  Alpine.data('crmClients', () => ({
    q: '',
    sort: 'sum',
    debtors: false,
    list: [],
    loading: true,
    error: null,
    types: CLIENT_TYPES,
    // Карточка клиента: данные, вход на сайт, персональные цены (ФТ-Ц-09, только владелец)
    card: null,
    form: null,
    prices: [],
    pricesLoading: false,
    pw: '',
    priceQ: '',
    newPrice: { product_id: '', price: '' },
    saving: false,
    formError: null,
    async init() {
      await this.load()
      this.$watch('debtors', () => this.load())
      if (params.get('id')) this.openCard({ id: params.get('id') })
      if (params.get('new') === '1') this.openNew()
    },
    async load() {
      this.loading = true
      try { this.list = await api.clients({ debtors: this.debtors || undefined }); this.error = null } catch (e) { this.error = e.message } finally { this.loading = false }
    },
    blank() { return { id: null, name: '', phone: '', city: '', client_type: 'home_baker', note: '' } },
    openNew() { this.formError = null; this.card = { id: null, orders: [], top_products: [] }; this.form = this.blank(); this.prices = [] },
    async openCard(c) {
      this.formError = null; this.pw = ''; this.priceQ = ''; this.newPrice = { product_id: '', price: '' }
      this.card = { ...c, loading: true, orders: [], top_products: [] }
      this.form = { id: c.id, name: c.name ?? '', phone: c.phone ?? '', city: c.city ?? '', client_type: c.client_type ?? 'other', note: c.note ?? '' }
      try {
        const full = await api.client(c.id)
        this.card = full
        this.form = { id: full.id, name: full.name, phone: full.phone ?? '', city: full.city ?? '', client_type: full.client_type, note: full.note ?? '' }
      } catch (e) { this.formError = e.message; this.card.loading = false }
      if (this.$store.crm.can('owner')) this.loadPrices()
    },
    close() { this.card = null; this.form = null; if (params.get('id') || params.get('new')) history.replaceState(null, '', location.pathname) },
    async loadPrices() {
      this.pricesLoading = true
      try { this.prices = await api.clientPrices(this.form.id) } catch (e) { this.formError = e.message } finally { this.pricesLoading = false }
    },
    async saveClient() {
      const f = this.form
      this.formError = null
      if (f.name.trim().length < 2) { this.formError = 'Укажите имя или название'; return }
      if (f.phone.trim() && !isPhone(f.phone)) { this.formError = 'Телефон в формате +7 (7XX) XXX-XX-XX'; return }
      const body = { name: f.name.trim(), phone: f.phone.trim() || null, city: f.city.trim() || null, client_type: f.client_type, note: f.note.trim() || null }
      this.saving = true
      try {
        const saved = f.id ? await api.updateClient(f.id, body) : await api.createClient(body)
        const i = this.list.findIndex(x => x.id === saved.id)
        if (i >= 0) this.list.splice(i, 1, { ...this.list[i], ...saved }); else this.list.unshift(saved)
        this.$store.crm.notify(f.id ? 'Клиент сохранён' : `Клиент ${saved.name} добавлен`)
        if (!f.id) { this.openCard(saved); return }
        this.card = { ...this.card, ...saved }
      } catch (e) { this.formError = e.message } finally { this.saving = false }
    },
    async setPassword() {
      this.formError = null
      if (!this.card.phone) { this.formError = 'Для входа на сайт у клиента должен быть телефон'; return }
      const probs = passwordProblems(this.pw)
      if (probs.length) { this.formError = `Пароль: не хватает — ${probs.join(', ')}`; return }
      this.saving = true
      try {
        const saved = await api.setClientPassword(this.card.id, this.pw)
        this.card = { ...this.card, ...saved }
        const i = this.list.findIndex(x => x.id === saved.id)
        if (i >= 0) this.list.splice(i, 1, { ...this.list[i], ...saved })
        this.$store.crm.notify(`Вход на сайт: телефон ${this.card.phone} и новый пароль`)
        this.pw = ''
      } catch (e) { this.formError = e.message } finally { this.saving = false }
    },
    suggestPw() { this.pw = genPassword() },
    // Товары для персональной цены: поиск по названию и артикулу, кроме уже настроенных
    get priceOptions() {
      const q = this.priceQ.trim().toLowerCase()
      if (q.length < 2) return []
      const taken = new Set(this.prices.map(p => p.product_id))
      return this.$store.crm.products
        .filter(p => !p.is_archived && !taken.has(p.id) && `${p.name} ${p.variant_name ?? ''} ${p.sku ?? ''}`.toLowerCase().includes(q))
        .slice(0, 8)
    },
    pickPrice(p) { this.newPrice = { product_id: p.id, price: tiynToString(tiyn(p.price)).replace('.00', '') }; this.priceQ = this.$store.crm.productName(p) },
    async addPrice() {
      this.formError = null
      const price = moneyInput(this.newPrice.price)
      if (!this.newPrice.product_id) { this.formError = 'Выберите товар из списка'; return }
      if (price === null) { this.formError = 'Цена — число, например 1650'; return }
      try {
        this.prices = await api.setClientPrice(this.form.id, this.newPrice.product_id, price)
        this.newPrice = { product_id: '', price: '' }; this.priceQ = ''
        this.$store.crm.notify('Персональная цена сохранена')
      } catch (e) { this.formError = e.message }
    },
    async updatePrice(row, value) {
      const price = moneyInput(value)
      if (price === null) { this.formError = 'Цена — число, например 1650'; return }
      try { this.prices = await api.setClientPrice(this.form.id, row.product_id, price); this.$store.crm.notify('Цена обновлена') } catch (e) { this.formError = e.message }
    },
    tiynToString: v => tiynToString(tiyn(v)).replace(/\.00$/, ''),
    async removePrice(row) {
      try { await api.deleteClientPrice(this.form.id, row.product_id); this.prices = this.prices.filter(p => p.product_id !== row.product_id) } catch (e) { this.formError = e.message }
    },
    get rows() {
      const q = this.q.trim().toLowerCase()
      const qd = phoneDigits(q)
      const list = this.list.filter(c => !q || `${c.name} ${c.city ?? ''}`.toLowerCase().includes(q) || (qd.length >= 4 && phoneDigits(c.phone).includes(qd)))
      const by = {
        sum: (a, b) => tiyn(b.orders_total) - tiyn(a.orders_total),
        due: (a, b) => tiyn(b.debt) - tiyn(a.debt),
        last: (a, b) => (b.last_order_at ?? '').localeCompare(a.last_order_at ?? ''),
      }
      return list.sort(by[this.sort])
    },
  }))

  // ================================================================ Склад: остатки, полки, ниже минимума
  Alpine.data('crmStock', () => ({
    tab: ['low', 'out', 'restock'].includes(params.get('level')) ? params.get('level') : '',
    restock: [],        // заявки «Сообщить о поступлении» (только менеджер и владелец)
    q: '',
    section: '',
    shown: 60,
    receipt: null,      // { productId, qty, location, reason }
    sending: false,
    serverError: null,
    movements: [],
    showMoves: false,

    init() {
      this.$watch('tab', t => {
        const url = new URL(location.href)
        t ? url.searchParams.set('level', t) : url.searchParams.delete('level')
        history.replaceState(null, '', url)
        this.shown = 60
      })
      ;['q', 'section'].forEach(k => this.$watch(k, () => (this.shown = 60)))
      if (this.$store.crm.can('office')) this.loadRestock()
    },
    async loadRestock() {
      try { this.restock = await api.restockList() } catch (e) { console.warn(e) }
    },
    get restockArrived() { return this.restock.some(r => milli(r.available) > 0) },
    async markRestock(r, status) {
      try {
        await api.restockUpdate(r.id, status)
        this.restock = this.restock.filter(x => x.id !== r.id)
        this.$store.crm.notify(status === 'notified' ? `Отмечено: ${r.name} знает о поступлении` : 'Заявка убрана')
      } catch (e) { this.$store.crm.notify(e.message) }
    },
    get kpi() {
      const s = this.$store.crm.stock
      return {
        total: s.length,
        low: s.filter(x => x.low_stock).length,
        out: s.filter(x => milli(x.available_qty) <= 0).length,
        reserved: s.filter(x => milli(x.reserved_qty) > 0).length,
      }
    },
    get list() {
      const crm = this.$store.crm
      const q = this.q.trim().toLowerCase()
      const sec = crm.sections.find(s => s.id === this.section)
      const list = crm.stock.filter(s => {
        if (this.tab === 'low' && !s.low_stock) return false
        if (this.tab === 'out' && milli(s.available_qty) > 0) return false
        if (sec && !sec.items.some(c => c.id === s.category_id)) return false
        if (q && !`${s.product_name} ${s.variant_name ?? ''} ${s.sku ?? ''} ${s.storage_location ?? ''}`.toLowerCase().includes(q)) return false
        return true
      })
      // Для закупки — сначала то, чего не хватает больше всего
      if (this.tab === 'low') list.sort((a, b) => shortage(b) - shortage(a))
      return list
    },
    get page() { return this.list.slice(0, this.shown) },
    shortage,
    level(s) {
      if (milli(s.available_qty) <= 0) return { label: 'Нет в наличии', cls: 'bg-error-soft text-error' }
      if (s.low_stock) return { label: 'Ниже минимума', cls: 'bg-status-confirmed-soft text-status-confirmed' }
      return { label: 'В норме', cls: 'bg-success-soft text-success' }
    },
    name(s) { return this.$store.crm.productName(this.$store.crm.product(s.product_id)) || s.product_name },

    openReceipt(s) {
      this.serverError = null
      this.receipt = { productId: s?.product_id ?? '', qty: s ? Math.max(1, Math.ceil(shortage(s) / 1000)) : 1, location: s?.storage_location ?? '', reason: '', search: '' }
    },
    get receiptChoices() {
      const q = (this.receipt?.search ?? '').trim().toLowerCase()
      return this.$store.crm.stock.filter(s => !q || `${s.product_name} ${s.sku ?? ''}`.toLowerCase().includes(q)).slice(0, 80)
    },
    pickReceipt(id) {
      this.receipt.productId = id
      this.receipt.location = this.$store.crm.stockOf(id)?.storage_location ?? ''
    },
    async saveReceipt() {
      const r = this.receipt
      if (!r.productId || !(Number(r.qty) > 0)) { this.serverError = 'Выберите товар и количество'; return }
      this.sending = true
      this.serverError = null
      try {
        await api.addMovement({
          product_id: r.productId,
          movement_type: 'in',
          quantity: String(Math.floor(Number(r.qty))),
          reason: r.reason.trim() || 'приход товара',
          storage_location: r.location.trim() || undefined,
        })
        await this.$store.crm.refreshStock()
        this.$store.crm.notify(`Приход записан: ${this.$store.crm.stockOf(r.productId)?.product_name}`)
        this.receipt = null
        if (this.showMoves) this.loadMoves()
      } catch (e) {
        this.serverError = e.message
      } finally {
        this.sending = false
      }
    },
    async loadMoves() {
      this.showMoves = true
      try { this.movements = await api.movements() } catch (e) { this.$store.crm.notify(e.message) }
    },
    moveLabel: { in: 'Приход', out: 'Списание', adjustment: 'Корректировка', reserve: 'Резерв', unreserve: 'Снят резерв' },
    exportLow() {
      const rows = [['Товар', 'Артикул', 'Полка', 'Остаток', 'Резерв', 'Доступно', 'Минимум', 'Не хватает до минимума', 'Ед.']]
      this.$store.crm.stock.filter(s => s.low_stock).sort((a, b) => shortage(b) - shortage(a)).forEach(s => rows.push([
        this.name(s), s.sku, s.storage_location, formatQty(s.stock_qty), formatQty(s.reserved_qty), formatQty(s.available_qty),
        formatQty(s.min_stock_qty ?? '0'), formatQty(shortage(s) / 1000), s.unit,
      ]))
      downloadCsv(`zakupka-${new Date().toISOString().slice(0, 10)}.csv`, rows)
    },
  }))

  // ================================================================ Сборка и выдача — GET /api/warehouse/assembly
  Alpine.data('crmPicking', () => ({
    selected: params.get('id') ?? null,
    queue: [],
    shortFor: null,     // id позиции, для которой вводят нехватку
    shortValue: '',
    busy: false,
    loading: true,
    error: null,
    deliveryNames: DELIVERY,

    // Компьютер: очередь слева, заказ справа. Телефон: заказ раскрывается под строкой очереди
    wide: matchMedia('(min-width: 64rem)').matches,
    async init() {
      matchMedia('(min-width: 64rem)').addEventListener('change', e => (this.wide = e.matches))
      await this.load()
      this.$watch('selected', () => { this.error = null; this.shortFor = null })
    },
    choose(id, el) {
      if (!this.wide && this.selected === id) { this.selected = null; return }
      this.selected = id
      if (!this.wide) this.$nextTick(() => el.scrollIntoView({ block: 'start', behavior: 'smooth' }))
    },
    async load() {
      try {
        this.queue = await api.assembly()
        if (this.selected && !this.o) this.selected = null
      } catch (e) {
        this.error = e.message
      } finally {
        this.loading = false
      }
    },
    get o() { return this.queue.find(q => q.id === this.selected) ?? null },
    get items() { return this.o ? byShelf(this.o.items) : [] },
    progress(q) { return `${q.items.filter(i => i.picked).length}/${q.items.length}` },
    get allPicked() { return this.items.length > 0 && this.items.every(i => i.picked) },
    // Ответ сервера — заказ целиком; подменяем позиции и признаки в очереди
    merge(updated) {
      const i = this.queue.findIndex(q => q.id === updated.id)
      if (i >= 0) this.queue.splice(i, 1, { ...this.queue[i], ...updated, total: this.queue[i].total })
      this.$store.crm.putOrder(updated)
    },
    async act(fn, message) {
      this.busy = true
      this.error = null
      try {
        const updated = await fn()
        this.merge(updated)
        if (message) this.$store.crm.notify(message)
        return updated
      } catch (e) {
        this.error = e.message
        return null
      } finally {
        this.busy = false
      }
    },
    pick(i, picked) { return this.act(() => api.pick(this.o.id, i.id, { picked })) },
    openShort(i) { this.shortFor = i.id; this.shortValue = formatQty(i.short_qty) === '0' ? '' : formatQty(i.short_qty).replace(',', '.') },
    async saveShort(i) {
      const v = String(this.shortValue).replace(',', '.').trim()
      if (v && !/^\d+(\.\d{1,3})?$/.test(v)) { this.error = 'Нехватка — число'; return }
      const ok = await this.act(() => api.pick(this.o.id, i.id, { picked: false, short_qty: v || '0' }),
        v && Number(v) > 0 ? `Нехватка отмечена — менеджер поправит состав` : 'Нехватка снята')
      if (ok) this.shortFor = null
    },
    async setStatus(status, message) {
      const updated = await this.act(() => api.updateOrder(this.o.id, { status }), message)
      if (!updated) return
      await Promise.all([this.load(), this.$store.crm.refreshStock()])
      if (status === 'handed_over') this.selected = null
    },
  }))

  // ================================================================ Каталог и цены (владелец)
  Alpine.data('crmCatalog', () => ({
    list: [],
    loading: true,
    error: null,
    q: '',
    category: '',
    archived: false,
    shown: 60,
    edit: null,         // форма товара; id — правка, без id — новый
    saving: false,
    formError: null,
    attrNames: [],

    async init() {
      await this.load()
      api.attributeNames().then(l => (this.attrNames = l.map(a => a.name))).catch(() => {})
      ;['q', 'category', 'archived'].forEach(k => this.$watch(k, () => (this.shown = 60)))
    },
    async load() {
      try { this.list = await api.products({ include_archived: true }); this.error = null } catch (e) { this.error = e.message } finally { this.loading = false }
    },
    get rows() {
      const q = this.q.trim().toLowerCase()
      return this.list.filter(p =>
        (this.archived ? p.is_archived : !p.is_archived) &&
        (!this.category || p.category_id === this.category) &&
        (!q || `${p.name} ${p.variant_name ?? ''} ${p.sku ?? ''}`.toLowerCase().includes(q)))
    },
    get page() { return this.rows.slice(0, this.shown) },
    groups() { return [...new Map(this.list.filter(p => p.group_id).map(p => [p.group_id, p.name])).entries()] },

    // ---------------- Категории (ФТ-К-02/03): добавить, переименовать, удалить пустую.
    // Раздел сайта для категории задан в catalog.js (GROUPS); новая попадает в «Другое», пока её туда не добавят
    cats: false,
    catForm: { name: '' },
    catError: null,
    get catTree() {
      const count = c => c.products_count ?? this.list.filter(p => p.category_id === c.id && !p.is_archived).length
      return buildSections(this.$store.crm.categories).map(sec => ({ ...sec, items: sec.items.map(c => ({ ...c, n: count(c) })) }))
    },
    async reloadCats() {
      const list = await api.categories()
      this.$store.crm.categories = list
      this.$store.crm.sections = buildSections(list)
    },
    async addCat() {
      this.catError = null
      const name = this.catForm.name.trim()
      if (name.length < 2) { this.catError = 'Название — от 2 символов'; return }
      try {
        await api.createCategory({ name })
        await this.reloadCats()
        this.catForm = { name: '' }
        this.$store.crm.notify(`Категория «${name}» добавлена`)
      } catch (e) { this.catError = e.message }
    },
    async renameCat(c, name) {
      this.catError = null
      name = name.trim()
      if (!name || name === c.name) return
      try { await api.updateCategory(c.id, { name }); await this.reloadCats(); this.$store.crm.notify('Категория переименована') } catch (e) { this.catError = e.message }
    },
    async deleteCat(c) {
      this.catError = null
      if (c.n) { this.catError = `В «${c.name}» есть товары — сначала перенесите их в другую категорию`; return }
      try { await api.deleteCategory(c.id); await this.reloadCats(); this.$store.crm.notify(`Категория «${c.name}» удалена`) } catch (e) { this.catError = e.message }
    },

    open(p) {
      this.formError = null
      this.edit = p
        ? { id: p.id, category_id: p.category_id, name: p.name, sku: p.sku ?? '', price: p.price.replace(/\.00$/, ''), unit: p.unit,
            variant_name: p.variant_name ?? '', group_id: p.group_id ?? '', description: p.description ?? '', is_archived: p.is_archived,
            attributes: (p.attributes ?? []).map(a => ({ ...a })) }
        : { category_id: this.category || this.$store.crm.categories[0]?.id, name: '', sku: '', price: '', unit: 'шт', variant_name: '', group_id: '',
            description: '', stock: '', storage_location: '', attributes: [{ name: '', value: '' }] }
    },
    async save() {
      const f = this.edit
      const price = moneyInput(f.price)
      if (!f.name.trim()) { this.formError = 'Укажите название'; return }
      if (price === null) { this.formError = 'Цена — число, например 1880'; return }
      if (!f.id && f.stock && !/^\d+$/.test(String(f.stock).trim())) { this.formError = 'Начальный остаток — целое число'; return }
      const attributes = f.attributes.map(a => ({ name: a.name.trim(), value: a.value.trim() })).filter(a => a.name && a.value)
      const body = {
        category_id: f.category_id, name: f.name.trim(), sku: f.sku.trim() || undefined, price, unit: f.unit.trim() || 'шт',
        variant_name: f.variant_name.trim() || undefined, group_id: f.group_id || undefined, description: f.description.trim() || undefined,
      }
      this.saving = true
      this.formError = null
      try {
        let p
        if (f.id) {
          p = await api.updateProduct(f.id, body)
          p = await api.setAttributes(f.id, attributes)
        } else {
          p = await api.createProduct({ ...body, attributes, stock: String(f.stock || '').trim() || undefined, storage_location: f.storage_location.trim() || undefined })
        }
        const i = this.list.findIndex(x => x.id === p.id)
        i >= 0 ? this.list.splice(i, 1, p) : this.list.unshift(p)
        this.$store.crm.notify(f.id ? `Сохранено: ${p.name}` : `Товар добавлен: ${p.name}`)
        this.edit = null
        this.$store.crm.refreshStock()
      } catch (e) {
        this.formError = e.message
      } finally {
        this.saving = false
      }
    },
    async toggleArchive(p) {
      try {
        const updated = p.is_archived ? await api.restoreProduct(p.id) : await api.archiveProduct(p.id)
        this.list.splice(this.list.findIndex(x => x.id === p.id), 1, updated)
        this.$store.crm.notify(updated.is_archived ? `Снят с продажи: ${p.name}` : `Снова в продаже: ${p.name}`)
        if (this.edit?.id === p.id) this.edit = null
      } catch (e) {
        this.$store.crm.notify(e.message)
      }
    },
  }))
})

window.bl = { retail, groupOfCategory, formatQty, milli }

window.Alpine = Alpine
Alpine.start()
