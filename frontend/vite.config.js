import { defineConfig } from 'vite'
import { existsSync, readFileSync, utimesSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('.', import.meta.url))
const partialsDir = resolve(root, 'partials')

// Страницы — отдельные .html файлы (без роутера). Сайт — в корне, CRM — в папке crm/
const pages = [
  'index', 'catalog', 'product', 'cart', 'checkout', 'favorites', 'contacts', 'account', 'tokens', '404',
  'crm/index', 'crm/login', 'crm/orders', 'crm/order', 'crm/clients', 'crm/stock', 'crm/picking', 'crm/catalog',
]

// Общие куски разметки (шапка, меню, подвал, карточка товара) подставляются при сборке:
// <!-- @include имя.html --> → содержимое partials/имя.html
function htmlIncludes() {
  const include = (html, depth = 0) =>
    html.replace(/<!--\s*@include\s+([\w.-]+)\s*-->/g, (_, file) => {
      if (depth > 8) throw new Error(`Слишком глубокая вложенность include: ${file}`)
      return include(readFileSync(resolve(partialsDir, file), 'utf8'), depth + 1)
    })

  return {
    name: 'html-includes',
    transformIndexHtml: { order: 'pre', handler: html => include(html) },
    handleHotUpdate({ file, server }) {
      if (file.startsWith(partialsDir)) {
        server.ws.send({ type: 'full-reload' })
        return []
      }
    },
  }
}

// Палитра читается из palette.toml в tailwind.config.js. Tailwind пересобирает стили, только когда
// меняется сам конфиг, поэтому при правке палитры обновляем дату изменения конфига
// и перезагружаем страницу — новые цвета применяются сразу.
function watchPalette() {
  return {
    name: 'watch-palette',
    configureServer(server) {
      const palette = resolve(root, 'palette.toml')
      const config = resolve(root, 'tailwind.config.js')
      server.watcher.add(palette)
      server.watcher.on('change', file => {
        if (file !== palette) return
        const now = new Date()
        utimesSync(config, now, now)
        server.ws.send({ type: 'full-reload' })
      })
    },
  }
}

// Несуществующий адрес — страница 404, а не главная (обычный Vite отдаёт index.html на всё подряд)
function notFoundPage() {
  // Страница существует? «/» и «/crm/» → index.html, «/crm» → crm/index.html
  const pageExists = (dir, url) => {
    const path = decodeURIComponent(url.split(/[?#]/)[0])
    if (path.startsWith('/@') || path.startsWith('/node_modules/') || path.startsWith('/src/') || path.startsWith('/api/')) return true
    if (/\.[a-z0-9]+$/i.test(path) && !path.endsWith('.html')) return true // ресурсы: картинки, JSON, шрифты
    const candidates = path.endsWith('/') ? [`${path}index.html`] : path.endsWith('.html') ? [path] : [`${path}.html`, `${path}/index.html`]
    return candidates.some(c => existsSync(resolve(dir, `.${c}`)))
  }
  const wantsHtml = req => req.method === 'GET' && req.headers.accept?.includes('text/html')
  const send404 = (res, html) => {
    res.statusCode = 404
    res.setHeader('Content-Type', 'text/html; charset=utf-8')
    res.end(html)
  }
  return {
    name: 'not-found-page',
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        if (!wantsHtml(req) || pageExists(root, req.url)) return next()
        send404(res, await server.transformIndexHtml('/404.html', readFileSync(resolve(root, '404.html'), 'utf8')))
      })
    },
    configurePreviewServer(server) {
      const dist = resolve(root, 'dist')
      server.middlewares.use((req, res, next) => {
        if (!wantsHtml(req) || pageExists(dist, req.url)) return next()
        send404(res, readFileSync(resolve(dist, '404.html'), 'utf8'))
      })
    },
  }
}

// Backend (Rust, backend/): cd backend && cargo run → http://localhost:8080.
// Запросы /api/* идут через dev-сервер, поэтому браузеру коллег в сети не нужен доступ к порту 8080.
const API_TARGET = process.env.BAKELOOM_API ?? 'http://localhost:8080'
const apiProxy = { '/api': { target: API_TARGET, changeOrigin: true } }

export default defineConfig({
  plugins: [htmlIncludes(), watchPalette(), notFoundPage()], // Tailwind — через postcss.config.js
  // host: true — сайт доступен с других компьютеров в локальной сети (http://<IP>:5174).
  // Порт 5174, чтобы можно было запускать одновременно с frontend/ (5173).
  // strictPort — порт не «уезжает»: адрес для коллег всегда один.
  server: { host: true, port: 5174, strictPort: true, open: true, proxy: apiProxy },
  preview: { host: true, port: 4174, strictPort: true, proxy: apiProxy },
  build: {
    rolldownOptions: {
      input: Object.fromEntries(pages.map(p => [p, resolve(root, `${p}.html`)])),
    },
  },
})
