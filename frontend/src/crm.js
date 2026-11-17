// CRM Bakeloom: владелец, менеджеры заказов и кладовщик.
// Данные — backend API (src/api.js): те же заказы и остатки видит сайт.
//
// Вход — POST /api/auth/login, дальше каждый запрос с токеном. Права проверяет сервер:
// меню по роли только убирает лишнее, а запрос без прав всё равно получит 403.
// Кладовщику сервер не отдаёт поля оплаты — интерфейс их просто не показывает.

import Alpine from 'alpinejs'
import { api, onUnauthorized, session } from './api.js'
import { buildSections, groupOfCategory, lineSum, retail } from './catalog.js'
import { formatQty, formatTiyn, milli, qtyToString, tiyn, tiynToString } from './money.js'
import {
  ACTIVE, CHANNELS, DELIVERY, EDITABLE, FLOW, NEXT, PAYMENT, PAYMENT_METHODS, STATUSES, STATUS_HINT, STATUS_ORDER,
  paymentKey,
} from './orders.js'
import { fmt, formatPhone, isPhone, persisted, phoneDigits, save } from './util.js'

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
const standalone = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true

// Роли сервера: admin — владелец, manager — менеджер, warehouse — кладовщик
const ROLES = {
  admin: { title: 'Владелец магазина', app: 'CRM · владелец' },
  manager: { title: 'Менеджер по заказам', app: 'CRM · менеджер' },
  warehouse: { title: 'Склад', app: 'CRM · склад' },
}
const OFFICE = ['admin', 'manager']

const NAV = [
  { group: 'Продажи', page: 'home', href: '/crm/', label: 'Сводка', icon: 'home', roles: OFFICE },
  { group: 'Продажи', page: 'orders', href: '/crm/orders.html', label: 'Заказы', icon: 'orders', badge: 'new', roles: OFFICE },
  { group: 'Продажи', page: 'clients', href: '/crm/clients.html', label: 'Клиенты', icon: 'clients', roles: OFFICE },
  { group: 'Склад', page: 'stock', href: '/crm/stock.html', label: 'Остатки и полки', icon: 'stock', badge: 'low', roles: ['admin', 'manager', 'warehouse'] },
  { group: 'Склад', page: 'picking', href: '/crm/picking.html', label: 'Сборка и выдача', icon: 'picking', badge: 'queue', roles: ['admin', 'warehouse'] },
  { group: 'Магазин', page: 'catalog', href: '/crm/catalog.html', label: 'Каталог и цены', icon: 'catalog', roles: ['admin'] },
]
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
// Своё фото: сервер принимает JPG/PNG/WebP до 10 МБ. Снимок с телефона бывает больше
// или в HEIC — тогда пережимаем в JPEG до 2400 px по длинной стороне (если браузер умеет его открыть)
const PHOTO_TYPES = ['image/jpeg', 'image/png', 'image/webp']
const PHOTO_MAX = 10 * 1024 * 1024
async function photoForUpload(file) {
  if (!file.type.startsWith('image/') && !/\.(heic|heif)$/i.test(file.name)) throw new Error('Это не изображение — выберите фото')
  if (PHOTO_TYPES.includes(file.type) && file.size <= PHOTO_MAX) return file
  let bmp
  try { bmp = await createImageBitmap(file) } catch {
    throw new Error('Браузер не смог открыть этот файл — сохраните фото как JPG или PNG')
  }
  const k = Math.min(1, 2400 / Math.max(bmp.width, bmp.height))
  const canvas = document.createElement('canvas')
  canvas.width = Math.round(bmp.width * k)
  canvas.height = Math.round(bmp.height * k)
  canvas.getContext('2d').drawImage(bmp, 0, 0, canvas.width, canvas.height)
  bmp.close?.()
  const blob = await new Promise(r => canvas.toBlob(r, 'image/jpeg', 0.9))
  if (!blob || blob.size > PHOTO_MAX) throw new Error('Фото слишком большое — выберите другое')
  return new File([blob], file.name.replace(/\.[^.]+$/, '') + '.jpg', { type: 'image/jpeg' })
}

const moneyInput = v => {
  const s = String(v ?? '').replace(/\s/g, '').replace(',', '.')
  return /^\d+(\.\d{1,2})?$/.test(s) ? s : null
}

// Остаток к закупке: сколько не хватает до минимума
const shortage = s => Math.max(0, milli(s.min_stock_qty) - milli(s.available_qty))
// К оплате считает сервер (due). Кладовщику поля нет — тогда и долга не видно
const remaining = o => (o?.due == null ? 0 : Math.max(0, tiyn(o.due)))
const isOutstanding = o => o.status !== 'cancelled' && o.payment_status !== 'refunded' && remaining(o) > 0
const byShelf = items => [...items].sort((a, b) => (a.storage_location ?? '').localeCompare(b.storage_location ?? '', 'ru', { numeric: true }))

document.addEventListener('alpine:init', () => {
  Alpine.store('fmt', { ...fmt, phone: formatPhone, money: formatTiyn, qty: formatQty, tiyn })

  // «Установить приложение»: кнопка видна, пока браузер готов установить и CRM открыта не из значка
  Alpine.store('pwa', {
    available: Boolean(installEvent),
    installed: standalone(),
    get canInstall() { return this.available && !this.installed },
    async install() {
      if (!installEvent) return
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
    periods: [['today', 'Сегодня'], ['week', 'Неделя'], ['month', 'Месяц'], ['all', 'Всё время']],
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
    types: { home_baker: 'Домашний кондитер', bakery: 'Кондитерская', cafe: 'Кафе', shop: 'Магазин', other: 'Другое' },
    async init() {
      await this.load()
      this.$watch('debtors', () => this.load())
    },
    async load() {
      this.loading = true
      try { this.list = await api.clients({ debtors: this.debtors || undefined }); this.error = null } catch (e) { this.error = e.message } finally { this.loading = false }
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
    tab: params.get('level') === 'low' ? 'low' : params.get('level') === 'out' ? 'out' : '',
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

    async init() {
      await this.load()
      this.$watch('selected', () => { this.error = null; this.shortFor = null })
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
    // Фото (#44): pending — выбранное, но ещё не отправленное (у нового товара уходит после создания)
    photo: { open: false, q: '', list: [], loading: false, error: null, pending: null, sending: false, sendError: null },

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

    open(p) {
      this.formError = null
      this.edit = p
        ? { id: p.id, category_id: p.category_id, name: p.name, sku: p.sku ?? '', price: p.price.replace(/\.00$/, ''), unit: p.unit,
            variant_name: p.variant_name ?? '', group_id: p.group_id ?? '', description: p.description ?? '', is_archived: p.is_archived,
            image_url: p.image_url ?? null, attributes: (p.attributes ?? []).map(a => ({ ...a })) }
        : { category_id: this.category || this.$store.crm.categories[0]?.id, name: '', sku: '', price: '', unit: 'шт', variant_name: '', group_id: '',
            description: '', stock: '', storage_location: '', image_url: null, attributes: [{ name: '', value: '' }] }
      this.photoReset()
    },

    // ---------------------------------------------------------------- фото товара
    photoReset() {
      if (this.photo.pending?.preview?.startsWith('blob:')) URL.revokeObjectURL(this.photo.pending.preview)
      this.photo = { open: false, q: '', list: [], loading: false, error: null, pending: null, sending: false, sendError: null }
    },
    photoError(e, what) {
      if (e.status === 503) return 'Подбор и загрузка фото пока не настроены на сервере. Товар сохраняется без фото — фото можно добавить позже.'
      if (e.status === 0) return `Нет связи с сервером — ${what}. Проверьте интернет и попробуйте ещё раз.`
      if (e.status === 413) return 'Файл больше 10 МБ — выберите фото поменьше.'
      if (e.status === 415) return 'Этот формат не подходит — нужен JPG, PNG или WebP.'
      return `${what[0].toUpperCase()}${what.slice(1)}: ${e.message}`
    },
    async photoSuggest() {
      const ph = this.photo
      if (!ph.open) { ph.open = true; ph.q = ph.q || this.edit.name.trim() }
      if (!ph.q.trim()) { ph.error = 'Введите название товара или запрос'; return }
      ph.loading = true
      ph.error = null
      try {
        const r = await api.photoSuggestions(ph.q.trim())
        ph.list = (Array.isArray(r) ? r : r?.photos ?? []).slice(0, 6)
        if (!ph.list.length) ph.error = 'Ничего не нашлось — попробуйте другой запрос, например по-английски'
      } catch (e) {
        ph.list = []
        ph.error = this.photoError(e, 'варианты не загрузились')
      } finally {
        ph.loading = false
      }
    },
    photoPick(s) {
      this.photoSet({ pexels_id: s.id, preview: s.thumb ?? s.src, alt: s.alt })
      this.photo.open = false
    },
    async photoFile(ev) {
      const file = ev.target.files?.[0]
      ev.target.value = ''           // тот же файл можно выбрать ещё раз
      if (!file) return
      this.photo.sendError = null
      try {
        const ready = await photoForUpload(file)
        this.photoSet({ file: ready, preview: URL.createObjectURL(ready), alt: file.name })
      } catch (e) {
        this.photo.sendError = e.message
      }
    },
    photoSet(pending) {
      if (this.photo.pending?.preview?.startsWith('blob:')) URL.revokeObjectURL(this.photo.pending.preview)
      this.photo.pending = pending
      this.photo.sendError = null
      // У существующего товара отправляем сразу; новый сначала надо создать — фото уйдёт после «Сохранить»
      if (this.edit.id) this.photoSend(this.edit.id)
    },
    // true — фото на месте или нечего отправлять; false — ошибка (текст в photo.sendError)
    async photoSend(pid) {
      const ph = this.photo
      if (!ph.pending) return true
      const form = new FormData()
      if (ph.pending.file) form.append('file', ph.pending.file, ph.pending.file.name)
      else form.append('pexels_id', String(ph.pending.pexels_id))
      ph.sending = true
      ph.sendError = null
      try {
        const p = await api.uploadPhoto(pid, form)
        const i = this.list.findIndex(x => x.id === pid)
        if (p?.id) { if (i >= 0) this.list.splice(i, 1, p) }
        else if (i >= 0 && p?.image_url) this.list[i].image_url = p.image_url
        if (this.edit?.id === pid) this.edit.image_url = p?.image_url ?? this.edit.image_url
        if (ph.pending.preview?.startsWith('blob:')) URL.revokeObjectURL(ph.pending.preview)
        ph.pending = null
        return true
      } catch (e) {
        ph.sendError = this.photoError(e, 'фото не отправлено')
        return false
      } finally {
        ph.sending = false
      }
    },
    async save() {
      const f = this.edit
      const isNew = !f.id
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
        this.$store.crm.refreshStock()
        // Фото нового товара — после создания. Не вышло — товар уже сохранён: форма остаётся открытой
        // в режиме правки, чтобы повторить отправку или закрыть без фото
        if (isNew && this.photo.pending) {
          f.id = p.id
          if (!(await this.photoSend(p.id))) {
            this.$store.crm.notify(`Товар добавлен без фото: ${p.name}`)
            return
          }
        }
        this.$store.crm.notify(isNew ? `Товар добавлен: ${p.name}` : `Сохранено: ${p.name}`)
        this.edit = null
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

window.bl = { retail, groupOfCategory, groupMedia: id => groupOfCategory(id).media, formatQty, milli }

window.Alpine = Alpine
Alpine.start()
