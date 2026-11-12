import { defineConfig } from 'vite'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('.', import.meta.url))
const partialsDir = resolve(root, 'partials')

// Страницы — отдельные .html файлы (без роутера)
const pages = ['index', 'catalog', 'product', 'cart', 'checkout', 'favorites', 'contacts', 'tokens']

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

export default defineConfig({
  plugins: [htmlIncludes()], // Tailwind — через postcss.config.js
  // host: true — сайт доступен с других компьютеров в локальной сети (http://<IP>:5173).
  // strictPort — порт всегда 5173, чтобы адрес для коллег не «уезжал».
  server: { host: true, port: 5173, strictPort: true, open: true },
  preview: { host: true, port: 4173, strictPort: true },
  build: {
    rolldownOptions: {
      input: Object.fromEntries(pages.map(p => [p, resolve(root, `${p}.html`)])),
    },
  },
})
