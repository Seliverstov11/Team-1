import Alpine from 'alpinejs'
import { api, onUnauthorized, session } from './api.js'
import {
  available, availableUnits, basePrice, buildCards, buildSections, defaultVariant, deliveryCost, groupById, groupOfCategory,
  inStock, isPersonal, lineSum, retail, variantLabel,
} from './catalog.js'
import { formatQty, formatTiyn, milli, qtyToString, tiyn, tiynToString } from './money.js'
import { DELIVERY, FLOW, PAYMENT, STATUSES, paymentKey } from './orders.js'
import { fmt, formatPhone, isPhone, persisted, save } from './util.js'
import { passwordProblems, passwordStrength } from './password.js'

const params = new URLSearchParams(location.search)

const fromSession = key => {
  try { return JSON.parse(sessionStorage.getItem(key)) } catch { return null }
}

const CART_KEY = 'bakeloom.cart.v2'      // { id фасовки: количество }
const DATA_KEY = 'bakeloom.catalog.v2'   // кэш каталога на время сессии
const MENU_KEY = 'bakeloom.menu'
const FAV_KEY = 'bakeloom.favorites.v2'  // { id карточки (group_id): true }
const ME_KEY = 'bakeloom.me.v2'          // профиль из /api/me + адрес доставки по умолчанию (только в браузере)
const LOCK_KEY = 'bakeloom.login.lock'   // { fails, until } — пауза после 5 неудачных входов

// Старые ключи демо-версий больше не нужны: данные — в backend, вход — по паролю
;['bakeloom.cart', 'bakeloom.favorites', 'bakeloom.db.v1', 'bakeloom.session', 'bakeloom.me'].forEach(k => { try { localStorage.removeItem(k) } catch { /* приватный режим */ } })

// ---------------------------------------------------------------- хранилища
document.addEventListener('alpine:init', () => {
  Alpine.store('ui', {
    page: document.body.dataset.page ?? '',
    section: params.get('section') ?? '',
    category: params.get('category') ?? '',
    catalogOpen: sessionStorage.getItem(MENU_KEY) !== '0',
    drawer: false,
    openDrawer() {
      this.drawer = true
      document.documentElement.classList.add('overflow-hidden')
      requestAnimationFrame(() => document.querySelector('#site-menu [data-close]')?.focus({ focusVisible: false }))
    },
    closeDrawer() {
      if (!this.drawer) return
      this.drawer = false
      document.documentElement.classList.remove('overflow-hidden')
    },
    toggleCatalog() {
      this.catalogOpen = !this.catalogOpen
      sessionStorage.setItem(MENU_KEY, this.catalogOpen ? '1' : '0')
    },
    // Что подсвечено в меню: в каталоге — выбранное, на карточке — раздел и категория товара
    get active() {
      if (this.page === 'catalog') {
        const shop = Alpine.store('shop')
        const cat = shop.categoryBySlug(this.category)
        return { section: cat ? groupOfCategory(cat.id).id : this.section, category: this.category }
      }
      if (this.page === 'product') {
        const shop = Alpine.store('shop')
        const p = shop.product(params.get('id'))
        if (!p) return { section: null, category: null }
        return { section: groupOfCategory(p.category_id).id, category: shop.category(p.category_id).slug }
      }
      return { section: null, category: null }
    },
  })

  Alpine.store('fmt', { ...fmt, phone: formatPhone, money: formatTiyn, qty: formatQty, tiyn })

  matchMedia('(min-width: 64rem)').addEventListener('change', e => e.matches && Alpine.store('ui').closeDrawer())

  // Каталог из API. Кэшируется в sessionStorage: при переходе между страницами
  // показывается сразу, свежие остатки подгружаются в фоне.
  // Кэш привязан к покупателю: у вошедшего могут быть свои цены
  const viewer = () => session.get()?.user?.id ?? 'guest'
  const cached = fromSession(DATA_KEY)?.viewer === viewer() ? fromSession(DATA_KEY) : null
  const index = (categories, products) => ({
    categories,
    products,
    sections: buildSections(categories),
    cards: buildCards(products),
    byId: new Map(products.map(p => [p.id, p])),
  })
  Alpine.store('shop', {
    ...(cached ? index(cached.categories, cached.products) : { categories: [], products: [], sections: [], cards: [], byId: new Map() }),
    ready: Boolean(cached),
    error: null,
    async load() {
      try {
        const [categories, products] = await Promise.all([api.categories(), api.products()])
        Object.assign(this, index(categories, products))
        this.error = null
        try { sessionStorage.setItem(DATA_KEY, JSON.stringify({ viewer: viewer(), categories, products })) } catch { /* переполнение — не страшно */ }
        Alpine.store('cart').reconcile()
      } catch (e) {
        console.error(e)
        if (!this.products.length) this.error = `Не удалось загрузить каталог: ${e.message}`
      } finally {
        this.ready = true
      }
    },
    // Обновить фасовки одной карточки (страница товара — свежие остатки)
    replaceGroup(list) {
      if (!list.length) return
      const ids = new Set(list.map(p => p.id))
      const products = [...this.products.filter(p => !ids.has(p.id)), ...list]
      Object.assign(this, index(this.categories, products))
    },
    product(id) { return this.byId.get(id) },
    card(id) { return this.cards.find(c => c.id === id) },
    cardOf(p) { return p ? this.card(p.group_id || p.id) : null },
    category(id) { return this.categories.find(c => c.id === id) ?? {} },
    categoryBySlug(slug) { return this.categories.find(c => c.slug === slug) },
    section(id) { return this.sections.find(s => s.id === id) },
    sectionOf(categoryId) { return groupOfCategory(categoryId) },
    cardsIn(categoryIds) { return this.cards.filter(c => categoryIds.includes(c.category_id)) },
    countIn(categoryIds) { return this.cardsIn(categoryIds).length },
  })

  // Корзина: { id фасовки: количество }, живёт в localStorage.
  // Цена строки — общая или персональная вошедшего покупателя; итог всё равно считает сервер.
  Alpine.store('cart', {
    items: persisted(CART_KEY, {}),
    adjusted: [],
    persist() { save(CART_KEY, this.items) },
    qty(id) { return this.items[id] ?? 0 },
    left(p) { return availableUnits(p) - this.qty(p.id) },
    add(p, n = 1) {
      const add = Math.min(n, this.left(p))
      if (add < 1 || !inStock(p)) return
      this.items[p.id] = this.qty(p.id) + add
      this.persist()
    },
    set(p, n) {
      const max = Math.max(availableUnits(p), 1)
      this.items[p.id] = Math.max(1, Math.min(Math.floor(Number(n) || 1), max))
      this.persist()
    },
    remove(id) { delete this.items[id]; this.persist() },
    clear() { this.items = {}; this.adjusted = []; this.persist() },
    // Остатки изменились — корзина не должна обещать больше, чем есть
    reconcile() {
      const shop = Alpine.store('shop')
      let changed = false
      Object.entries(this.items).forEach(([id, qty]) => {
        const p = shop.product(id)
        if (!p) return
        const s = availableUnits(p)
        if (s > 0 && qty > s) {
          this.items[id] = s
          const name = fullName(p)
          if (!this.adjusted.includes(name)) this.adjusted.push(name)
          changed = true
        }
      })
      if (changed) this.persist()
    },
    get lines() {
      const shop = Alpine.store('shop')
      return Object.entries(this.items)
        .map(([id, qty]) => ({ product: shop.product(id), qty }))
        .filter(l => l.product)
        .map(l => ({
          ...l,
          available: inStock(l.product),
          price: retail(l.product),
          sum: lineSum(l.product, l.qty),
        }))
    },
    get okLines() { return this.lines.filter(l => l.available) },
    get blocked() { return this.lines.some(l => !l.available) },
    get count() { return this.okLines.reduce((s, l) => s + l.qty, 0) },
    get total() { return this.okLines.reduce((s, l) => s + l.sum, 0) }, // тиыны
  })

  // Избранное — по карточке (группе фасовок)
  Alpine.store('fav', {
    ids: persisted(FAV_KEY, {}),
    has(id) { return Boolean(this.ids[id]) },
    toggle(id) {
      if (this.ids[id]) delete this.ids[id]
      else this.ids[id] = true
      save(FAV_KEY, this.ids)
    },
    get count() { return Object.keys(this.ids).length },
  })

  // Покупатель: вход по телефону и паролю (/api/auth/client/login), профиль — /api/me.
  // Адрес доставки по умолчанию сервер не хранит — он остаётся в браузере.
  const storedMe = () => (session.get()?.kind === 'client' ? persisted(ME_KEY, null) : null)
  Alpine.store('account', {
    me: storedMe(),
    async refresh() {
      if (session.get()?.kind !== 'client') { this.forget(); return }
      try {
        const profile = await api.me()
        this.me = { ...profile, phone: formatPhone(profile.phone), address: this.me?.address ?? '' }
        save(ME_KEY, this.me)
      } catch (e) {
        if (e.status === 401) this.forget()
      }
    },
    async signedIn() {
      try { sessionStorage.removeItem(DATA_KEY) } catch { /* приватный режим */ }
      await this.refresh()
      Alpine.store('shop').load() // персональные цены
    },
    async login(phone, password) {
      await api.clientLogin(formatPhone(phone), password)
      await this.signedIn()
    },
    async register(body) {
      await api.clientRegister(body)
      await this.signedIn()
    },
    async update(patch) {
      const { address, ...server } = patch
      if (Object.keys(server).length) Object.assign(this.me, await api.updateMe(server), { phone: this.me.phone })
      if (address !== undefined) this.me.address = address
      save(ME_KEY, this.me)
    },
    forget() {
      this.me = null
      try { localStorage.removeItem(ME_KEY) } catch { /* приватный режим */ }
    },
    async logout() {
      await api.logout()
      this.forget()
      try { sessionStorage.removeItem(DATA_KEY) } catch { /* приватный режим */ }
      Alpine.store('shop').load()
    },
  })
  onUnauthorized(() => Alpine.store('account').forget())

  Alpine.store('search', { q: params.get('q') ?? '' })

  window.addEventListener('storage', e => {
    if (e.key === CART_KEY) Alpine.store('cart').items = persisted(CART_KEY, {})
    if (e.key === FAV_KEY) Alpine.store('fav').ids = persisted(FAV_KEY, {})
    if (e.key === ME_KEY) Alpine.store('account').me = storedMe()
  })

  // ---------------------------------------------------------------- карточка в сетке
  // Переключатель фасовки прямо в карточке: цена, остаток и артикул — выбранной фасовки
  Alpine.data('productCard', card => ({
    vid: defaultVariant(card).id,
    n: 1,
    get p() { return card.variants.find(v => v.id === this.vid) ?? card.variants[0] },
    pick(v) { this.vid = v.id; this.n = 1 },
  }))

  // ---------------------------------------------------------------- каталог
  // Две ступени фильтра: раздел → категория. Состояние — в адресе.
  Alpine.data('catalogPage', () => ({
    section: params.get('section') ?? '',
    category: params.get('category') ?? '',
    sort: params.get('sort') ?? '',
    onlyInStock: params.get('stock') === '1',
    shown: 48,

    init() {
      // Пришли по ссылке на категорию — раскрыть её раздел
      const syncSection = () => {
        const cat = this.$store.shop.categoryBySlug(this.category)
        if (cat) this.section = groupOfCategory(cat.id).id
      }
      syncSection()
      this.$watch('$store.shop.categories', syncSection)
      const sync = () => {
        const url = new URL(location.href)
        const set = (k, v) => (v ? url.searchParams.set(k, v) : url.searchParams.delete(k))
        this.$store.ui.section = this.section
        this.$store.ui.category = this.category
        set('section', this.category ? '' : this.section)
        set('category', this.category)
        set('q', this.$store.search.q.trim())
        set('sort', this.sort)
        set('stock', this.onlyInStock ? '1' : '')
        history.replaceState(null, '', url)
        this.shown = 48
      }
      ;['section', 'category', 'sort', 'onlyInStock', '$store.search.q'].forEach(k => this.$watch(k, sync))
    },

    pickSection(id) {
      this.section = this.section === id ? '' : id
      this.category = ''
    },
    pickCategory(slug) { this.category = this.category === slug ? '' : slug },

    get currentSection() { return this.$store.shop.section(this.section) },
    get title() {
      return this.$store.shop.categoryBySlug(this.category)?.name ?? this.currentSection?.name ?? 'Все товары'
    },
    get list() {
      const shop = this.$store.shop
      const cat = shop.categoryBySlug(this.category)
      const sec = this.currentSection
      const q = this.$store.search.q.trim().toLowerCase()
      const list = shop.cards.filter(c =>
        (!cat || c.category_id === cat.id) &&
        (cat || !sec || sec.items.some(i => i.id === c.category_id)) &&
        (!this.onlyInStock || c.inStock) &&
        (!q || c.search.includes(q)))
      if (this.sort === 'price-asc') list.sort((a, b) => a.minPrice - b.minPrice)
      if (this.sort === 'price-desc') list.sort((a, b) => b.minPrice - a.minPrice)
      return list
    },
    get page() { return this.list.slice(0, this.shown) },
    get hasFilters() { return Boolean(this.section || this.category || this.$store.search.q.trim() || this.sort || this.onlyInStock) },
    reset() { this.section = ''; this.category = ''; this.sort = ''; this.onlyInStock = false; this.$store.search.q = '' },
  }))

  // ---------------------------------------------------------------- карточка товара: ?id=<id фасовки>
  Alpine.data('productPage', () => ({
    id: params.get('id'),
    n: 1,
    async init() {
      // Свежие цены и остатки всех фасовок этой карточки
      const fresh = async () => {
        const p = this.$store.shop.product(this.id) ?? await api.product(this.id).catch(() => null)
        if (!p) return
        const list = await api.products({ group_id: p.group_id || undefined }).catch(() => [])
        this.$store.shop.replaceGroup(p.group_id ? list : [p])
      }
      fresh()
    },
    get p() { return this.$store.shop.product(this.id) },
    get card() { return this.$store.shop.cardOf(this.p) },
    pick(v) {
      this.id = v.id
      this.n = 1
      const url = new URL(location.href)
      url.searchParams.set('id', v.id)
      history.replaceState(null, '', url)
    },
    get related() {
      const p = this.p
      if (!p) return []
      return this.$store.shop.cards.filter(c => c.category_id === p.category_id && c.id !== (p.group_id || p.id)).slice(0, 4)
    },
  }))

  // ---------------------------------------------------------------- профиль
  // Вход и регистрация по телефону и паролю. Гость отслеживает заказ по номеру и телефону.
  Alpine.data('accountPage', () => ({
    tab: params.get('tab') ?? 'orders',
    mode: params.get('mode') === 'register' ? 'register' : params.get('order_no') ? 'track' : 'login',
    login: { phone: '', password: '' },
    reg: { name: '', phone: '', city: '', password: '', confirm: '' },
    showPassword: false,
    caps: false,
    authErrors: {},
    authError: null,
    authBusy: false,
    lock: persisted(LOCK_KEY, { fails: 0, until: 0 }),
    now: Date.now(),
    track: { order_no: params.get('order_no') ?? '', phone: formatPhone(params.get('phone') ?? '') },
    tracked: null,
    trackError: null,
    profile: { name: '', city: '', address: '' },
    profileErrors: {},
    profileError: null,
    savedAt: 0,
    orders: [],
    details: {},       // id заказа → состав (GET /api/orders/{id})
    loading: false,
    error: null,
    open: params.get('order') ?? null,
    statuses: STATUSES,
    payment: PAYMENT,
    delivery: DELIVERY,
    flow: FLOW,
    notice: null,

    init() {
      const fill = () => {
        const me = this.$store.account.me
        if (!me) return
        this.profile = { name: me.name ?? '', city: me.city ?? '', address: me.address ?? '' }
        this.loadOrders()
      }
      fill()
      this.$watch('$store.account.me?.id', fill)
      this.$store.account.refresh()
      this.$watch('tab', tab => {
        const url = new URL(location.href)
        tab === 'orders' ? url.searchParams.delete('tab') : url.searchParams.set('tab', tab)
        history.replaceState(null, '', url)
      })
      this.$watch('open', id => id && this.loadDetail(id))
      if (this.open) this.loadDetail(this.open)
      if (this.track.order_no && this.track.phone) this.submitTrack()
      setInterval(() => (this.now = Date.now()), 1000)
    },

    // ---- вход: после 5 неудачных попыток — пауза 5 минут (сервер проверяет пароль, пауза — защита от перебора с этой страницы)
    get lockLeft() { return Math.max(0, Math.ceil((this.lock.until - this.now) / 1000)) },
    capsCheck(e) { this.caps = e.getModifierState?.('CapsLock') ?? false },
    async submitLogin() {
      const e = {}
      if (!isPhone(this.login.phone)) e.phone = 'Номер в формате +7 (7XX) XXX-XX-XX'
      if (!this.login.password) e.password = 'Введите пароль'
      this.authErrors = e
      this.authError = null
      if (Object.keys(e).length || this.lockLeft) return
      this.authBusy = true
      try {
        await this.$store.account.login(this.login.phone, this.login.password)
        this.lock = { fails: 0, until: 0 }
        this.login.password = ''
      } catch (err) {
        if (err.status === 401) {
          const fails = this.lock.fails + 1
          this.lock = fails >= 5 ? { fails: 0, until: Date.now() + 5 * 60_000 } : { fails, until: 0 }
          this.authError = fails >= 5 ? 'Слишком много попыток. Попробуйте через 5 минут' : `Неверный телефон или пароль. Осталось попыток: ${5 - fails}`
        } else this.authError = err.message
      } finally {
        save(LOCK_KEY, this.lock)
        this.authBusy = false
      }
    },

    // ---- регистрация
    get strength() { return passwordStrength(this.reg.password) },
    get problems() { return passwordProblems(this.reg.password) },
    async submitRegister() {
      const e = {}
      if (this.reg.name.trim().length < 2) e.name = 'Укажите имя'
      if (!isPhone(this.reg.phone)) e.phone = 'Номер в формате +7 (7XX) XXX-XX-XX'
      if (this.problems.length) e.password = `Пароль: ${this.problems.join(', ')}`
      if (this.reg.password !== this.reg.confirm) e.confirm = 'Пароли не совпадают'
      this.authErrors = e
      this.authError = null
      if (Object.keys(e).length) return
      this.authBusy = true
      try {
        await this.$store.account.register({
          name: this.reg.name.trim(),
          phone: formatPhone(this.reg.phone),
          password: this.reg.password,
          city: this.reg.city.trim() || null,
        })
        this.reg.password = this.reg.confirm = ''
      } catch (err) {
        this.authError = err.message
      } finally {
        this.authBusy = false
      }
    },

    // ---- отслеживание без входа: номер заказа + телефон (ФТ-ЛК-03)
    async submitTrack() {
      this.trackError = null
      this.tracked = null
      if (!this.track.order_no.trim() || !isPhone(this.track.phone)) { this.trackError = 'Укажите номер заказа и телефон, на который он оформлен'; return }
      try {
        this.tracked = await api.track(this.track.order_no.trim(), formatPhone(this.track.phone))
      } catch (err) {
        this.trackError = err.status === 404 ? 'Заказ с таким номером и телефоном не найден' : err.message
      }
    },

    async loadOrders() {
      this.loading = true
      try {
        this.orders = await api.myOrders()
        this.error = null
      } catch (e) {
        this.error = e.message
      } finally {
        this.loading = false
      }
    },
    async loadDetail(id) {
      if (this.details[id]) return
      try { this.details[id] = await api.order(id) } catch (e) { this.notice = { id, text: e.message } }
    },

    async saveProfile() {
      const e = {}
      if (this.profile.name.trim().length < 2) e.name = 'Укажите имя'
      this.profileErrors = e
      this.profileError = null
      if (Object.keys(e).length) return
      try {
        await this.$store.account.update({ name: this.profile.name.trim(), city: this.profile.city.trim(), address: this.profile.address.trim() })
        this.savedAt = Date.now()
        setTimeout(() => (this.savedAt = 0), 2500)
      } catch (err) {
        this.profileError = err.message
      }
    },

    get stats() {
      const ok = this.orders.filter(o => o.status !== 'cancelled')
      return {
        count: ok.length,
        sum: ok.reduce((s, o) => s + tiyn(o.total), 0),
        active: ok.filter(o => !['completed', 'delivered'].includes(o.status)).length,
      }
    },
    // Путь клиента: самовывоз заканчивается выдачей, доставка — получением
    flowFor(o) { return o.delivery_method === 'pickup' ? ['new', 'confirmed', 'assembled', 'handed_over', 'completed'] : FLOW },
    stepIn(o) { return this.flowFor(o).indexOf(o.status) },
    payKey: paymentKey,
    // Повторить: положить в корзину то, что есть в наличии
    repeat(o) {
      const d = this.details[o.id]
      if (!d) return
      let added = 0
      d.items.forEach(i => {
        const p = this.$store.shop.product(i.product_id)
        if (!p) return
        const before = this.$store.cart.qty(p.id)
        this.$store.cart.add(p, Math.floor(milli(i.quantity) / 1000))
        added += this.$store.cart.qty(p.id) - before
      })
      if (added) location.href = '/cart.html'
      else this.notice = { id: o.id, text: 'Товаров из этого заказа сейчас нет в наличии' }
    },
  }))

  // ---------------------------------------------------------------- оформление
  Alpine.data('checkoutPage', () => ({
    form: { name: '', phone: '', delivery_method: 'pickup', city: 'Астана', address: '', comment: '' },
    errors: {},
    touched: false,
    sending: false,
    serverError: null,
    done: null,
    delivery: DELIVERY,

    init() {
      const me = this.$store.account.me
      if (!me) return
      this.form.name ||= me.name
      this.form.phone ||= me.phone
      if (me.city) this.form.city = me.city
      if (me.address && !this.form.address) this.form.address = me.address
    },
    // Астана — 1 200 ₸, межгород — 2 500 ₸ (ТЗ), самовывоз — бесплатно
    get deliveryTiyn() { return deliveryCost(this.form.delivery_method, this.form.city) },

    validate() {
      const e = {}
      if (this.form.name.trim().length < 2) e.name = 'Укажите имя'
      if (!this.form.phone.trim()) e.phone = 'Укажите телефон'
      else if (!isPhone(this.form.phone)) e.phone = 'Номер в формате +7 (7XX) XXX-XX-XX'
      if (this.form.delivery_method !== 'pickup' && !this.form.city.trim()) e.city = 'Укажите город'
      if (this.form.delivery_method !== 'pickup' && this.form.address.trim().length < 5) e.address = 'Укажите адрес доставки'
      this.errors = e
      return Object.keys(e).length === 0
    },
    formatPhone() { this.form.phone = formatPhone(this.form.phone) },
    async submit() {
      this.touched = true
      this.serverError = null
      const cart = this.$store.cart
      if (!this.validate()) {
        // кнопка заказа на телефоне закреплена внизу — показываем, какое поле исправить (#47)
        this.$nextTick(() => document.querySelector('form .is-invalid')?.focus())
        return
      }
      if (!cart.okLines.length || cart.blocked) return
      this.sending = true
      const expected = cart.total
      const lines = cart.okLines.map(l => ({ ...l }))
      try {
        const order = await api.createOrder({
          client_name: this.form.name.trim(),
          client_phone: formatPhone(this.form.phone),
          city: this.form.city.trim() || null,
          delivery_method: this.form.delivery_method,
          delivery_address: this.form.delivery_method === 'pickup' ? null : this.form.address.trim(),
          delivery: this.form.delivery_method === 'pickup' ? null : tiynToString(this.deliveryTiyn),
          note: this.form.comment.trim() || null,
          items: lines.map(l => ({ product_id: l.product.id, quantity: qtyToString(l.qty) })),
        })
        // Суммы корзины и backend считаются одинаково — расхождение означает, что цены поменялись
        const mismatch = tiyn(order.items_total) !== expected
        if (mismatch) console.warn('Сумма заказа отличается от корзины', { cart: expected, backend: order.items_total })
        this.done = { order, mismatch, phone: formatPhone(this.form.phone) }
        cart.clear()
        this.$store.shop.load()
        window.scrollTo({ top: 0, behavior: 'smooth' })
      } catch (e) {
        this.serverError = e.message
        this.$store.shop.load() // остатки могли измениться — обновить корзину
      } finally {
        this.sending = false
      }
    },
  }))
})

// Полное имя фасовки для списков: «Шоколад тёмный 54 % в каллетах, 1 кг»
export function fullName(p) {
  return p.variant_name && !p.name.includes(p.variant_name) && p.group_id && Alpine.store('shop').cardOf(p)?.multi
    ? `${p.name}, ${p.variant_name}` : p.name
}

// Хелперы для разметки
window.bl = {
  fullName,
  variantLabel,
  inStock,
  units: availableUnits,
  available,
  retail,
  basePrice,
  isPersonal,
  groupMedia: categoryId => groupOfCategory(categoryId).media,
  section: id => groupById(id),
}

window.Alpine = Alpine
Alpine.start()
Alpine.store('shop').load()

// Просмотр страницы — для аналитики владельца (устройство и браузер сервер берёт из User-Agent, IP не хранит)
const tabId = (() => {
  try {
    let id = sessionStorage.getItem('bakeloom.tab')
    if (!id) sessionStorage.setItem('bakeloom.tab', (id = crypto.randomUUID()))
    return id
  } catch { return undefined }
})()
api.visit({ path: location.pathname, referrer: document.referrer || undefined, session_id: tabId })
