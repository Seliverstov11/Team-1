-- Сладкий Мастер — начальная схема (SQLite / libSQL)
--
-- Деньги и количества хранятся целыми числами: в SQLite нет типа для
-- дробей без потери точности, а float даёт 1880.0000000002 и расхождения
-- в суммах заказов.
--   price_tiyn   — цена в тиынах.    1 880 ₸ = 188000
--   *_milli      — количество ×1000.  2,5 кг  = 2500
-- Пересчёт только при выводе наружу.

CREATE TABLE categories (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    slug       TEXT NOT NULL UNIQUE,
    parent_id  TEXT REFERENCES categories(id),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE products (
    id              TEXT PRIMARY KEY,
    category_id     TEXT NOT NULL REFERENCES categories(id),
    name            TEXT NOT NULL,
    sku             TEXT UNIQUE,
    description     TEXT,
    price_tiyn      INTEGER NOT NULL CHECK (price_tiyn >= 0),
    unit            TEXT NOT NULL DEFAULT 'шт',
    is_weight       INTEGER NOT NULL DEFAULT 0 CHECK (is_weight IN (0, 1)),
    stock_milli     INTEGER NOT NULL DEFAULT 0,
    reserved_milli  INTEGER NOT NULL DEFAULT 0,
    min_stock_milli INTEGER,
    image_url       TEXT,
    is_archived     INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_products_category ON products(category_id);
CREATE INDEX idx_products_archived ON products(is_archived);

CREATE TABLE employees (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    phone      TEXT,
    role       TEXT NOT NULL CHECK (role IN ('admin', 'manager', 'warehouse')),
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE clients (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    phone      TEXT,
    whatsapp   TEXT,
    note       TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE orders (
    id               TEXT PRIMARY KEY,
    client_id        TEXT REFERENCES clients(id),
    client_name      TEXT NOT NULL,
    client_phone     TEXT,
    status           TEXT NOT NULL DEFAULT 'new'
                       CHECK (status IN ('new','confirmed','assembling','ready','delivered','cancelled')),
    total_tiyn       INTEGER NOT NULL DEFAULT 0 CHECK (total_tiyn >= 0),
    note             TEXT,
    source           TEXT CHECK (source IN ('site','whatsapp','phone','other')),
    assigned_to      TEXT REFERENCES employees(id),
    cancelled_reason TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_orders_status  ON orders(status);
CREATE INDEX idx_orders_created ON orders(created_at DESC);

-- Название и цена сохраняются снимком: если потом поднимем цену или
-- переименуем товар, старый заказ не изменится.
CREATE TABLE order_items (
    id             TEXT PRIMARY KEY,
    order_id       TEXT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id     TEXT REFERENCES products(id),
    product_name   TEXT NOT NULL,
    product_sku    TEXT,
    unit           TEXT NOT NULL,
    price_tiyn     INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL CHECK (quantity_milli > 0),
    subtotal_tiyn  INTEGER NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_order_items_order ON order_items(order_id);

CREATE TABLE stock_movements (
    id             TEXT PRIMARY KEY,
    product_id     TEXT NOT NULL REFERENCES products(id),
    movement_type  TEXT NOT NULL
                     CHECK (movement_type IN ('in','out','adjustment','reserve','unreserve')),
    quantity_milli INTEGER NOT NULL,
    reason         TEXT,
    order_id       TEXT REFERENCES orders(id),
    employee_id    TEXT REFERENCES employees(id),
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_movements_product ON stock_movements(product_id);
CREATE INDEX idx_movements_created ON stock_movements(created_at DESC);
