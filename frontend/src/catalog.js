// Каталог заказчика: 23 категории, 289 позиций, 262 карточки.
//
// 1. Категорий много и две трети — упаковка, плоский список фильтров не помещается.
//    Поэтому две ступени: раздел (GROUPS) → категория. Раздел — только интерфейс,
//    в данных его нет; категория, которой нет в списке, попадает в «Другое».
// 2. Часть позиций — фасовки одного товара с общим group_id (шоколад 100 г / 500 г / 1 кг).
//    Карточка одна, внутри переключатель; у каждой фасовки свои цена, остаток и артикул.
// 3. Цена одна для всех (заказчик, В3 27:47): опт выключен, wholesale_price приходит null.
//    Вошедшему покупателю с договорной ценой сервер отдаёт её в personal_price — показываем её.
//    Итог заказа всё равно считает сервер.

import { lineTotal, milli, tiyn } from './money.js'

export const GROUPS = [
  { id: 'ingredients', name: 'Ингредиенты', media: 'ingredients',
    categories: ['c-shokolad-i-ingredienty'] },
  { id: 'decor', name: 'Декор, свечи, ленты', media: 'decor',
    categories: ['c-dekor', 'c-svechi', 'c-lenty'] },
  { id: 'cake-boxes', name: 'Коробки для тортов', media: 'box',
    categories: ['c-korobki-dlya-torta-1-kg', 'c-kor-dlya-torta-meruert-1-3-kg', 'c-korobki-dlya-torta-1-5-kg', 'c-korobki-dlya-torta-2-5-kg',
      'c-korobki-dlya-torta-3-kg', 'c-korobki-d-torta-4-kg', 'c-korobki-dlya-zakaznyh-tortov'] },
  { id: 'pastry-boxes', name: 'Коробки для выпечки', media: 'box',
    categories: ['c-korobki-dlya-kapkeykov', 'c-korobki-dlya-pirogov', 'c-korobki-dlya-pechene', 'c-korobki-dlya-pryanikov'] },
  { id: 'food-boxes', name: 'Пицца и фаст-фуд', media: 'box',
    categories: ['c-korobki-dlya-piccy-belye', 'c-korobki-dlya-fast-fuda'] },
  { id: 'boards', name: 'Подложки, круги, поддоны', media: 'board',
    categories: ['c-podlozhki', 'c-krugi', 'c-poddony'] },
  { id: 'packaging', name: 'Пакеты и упаковка', media: 'bag',
    categories: ['c-upakovka', 'c-pakety'] },
  { id: 'stationery', name: 'Канцелярия', media: 'other',
    categories: ['c-kancelyariya'] },
]
const OTHER = { id: 'other', name: 'Другое', media: 'other', categories: [] }

const groupByCategory = new Map(GROUPS.flatMap(g => g.categories.map(c => [c, g])))
export const groupOfCategory = categoryId => groupByCategory.get(categoryId) ?? OTHER
export const groupById = id => GROUPS.find(g => g.id === id) ?? (id === 'other' ? OTHER : null)

// Разделы с их категориями в порядке API; пустые разделы не показываем
export function buildSections(categories) {
  const sections = [...GROUPS, OTHER].map(g => ({ ...g, items: [] }))
  const byId = new Map(sections.map(s => [s.id, s]))
  categories.forEach(c => byId.get(groupOfCategory(c.id).id).items.push(c))
  return sections.filter(s => s.items.length)
}

// ---------------------------------------------------------------- товар (одна фасовка)
export const available = p => Math.max(0, milli(p?.available_qty)) // в тысячных
export const availableUnits = p => Math.floor(available(p) / 1000)
export const inStock = p => available(p) > 0

export const basePrice = p => tiyn(p.price)
export const retail = p => tiyn(p.personal_price ?? p.price)
export const isPersonal = p => Boolean(p?.personal_price) && tiyn(p.personal_price) !== tiyn(p.price)
export const lineSum = (p, qty) => lineTotal(retail(p), qty * 1000)

// Доставка с сайта: Астана — 1 200 ₸, межгород — 2 500 ₸, самовывоз бесплатно
// Как на сервере (backend site_delivery): курьер по Астане 1 200 ₸ (город не указан — тоже Астана),
// любая другая доставка — 2 500 ₸, самовывоз бесплатно. Итог заказа всё равно считает сервер
export const isAstana = city => { const c = String(city ?? '').trim().toLowerCase(); return !c || c.includes('астана') || c.includes('astana') }
export const deliveryCost = (method, city) =>
  method === 'pickup' ? 0 : method === 'courier' && isAstana(city) ? 120000 : 250000

// Подпись фасовки; у одиночных товаров она дублирует размер из характеристик — тогда не нужна
export const variantLabel = p => p.variant_name || p.unit

// ---------------------------------------------------------------- карточка (группа фасовок)
const collator = new Intl.Collator('ru', { numeric: true })

export function buildCards(products) {
  const byGroup = new Map()
  products.forEach(p => {
    const key = p.group_id || p.id
    if (!byGroup.has(key)) byGroup.set(key, [])
    byGroup.get(key).push(p)
  })
  return [...byGroup.entries()].map(([id, variants]) => {
    variants.sort((a, b) => retail(a) - retail(b) || collator.compare(a.variant_name ?? '', b.variant_name ?? ''))
    const first = variants[0]
    return {
      id,
      name: first.name,
      category_id: first.category_id,
      variants,
      multi: variants.length > 1,
      minPrice: Math.min(...variants.map(retail)),
      inStock: variants.some(inStock),
      search: variants.map(v => `${v.name} ${v.sku ?? ''} ${v.variant_name ?? ''}`).join(' ').toLowerCase(),
    }
  }).sort((a, b) => collator.compare(a.name, b.name))
}

// Фасовка по умолчанию: первая в наличии, иначе самая дешёвая
export const defaultVariant = card => card.variants.find(inStock) ?? card.variants[0]
