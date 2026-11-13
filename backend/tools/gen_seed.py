# -*- coding: utf-8 -*-
"""Генерация 005_catalog_from_customer.sql из таблицы заказчика."""
import sys, re, datetime
from collections import OrderedDict

src = sys.argv[1]
sheets, cur = OrderedDict(), None
for line in open(src, encoding='utf8'):
    line = line.rstrip('\n')
    if line.startswith('===SHEET=== '):
        cur = line[len('===SHEET=== '):]; sheets[cur] = []
    elif cur:
        sheets[cur].append(line.split('\t'))

def rows(name):
    data = [r for r in sheets[name] if any(c.strip() for c in r)]
    return data[1:]            # без заголовка

def q(s):
    if s is None or s == '': return 'NULL'
    return "'" + str(s).replace("'", "''") + "'"

def tiyn(v):
    v = (v or '').replace(' ', '').replace(' ', '').replace(',', '.')
    if not v: return 0
    return int(round(float(v) * 100))

def milli(v):
    v = (v or '').replace(' ', '').replace(' ', '').replace(',', '.')
    if not v: return 0
    return int(round(float(v) * 1000))

def intval(v):
    m = re.search(r'\d+', (v or '').replace(' ', ''))
    return int(m.group(0)) if m else 'NULL'

def serial_date(v):
    try: n = int(float(v))
    except ValueError: return None
    return (datetime.date(1899, 12, 30) + datetime.timedelta(days=n)).isoformat()

TR = {'а':'a','б':'b','в':'v','г':'g','д':'d','е':'e','ё':'e','ж':'zh','з':'z','и':'i','й':'y',
      'к':'k','л':'l','м':'m','н':'n','о':'o','п':'p','р':'r','с':'s','т':'t','у':'u','ф':'f',
      'х':'h','ц':'c','ч':'ch','ш':'sh','щ':'sch','ъ':'','ы':'y','ь':'','э':'e','ю':'yu','я':'ya'}
def slugify(s):
    s = ''.join(TR.get(ch, ch) for ch in s.lower())
    s = re.sub(r'[^a-z0-9]+', '-', s).strip('-')
    return s or 'cat'

out = []
w = out.append
w("-- Боевой каталог заказчика. Сгенерирован из «Сладкий_мастер_данные_для_разработки_1.xlsx»")
w("-- (передана 12.11.2026). Правки вносить в таблицу и перегенерировать, а не руками.")
w("--")
w("-- Демо-набор из 002 удаляется: фронт и бэк должны показывать одни и те же данные.")
w("")
w("DELETE FROM order_items;")
w("DELETE FROM stock_movements;")
w("DELETE FROM orders;")
w("DELETE FROM product_attributes;")
w("DELETE FROM products;")
w("DELETE FROM categories;")
w("DELETE FROM clients;")
w("DELETE FROM employees;")
w("")

# ─── Категории ───────────────────────────────────────────────────────────────
cat_rows = rows('Каталог')
cats = OrderedDict()
for r in cat_rows:
    name = r[2].strip()
    if name and name not in cats:
        cats[name] = 'c-' + slugify(name)
w("INSERT INTO categories (id, name, slug, sort_order) VALUES")
vals = [f"  ({q(cid)}, {q(name)}, {q(slugify(name))}, {i})"
        for i, (name, cid) in enumerate(cats.items(), 1)]
w(',\n'.join(vals) + ';')
w("")

# ─── Остатки по SKU ──────────────────────────────────────────────────────────
stock = {}
for r in rows('Остатки'):
    r = r + [''] * (11 - len(r))
    stock[r[0].strip()] = dict(on_hand=r[4], reserved=r[5], min_stock=r[7], place=r[9].strip())

# ─── Товары ──────────────────────────────────────────────────────────────────
ATTR_COLS = [(10, 'Вес / размер'), (11, 'Состав / материал'), (12, 'Цвет'),
             (13, 'Срок годности'), (14, 'Условия хранения')]
prods, attrs = [], []
for r in cat_rows:
    r = r + [''] * (19 - len(r))
    sku = r[0].strip()
    if not sku: continue
    pid = 'p-' + sku.lower()
    st = stock.get(sku, {})
    prods.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q(pid), q(cats[r[2].strip()]), q(r[3].strip()), q(sku),
        q(r[15].strip() or None), tiyn(r[7]), q(r[5].strip() or 'шт'),
        milli(st.get('on_hand', 0)), milli(st.get('reserved', 0)),
        milli(st.get('min_stock', 0)) or 'NULL',
        q('g-' + r[1].strip().lower()), q(r[4].strip() or None),
        tiyn(r[8]) if r[8].strip() else 'NULL',
        intval(r[9]), q(r[6].strip() or None),
        q(st.get('place') or None), q(r[16].strip() or None)))
    n = 0
    for idx, label in ATTR_COLS:
        v = r[idx].strip()
        if v:
            n += 1
            attrs.append(f"  ({q(f'{pid}-a{n}')}, {q(pid)}, {q(label)}, {q(v)}, {n})")

w("INSERT INTO products (id, category_id, name, sku, description, price_tiyn, unit,")
w("                      stock_milli, reserved_milli, min_stock_milli,")
w("                      group_id, variant_name, wholesale_price_tiyn, wholesale_min_qty,")
w("                      supply_pack_size, storage_location, image_name) VALUES")
w(',\n'.join(prods) + ';')
w("")
w("INSERT INTO product_attributes (id, product_id, name, value, sort_order) VALUES")
w(',\n'.join(attrs) + ';')
w("")

# ─── Сотрудники и клиенты ────────────────────────────────────────────────────
order_rows = rows('Заказы')
emps = OrderedDict()
for r in order_rows:
    nm = r[16].strip()
    if nm and nm not in emps: emps[nm] = 'e-' + slugify(nm)
w("INSERT INTO employees (id, name, role) VALUES")
w(',\n'.join(f"  ({q(i)}, {q(n)}, 'manager')" for n, i in emps.items()) + ';')
w("")

CLIENT_TYPE = {'Домашний кондитер': 'home_baker', 'Кондитерская': 'bakery',
               'Кафе': 'cafe', 'Магазин': 'shop'}
clients = OrderedDict()
for r in order_rows:
    key = (r[3].strip(), r[5].strip())
    if key[0] and key not in clients:
        clients[key] = dict(id='cl-%03d' % (len(clients) + 1), city=r[6].strip(),
                            ctype=CLIENT_TYPE.get(r[4].strip(), 'other'))
w("INSERT INTO clients (id, name, phone, city, client_type) VALUES")
w(',\n'.join("  ({}, {}, {}, {}, {})".format(q(c['id']), q(nm), q(ph or None), q(c['city'] or None), q(c['ctype']))
             for (nm, ph), c in clients.items()) + ';')
w("")

# ─── Заказы ──────────────────────────────────────────────────────────────────
STATUS = {'Новый': 'new', 'Подтверждён': 'confirmed', 'Собран': 'assembled',
          'Передан в доставку': 'handed_over', 'Отправлен': 'shipped',
          'Доставлен': 'delivered', 'Завершён': 'completed', 'Отменён': 'cancelled'}
CHANNEL = {'Сайт': 'site', 'Сайт (старая страница)': 'site_legacy', 'WhatsApp': 'whatsapp',
           'Instagram': 'instagram', 'Телефон': 'phone'}
DELIVERY = {'Самовывоз': 'pickup', 'Курьер по городу': 'courier',
            'СДЭК': 'cdek', 'Казпочта': 'kazpost'}
PAY = {'Наличные': 'cash', 'Kaspi перевод': 'kaspi_transfer', 'Онлайн': 'online', 'Не выбран': 'none'}
PAYST = {'Оплачен': 'paid', 'Не оплачен': 'unpaid', 'Возврат': 'refunded'}

olines = []
for r in order_rows:
    r = r + [''] * (18 - len(r))
    num = r[0].strip()
    if not num: continue
    cl = clients.get((r[3].strip(), r[5].strip()))
    created = serial_date(r[1]) or '2026-11-12'
    olines.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q('o-' + num), q(num), q(cl['id'] if cl else None), q(r[3].strip()), q(r[5].strip() or None),
        q(STATUS.get(r[9].strip(), 'new')), tiyn(r[12]), tiyn(r[13]), tiyn(r[14]), tiyn(r[15]),
        q(CHANNEL.get(r[2].strip(), 'other')), q(r[6].strip() or None),
        q(DELIVERY.get(r[7].strip(), 'pickup')), q(r[8].strip() or None),
        q(PAY.get(r[10].strip(), 'none')), q(PAYST.get(r[11].strip(), 'unpaid')),
        q(emps.get(r[16].strip())), q(r[17].strip() or None)))
w("INSERT INTO orders (id, order_no, client_id, client_name, client_phone, status,")
w("                    items_tiyn, delivery_tiyn, total_tiyn, paid_tiyn,")
w("                    channel, city, delivery_method, delivery_address,")
w("                    payment_method, payment_status, assigned_to, note, created_at) VALUES")
w(',\n'.join(l[:-1] + ", '" + (serial_date(r[1]) or '2026-11-12') + "T09:00:00Z')"
             for l, r in zip(olines, [x for x in order_rows if x[0].strip()])) + ';')
w("")

ilines = []
for r in rows('Состав заказов'):
    r = r + [''] * (9 - len(r))
    num, ln = r[0].strip(), r[1].strip()
    if not num: continue
    sku = r[2].strip()
    ilines.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q(f'oi-{num}-{ln}'), q('o-' + num), q('p-' + sku.lower()), q(r[3].strip()),
        q(sku), q(r[4].strip() or 'шт'), tiyn(r[6]), milli(r[5]),
        intval(r[7]) if intval(r[7]) != 'NULL' else 0, tiyn(r[8])))
w("INSERT INTO order_items (id, order_id, product_id, product_name, product_sku, unit,")
w("                         price_tiyn, quantity_milli, discount_percent, subtotal_tiyn) VALUES")
w(',\n'.join(ilines) + ';')
w("")
print('\n'.join(out))
sys.stderr.write(f"категорий {len(cats)}, товаров {len(prods)}, характеристик {len(attrs)}, "
                 f"клиентов {len(clients)}, заказов {len(olines)}, строк заказов {len(ilines)}\n")
