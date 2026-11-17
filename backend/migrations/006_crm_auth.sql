-- Вход и роли, журнал заказа, сборка с нехваткой, частичная оплата, аналитика.
-- Требования — ТЗ BAKELOOM v1.0 от 15.11.2026, номера пунктов в комментариях.

-- ─── Учётные записи ──────────────────────────────────────────────────────────
-- Сотрудники входят под личной учётной записью, роль определяет доступ
-- (ФТ-Р-01). Роль admin — владелец магазина.
ALTER TABLE employees ADD COLUMN login         TEXT;
ALTER TABLE employees ADD COLUMN password_hash TEXT;
ALTER TABLE employees ADD COLUMN last_login_at TEXT;
CREATE UNIQUE INDEX idx_employees_login ON employees(login);

-- Покупатель входит, чтобы видеть персональные цены и свои заказы
-- (ФТ-ЛК-01, ФТ-ЛК-02). phone_norm — последние 10 цифр телефона: по нему
-- заказы с сайта и из мессенджеров склеиваются в одного клиента (ФТ-КЛ-01).
ALTER TABLE clients ADD COLUMN phone_norm    TEXT;
ALTER TABLE clients ADD COLUMN password_hash TEXT;
ALTER TABLE clients ADD COLUMN last_login_at TEXT;
CREATE INDEX idx_clients_phone ON clients(phone_norm);

-- В базе хранится только хеш токена: утечка таблицы не даёт войти.
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('employee', 'client')),
    subject_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    expires_at TEXT NOT NULL
);
CREATE INDEX idx_sessions_subject ON sessions(kind, subject_id);

-- ─── Строки заказа ───────────────────────────────────────────────────────────
-- price_kind — по какой цене посчитана строка: retail | wholesale | personal.
-- picked и short_milli — отметки кладовщика при сборке: галочка у собранной
-- позиции и сколько не хватило (ФТ-СК-07, ФТ-З-08).
ALTER TABLE order_items ADD COLUMN price_kind  TEXT;
ALTER TABLE order_items ADD COLUMN picked      INTEGER NOT NULL DEFAULT 0 CHECK (picked IN (0, 1));
ALTER TABLE order_items ADD COLUMN short_milli INTEGER NOT NULL DEFAULT 0 CHECK (short_milli >= 0);

-- ─── Заказы ──────────────────────────────────────────────────────────────────
-- Новое: частичная оплата (предоплата), признак «срочный» (ФТ-М-11),
-- признак «проблемный» с описанием (ФТ-З-08). CHECK в SQLite на месте
-- не меняется, поэтому таблица пересобирается.
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
    payment_method   TEXT NOT NULL DEFAULT 'none'
                       CHECK (payment_method IN ('none','cash','kaspi_transfer','online')),
    payment_status   TEXT NOT NULL DEFAULT 'unpaid'
                       CHECK (payment_status IN ('unpaid','partial','paid','refunded')),
    is_urgent        INTEGER NOT NULL DEFAULT 0 CHECK (is_urgent  IN (0, 1)),
    is_problem       INTEGER NOT NULL DEFAULT 0 CHECK (is_problem IN (0, 1)),
    problem_note     TEXT,
    note             TEXT,
    source           TEXT,
    assigned_to      TEXT REFERENCES employees(id),
    cancelled_reason TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT INTO orders_new (id, order_no, client_id, client_name, client_phone, status,
                        items_tiyn, delivery_tiyn, total_tiyn, paid_tiyn, channel, city,
                        delivery_method, delivery_address, payment_method, payment_status,
                        note, source, assigned_to, cancelled_reason, created_at, updated_at)
SELECT id, order_no, client_id, client_name, client_phone, status,
       items_tiyn, delivery_tiyn, total_tiyn, paid_tiyn, channel, city,
       delivery_method, delivery_address, payment_method, payment_status,
       note, source, assigned_to, cancelled_reason, created_at, updated_at
FROM orders;

DROP TABLE orders;
ALTER TABLE orders_new RENAME TO orders;

CREATE INDEX idx_orders_status  ON orders(status);
CREATE INDEX idx_orders_created ON orders(created_at DESC);
CREATE INDEX idx_orders_client  ON orders(client_id);
CREATE INDEX idx_orders_payment ON orders(payment_status);

PRAGMA foreign_keys = ON;

-- ─── Журнал заказа ───────────────────────────────────────────────────────────
-- Кто, когда и что изменил: статус, состав, оплату, адрес (ФТ-Р-06).
-- Записи не удаляются и не правятся (ФТ-Р-07). amount_tiyn — сумма
-- для событий оплаты и возврата: по ней дашборд считает поступления.
CREATE TABLE order_events (
    id          TEXT PRIMARY KEY,
    order_id    TEXT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    message     TEXT NOT NULL,
    amount_tiyn INTEGER,
    actor_kind  TEXT NOT NULL DEFAULT 'system'
                  CHECK (actor_kind IN ('employee', 'client', 'system')),
    actor_id    TEXT,
    actor_name  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_events_order ON order_events(order_id, created_at);
CREATE INDEX idx_events_kind  ON order_events(kind, created_at);

-- ─── Посещения сайта ─────────────────────────────────────────────────────────
-- Для раздела аналитики: с каких устройств и браузеров заходят (ФТ-В-04).
-- Хранится только тип устройства, браузер и система, без IP.
CREATE TABLE visits (
    id         TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    path       TEXT,
    referrer   TEXT,
    device     TEXT NOT NULL,
    browser    TEXT NOT NULL,
    os         TEXT NOT NULL,
    session_id TEXT,
    client_id  TEXT
);
CREATE INDEX idx_visits_created ON visits(created_at);
