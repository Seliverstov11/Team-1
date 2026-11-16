// CRM Bakeloom: менеджер заказов, склад (кладовщик) и сводка для владельца.
// Данные — backend API (src/api.js): те же заказы и остатки видит сайт.
//
// Авторизации и ролей на backend пока нет, эндпоинты открыты. Переключатель
// «войти как» и скрытие разделов по роли — только интерфейс, не защита.

import Alpine from 'alpinejs'
import { api } from './api.js'
import { buildSections, groupOfCategory, hasWholesale, lineSum, retail, unitPrice, wholesale, wholesaleMin } from './catalog.js'
import { formatQty, formatTiyn, milli, qtyToString, tiyn, tiynToString } from './money.js'
import { ACTIVE, CHANNELS, DELIVERY, FLOW, NEXT, PAYMENT, PAYMENT_METHODS, STATUSES, STATUS_HINT, STATUS_ORDER, paymentKey } from './orders.js'
import { fmt, formatPhone, isPhone, persisted, phoneDigits, save } from './util.js'

const params = new URLSearchParams(location.search)
const USER_KEY = 'bakeloom.crm.user.v2'

// Роли интерфейса. Менеджеры — из /api/employees, владелец и кладовщик — только на фронте
const OWNER = { id: 'owner', name: 'Владелец', role: 'owner', title: 'Владелец магазина', app: 'CRM · владелец' }
const WAREHOUSE = { id: 'warehouse', name: 'Кладовщик', role: 'warehouse', title: 'Склад', app: 'CRM · склад' }

const NAV = [
  { group: 'Продажи', page: 'home', href: '/crm/', label: 'Сводка', icon: 'home', roles: ['owner', 'manager'] },
  { group: 'Продажи', page: 'orders', href: '/crm/orders.html', label: 'Заказы', icon: 'orders', badge: 'new', roles: ['owner', 'manager'] },
  { group: 'Продажи', page: 'clients', href: '/crm/clients.html', label: 'Клиенты', icon: 'clients', roles: ['owner', 'manager'] },
  { group: 'Склад', page: 'stock', href: '/crm/stock.html', label: 'Остатки и полки', icon: 'stock', badge: 'low', roles: ['owner', 'manager', 'warehouse'] },
  { group: 'Склад', page: 'picking', href: '/crm/picking.html', label: 'Сборка и выдача', icon: 'picking', badge: 'queue', roles: ['owner', 'warehouse'] },
]

const csvCell = v => `"${String(v ?? '').replaceAll('"', '""')}"`
function downloadCsv(name, rows) {
  const blob = new Blob(['﻿' + rows.map(r => r.map(csvCell).join(';')).join('\r\n')], { type: 'text/csv;charset=utf-8' })
  const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: name })
  a.click()
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
}

// Остаток к закупке: сколько не хватает до минимума
const shortage = s => Math.max(0, milli(s.min_stock_qty) - milli(s.available_qty))
const remaining = o => Math.max(0, tiyn(o.total) - tiyn(o.paid))
const isOutstanding = o => o.status !== 'cancelled' && o.payment_status !== 'refunded' && remaining(o) > 0

document.addEventListener('alpine:init', () => {
  Alpine.store('fmt', { ...fmt, phone: formatPhone, money: formatTiyn, qty: formatQty, tiyn })

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

    employees: [],
    userId: persisted(USER_KEY, null),
    orders: [],
    stock: [],
    products: [],
    categories: [],
    sections: [],
    stockById: new Map(),
    productById: new Map(),

    async init() {
      try {
        const [employees, orders, stock, products, categories] = await Promise.all([
          api.employees(), api.allOrders(), api.stock(), api.products(), api.categories(),
        ])
        this.employees = employees
        this.categories = categories
        this.sections = buildSections(categories)
        this.setOrders(orders)
        this.setStock(stock)
        this.products = products
        this.productById = new Map(products.map(p => [p.id, p]))
        // Роль не подходит к странице — открыть первый доступный раздел
        const allowed = NAV.filter(i => i.roles.includes(this.user.role)).map(i => i.page)
        if (this.page && !allowed.includes(this.page) && this.page !== 'order') location.replace(NAV.find(i => i.page === allowed[0]).href)
        if (this.page === 'order' && !['owner', 'manager'].includes(this.user.role)) location.replace('/crm/picking.html')
      } catch (e) {
        console.error(e)
        this.error = `Не удалось загрузить данные: ${e.message}`
      } finally {
        this.ready = true
      }
    },
    setOrders(list) { this.orders = list },
    setStock(list) {
      this.stock = list
      this.stockById = new Map(list.map(s => [s.product_id, s]))
    },
    async refreshOrders() { this.setOrders(await api.allOrders()) },
    async refreshStock() { this.setStock(await api.stock()) },
    // Заказ изменился (PATCH вернул новую версию) — подменить в списке
    putOrder(o) {
      const i = this.orders.findIndex(x => x.id === o.id)
      if (i >= 0) this.orders.splice(i, 1, { ...this.orders[i], ...o })
      else this.orders.unshift(o)
    },

    // ---------------------------------------------------------------- пользователь
    get staffList() {
      const managers = this.employees.filter(e => e.role === 'manager')
        .map(e => ({ ...e, role: 'manager', title: 'Менеджер по заказам', app: 'CRM · менеджер' }))
      return [...managers, OWNER, WAREHOUSE]
    },
    get user() { return this.staffList.find(s => s.id === this.userId) ?? this.staffList[0] ?? OWNER },
    setUser(id) {
      this.userId = id
      save(USER_KEY, id)
      const page = NAV.find(i => i.page === this.page)
      if (page && !page.roles.includes(this.user.role)) location.href = NAV.find(i => i.roles.includes(this.user.role)).href
    },
    get isManager() { return this.user.role === 'manager' },
    employee(id) { return this.employees.find(e => e.id === id) },
    employeeName(id) { return id ? this.employee(id)?.name ?? id : 'не назначен' },

    get nav() {
      const items = NAV.filter(i => i.roles.includes(this.user.role))
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
      return p.variant_name && !p.name.includes(p.variant_name) && this.products.filter(x => x.group_id === p.group_id).length > 1
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

  matchMedia('(min-width: 64rem)').addEventListener('change', e => e.matches && Alpine.store('crm').closeDrawer())

  // ================================================================ Сводка (владелец, менеджер)
  Alpine.data('crmHome', () => ({
    get kpi() {
      const crm = this.$store.crm
      const orders = crm.orders
      const live = orders.filter(o => o.status !== 'cancelled')
      const unpaid = orders.filter(isOutstanding)
      return {
        total: orders.length,
        inWork: orders.filter(o => ACTIVE.includes(o.status)).length,
        newCount: orders.filter(o => o.status === 'new').length,
        unpaidCount: unpaid.length,
        unpaidSum: unpaid.reduce((s, o) => s + remaining(o), 0),
        unpaidShare: live.length ? Math.round((unpaid.length / live.length) * 100) : 0,
        paidSum: orders.reduce((s, o) => s + (o.payment_status === 'paid' || o.payment_status === 'unpaid' ? tiyn(o.paid) : 0), 0),
        revenue: live.reduce((s, o) => s + tiyn(o.total), 0),
        low: crm.stock.filter(s => s.low_stock).length,
        out: crm.stock.filter(s => milli(s.available_qty) <= 0).length,
      }
    },
    get unpaid() {
      return this.$store.crm.orders.filter(isOutstanding).sort((a, b) => remaining(b) - remaining(a)).slice(0, 8)
    },
    get fresh() { return this.$store.crm.orders.filter(o => o.status === 'new').slice(0, 6) },
    get byStatus() {
      const orders = this.$store.crm.orders
      return STATUS_ORDER.map(s => ({ s, n: orders.filter(o => o.status === s).length }))
    },
    get byChannel() {
      const orders = this.$store.crm.orders
      const max = Math.max(1, ...Object.keys(CHANNELS).map(c => orders.filter(o => o.channel === c).length))
      return Object.entries(CHANNELS).map(([id, label]) => {
        const n = orders.filter(o => o.channel === id).length
        return { id, label, n, pct: Math.round((n / max) * 100) }
      }).filter(c => c.n)
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
      ;['q', 'channel', 'manager'].forEach(k => this.$watch(k, () => { this.page = 1; this.syncUrl() }))
      // Заказ поменялся в карточке или создан здесь — список обновить
      this.$watch('$store.crm.orders', () => this.fetch())
    },
    syncUrl() {
      const url = new URL(location.href)
      const set = (k, v) => (v ? url.searchParams.set(k, v) : url.searchParams.delete(k))
      set('status', this.status); set('pay', this.pay); set('q', this.q.trim()); set('channel', this.channel)
      url.searchParams.delete('new')
      history.replaceState(null, '', url)
    },
    // Фильтры по статусу и оплате — на backend (?status=&payment_status=). «Частично» — это unpaid с суммой
    async fetch() {
      this.loading = true
      try {
        const payment_status = this.pay === 'partial' ? 'unpaid' : this.pay
        this.list = await api.allOrders({ status: this.status, payment_status })
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
        if (this.pay === 'partial' && paymentKey(o) !== 'partial') return false
        if (this.channel && o.channel !== this.channel) return false
        if (this.manager && o.assigned_to !== this.manager) return false
        if (q) {
          const hay = `${o.order_no} ${o.client_name} ${o.city ?? ''} ${o.delivery_address ?? ''}`.toLowerCase()
          if (!hay.includes(q) && !(qd.length >= 4 && phoneDigits(o.client_phone).includes(qd))) return false
        }
        return true
      })
    },
    get pages() { return Math.max(1, Math.ceil(this.filtered.length / this.perPage)) },
    get rows() { return this.filtered.slice((this.page - 1) * this.perPage, this.page * this.perPage) },
    get hasFilters() { return Boolean(this.status || this.pay || this.q || this.channel || this.manager) },
    reset() { Object.assign(this, { status: '', pay: '', q: '', channel: '', manager: '' }) },

    get kpi() {
      const orders = this.$store.crm.orders
      const unpaid = orders.filter(isOutstanding)
      return {
        inWork: orders.filter(o => ACTIVE.includes(o.status)).length,
        waiting: orders.filter(o => o.status === 'new').length,
        unpaid: unpaid.length,
        unpaidSum: unpaid.reduce((s, o) => s + remaining(o), 0),
        toShip: orders.filter(o => o.status === 'assembled').length,
      }
    },
    exportCsv() {
      const crm = this.$store.crm
      const rows = [['№', 'Дата', 'Клиент', 'Телефон', 'Канал', 'Город', 'Статус', 'Товары', 'Доставка', 'Итого', 'Оплачено', 'Оплата', 'Менеджер']]
      this.filtered.forEach(o => rows.push([
        o.order_no, fmt.dateTime(o.created_at), o.client_name, o.client_phone, CHANNELS[o.channel], o.city, STATUSES[o.status].label,
        o.items_total.replace('.', ','), o.delivery.replace('.', ','), o.total.replace('.', ','), o.paid.replace('.', ','),
        PAYMENT[paymentKey(o)].label, crm.employeeName(o.assigned_to),
      ]))
      downloadCsv(`zakazy-${new Date().toISOString().slice(0, 10)}.csv`, rows)
    },
  }))

  // ---------------------------------------------------------------- новый заказ (звонок, WhatsApp, Instagram)
  Alpine.data('newOrder', () => ({
    form: { phone: '', name: '', channel: 'whatsapp', city: 'Астана', delivery_method: 'pickup', address: '', delivery: '', payment_method: 'none', note: '' },
    lines: [],      // { productId, qty }
    add: '',
    search: '',
    errors: {},
    sending: false,
    serverError: null,
    channelOptions: Object.entries(CHANNELS),
    get known() {
      const d = phoneDigits(this.form.phone)
      return d.length === 11 ? this.$store.crm.orders.find(o => phoneDigits(o.client_phone) === d) : null
    },
    fillKnown() { if (this.known && !this.form.name) this.form.name = this.known.client_name },
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
    price(l) { return unitPrice(this.$store.crm.product(l.productId), Number(l.qty) || 0) },
    sum(l) { return lineSum(this.$store.crm.product(l.productId), Math.max(0, Number(l.qty) || 0)) },
    isWholesale(l) { const p = this.$store.crm.product(l.productId); return hasWholesale(p) && l.qty >= wholesaleMin(p) },
    get items() { return this.lines.reduce((s, l) => s + this.sum(l), 0) },
    get deliveryTiyn() { return this.form.delivery_method === 'pickup' ? 0 : tiyn(String(this.form.delivery).replace(',', '.').replace(/\s/g, '')) },
    get total() { return this.items + this.deliveryTiyn },
    validate() {
      const e = {}
      if (this.form.name.trim().length < 2) e.name = 'Укажите клиента'
      if (this.form.phone && !isPhone(this.form.phone)) e.phone = 'Номер в формате +7 (7XX) XXX-XX-XX'
      if (!this.lines.length) e.lines = 'Добавьте хотя бы один товар'
      if (this.form.delivery && !/^\d+([.,]\d{1,2})?$/.test(String(this.form.delivery).replace(/\s/g, ''))) e.delivery = 'Сумма, например 1200'
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
          client_phone: this.form.phone ? formatPhone(this.form.phone) : null,
          channel: this.form.channel,
          city: this.form.city.trim() || null,
          delivery_method: this.form.delivery_method,
          delivery_address: this.form.delivery_method === 'pickup' ? null : this.form.address.trim(),
          delivery: this.form.delivery_method === 'pickup' ? null : tiynToString(this.deliveryTiyn),
          payment_method: this.form.payment_method,
          assigned_to: crm.isManager ? crm.user.id : null,
          note: this.form.note.trim() || null,
          items: this.lines.map(l => ({ product_id: l.productId, quantity: qtyToString(l.qty) })),
        })
        crm.notify(`Заказ №${o.order_no} создан`)
        await Promise.all([crm.refreshOrders(), crm.refreshStock()])
        location.href = `/crm/order.html?id=${o.id}`
      } catch (e) {
        this.serverError = e.message
      } finally {
        this.sending = false
      }
    },
  }))

  // ================================================================ Карточка заказа
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
    get options() { return this.o ? NEXT[this.o.status] : [] },
    get step() { return FLOW.indexOf(this.o.status) },
    get closed() { return ['cancelled', 'completed'].includes(this.o.status) },
    get paidT() { return tiyn(this.o.paid) },
    get totalT() { return tiyn(this.o.total) },
    get due() { return Math.max(0, this.totalT - this.paidT) },
    get otherOrders() {
      const d = phoneDigits(this.o.client_phone)
      if (!d) return []
      return this.$store.crm.orders.filter(x => x.id !== this.o.id && phoneDigits(x.client_phone) === d)
    },
    free(i) { return this.$store.crm.stockOf(i.product_id) },

    async patch(body, message) {
      this.busy = true
      this.error = null
      try {
        const updated = await api.updateOrder(this.o.id, body)
        Object.assign(this.o, updated)
        this.$store.crm.putOrder(updated)
        if (body.status) this.$store.crm.refreshStock()
        this.$store.crm.notify(message)
        return true
      } catch (e) {
        this.error = e.message
        return false
      } finally {
        this.busy = false
      }
    },

    openStatus() {
      this.next = this.options[0] ?? ''
      this.reason = ''
      this.refundOk = false
      this.modal = true
    },
    get needsRefund() { return this.next === 'cancelled' && this.paidT > 0 },
    async saveStatus() {
      if (!this.next) return
      const body = { status: this.next }
      if (this.next === 'cancelled') {
        body.cancelled_reason = this.reason.trim() || null
        // Отмена оплаченного — только с возвратом, одним запросом
        if (this.paidT > 0) {
          if (!this.refundOk) return
          body.payment_status = 'refunded'
        }
      }
      if (this.$store.crm.isManager && !this.o.assigned_to) body.assigned_to = this.$store.crm.user.id
      if (await this.patch(body, `Заказ №${this.o.order_no}: ${STATUSES[this.next].label}`)) this.modal = false
    },

    // ---- оплата: только вручную, система о платеже не узнаёт
    openPay() {
      this.payForm = { method: this.o.payment_method !== 'none' && this.o.payment_method !== 'online' ? this.o.payment_method : 'kaspi_transfer', kind: 'full', amount: '' }
      this.payModal = true
    },
    get payAmount() { return tiyn(String(this.payForm.amount).replace(',', '.').replace(/\s/g, '')) },
    get payValid() { return this.payForm.kind === 'full' || (this.payAmount > 0 && this.payAmount < this.totalT) },
    async savePay() {
      if (!this.payValid) return
      const body = this.payForm.kind === 'full'
        ? { payment_status: 'paid', payment_method: this.payForm.method }
        : { payment_status: 'unpaid', paid: tiynToString(this.payAmount), payment_method: this.payForm.method }
      const msg = this.payForm.kind === 'full' ? `Заказ №${this.o.order_no} отмечен оплаченным` : `Предоплата ${formatTiyn(this.payAmount)} записана`
      if (await this.patch(body, msg)) this.payModal = false
    },
    async unmarkPay() { await this.patch({ payment_status: 'unpaid', paid: '0' }, 'Отметка об оплате снята') },
    async refund() { await this.patch({ payment_status: 'refunded' }, 'Возврат отмечен') },

    // ---- доставка: итог = товары + доставка
    editDelivery() { this.deliveryValue = this.o.delivery.replace(/\.00$/, ''); this.deliveryEdit = true },
    async saveDelivery() {
      const v = String(this.deliveryValue).replace(/\s/g, '').replace(',', '.')
      if (!/^\d+(\.\d{1,2})?$/.test(v)) { this.error = 'Стоимость доставки — число, например 1200'; return }
      if (await this.patch({ delivery: v }, 'Стоимость доставки сохранена')) this.deliveryEdit = false
    },
    async assign(id) { await this.patch({ assigned_to: id }, `Менеджер: ${this.$store.crm.employeeName(id)}`) },
  }))

  // ================================================================ Клиенты — из заказов (отдельного API клиентов нет)
  Alpine.data('crmClients', () => ({
    q: '',
    sort: 'sum',
    get rows() {
      const map = new Map()
      this.$store.crm.orders.forEach(o => {
        const key = phoneDigits(o.client_phone) || o.client_id || o.client_name
        if (!map.has(key)) map.set(key, { key, name: o.client_name, phone: o.client_phone, city: o.city, orders: [], channels: new Set() })
        const c = map.get(key)
        c.orders.push(o)
        c.channels.add(o.channel)
        if (!c.city && o.city) c.city = o.city
      })
      const q = this.q.trim().toLowerCase()
      const list = [...map.values()].map(c => {
        const live = c.orders.filter(o => o.status !== 'cancelled')
        return {
          ...c,
          count: live.length,
          sum: live.reduce((s, o) => s + tiyn(o.total), 0),
          due: c.orders.filter(isOutstanding).reduce((s, o) => s + remaining(o), 0),
          last: c.orders.reduce((a, b) => (a.created_at > b.created_at ? a : b)),
          channels: [...c.channels],
        }
      }).filter(c => !q || `${c.name} ${c.phone ?? ''} ${c.city ?? ''}`.toLowerCase().includes(q))
      const by = { sum: (a, b) => b.sum - a.sum, due: (a, b) => b.due - a.due, last: (a, b) => b.last.created_at.localeCompare(a.last.created_at) }
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
        const p = crm.product(s.product_id)
        if (sec && !sec.items.some(c => c.id === p?.category_id)) return false
        if (q && !`${s.product_name} ${p?.variant_name ?? ''} ${s.sku ?? ''} ${s.storage_location ?? ''}`.toLowerCase().includes(q)) return false
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
          storage_location: r.location.trim() || null,
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
      const crm = this.$store.crm
      const rows = [['Товар', 'Артикул', 'Полка', 'Остаток', 'Резерв', 'Доступно', 'Минимум', 'Не хватает до минимума', 'Ед.']]
      crm.stock.filter(s => s.low_stock).sort((a, b) => shortage(b) - shortage(a)).forEach(s => rows.push([
        this.name(s), s.sku, s.storage_location, formatQty(s.stock_qty), formatQty(s.reserved_qty), formatQty(s.available_qty),
        formatQty(s.min_stock_qty ?? '0'), formatQty(shortage(s) / 1000), s.unit,
      ]))
      downloadCsv(`zakupka-${new Date().toISOString().slice(0, 10)}.csv`, rows)
    },
  }))

  // ================================================================ Сборка и выдача (кладовщик)
  Alpine.data('crmPicking', () => ({
    selected: params.get('id') ?? null,
    o: null,
    checked: {},
    busy: false,
    error: null,
    deliveryNames: DELIVERY,

    init() {
      this.$watch('selected', id => this.load(id))
      if (this.selected) this.load(this.selected)
    },
    get queue() {
      return this.$store.crm.orders
        .filter(o => o.status === 'confirmed' || o.status === 'assembled')
        .sort((a, b) => (a.status === 'confirmed') - (b.status === 'confirmed') || a.created_at.localeCompare(b.created_at))
        .reverse()
    },
    async load(id) {
      this.o = null
      this.checked = {}
      this.error = null
      if (!id) return
      try { this.o = await api.order(id) } catch (e) { this.error = e.message }
    },
    // Маршрут по складу: по полкам
    get items() {
      if (!this.o) return []
      const crm = this.$store.crm
      return [...this.o.items].sort((a, b) => crm.shelf(a.product_id).localeCompare(crm.shelf(b.product_id), 'ru', { numeric: true }))
    },
    get allChecked() { return this.items.length > 0 && this.items.every(i => this.checked[i.id]) },
    async setStatus(status, message) {
      this.busy = true
      this.error = null
      try {
        const updated = await api.updateOrder(this.o.id, { status })
        Object.assign(this.o, updated)
        this.$store.crm.putOrder(updated)
        await this.$store.crm.refreshStock()
        this.$store.crm.notify(message)
        if (status === 'handed_over') this.selected = null
      } catch (e) {
        this.error = e.message
      } finally {
        this.busy = false
      }
    },
  }))
})

window.bl = { retail, wholesale, hasWholesale, wholesaleMin, groupOfCategory, formatQty, milli }

window.Alpine = Alpine
Alpine.start()
