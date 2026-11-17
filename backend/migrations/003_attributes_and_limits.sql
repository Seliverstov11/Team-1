-- Требования третьей встречи с заказчиком.
--
-- Характеристики динамические (29:11): «чтобы и категорию можно было добавить,
-- и товар в неё, и характеристики чтобы были динамические. Для всех только
-- цвет, вес и материал. Для свечек ещё тип. Когда добавляешь новый товар,
-- для него можно самому придумать характеристику и добавить».
-- Поэтому не колонки в products, а пары «название — значение».

CREATE TABLE product_attributes (
    id         TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    value      TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_attrs_product ON product_attributes(product_id);
CREATE UNIQUE INDEX idx_attrs_unique ON product_attributes(product_id, name);

-- Минимум и шаг заказа (17:08): «продавался от, например, от 500 грамм,
-- то есть и сделать шаг какой-то, по сколько можно увеличить».
-- По умолчанию одна единица — для штучного товара это и есть нужное поведение.
ALTER TABLE products ADD COLUMN min_order_milli  INTEGER NOT NULL DEFAULT 1000;
ALTER TABLE products ADD COLUMN order_step_milli INTEGER NOT NULL DEFAULT 1000;

-- Адресное хранение (26:52): «у нас есть, где на складе оно лежит. Полка
-- такая-то. Когда принимаем, мы сразу указываем, куда мы его принимаем».
-- Сканера штрихкодов у заказчика нет, всё вручную.
ALTER TABLE products        ADD COLUMN storage_location TEXT;
ALTER TABLE stock_movements ADD COLUMN storage_location TEXT;

-- ─── Демо-значения ───────────────────────────────────────────────────────────

UPDATE products SET min_order_milli = 500, order_step_milli = 100, storage_location = 'A-03'
  WHERE sku = 'CB-54-1KG';
UPDATE products SET min_order_milli = 500, order_step_milli = 500, storage_location = 'A-06'
  WHERE sku = 'MS-1000W';
UPDATE products SET storage_location = 'B-02' WHERE sku = 'KR-0412';
UPDATE products SET storage_location = 'C-11' WHERE sku = 'UP-3030';
UPDATE products SET storage_location = 'C-08' WHERE sku = 'PD-32G';
UPDATE products SET storage_location = 'B-07' WHERE sku = 'PS-0050';
UPDATE products SET storage_location = 'D-01' WHERE sku = 'SV-25G';
UPDATE products SET storage_location = 'E-04' WHERE sku = 'FM-0022';

INSERT INTO product_attributes (id, product_id, name, value, sort_order)
SELECT 'at-choc-1', id, 'Производитель',       'Callebaut',              1 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-2', id, 'Страна',     'Бельгия',                2 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-3', id, 'Какао',      '54,5 %',                 3 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-4', id, 'Текучесть',  '3 капли',                4 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-5', id, 'Температура','45 → 27 → 31–32 °C',     5 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-6', id, 'Хранение',   '+15…+18 °C, без запахов',6 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-choc-7', id, 'Срок годности','24 месяца',            7 FROM products WHERE sku = 'CB-54-1KG'
UNION ALL SELECT 'at-mast-1', id, 'Цвет',       'Белый',                  1 FROM products WHERE sku = 'MS-1000W'
UNION ALL SELECT 'at-mast-2', id, 'Вес',        '1 кг',                   2 FROM products WHERE sku = 'MS-1000W'
UNION ALL SELECT 'at-mast-3', id, 'Назначение', 'Обтяжка и лепка',        3 FROM products WHERE sku = 'MS-1000W'
UNION ALL SELECT 'at-box-1',  id, 'Материал',   'Крафт-картон',           1 FROM products WHERE sku = 'UP-3030'
UNION ALL SELECT 'at-box-2',  id, 'Размеры',    '30 × 30 × 20 см',        2 FROM products WHERE sku = 'UP-3030'
UNION ALL SELECT 'at-box-3',  id, 'Окно',       'Есть',                   3 FROM products WHERE sku = 'UP-3030'
UNION ALL SELECT 'at-cand-1', id, 'Цвет',       'Золото',                 1 FROM products WHERE sku = 'SV-25G'
UNION ALL SELECT 'at-cand-2', id, 'Тип',        'Цифры',                  2 FROM products WHERE sku = 'SV-25G'
UNION ALL SELECT 'at-cand-3', id, 'Высота',     '7 см',                   3 FROM products WHERE sku = 'SV-25G'
UNION ALL SELECT 'at-form-1', id, 'Материал',   'Сталь, антипригарное',   1 FROM products WHERE sku = 'FM-0022'
UNION ALL SELECT 'at-form-2', id, 'Диаметр',    '22 см',                  2 FROM products WHERE sku = 'FM-0022';
