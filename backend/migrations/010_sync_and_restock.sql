-- 010: избранное и корзина вошедшего покупателя — одни и те же на телефоне и компьютере;
-- заявки «Сообщить о поступлении» на товары, которых нет в наличии.
-- Номер 010, а не 009: 009_product_images занят веткой фото (#46).

CREATE TABLE IF NOT EXISTS client_favorites (
    client_id  TEXT NOT NULL REFERENCES clients(id),
    item_key   TEXT NOT NULL,                  -- id карточки, как на сайте: group_id или id товара
    created_at TEXT NOT NULL,
    PRIMARY KEY (client_id, item_key)
);

CREATE TABLE IF NOT EXISTS client_cart (
    client_id  TEXT NOT NULL REFERENCES clients(id),
    product_id TEXT NOT NULL REFERENCES products(id),
    qty        INTEGER NOT NULL CHECK (qty > 0),  -- штук (упаковок), как в корзине сайта
    updated_at TEXT NOT NULL,
    PRIMARY KEY (client_id, product_id)
);

CREATE TABLE IF NOT EXISTS restock_requests (
    id          TEXT PRIMARY KEY,
    product_id  TEXT NOT NULL REFERENCES products(id),
    client_id   TEXT REFERENCES clients(id),     -- если покупатель вошёл на сайт
    name        TEXT NOT NULL,
    phone       TEXT NOT NULL,
    phone_norm  TEXT NOT NULL,
    qty         INTEGER,                         -- сколько хочет, если указал
    status      TEXT NOT NULL DEFAULT 'waiting'  -- waiting | notified | cancelled
                CHECK (status IN ('waiting', 'notified', 'cancelled')),
    created_at  TEXT NOT NULL,
    handled_by  TEXT REFERENCES employees(id),
    handled_at  TEXT
);
CREATE INDEX IF NOT EXISTS restock_by_status ON restock_requests(status, product_id);
