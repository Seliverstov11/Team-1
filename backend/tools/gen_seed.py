# -*- coding: utf-8 -*-
"""Генерация демо-данных из таблицы заказчика.

    python3 tools/xlsx2tsv.py <таблица>.xlsx > /tmp/data.tsv
    python3 tools/gen_seed.py /tmp/data.tsv --per-category 5 > migrations/007_seed_sample.sql

--per-category N — сколько карточек товара взять в каждую категорию. Заказчик
просил не переносить весь каталог: часть позиций в его таблице уже не
продаётся, для разработки достаточно категорий и нескольких товаров в каждую,
реальное наполнение он сделает сам (встреча 3, 28:37–29:08). Карточка — это
группа фасовок с общим «ID товара», поэтому строк товаров выходит больше N.
В первую очередь берутся товары из примеров заказов, чтобы заказы сохранили
связь с каталогом. Без флага переносится всё.
"""
import argparse, sys, re, datetime
from collections import OrderedDict, defaultdict

ap = argparse.ArgumentParser()
ap.add_argument('src')
ap.add_argument('--per-category', type=int, default=None)
args = ap.parse_args()

sheets, cur = OrderedDict(), None
for line in open(args.src, encoding='utf8'):
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

def num(v):
    return (v or '').replace(' ', '').replace(' ', '').replace(',', '.')

def tiyn(v):
    v = num(v)
    return int(round(float(v) * 100)) if v else 0

def milli(v):
    v = num(v)
    return int(round(float(v) * 1000)) if v else 0

def intval(v):
    m = re.search(r'\d+', (v or '').replace(' ', ''))
    return int(m.group(0)) if m else 'NULL'

def serial_date(v):
    try: n = int(float(v))
    except ValueError: return None
    return (datetime.date(1899, 12, 30) + datetime.timedelta(days=n)).isoformat()

def phone_norm(p):
    d = re.sub(r'\D', '', p or '')
    return d[-10:] if len(d) >= 10 else (d or None)

TR = {'а':'a','б':'b','в':'v','г':'g','д':'d','е':'e','ё':'e','ж':'zh','з':'z','и':'i','й':'y',
      'к':'k','л':'l','м':'m','н':'n','о':'o','п':'p','р':'r','с':'s','т':'t','у':'u','ф':'f',
      'х':'h','ц':'c','ч':'ch','ш':'sh','щ':'sch','ъ':'','ы':'y','ь':'','э':'e','ю':'yu','я':'ya'}
def slugify(s):
    s = ''.join(TR.get(ch, ch) for ch in s.lower())
    s = re.sub(r'[^a-z0-9]+', '-', s).strip('-')
    return s or 'cat'

cat_rows = [r + [''] * (19 - len(r)) for r in rows('Каталог') if r[0].strip()]
order_rows = [r + [''] * (18 - len(r)) for r in rows('Заказы') if r[0].strip()]
item_rows = [r + [''] * (9 - len(r)) for r in rows('Состав заказов') if r[0].strip()]

# ─── Отбор товаров ───────────────────────────────────────────────────────────
order_skus = {r[2].strip() for r in item_rows}
groups = defaultdict(OrderedDict)            # категория → группа → [строки]
for r in cat_rows:
    groups[r[2].strip()].setdefault(r[1].strip(), []).append(r)

chosen = []
for cat, g in groups.items():
    in_orders = [k for k, v in g.items() if any(x[0].strip() in order_skus for x in v)]
    order = in_orders + [k for k in g if k not in in_orders]
    keep = order if args.per_category is None else order[:args.per_category]
    for k in keep:
        chosen.extend(g[k])
chosen_skus = {r[0].strip() for r in chosen}

out = []
w = out.append
w("-- Демо-данные BAKELOOM из таблицы заказчика (передана 12.11.2026).")
w("-- Сгенерировано tools/gen_seed.py: правки вносить в таблицу и перегенерировать, а не руками.")
if args.per_category:
    w(f"-- В каталоге по {args.per_category} карточек на категорию: заказчик просил не переносить весь")
    w("-- каталог, часть позиций устарела, наполнение он сделает сам (встреча 3, 28:37–29:08).")
w("-- Резерв пересчитан по активным заказам из примеров, чтобы отмена заказа")
w("-- возвращала ровно то, что было зарезервировано.")
w("")
for t in ('order_events', 'order_items', 'stock_movements', 'orders', 'client_prices',
          'product_attributes', 'products', 'categories', 'sessions', 'clients', 'employees', 'visits'):
    w(f"DELETE FROM {t};")
w("")

# ─── Категории ───────────────────────────────────────────────────────────────
cats = OrderedDict()
for r in cat_rows:
    name = r[2].strip()
    if name and name not in cats:
        cats[name] = 'c-' + slugify(name)
w("INSERT INTO categories (id, name, slug, sort_order) VALUES")
w(',\n'.join(f"  ({q(cid)}, {q(name)}, {q(slugify(name))}, {i})"
             for i, (name, cid) in enumerate(cats.items(), 1)) + ';')
w("")

# ─── Остатки ─────────────────────────────────────────────────────────────────
stock = {}
for r in rows('Остатки'):
    r = r + [''] * (11 - len(r))
    stock[r[0].strip()] = dict(on_hand=r[4], min_stock=r[7], place=r[9].strip())

ACTIVE = {'Новый', 'Подтверждён', 'Собран'}     # резерв ставится при оформлении
active_orders = {r[0].strip() for r in order_rows if r[9].strip() in ACTIVE}
reserved = defaultdict(int)
for r in item_rows:
    if r[0].strip() in active_orders:
        reserved[r[2].strip()] += milli(r[5])

# ─── Товары ──────────────────────────────────────────────────────────────────
ATTR_COLS = [(10, 'Вес / размер'), (11, 'Состав / материал'), (12, 'Цвет'),
             (13, 'Срок годности'), (14, 'Условия хранения')]
prods, attrs = [], []
for r in chosen:
    sku = r[0].strip()
    pid = 'p-' + sku.lower()
    st = stock.get(sku, {})
    on_hand = milli(st.get('on_hand', 0))
    res = reserved.get(sku, 0)
    on_hand = max(on_hand, res)
    prods.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q(pid), q(cats[r[2].strip()]), q(r[3].strip()), q(sku),
        q(r[15].strip() or None), tiyn(r[7]), q(r[5].strip() or 'шт'),
        on_hand, res, milli(st.get('min_stock', 0)) or 'NULL',
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

# ─── Сотрудники ──────────────────────────────────────────────────────────────
# Пароли не хранятся в репозитории: при первом старте backend выставляет
# пароль из BOOTSTRAP_PASSWORD всем, у кого его ещё нет (см. README).
emps = OrderedDict()
emps['Владелец'] = ('e-owner', 'owner', 'admin')
for r in order_rows:
    nm = r[16].strip()
    if nm and nm not in emps:
        login = slugify(nm.split()[-1])
        emps[nm] = ('e-' + slugify(nm), login, 'manager')
emps['Кладовщик'] = ('e-sklad', 'sklad', 'warehouse')
w("INSERT INTO employees (id, name, login, role) VALUES")
w(',\n'.join(f"  ({q(i)}, {q(n)}, {q(l)}, {q(role)})" for n, (i, l, role) in emps.items()) + ';')
w("")

# ─── Клиенты ─────────────────────────────────────────────────────────────────
CLIENT_TYPE = {'Домашний кондитер': 'home_baker', 'Кондитерская': 'bakery',
               'Кафе': 'cafe', 'Магазин': 'shop'}
clients = OrderedDict()
for r in order_rows:
    key = (r[3].strip(), r[5].strip())
    if key[0] and key not in clients:
        clients[key] = dict(id='cl-%03d' % (len(clients) + 1), city=r[6].strip(),
                            ctype=CLIENT_TYPE.get(r[4].strip(), 'other'))
w("INSERT INTO clients (id, name, phone, phone_norm, city, client_type) VALUES")
w(',\n'.join("  ({}, {}, {}, {}, {}, {})".format(
    q(c['id']), q(nm), q(ph or None), q(phone_norm(ph)), q(c['city'] or None), q(c['ctype']))
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

olines, events = [], []
for r in order_rows:
    no = r[0].strip()
    cl = clients.get((r[3].strip(), r[5].strip()))
    created = (serial_date(r[1]) or '2026-11-12') + 'T09:00:00Z'
    olines.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q('o-' + no), q(no), q(cl['id'] if cl else None), q(r[3].strip()), q(r[5].strip() or None),
        q(STATUS.get(r[9].strip(), 'new')), tiyn(r[12]), tiyn(r[13]), tiyn(r[14]), tiyn(r[15]),
        q(CHANNEL.get(r[2].strip(), 'other')), q(r[6].strip() or None),
        q(DELIVERY.get(r[7].strip(), 'pickup')), q(r[8].strip() or None),
        q(PAY.get(r[10].strip(), 'none')), q(PAYST.get(r[11].strip(), 'unpaid')),
        q(emps.get(r[16].strip(), (None,))[0]), q(r[17].strip() or None), q(created)))
    events.append(f"  ({q('ev-' + no)}, {q('o-' + no)}, 'created', "
                  f"{q('Заказ перенесён из таблицы заказчика')}, 'system', {q(created)})")
w("INSERT INTO orders (id, order_no, client_id, client_name, client_phone, status,")
w("                    items_tiyn, delivery_tiyn, total_tiyn, paid_tiyn,")
w("                    channel, city, delivery_method, delivery_address,")
w("                    payment_method, payment_status, assigned_to, note, created_at) VALUES")
w(',\n'.join(olines) + ';')
w("")

unit_by_sku = {r[0].strip(): (r[5].strip() or 'шт') for r in cat_rows}
ilines, unlinked = [], 0
for r in item_rows:
    no, ln, sku = r[0].strip(), r[1].strip(), r[2].strip()
    linked = sku in chosen_skus
    unlinked += not linked
    disc = intval(r[7])
    ilines.append("  ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})".format(
        q(f'oi-{no}-{ln}'), q('o-' + no), q('p-' + sku.lower()) if linked else 'NULL',
        q(r[3].strip() + (', ' + r[4].strip() if r[4].strip() else '')),
        q(sku), q(unit_by_sku.get(sku, 'шт')), tiyn(r[6]), milli(r[5]), disc if disc != 'NULL' else 0, tiyn(r[8]),
        1 if r[0].strip() not in active_orders else 0))
w("-- Название строки сохраняется снимком вместе с фасовкой. Товары, которые не попали")
w("-- в сокращённый каталог, остаются в заказе без ссылки на карточку.")
w("INSERT INTO order_items (id, order_id, product_id, product_name, product_sku, unit,")
w("                         price_tiyn, quantity_milli, discount_percent, subtotal_tiyn, picked) VALUES")
w(',\n'.join(ilines) + ';')
w("")
w("INSERT INTO order_events (id, order_id, kind, message, actor_kind, created_at) VALUES")
w(',\n'.join(events) + ';')
print('\n'.join(out))
sys.stderr.write(f"категорий {len(cats)}, товаров {len(prods)}, характеристик {len(attrs)}, "
                 f"сотрудников {len(emps)}, клиентов {len(clients)}, заказов {len(olines)}, "
                 f"строк заказов {len(ilines)} (без ссылки на товар: {unlinked})\n")
