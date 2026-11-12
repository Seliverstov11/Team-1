/**
 * Дизайн-токены Bakeloom: цвета, шрифты, размеры текста, радиусы и тени.
 * Цвета задаются в palette.toml, здесь — всё остальное. В разметке пишем классы по смыслу
 * (bg-brand-dark, text-error, rounded-card), а не цвета (bg-[#4A5236]).
 *
 * Источник: Figma «Сладкий мастер — макет сайта» → страница «🌿 UI-кит · Минималистичный & Премиум»,
 * локальные стили «Премиум/…». Палитра с именами стилей Figma — в palette.toml.
 * Наглядно все токены — на странице /tokens.html.
 *
 * Tailwind 4 читает этот файл через `@config` в src/input.css.
 */

import { readFileSync } from 'node:fs'
import { parse } from 'smol-toml'

// palette.toml: [группа.оттенок] hex = "…"  →  { группа: { оттенок: '#…' } }
const palette = Object.fromEntries(
  Object.entries(parse(readFileSync(new URL('./palette.toml', import.meta.url), 'utf8'))).map(
    ([group, shades]) => [group, Object.fromEntries(Object.entries(shades).map(([shade, c]) => [shade, c.hex]))],
  ),
)

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./*.html', './partials/**/*.html', './src/**/*.js'],

  theme: {
    extend: {
      // Цвета — из palette.toml (единственный источник палитры)
      colors: palette,

      fontFamily: {
        // Заголовки, цены, логотип
        heading: ['"Cormorant Garamond"', 'Cormorant', 'Garamond', 'Georgia', '"Times New Roman"', 'serif'],
        // Основной текст, кнопки, подписи
        body: ['Montserrat', '"Segoe UI"', 'Roboto', '"Helvetica Neue"', 'Arial', 'system-ui', 'sans-serif'],
      },

      // Стили текста «Премиум/…» из UI-кита: [размер, { высота строки, разрядка }]
      fontSize: {
        display: ['4rem', { lineHeight: '1.05' }],                          // Display — Cormorant Light 64
        h1: ['3rem', { lineHeight: '1.1' }],                                // H1 — Cormorant Medium 48
        h2: ['2.125rem', { lineHeight: '1.15' }],                           // H2 — Cormorant Medium 34
        h3: ['1.5rem', { lineHeight: '1.25' }],                             // H3 — Cormorant SemiBold 24
        price: ['1.5rem', { lineHeight: '1.2' }],                           // Price — Cormorant SemiBold 24
        lead: ['1.125rem', { lineHeight: '1.65' }],                         // Lead — Montserrat Light 18
        body: ['0.9375rem', { lineHeight: '1.65' }],                        // Body — Montserrat Regular 15
        small: ['0.8125rem', { lineHeight: '1.55' }],                       // Small — Montserrat Regular 13
        button: ['0.75rem', { lineHeight: '1.2', letterSpacing: '0.14em' }],   // Button — Montserrat Medium 12, капс
        overline: ['0.6875rem', { lineHeight: '1.4', letterSpacing: '0.18em' }], // Overline — Montserrat Medium 11, капс
      },

      // Радиусы компонентов UI-кита
      borderRadius: {
        check: '0.3125rem',   // 5px — чекбокс
        control: '0.625rem',  // 10px — кнопки, поля, иконки-кнопки
        search: '0.75rem',    // 12px — поиск, фото в карточке
        card: '1.125rem',     // 18px — карточка товара, шапка, панели
        panel: '1.5rem',      // 24px — боковое меню, баннер
      },

      // Стили эффектов «Премиум/…»
      boxShadow: {
        subtle: '0 8px 28px rgb(59 51 31 / 0.07)', // Shadow Subtle
        lift: '0 18px 44px rgb(59 51 31 / 0.12)',  // Shadow Lift — hover карточки
      },

      // Отступы: в UI-ките свои не заданы — используем стандартную шкалу Tailwind.
    },
  },
}
