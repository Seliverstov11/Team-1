# Фото товаров (#46, #42)

Заказчик фото не передал — подбираем со стока в едином стиле (ФТ-Т-07): фон убран,
подложка `#F7F4EC` (brand.light), квадрат 800×800, товар по центру с полями 12 %, мягкая тень, webp.
Сайт берёт `image_url`, а если фото нет — показывает заглушку раздела (`frontend/partials/product-image.html`).

## Что нужно

- Python 3.11+ и `pip install pillow numpy onnxruntime`.
  Фон убирает модель `isnet-general-use` из rembg — та же, что в `rembg -m isnet-general-use`, но запускается напрямую:
  пакет `rembg` тянет numba, которая не ставится на свежий Python. Модель скачается сама в `~/.u2net/` (около 170 МБ, один раз).
  Работает на CPU, около 1,3 с на фото.
- Ключ Pexels API (бесплатно: <https://www.pexels.com/api/> → «Your API key»).
  Только в переменной окружения или в `tools/photos/.env` — оба в `.gitignore`, в репозиторий не попадают:

  ```bash
  export PEXELS_API_KEY=…            # или: echo 'PEXELS_API_KEY=…' > tools/photos/.env
  ```

## Запуск

Из корня репозитория:

```bash
python3 tools/photos/photos.py items     # 289 позиций из миграций 001–006 → work/items.json, запросы → queries.json
python3 tools/photos/photos.py fetch     # по 4 кандидата на товар с Pexels → work/raw/
python3 tools/photos/photos.py process   # фон, подложка, тень → work/out/ (CPU, ~1–2 с на фото)
python3 tools/photos/photos.py serve     # проверка: http://localhost:8765/review.html
python3 tools/photos/photos.py build     # frontend/public/img/products/*.webp + backend/migrations/009_product_images.sql (и строка в backend/src/db.rs)
```

Все шаги повторяемые: уже скачанное и обработанное берётся из `work/` (флаг `--refresh` — заново).

- **`queries.json`** — поисковый запрос (по-английски, Pexels так ищет точнее) для каждой позиции.
  Генерируется по правилам из `photos.py`, можно править руками: `items` существующие запросы не трогает. После правки — `fetch` и `process`.
- **`selection.json`** — файл выбора, результат проверки: `{SKU: {photo: id Pexels | null, reviewed}}`. `null` — «нет фото».

## Проверка (review.html)

Позиции сеткой по разделам каталога: выбранное фото крупно, ниже 4 варианта.
Клик по варианту выбирает его для всех фасовок товара; «Подходит» отмечает позицию просмотренной, «Нет фото» — оставить заглушку.
Красная метка «край кадра» — товар касался края исходного фото, проверьте, не обрезан ли.
Выбор сохраняется в `selection.json` сразу. Клавиши: ↑/↓, 1–4, Enter, 0.

Заменять фото, если: не тот товар или не та форма/размер (сверяться с названием и характеристиками — товары похожи, ФТ-Т-02);
на фото люди, текст, чужие логотипы, водяные знаки; после удаления фона товар обрезан или «рваный».

## Свои фото заказчика

Шаг `fetch` (Pexels) пропускается — ключ не нужен:

```bash
python3 tools/photos/photos.py items
python3 tools/photos/photos.py local ~/Загрузки/фото-заказчика   # файлы <артикул>.jpg, <артикул>_2.jpg …
python3 tools/photos/photos.py process && python3 tools/photos/photos.py serve
python3 tools/photos/photos.py build
```

Имя файла — артикул (`CHO-D54-100.jpg`) или имя из `image_name`; несколько снимков одного товара — `_2`, `_3`.
Подходит любой формат, который открывает Pillow; поворот снимка с телефона учитывается.
Свои фото заменяют стоковых кандидатов только у этих позиций — остальные остаются с Pexels.

## Лицензия

Фото Pexels бесплатны для коммерческого использования, указание автора не обязательно.
Авторы и ссылки всё равно сохраняются в `frontend/public/img/products/CREDITS.json`.
