/**
 * Дизайн-токены Bakeloom — единственное место, где задаются цвета, шрифты, размеры текста,
 * радиусы и тени. В разметке пишем классы по смыслу (bg-brand-dark, text-error, rounded-card),
 * а не цвета (bg-[#4A5236]). Поменять оттенок = поправить одну строку здесь.
 *
 * Источник: Figma «Сладкий мастер — макет сайта» → страница «🌿 UI-кит · Минималистичный & Премиум»,
 * локальные стили «Премиум/…» (в комментариях — имя стиля в Figma).
 * Наглядно все токены — на странице /tokens.html.
 *
 * Tailwind 4 читает этот файл через `@config` в src/input.css.
 */

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./*.html', './partials/**/*.html', './src/**/*.js'],

  theme: {
    extend: {
      colors: {
        // Фирменные цвета
        brand: {
          dark: '#4A5236',          // Премиум/Primary/Olive — основной: кнопки, меню, баннер
          'dark-hover': '#3A4129',  // Премиум/Primary/Olive Dark — hover основной кнопки, низ градиентов
          'dark-raised': '#56603F', // верх градиента баннера (пример главной в Figma)
          'dark-inset': '#555E40',  // плашка-иллюстрация на баннере (пример главной в Figma)
          'dark-soft': '#E6E8DC',   // Премиум/Primary/Olive Soft — светлая подложка в тон основному
          light: '#F7F4EC',         // Премиум/Surface/Ivory — фон страницы
          gold: '#B8975A',          // Премиум/Accent/Gold — акцент, рамки, подписи-«капс»
          'gold-hover': '#9A7B43',  // Премиум/Accent/Gold Dark — hover золотой кнопки
          'gold-light': '#D9C499',  // Премиум/Accent/Gold Light — золото на тёмном фоне, рамки при hover
          'gold-soft': '#F2EBDA',   // Премиум/Accent/Gold Soft — подложка иконок, выделение
        },

        // Поверхности
        surface: {
          DEFAULT: '#FFFFFF',       // Премиум/Surface/White — карточки, шапка, панели
          sand: '#EDE6D6',          // Премиум/Surface/Sand — вторичная подложка
        },

        // Текст
        ink: {
          DEFAULT: '#2B2620',       // Премиум/Text/Espresso — основной текст
          muted: '#8C8172',         // Премиум/Text/Muted — второстепенный текст, подписи
          placeholder: '#B5AD9E',   // плейсхолдеры полей (компонент Input в UI-ките)
        },

        // Контент на тёмном (оливковом) фоне: меню, баннер
        'on-dark': {
          DEFAULT: '#E9E4D4',       // основной текст на тёмном
          muted: '#A9AE92',         // подписи на тёмном
          subtle: '#C9C9B4',        // подзаголовок баннера
          line: '#5E6748',          // разделители на тёмном
        },

        // Линии и рамки
        line: {
          DEFAULT: '#E3DCCB',       // Премиум/Border/Hairline — рамки полей, разделители
          strong: '#C9C0AE',        // рамка чекбокса (компонент Checkbox)
        },

        // Состояния
        success: {
          DEFAULT: '#7C8B5E',       // Премиум/Status/Sage — «в наличии», успех
          soft: '#E9EDE0',          // Премиум/Status/Sage Soft — фон уведомления об успехе
        },
        error: {
          DEFAULT: '#B4664A',       // Премиум/Status/Terracotta — ошибки, «нет в наличии»
          soft: '#F5E6DF',          // Премиум/Status/Terracotta Soft — фон ошибки
        },
        disabled: '#D9D3C6',        // Премиум/State/Disabled — неактивные кнопки и поля

        // Подложки фото товара по категориям (пока image_url пустой)
        media: {
          ingredients: '#EFE9DC',
          packaging: '#E8E2D0',
          decor: '#E6E9DC',
          tools: '#F2EBDA',
        },
      },

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
