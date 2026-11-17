// Service worker CRM BAKELOOM: держит оболочку приложения (страницы, скрипты, стили, шрифты, значки),
// чтобы установленная CRM открывалась сразу. Данные (/api/*) не кэшируются никогда — только с сервера.
//
// Всё, кроме API, берётся сначала из сети и только без сети — из кэша. Поэтому после выкладки новой
// версии CRM подхватывает её при следующем открытии, переустанавливать приложение не нужно.
// Меняется логика этого файла — поднимите VERSION: браузер увидит новый sw.js и заменит старый сам.

const VERSION = 'v1'
const CACHE = `bakeloom-crm-${VERSION}`

const SHELL = [
  '/crm/',
  '/crm/login.html',
  '/crm/orders.html',
  '/crm/order.html',
  '/crm/picking.html',
  '/crm/stock.html',
  '/crm/clients.html',
  '/crm/catalog.html',
  '/crm/manifest.webmanifest',
  '/crm/icons/icon-192.png',
  '/crm/icons/icon-512.png',
  '/img/logo.svg',
]

self.addEventListener('install', event => {
  // Без await на отдельных страницах: одна недоступная не должна срывать установку
  event.waitUntil(caches.open(CACHE).then(cache => Promise.all(SHELL.map(url => cache.add(url).catch(() => null)))))
  self.skipWaiting()
})

self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    const keys = await caches.keys()
    await Promise.all(keys.filter(k => k.startsWith('bakeloom-crm-') && k !== CACHE).map(k => caches.delete(k)))
    await self.clients.claim()
  })())
})

// Что не трогаем: чужие адреса, не-GET, API, служебные запросы dev-сервера Vite
const bypass = url =>
  url.origin !== self.location.origin ||
  url.pathname.startsWith('/api/') ||
  url.pathname.startsWith('/@') ||
  url.pathname.startsWith('/node_modules/') ||
  url.pathname === '/crm/sw.js'

const OFFLINE = `<!doctype html><html lang="ru"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Нет связи — BAKELOOM CRM</title><body style="margin:0;display:grid;place-items:center;min-height:100vh;background:#353B26;color:#F7F4EC;font:15px/1.5 system-ui,sans-serif;text-align:center">
<div><p style="font-size:22px;margin:0 0 8px">Нет связи с сервером</p><p style="opacity:.75;margin:0 0 20px">Проверьте интернет и попробуйте снова</p>
<button onclick="location.reload()" style="font:inherit;padding:10px 22px;border:0;border-radius:8px;background:#B8975A;color:#353B26;cursor:pointer">Обновить</button></div></body></html>`

self.addEventListener('fetch', event => {
  const { request } = event
  if (request.method !== 'GET') return
  const url = new URL(request.url)
  if (bypass(url)) return

  event.respondWith((async () => {
    const cache = await caches.open(CACHE)
    try {
      const fresh = await fetch(request)
      if (fresh.ok && fresh.type === 'basic') cache.put(request, fresh.clone())
      return fresh
    } catch {
      // Без сети: страница заказа с любым ?id= — из той же сохранённой страницы
      const cached = await cache.match(request, { ignoreSearch: request.mode === 'navigate' })
      if (cached) return cached
      if (request.mode === 'navigate') return new Response(OFFLINE, { headers: { 'Content-Type': 'text/html; charset=utf-8' } })
      return Response.error()
    }
  })())
})
