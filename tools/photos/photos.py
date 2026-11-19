# -*- coding: utf-8 -*-
"""Фото товаров: подбор на Pexels, обработка в единый стиль, проверка, выгрузка.

    python3 tools/photos/photos.py items      # позиции из миграций 001–006 → work/items.json
    python3 tools/photos/photos.py fetch      # кандидаты с Pexels → work/candidates.json + work/raw/
    python3 tools/photos/photos.py local DIR  # или свои фото заказчика вместо стока (без Pexels)
    python3 tools/photos/photos.py render     # свои рендеры в Blender по названию и размерам (#56)
    python3 tools/photos/photos.py process    # убрать фон, подложка, 800×800 webp → work/out/
    python3 tools/photos/photos.py serve      # страница проверки http://localhost:8765/review.html
    python3 tools/photos/photos.py build      # по selection.json → frontend/public/img/products + миграция

Подробности — README.md рядом.
"""
import argparse, glob, http.server, json, os, re, shutil, sqlite3, subprocess, sys, time, urllib.parse, urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WORK = HERE / 'work'                       # кэш: исходники и обработанные кандидаты (в .gitignore)
SELECTION = HERE / 'selection.json'        # файл выбора — результат проверки, коммитится
QUERIES = HERE / 'queries.json'            # поисковые запросы по позициям, правится руками
OUT_DIR = ROOT / 'frontend' / 'public' / 'img' / 'products'
MIGRATION = ROOT / 'backend' / 'migrations' / '009_product_images.sql'
DB_RS = ROOT / 'backend' / 'src' / 'db.rs'                  # список миграций бэкенда (include_str!)

SIZE = 800                                  # итоговый квадрат, px
FIELD = 0.12                                # поле от края до товара
BG = (247, 244, 236)                        # brand.light #F7F4EC — подложка
SHADOW = (59, 51, 31)                       # тот же тон, что у shadow-subtle на сайте
CANDIDATES = 4


def load_json(p, default=None):
    return json.loads(Path(p).read_text('utf8')) if Path(p).exists() else default


def save_json(p, data):
    Path(p).parent.mkdir(parents=True, exist_ok=True)
    tmp = Path(str(p) + '.tmp')
    tmp.write_text(json.dumps(data, ensure_ascii=False, indent=1) + '\n', 'utf8')
    tmp.replace(p)


# ---------- items: все позиции каталога заказчика ----------

def cmd_items(_):
    """Полный каталог из 005 (289 позиций); 007 потом оставляет часть — фото нужны всем."""
    db = sqlite3.connect(':memory:')
    for f in sorted(glob.glob(str(ROOT / 'backend/migrations/00[1-6]_*.sql'))):
        db.executescript(Path(f).read_text('utf8'))
    attrs = {}
    for pid, name, value in db.execute('SELECT product_id, name, value FROM product_attributes ORDER BY sort_order'):
        attrs.setdefault(pid, {})[name] = value
    items = [dict(id=r[0], sku=r[1], name=r[2], variant=r[3], group=r[4], image_name=r[5],
                  category=r[6], category_id=r[7], description=r[8], attrs=attrs.get(r[0], {}))
             for r in db.execute('''SELECT p.id, p.sku, p.name, p.variant_name, p.group_id, p.image_name,
                                           c.name, c.id, p.description
                                    FROM products p JOIN categories c ON c.id = p.category_id
                                    ORDER BY c.sort_order, p.rowid''')]
    save_json(WORK / 'items.json', items)

    queries = load_json(QUERIES, {})
    added = 0
    for it in items:
        if it['sku'] not in queries:
            queries[it['sku']] = guess_query(it)
            added += 1
    save_json(QUERIES, {i['sku']: queries[i['sku']] for i in items})
    print(f'позиций: {len(items)}, групп: {len({i["group"] for i in items})}, новых запросов: {added}')


# Pexels ищет заметно лучше по-английски. Правила — от частного к общему;
# запрос конкретной позиции можно поправить в queries.json, он не перезаписывается.
RULES = [
    (r'краситель', lambda it: f'{color_en(it)} food coloring gel'.strip()),
    (r'какао.?порошок', 'cocoa powder'),
    (r'какао.?масло.*порошк', 'cocoa butter powder'),
    (r'какао.?масло', 'cocoa butter'),
    (r'вафельн.*крошк', 'crushed wafer crumbs'),
    (r'термостабильн.*капл', 'chocolate chips'),
    (r'глазурь.*бел|бел.*глазурь', 'white chocolate glaze'),
    (r'глазурь', 'chocolate glaze'),
    (r'розов.*шоколад', 'ruby chocolate'),
    (r'карамел', 'caramel chocolate pieces'),
    (r'шоколадн.*масс', 'melted chocolate'),
    (r'(горьк|т[её]мн).*шоколад|шоколад.*т[её]мн', 'dark chocolate callets'),
    (r'(смесь)?.*бел.*шоколад|шоколад.*бел', 'white chocolate callets'),
    (r'молочн.*шоколад|шоколад.*молочн', 'milk chocolate callets'),
    (r'шоколад', 'chocolate callets'),
    (r'свеча-цифра|свеч.*цифр', 'number candle cake'),
    (r'свечи-фонтан|фонтан', 'sparkler candle'),
    (r'свеч', 'birthday candles'),
    (r'атласн', 'satin ribbon spool'),
    (r'репсов', 'grosgrain ribbon spool'),
    (r'посыпк|драже', 'rainbow sprinkles'),
    (r'топпер', 'happy birthday cake topper'),
    (r'шестигран', 'hexagon cake board'),
    (r'подложк.*(золот|металлиз)|металлизир', 'gold cake board'),
    (r'подложк|круг|поднос', 'white cake board'),
    (r'скоросшиват|папк', 'cardboard file folder'),
    (r'саше', 'paper candy bag'),
    (r'курица гриль', 'paper bag grilled chicken'),
    (r'багетн', 'baguette paper bag'),
    (r'наполнител', 'padded envelope'),
    (r'пакет.*фаст|фаст.*пакет', 'paper food bag'),
    (r'пакет|^b$', 'kraft paper bag handles'),
    (r'фри', 'french fries paper cup'),
    (r'бургер', 'kraft burger box'),
    (r'кофе', 'coffee cup holder'),
    (r'детское меню', 'kids meal box'),
    (r'пицц', 'pizza box'),
    (r'пряник', 'gingerbread cookies box'),
    (r'капкейк', 'cupcake box'),
    (r'макарон', 'macarons box'),
    (r'кулич', 'easter cake box'),
    (r'конфет|под шоколад', 'chocolate gift box'),
    (r'эклер', 'eclairs box'),
    (r'рулет', 'cake roll box'),
    (r'бенто', 'bento cake box'),
    (r'фитнес|эко упаковк', 'kraft food container'),
    (r'пирог', 'pie box'),
    (r'пирожн|десерт', 'pastry box window'),
    (r'печень', 'cookie box'),
    (r'поддон', 'paper pastry tray'),
    (r'подарочн|микрогофр', 'gift box'),
    (r'прозрачн', 'clear plastic cake box'),
    (r'гофро', 'corrugated cardboard box'),
    (r'торт|крышк|дно|коробк', 'cake box'),
]

COLORS = {'красн': 'red', 'син': 'blue', 'ж[её]лт': 'yellow', 'зел[её]н': 'green', 'розов': 'pink',
          'бел': 'white', 'ч[её]рн': 'black', 'золот': 'gold'}


def color_en(it):
    s = f"{it['variant'] or ''} {it['attrs'].get('Цвет', '')}".lower()
    return next((en for ru, en in COLORS.items() if re.search(ru, s)), '')


def guess_query(it):
    s = f"{it['name']} {it['category']}".lower()
    for pat, q in RULES:
        if re.search(pat, s):
            q = q(it) if callable(q) else q
            if 'box' in q and 'крафт' in s and 'kraft' not in q:
                q = 'kraft ' + q
            elif 'box' in q and 'white' not in q and 'kraft' not in q and re.search(r'\bбел', s):
                q = 'white ' + q
            return q
    return it['name']


# ---------- fetch: кандидаты с Pexels ----------

def pexels_key():
    key = os.environ.get('PEXELS_API_KEY')
    if not key and (HERE / '.env').exists():
        m = re.search(r'PEXELS_API_KEY\s*=\s*(\S+)', (HERE / '.env').read_text())
        key = m and m.group(1).strip('"\'')
    if not key:
        sys.exit('Нет ключа: export PEXELS_API_KEY=… или строка PEXELS_API_KEY=… в tools/photos/.env')
    return key


def http_get(url, headers=None, tries=4):
    for i in range(tries):
        try:
            req = urllib.request.Request(url, headers={'User-Agent': 'bakeloom-photos/1.0', **(headers or {})})
            with urllib.request.urlopen(req, timeout=60) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 429:
                print('  Pexels: лимит запросов, жду минуту'); time.sleep(60)
            elif i == tries - 1 or e.code < 500:
                raise
        except OSError:
            if i == tries - 1:
                raise
        time.sleep(2 * (i + 1))


def cmd_fetch(args):
    key = pexels_key()
    items = load_json(WORK / 'items.json') or sys.exit('Сначала: photos.py items')
    queries = load_json(QUERIES, {})
    cache = load_json(WORK / 'search.json', {})          # запрос → результаты Pexels
    # Один запрос на несколько групп (крышки 20 размеров) — берём больше результатов,
    # чтобы у разных товаров были разные фото, а не одна коробка на 30 карточек.
    groups_by_q = {}
    for it in items:
        groups_by_q.setdefault(queries[it['sku']], []).append(it['group'])
    groups_by_q = {q: list(dict.fromkeys(g)) for q, g in groups_by_q.items()}
    for q, groups in groups_by_q.items():
        need = min(80, args.per_query * len(groups) + args.per_query)
        if q in cache and len(cache[q]) >= need and not args.refresh:
            continue
        url = 'https://api.pexels.com/v1/search?' + urllib.parse.urlencode(
            {'query': q, 'per_page': need})
        data = json.loads(http_get(url, {'Authorization': key}))
        cache[q] = [dict(id=p['id'], src=p['src']['large'], thumb=p['src']['medium'], url=p['url'],
                         alt=p.get('alt') or '', photographer=p['photographer'],
                         photographer_url=p['photographer_url']) for p in data.get('photos', [])]
        save_json(WORK / 'search.json', cache)
        print(f'  {q!r}: {len(cache[q])}')

    # Фасовки одной группы получают одни и те же кандидаты; разные группы с общим
    # запросом — разные «окна» по результатам (по кругу, если результатов мало).
    raw = WORK / 'raw'; raw.mkdir(parents=True, exist_ok=True)
    cands = {}
    for it in items:
        q = queries[it['sku']]
        res = cache.get(q, [])
        k = groups_by_q[q].index(it['group'])
        photos = [res[(k * args.per_query + j) % len(res)] for j in range(min(args.per_query, len(res)))] if res else []
        cands[it['sku']] = photos
        for p in photos:
            f = raw / f"{p['id']}.jpg"
            if not f.exists():
                f.write_bytes(http_get(p['src']))
    save_json(WORK / 'candidates.json', cands)
    print(f'кандидатов: {sum(map(len, cands.values()))} для {len(cands)} позиций, '
          f'без кандидатов: {sum(1 for v in cands.values() if not v)}')


# ---------- local: свои фото заказчика вместо стока ----------

def cmd_local(args):
    """Файлы <артикул>.jpg или <артикул>_2.jpg (несколько вариантов) — кандидаты этой позиции.
    Подходит и имя из image_name (CHO-D54-100.jpg). Форматы — всё, что открывает Pillow."""
    from PIL import Image, ImageOps
    items = load_json(WORK / 'items.json') or sys.exit('Сначала: photos.py items')
    keys = {}
    for it in items:
        keys[it['sku'].lower()] = it['sku']
        if it['image_name']:
            keys[Path(it['image_name']).stem.lower()] = it['sku']
    cands = load_json(WORK / 'candidates.json', {})
    raw = WORK / 'raw'; raw.mkdir(parents=True, exist_ok=True)
    got, unknown = {}, []
    for f in sorted(Path(args.dir).iterdir()):
        if not f.is_file() or f.name.startswith('.'):
            continue
        stem = re.sub(r'[_ ]\d+$', '', f.stem).lower()
        sku = keys.get(stem)
        if not sku:
            unknown.append(f.name); continue
        try:
            img = ImageOps.exif_transpose(Image.open(f)).convert('RGB')   # поворот с телефона
        except Exception as e:
            print(f'  пропущен {f.name}: {e}'); continue
        pid = 'own-' + re.sub(r'[^a-z0-9-]+', '-', f.stem.lower()).strip('-')
        img.save(raw / f'{pid}.jpg', quality=95)
        got.setdefault(sku, []).append(dict(id=pid, src=None, thumb=None, url=None, alt=f.name,
                                            photographer='фото заказчика', photographer_url=None))
    for sku, photos in got.items():
        cands[sku] = photos            # стоковые кандидаты этой позиции заменяются своими
    save_json(WORK / 'candidates.json', cands)
    print(f'своих фото: {sum(map(len, got.values()))} для {len(got)} позиций')
    if unknown:
        print(f'не узнал артикул в {len(unknown)} файлах: {", ".join(unknown[:10])}')


# ---------- render: свои рендеры в Blender (#56) ----------

RENDER_CREDIT = 'рендер Bakeloom (Blender)'


def cmd_render(args):
    """Упаковка, декор и шоколад по названию и размерам → work/raw/render-<ключ>.png (см. specs.py).
    Одинаковые по виду позиции получают один рендер. Кандидат «рендер» ставится первым,
    свои фото заказчика (local) — ещё раньше."""
    import specs
    items = load_json(WORK / 'items.json') or sys.exit('Сначала: photos.py items')
    by_key, sku_key = {}, {}
    for it in items:
        sp = specs.spec(it)
        if sp:
            by_key.setdefault(sp['key'], sp)
            sku_key[it['sku']] = sp['key']
    spec_file = WORK / 'render_specs.json'
    save_json(spec_file, list(by_key.values()))
    blender = args.blender or os.environ.get('BLENDER') or shutil.which('blender')
    if not blender:
        sys.exit('Нужен Blender 4.2+: в PATH, в переменной BLENDER или --blender /путь/к/blender')
    raw = WORK / 'raw'; raw.mkdir(parents=True, exist_ok=True)
    cmd = [blender, '-b', '--factory-startup', '-P', str(HERE / 'render_blender.py'), '--',
           str(spec_file), str(raw), '--samples', str(args.samples)]
    if args.only:
        cmd += ['--only', args.only]
    if args.refresh:
        cmd += ['--refresh']
    print(f'рендеров: {len(by_key)} для {len(sku_key)} позиций (без рендера: {len(items) - len(sku_key)})')
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    for line in proc.stdout:
        if line.startswith(('RENDER', 'FAIL', 'Traceback', 'Error')):
            print(' ', line.rstrip())
    proc.wait()

    cands = load_json(WORK / 'candidates.json', {})
    for it in items:
        key = sku_key.get(it['sku'])
        if not key or not (raw / f'render-{key}.png').exists():
            continue
        lst = [p for p in cands.get(it['sku'], []) if not str(p['id']).startswith('render-')]
        own = [p for p in lst if str(p['id']).startswith('own-')]
        rest = [p for p in lst if not str(p['id']).startswith('own-')]
        cands[it['sku']] = own + [dict(id=f'render-{key}', src=None, thumb=None, url=None,
                                       alt=f"{it['name']} {it['variant'] or ''}".strip(),
                                       photographer=RENDER_CREDIT, photographer_url=None)] + rest
    save_json(WORK / 'candidates.json', cands)


def place_render(pid):
    """Рендер с прозрачным фоном и настоящей тенью: кадрируем по рамке товара (без тени)
    и кладём на подложку целиком, чтобы тень не обрезалась."""
    import numpy as np
    from PIL import Image
    src = Image.open(WORK / 'raw' / f'{pid}.png').convert('RGBA')
    x0, y0, x1, y1 = json.loads((WORK / 'raw' / f'{pid}.json').read_text())['bbox']
    a = np.asarray(src.getchannel('A'), dtype=np.float32)
    # тень у краёв кадра рендера гасим плавно, чтобы на подложке не было ступеньки
    h, w = a.shape
    yy, xx = np.mgrid[0:h, 0:w]
    edge = np.minimum.reduce([xx, yy, w - 1 - xx, h - 1 - yy]) / (0.08 * w)
    a *= np.clip(edge, 0, 1)
    src.putalpha(Image.fromarray(a.astype('uint8')))
    inner = SIZE * (1 - 2 * FIELD)
    k = min(inner / (x1 - x0), inner / (y1 - y0))
    img = src.resize((round(w * k), round(h * k)), Image.LANCZOS)
    cx, cy = (x0 + x1) / 2 * k, (y0 + y1) / 2 * k
    canvas = Image.new('RGBA', (SIZE, SIZE), BG + (255,))
    layer = Image.new('RGBA', (SIZE, SIZE), (0, 0, 0, 0))
    layer.paste(img, (round(SIZE / 2 - cx), round(SIZE / 2 - cy)))
    canvas.alpha_composite(layer)
    return canvas.convert('RGB')


# ---------- process: единый стиль ----------

def cmd_process(args):
    import numpy as np
    from PIL import Image, ImageFilter
    cands = load_json(WORK / 'candidates.json') or sys.exit('Сначала: photos.py fetch')
    ids = sorted({str(p['id']) for v in cands.values() for p in v})
    out = WORK / 'out'; out.mkdir(parents=True, exist_ok=True)
    todo = [i for i in ids if args.refresh or not (out / f'{i}.webp').exists()]
    print(f'обработать: {len(todo)} из {len(ids)}')
    if not todo:
        return
    meta = load_json(WORK / 'process.json', {})
    remover = None
    for n, pid in enumerate(todo, 1):
        if (WORK / 'raw' / f'{pid}.png').exists():           # свой рендер: фон уже прозрачный
            place_render(pid).save(out / f'{pid}.webp', 'WEBP', quality=86, method=6)
            meta[str(pid)] = {'render': True}
            continue
        remover = remover or BackgroundRemover(args.model)
        img = Image.open(WORK / 'raw' / f'{pid}.jpg').convert('RGB')
        mask = remover.mask(img)
        rgba = img.copy(); rgba.putalpha(mask)
        a = np.asarray(mask)
        ys, xs = np.nonzero(a > 24)
        if len(xs) == 0:
            meta[str(pid)] = {'empty': True}
            Image.new('RGB', (SIZE, SIZE), BG).save(out / f'{pid}.webp', quality=86)
            continue
        box = (xs.min(), ys.min(), xs.max() + 1, ys.max() + 1)
        # «Обрезан»: товар упирается в край исходного кадра — проверяющему подсказка на странице.
        h, w = a.shape
        edge = max((a[:, :3] > 128).mean(), (a[:, -3:] > 128).mean(), (a[:3, :] > 128).mean(), (a[-3:, :] > 128).mean())
        meta[str(pid)] = {'cropped': bool(edge > 0.08), 'fill': round(float((a > 128).mean()), 3)}
        obj = rgba.crop(box)
        inner = int(SIZE * (1 - 2 * FIELD))
        k = min(inner / obj.width, inner / obj.height)
        obj = obj.resize((max(1, round(obj.width * k)), max(1, round(obj.height * k))), Image.LANCZOS)
        x, y = (SIZE - obj.width) // 2, (SIZE - obj.height) // 2

        canvas = Image.new('RGBA', (SIZE, SIZE), BG + (255,))
        # мягкая тень: размытый силуэт, чуть ниже товара
        sil = Image.new('L', (SIZE, SIZE), 0)
        sil.paste(obj.getchannel('A').point(lambda v: v * 0.16), (x, y + round(SIZE * 0.022)))
        sil = sil.filter(ImageFilter.GaussianBlur(SIZE * 0.025))
        canvas.alpha_composite(Image.merge('RGBA', [Image.new('L', (SIZE, SIZE), c) for c in SHADOW] + [sil]))
        canvas.alpha_composite(obj, (x, y))
        canvas.convert('RGB').save(out / f'{pid}.webp', 'WEBP', quality=86, method=6)
        if n % 10 == 0 or n == len(todo):
            print(f'  {n}/{len(todo)}')
            save_json(WORK / 'process.json', meta)
    save_json(WORK / 'process.json', meta)


class BackgroundRemover:
    """rembg без rembg: та же модель (isnet-general-use) напрямую через onnxruntime.
    Пакет rembg тянет numba, которой нет под свежий Python; сам он делает ровно это."""
    URL = 'https://github.com/danielgatis/rembg/releases/download/v0.0.0/{}.onnx'

    def __init__(self, model):
        import onnxruntime as ort
        path = Path(os.environ.get('U2NET_HOME', Path.home() / '.u2net')) / f'{model}.onnx'
        if not path.exists():
            print(f'качаю модель {model} (~170 МБ) → {path}')
            path.parent.mkdir(parents=True, exist_ok=True)
            urllib.request.urlretrieve(self.URL.format(model), path)
        self.sess = ort.InferenceSession(str(path), providers=['CPUExecutionProvider'])
        self.inp = self.sess.get_inputs()[0]
        self.side = self.inp.shape[2] if isinstance(self.inp.shape[2], int) else 1024

    def mask(self, img):
        import numpy as np
        from PIL import Image
        x = np.asarray(img.resize((self.side, self.side), Image.LANCZOS), dtype=np.float32) / 255.0
        x = (x - 0.5) / 1.0                     # нормализация isnet, как в rembg
        x = x.transpose(2, 0, 1)[None]
        pred = self.sess.run(None, {self.inp.name: x})[0][0, 0]
        pred = (pred - pred.min()) / (pred.max() - pred.min() + 1e-8)
        m = Image.fromarray((pred * 255).astype('uint8')).resize(img.size, Image.LANCZOS)
        return m


# ---------- serve: страница проверки ----------

def cmd_serve(args):
    class H(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=str(HERE), **kw)

        def log_message(self, *a):
            pass

        def do_GET(self):
            if self.path.startswith('/api/data'):
                body = json.dumps({
                    'items': load_json(WORK / 'items.json', []),
                    'queries': load_json(QUERIES, {}),
                    'candidates': load_json(WORK / 'candidates.json', {}),
                    'process': load_json(WORK / 'process.json', {}),
                    'selection': load_json(SELECTION, {}),
                }, ensure_ascii=False).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json; charset=utf-8')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                super().do_GET()

        def do_POST(self):
            if self.path != '/api/selection':
                return self.send_error(404)
            data = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            save_json(SELECTION, dict(sorted(data.items())))
            self.send_response(204); self.end_headers()

    srv = http.server.ThreadingHTTPServer(('127.0.0.1', args.port), H)
    print(f'страница проверки: http://localhost:{args.port}/review.html  (Ctrl+C — выход)')
    srv.serve_forever()


# ---------- build: готовые фото + миграция ----------

def cmd_build(_):
    items = load_json(WORK / 'items.json') or sys.exit('Сначала: photos.py items')
    sel = load_json(SELECTION, {})
    cands = load_json(WORK / 'candidates.json', {})
    missing = [i['sku'] for i in items if not sel.get(i['sku'], {}).get('reviewed')]
    if missing:
        print(f'внимание: не просмотрено {len(missing)}: {", ".join(missing[:10])}…')
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for f in OUT_DIR.glob('*.webp'):
        f.unlink()
    lines, no_photo, credits = [], [], {}
    for it in items:
        s = sel.get(it['sku'], {})
        pid = s.get('photo')
        name = Path(it['image_name'] or it['sku']).stem + '.webp'
        if not pid:
            no_photo.append(it)
            continue
        src = WORK / 'out' / f'{pid}.webp'
        if not src.exists():
            sys.exit(f'{it["sku"]}: нет обработанного {src} — запустите process')
        shutil.copyfile(src, OUT_DIR / name)
        p = next((p for p in cands.get(it['sku'], []) if str(p['id']) == str(pid)), None) or {}
        if str(pid).startswith('render-'):
            credits[name] = {'source': RENDER_CREDIT, 'license': 'собственная работа команды'}
        elif str(pid).startswith('own-'):
            credits[name] = {'source': 'фото заказчика', 'file': p.get('alt')}
        else:
            credits[name] = {'pexels': p.get('url'), 'photographer': p.get('photographer'), 'license': 'Pexels License'}
        sku = it['sku'].replace("'", "''")
        lines.append(f"UPDATE products SET image_url = '/img/products/{name}' WHERE sku = '{sku}';")
    MIGRATION.write_text(
        '-- Фото товаров (#42, #46): сгенерировано tools/photos/photos.py build из selection.json.\n'
        '-- Руками не править — поменять выбор на странице проверки и пересобрать.\n'
        '-- Позиции без подходящего фото остаются без image_url: сайт показывает заглушку раздела.\n'
        '-- Источники: свои рендеры (Blender), фото заказчика, Pexels — по файлам в frontend/public/img/products/CREDITS.json.\n\n'
        + '\n'.join(lines) + '\n', 'utf8')
    save_json(OUT_DIR / 'CREDITS.json', credits)
    register_migration()
    print(f'фото: {len(lines)}, без фото: {len(no_photo)} → {OUT_DIR.relative_to(ROOT)}, {MIGRATION.relative_to(ROOT)}')
    for it in no_photo:
        print(f'  нет фото: {it["sku"]} — {it["name"]} {it["variant"] or ""} ({it["category"]})')


def register_migration():
    """Бэкенд встраивает миграции в бинарник списком в db.rs — добавляем 009 один раз."""
    src = DB_RS.read_text('utf8')
    name = MIGRATION.stem
    if f'"{name}"' in src:
        return
    anchor = '];\n\nasync fn migrate'
    if anchor not in src:
        print(f'внимание: не нашёл список MIGRATIONS в {DB_RS.relative_to(ROOT)} — добавьте {name} руками')
        return
    line = f'    ("{name}", include_str!("../migrations/{MIGRATION.name}")),\n'
    DB_RS.write_text(src.replace(anchor, line + anchor, 1), 'utf8')
    print(f'миграция {name} добавлена в {DB_RS.relative_to(ROOT)}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    sub.add_parser('items')
    f = sub.add_parser('fetch'); f.add_argument('--per-query', type=int, default=CANDIDATES); f.add_argument('--refresh', action='store_true')
    p = sub.add_parser('process'); p.add_argument('--model', default='isnet-general-use'); p.add_argument('--refresh', action='store_true')
    s = sub.add_parser('serve'); s.add_argument('--port', type=int, default=8765)
    r = sub.add_parser('render'); r.add_argument('--samples', type=int, default=96)
    r.add_argument('--blender'); r.add_argument('--only', help='ключи или виды через запятую'); r.add_argument('--refresh', action='store_true')
    l = sub.add_parser('local'); l.add_argument('dir', help='папка с фото: <артикул>.jpg, <артикул>_2.jpg…')
    sub.add_parser('build')
    a = ap.parse_args()
    {'items': cmd_items, 'fetch': cmd_fetch, 'local': cmd_local, 'render': cmd_render, 'process': cmd_process, 'serve': cmd_serve, 'build': cmd_build}[a.cmd](a)


if __name__ == '__main__':
    main()
