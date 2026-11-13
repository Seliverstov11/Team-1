-- Схема под боевые данные заказчика (таблица от 12.11.2026).
-- Снимок таблицы показал четыре вещи, которых схема не покрывала.

-- 1. Варианты товара (29:11, 18:09). В таблице один товар — это несколько
-- строк с общим «ID товара»: CHO-D54 продаётся как 100 г, 500 г и 1 кг,
-- у каждой фасовки свои цена и остаток. Карточка на сайте одна,
-- переключатель внутри. Поэтому не отдельная таблица вариантов, а группа:
-- строка товара остаётся единицей продажи, group_id склеивает их в карточку.
ALTER TABLE products ADD COLUMN group_id     TEXT;
ALTER TABLE products ADD COLUMN variant_name TEXT;
CREATE INDEX idx_products_group ON products(group_id);

-- 2. Оптовая цена. В таблице два прайса: розница и опт «от N единиц».
-- Это не скидка и не промокод, которых заказчик не захотел, а второй
-- уровень цены, который считается по количеству в строке заказа.
ALTER TABLE products ADD COLUMN wholesale_price_tiyn INTEGER
    CHECK (wholesale_price_tiyn IS NULL OR wholesale_price_tiyn >= 0);
ALTER TABLE products ADD COLUMN wholesale_min_qty    INTEGER;

-- Фасовка поставки приходит текстом («200 шт.», «1 рулон»): это справочная
-- величина для склада, не число для расчётов.
ALTER TABLE products ADD COLUMN supply_pack_size TEXT;
-- Фотографий заказчик не передал, подбирает команда. В таблице указано имя
-- будущего файла — храним его отдельно от готового image_url.
ALTER TABLE products ADD COLUMN image_name TEXT;

-- 3. Персональные цены (28:03): «клиент заходит под своей учётной записью
-- и видит цены, отличные от общих».
ALTER TABLE clients ADD COLUMN city        TEXT;
ALTER TABLE clients ADD COLUMN client_type TEXT NOT NULL DEFAULT 'other'
    CHECK (client_type IN ('home_baker','bakery','cafe','shop','other'));

CREATE TABLE client_prices (
    client_id  TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    product_id TEXT NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    price_tiyn INTEGER NOT NULL CHECK (price_tiyn >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (client_id, product_id)
);

-- 4. Заказы. В таблице у заказа есть номер, канал, город, способ получения
-- и стоимость доставки, а оплата хранится отдельно от самого заказа —
-- «способ и факт оплаты хранятся отдельно». Статусов в данных восемь,
-- в прежнем CHECK было шесть. CHECK в SQLite не меняется на месте,
-- поэтому таблица пересобирается.
ALTER TABLE order_items ADD COLUMN discount_percent INTEGER NOT NULL DEFAULT 0
    CHECK (discount_percent BETWEEN 0 AND 100);

PRAGMA foreign_keys = OFF;

CREATE TABLE orders_new (
    id               TEXT PRIMARY KEY,
    order_no         TEXT UNIQUE,
    client_id        TEXT REFERENCES clients(id),
    client_name      TEXT NOT NULL,
    client_phone     TEXT,
    status           TEXT NOT NULL DEFAULT 'new'
                       CHECK (status IN ('new','confirmed','assembled','handed_over',
                                         'shipped','delivered','completed','cancelled')),
    items_tiyn       INTEGER NOT NULL DEFAULT 0 CHECK (items_tiyn    >= 0),
    delivery_tiyn    INTEGER NOT NULL DEFAULT 0 CHECK (delivery_tiyn >= 0),
    total_tiyn       INTEGER NOT NULL DEFAULT 0 CHECK (total_tiyn    >= 0),
    paid_tiyn        INTEGER NOT NULL DEFAULT 0 CHECK (paid_tiyn     >= 0),
    channel          TEXT NOT NULL DEFAULT 'site'
                       CHECK (channel IN ('site','site_legacy','whatsapp','instagram','phone','other')),
    city             TEXT,
    delivery_method  TEXT NOT NULL DEFAULT 'pickup'
                       CHECK (delivery_method IN ('pickup','courier','cdek','kazpost')),
    delivery_address TEXT,
    -- Онлайн-оплаты не будет (26:01): статический QR Kaspi и наличные.
    -- Система о платеже не узнаёт, поэтому факт оплаты ставит менеджер.
    payment_method   TEXT NOT NULL DEFAULT 'none'
                       CHECK (payment_method IN ('none','cash','kaspi_transfer','online')),
    payment_status   TEXT NOT NULL DEFAULT 'unpaid'
                       CHECK (payment_status IN ('unpaid','paid','refunded')),
    note             TEXT,
    source           TEXT,
    assigned_to      TEXT REFERENCES employees(id),
    cancelled_reason TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT INTO orders_new (id, client_id, client_name, client_phone, status, items_tiyn,
                        total_tiyn, note, source, assigned_to, cancelled_reason,
                        created_at, updated_at)
SELECT id, client_id, client_name, client_phone,
       CASE status WHEN 'assembling' THEN 'assembled' WHEN 'ready' THEN 'assembled' ELSE status END,
       total_tiyn, total_tiyn, note, source, assigned_to, cancelled_reason, created_at, updated_at
FROM orders;

DROP TABLE orders;
ALTER TABLE orders_new RENAME TO orders;

CREATE INDEX idx_orders_status   ON orders(status);
CREATE INDEX idx_orders_created  ON orders(created_at DESC);
CREATE INDEX idx_orders_client   ON orders(client_id);
CREATE INDEX idx_orders_payment  ON orders(payment_status);

PRAGMA foreign_keys = ON;
