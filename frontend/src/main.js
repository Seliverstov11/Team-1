import Alpine from 'alpinejs'
import { createOrder, getCategories, getProducts } from './api.js'

const params = new URLSearchParams(location.search)
const money = new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 })

// ---------------------------------------------------------------- утилиты
const fmt = {
  price: value => `${money.format(Number(value))} ₸`,
  plural(n, one, few, many) {
    const m10 = n % 10, m100 = n % 100
    if (m10 === 1 && m100 !== 11) return one
    if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few
    return many
  },
}
const stock = p => Number(p?.available_qty ?? 0)

function persisted(key, fallback) {
  try { return JSON.parse(localStorage.getItem(key)) ?? fallback } catch { return fallback }
}
const save = (key, value) => localStorage.setItem(key, JSON.stringify(value))

persisted.session = key => {
  try { return JSON.parse(sessionStorage.getItem(key)) } catch { return null }
}

const CART_KEY = 'bakeloom.cart'
const DATA_KEY = 'bakeloom.data'
const MENU_KEY = 'bakeloom.menu'
const FAV_KEY = 'bakeloom.favorites'

// ---------------------------------------------------------------- хранилища
document.addEventListener('alpine:init', () => {
  // Состояние интерфейса. Раскрытие меню каталога помним в рамках сессии,
  // чтобы при переходе между страницами меню не «прыгало».
  Alpine.store('ui', {
    page: document.body.dataset.page ?? '',
    category: params.get('category') ?? '', // выбранная категория в каталоге
    catalogOpen: sessionStorage.getItem(MENU_KEY) !== '0',
    toggleCatalog() {
      this.catalogOpen = !this.catalogOpen
      sessionStorage.setItem(MENU_KEY, this.catalogOpen ? '1' : '0')
    },
    // Какая категория подсвечена в меню: в каталоге — выбранная, на карточке — категория товара
    get activeCategory() {
      if (this.page === 'catalog') return this.category
      if (this.page === 'product') {
        const shop = Alpine.store('shop')
        return shop.category(shop.product(params.get('id'))?.category_id).slug ?? ''
      }
      return null
    },
  })

  Alpine.store('fmt', fmt)

  // Справочники: товары и категории
  // Товары и категории. Справочник кэшируется в sessionStorage: при переходе между
  // страницами он доступен сразу, без пустого кадра, а свежие данные подгружаются в фоне.
  const cached = persisted.session(DATA_KEY)
  Alpine.store('shop', {
    products: cached?.products ?? [],
    categories: cached?.categories ?? [],
    ready: Boolean(cached),
    error: null,
    async load() {
      try {
        const [categories, products] = await Promise.all([getCategories(), getProducts()])
        this.categories = categories
        this.products = products
        sessionStorage.setItem(DATA_KEY, JSON.stringify({ categories, products }))
      } catch (e) {
        console.error(e)
        if (!this.products.length) this.error = 'Не удалось загрузить товары. Обновите страницу.'
      } finally {
        this.ready = true
      }
    },
    product(id) { return this.products.find(p => p.id === id) },
    category(id) { return this.categories.find(c => c.id === id) ?? {} },
    categoryBySlug(slug) { return this.categories.find(c => c.slug === slug) },
    countIn(id) { return this.products.filter(p => p.category_id === id).length },
    stock,
  })

  // Корзина: { id товара: количество }, живёт в localStorage
  Alpine.store('cart', {
    items: persisted(CART_KEY, {}),
    persist() { save(CART_KEY, this.items) },
    qty(id) { return this.items[id] ?? 0 },
    left(p) { return stock(p) - this.qty(p.id) },
    add(p, n = 1) {
      const add = Math.min(n, this.left(p))
      if (add < 1) return
      this.items[p.id] = this.qty(p.id) + add
      this.persist()
    },
    set(p, n) {
      const value = Math.max(1, Math.min(Math.floor(Number(n) || 1), stock(p)))
      this.items[p.id] = value
      this.persist()
    },
    remove(id) { delete this.items[id]; this.persist() },
    clear() { this.items = {}; this.persist() },
    // Позиции с данными товара; пока справочник не загружен — пусто
    get lines() {
      const shop = Alpine.store('shop')
      return Object.entries(this.items)
        .map(([id, qty]) => ({ product: shop.product(id), qty }))
        .filter(l => l.product)
    },
    get count() { return Object.values(this.items).reduce((s, n) => s + n, 0) },
    get total() { return this.lines.reduce((s, l) => s + Number(l.product.price) * l.qty, 0) },
  })

  // Избранное: { id: true }, живёт в localStorage
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

  // Поиск в шапке: на странице каталога фильтрует сразу, на остальных ведёт в каталог
  Alpine.store('search', { q: params.get('q') ?? '' })

  // Синхронизация между вкладками
  window.addEventListener('storage', e => {
    if (e.key === CART_KEY) Alpine.store('cart').items = persisted(CART_KEY, {})
    if (e.key === FAV_KEY) Alpine.store('fav').ids = persisted(FAV_KEY, {})
  })

  // ---------------------------------------------------------------- страницы

  // Каталог: фильтр по категории, поиск, сортировка. Состояние отражается в адресе,
  // чтобы ссылки из меню и поиска из шапки открывали нужную выборку.
  Alpine.data('catalogPage', () => ({
    category: params.get('category') ?? '',
    sort: params.get('sort') ?? '',

    init() {
      const sync = () => {
        const url = new URL(location.href)
        const set = (k, v) => (v ? url.searchParams.set(k, v) : url.searchParams.delete(k))
        this.$store.ui.category = this.category
        set('category', this.category)
        set('q', this.$store.search.q.trim())
        set('sort', this.sort)
        history.replaceState(null, '', url)
      }
      this.$watch('category', sync)
      this.$watch('sort', sync)
      this.$watch('$store.search.q', sync)
    },

    get categoryName() {
      return this.$store.shop.categoryBySlug(this.category)?.name ?? 'Все товары'
    },
    get list() {
      const shop = this.$store.shop
      const cat = shop.categoryBySlug(this.category)
      const q = this.$store.search.q.trim().toLowerCase()
      const list = shop.products.filter(p =>
        (!cat || p.category_id === cat.id) && (!q || p.name.toLowerCase().includes(q)))
      if (this.sort === 'price-asc') list.sort((a, b) => a.price - b.price)
      if (this.sort === 'price-desc') list.sort((a, b) => b.price - a.price)
      return list
    },
    get hasFilters() { return Boolean(this.category || this.$store.search.q.trim() || this.sort) },
    reset() { this.category = ''; this.sort = ''; this.$store.search.q = '' },
  }))

  // Карточка товара: ?id=<uuid>
  Alpine.data('productPage', () => ({
    id: params.get('id'),
    n: 1,
    get p() { return this.$store.shop.product(this.id) },
    // В контракте нет отдельного поля фасовки: берём её из хвоста названия («…, 1 кг»)
    get pack() {
      const tail = this.p?.name.split(',').at(-1).trim()
      return tail && /\d/.test(tail) && tail !== this.p.name ? tail : null
    },
    get related() {
      const p = this.p
      if (!p) return []
      return this.$store.shop.products.filter(x => x.category_id === p.category_id && x.id !== p.id).slice(0, 4)
    },
  }))

  // Оформление заказа
  Alpine.data('checkoutPage', () => ({
    form: { name: '', phone: '', delivery: 'pickup', address: '', comment: '' },
    errors: {},
    touched: false,
    sending: false,
    done: null, // { number, lines, total } после «Оформить»

    validate() {
      const e = {}
      if (this.form.name.trim().length < 2) e.name = 'Укажите имя'
      const digits = this.form.phone.replace(/\D/g, '')
      if (!digits) e.phone = 'Укажите телефон'
      else if (!/^[78]\d{10}$/.test(digits)) e.phone = 'Номер в формате +7 (7XX) XXX-XX-XX'
      this.errors = e
      return Object.keys(e).length === 0
    },
    // Мягкая маска казахстанского номера: +7 (7XX) XXX-XX-XX
    formatPhone() {
      let d = this.form.phone.replace(/\D/g, '')
      if (!d) return
      if (d[0] === '8') d = '7' + d.slice(1)
      if (d[0] !== '7') d = '7' + d
      d = d.slice(0, 11)
      const parts = ['+7']
      if (d.length > 1) parts.push(` (${d.slice(1, 4)}`)
      if (d.length >= 4) parts[1] += ')'
      if (d.length > 4) parts.push(` ${d.slice(4, 7)}`)
      if (d.length > 7) parts.push(`-${d.slice(7, 9)}`)
      if (d.length > 9) parts.push(`-${d.slice(9, 11)}`)
      this.form.phone = parts.join('')
    },
    async submit() {
      this.touched = true
      if (!this.validate() || !this.$store.cart.lines.length) return
      this.sending = true
      const cart = this.$store.cart
      const { number } = await createOrder({ ...this.form, items: cart.items })
      this.done = { number, lines: cart.lines, total: cart.total }
      cart.clear()
      this.sending = false
      window.scrollTo({ top: 0, behavior: 'smooth' })
    },
  }))
})

window.Alpine = Alpine
Alpine.start()
Alpine.store('shop').load()
