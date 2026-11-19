"""Что рендерить для позиции: название + размер → параметры сцены Blender (#56).

Стоки не находят «крышку 25×25×11 с окном», поэтому упаковку, декор и (до фото
производителя) шоколад рисуем сами: форма, пропорции, окно, материал берутся из
названия и характеристик. Одинаковые по виду позиции получают один ключ — один рендер.
Возвращает None, если товар по названию не понять: такие позиции остаются «нет фото».
"""
import hashlib
import json
import re

NUM = r'(\d+(?:[.,]\d+)?)'


def nums(s):
    return [float(x.replace(',', '.')) for x in re.findall(NUM, s or '')]


def dims(it):
    """Ш×Г×В в см из варианта или названия: «25х25х11», «Ø 26», «24*14*28»."""
    for s in (it['variant'] or '', it['name']):
        s = s.replace('Толщина 3,5 мм.', '')
        m = re.search(NUM + r'\s*[хxх*×]\s*' + NUM + r'(?:\s*[хxх*×]\s*' + NUM + ')?', s)
        if m:
            return [float(g.replace(',', '.')) for g in m.groups() if g]
        m = re.search(r'[ØО]\s*' + NUM, s)
        if m:
            return [float(m.group(1).replace(',', '.'))]
    return []


def color(it):
    """Материал поверхности: white, kraft, full (полноцвет), one (в 1 краску), clear, gold…"""
    s = f"{it['name']} {it['variant'] or ''}".lower()
    a = it['attrs'].get('Цвет', '').lower()
    if 'полноцвет' in s or 'п/цв' in s:
        return 'full'
    if 'в 1 краску' in s:
        return 'one'
    if 'крафт' in s or 'kraft' in s or a == 'крафт':
        return 'kraft'
    if 'прозрачн' in s or a == 'прозрачный':
        return 'clear'
    if 'brown' in s:
        return 'brown'
    if 'beige' in s:
        return 'beige'
    if 'золот' in s or a == 'золото':
        return 'gold'
    if 'металлиз' in s:
        return 'silver'
    return 'white'


def window(s):
    if 'с окнами' in s:
        return 'sides'
    if 'окн' in s:
        return 'top'
    return 'none'


def corr(s):
    return bool(re.search(r'гофро|микрогофр', s))


def pack(it):
    s = f"{it['name']} {it['category']}".lower()
    n = it['name'].lower()
    d = dims(it)
    c, win = color(it), window(n)
    base = dict(color=c, window=win, corr=corr(n))
    W, D, H = (d + [None, None, None])[:3]

    # --- плоское: круги, подложки
    if it['category'] == 'Круги' or 'подложк' in n or 'поднос' in n:
        thick = 0.35 if ('поднос' in n or 'усилен' in n) else (0.3 if 'гофро' in n else 0.15)
        if 'шестигран' in n:
            return dict(kind='disk', sides=6, w=26, d=26, t=0.3, **base)
        if 'металлиз' in n:
            base['color'] = 'silver'
        if 'поднос' in n:
            base['color'] = 'white'; thick = 0.35
        if len(d) == 1:
            return dict(kind='disk', sides=0, w=d[0], d=d[0], t=thick, **base)
        if 'кругл' in n:
            return dict(kind='disk', sides=0, w=nums(it['variant'])[0], d=nums(it['variant'])[0], t=thick, **base)
        return dict(kind='board', w=W, d=D or W, t=thick, **base)

    # --- пакеты
    if it['category'] == 'Пакеты' or 'саше' in n:
        if 'саше' in n:
            w, h = nums(it['variant'])[:2]
            return dict(kind='sachet', w=w, h=h, color='clear')
        if 'наполнител' in n:
            return None                                    # по названию не понять, что за товар
        if n == 'b':                                       # «B 24*14*28»: по размерам — бумажный пакет, уточнить у заказчика
            return dict(kind='bag', w=d[0], d=d[1], h=d[2], handles=False, **base)
        if 'с ручкой' in n or 'эко' in n:
            w, dd, h = d
            if 'эко' in n:                                 # 21×30×11: Ш×В×Г
                w, h, dd = d
            elif h > w:                                    # 29*12*22 → Ш×Г×В уже так
                pass
            return dict(kind='bag', w=w, d=dd, h=h, handles=True, **base)
        if 'фаст' in n:
            return dict(kind='bag', w=d[0], d=d[0] * 0.4, h=d[1], handles=False, **base)
        if 'куриц' in n:
            return dict(kind='bag', w=d[0], d=d[2], h=d[1], handles=False, **base)
        if 'багет' in n:
            return dict(kind='bag', w=d[2], d=d[1], h=d[0], handles=False, window_strip=True, **base)
        return None

    # --- фастфуд
    if 'фри' in n:
        return dict(kind='fries', w=9, d=5, h=12, **base)
    if 'держатель для кофе' in n:
        return dict(kind='cupholder', w=22, d=11, h=14, **base)
    if 'детское меню' in n:
        return dict(kind='kidsbox', w=W, d=D, h=H, **base)
    if 'бургер' in n:
        return dict(kind='burger', w=W, d=D, h=H, **base)

    # --- канцелярия
    if it['category'] == 'Канцелярия':
        mel = 'немелов' not in n
        return dict(kind='folder', w=23.5, d=31, ties='завязк' in n, fastener='скоросшив' in n,
                    color='white' if mel else 'offwhite')

    # --- поддоны
    if 'поддон' in n:
        return dict(kind='pallet', w=W, d=D, h=H, wave='волна' in n, **base)

    if 'капкейк' in n and it['category'] == 'Упаковка':   # «Коробка для капкейков с вкладышем» (крафт): окно и ложемент
        k = int(nums(it['variant'])[0])
        w, dd = {4: (16.5, 16.5), 6: (24, 16.5), 12: (31.5, 24)}[k]
        return dict(kind='box', w=w, d=dd, h=10, holes=k, **dict(base, window='top'))

    # --- перегородки, вкладыши
    if 'перегор' in n and 'компл' in n:
        return dict(kind='dividers', w=16, d=16, h=3.5, cells=2, **base)
    if 'вкладыш' in n:
        m = re.search(r'под (\d+)', n)
        if m:
            k = int(m.group(1))
            return dict(kind='insert', w=W, d=D, h=H or 2, holes=k, **base)
        return dict(kind='insert', w=W, d=D, h=H or 2, holes=1, **base)          # кулич: одно гнездо

    # --- конфеты с перегородками
    if 'перегородок' in n:
        return dict(kind='candygrid', w=W, d=D, h=H, **base)

    # --- дно: гофродно — плоский лист гофрокартона, остальное — лоток с невысоким бортом
    if n.startswith('гофродно'):
        return dict(kind='board', w=W, d=D or W, t=0.4, **dict(base, corr=True))
    if n.startswith('дно') or 'дно ' in n[:12]:
        hh = H if H and H <= 6 else (3 if not H else 4)
        return dict(kind='tray', w=W, d=D or W, h=hh, **dict(base, color='white' if c == 'clear' else c))

    # --- прозрачные крышки / короба
    if c == 'clear' or 'прозрачн' in n:
        kind = 'box' if n.startswith('коробка') else 'lid'
        return dict(kind=kind, w=W, d=D, h=H, **dict(base, color='clear'))

    # --- пицца, печенье в гофре: плоские короба-самосборы
    if it['category'] == 'Коробки для пиццы белые':
        if n.startswith('крышк') or n.startswith('дно'):
            pass
        return dict(kind='flatbox', w=W, d=D, h=H, **dict(base, corr=True))
    if 'гофрокоробка под печенье' in n:
        return dict(kind='flatbox', w=W, d=D, h=H, **dict(base, corr=True))

    # --- ажурная крышка
    if 'ажурн' in n:
        return dict(kind='lid', w=W, d=D, h=H, **dict(base, window='lace'))

    # --- коробки с ручкой, с обечайкой
    if 'с ручкой' in n:
        return dict(kind='handlebox', w=W, d=D, h=H, **base)
    if 'обеч' in n:
        return dict(kind='sleevebox', w=W, d=D, h=H, **base)

    # --- кулич: высокий, окно спереди
    if 'кулич' in n:
        return dict(kind='box', w=W, d=D, h=H, **dict(base, window='front' if win == 'top' else win))
    if 'фитнес' in n:                                   # 8,9×4,9×13: высокая, окно спереди
        return dict(kind='box', w=W, d=D, h=H, **dict(base, color='kraft', window='front'))

    # --- коробки под капкейки: с окном сверху, если указано; гнёзда видно в окно
    if 'капкейк' in s and it['category'] != 'Упаковка':
        return dict(kind='box', w=W, d=D, h=H, **base)
    if 'коробка для торта' in n and 'окн' in n:
        return dict(kind='box', w=W, d=D, h=H, **base)

    # --- крышки (в т.ч. с окном), закрытые коробки
    if n.startswith('крышк') or 'крышка' in n:
        return dict(kind='lid', w=W, d=D or W, h=H or 10, **base)
    if 'гофрокоробка' in n:
        return dict(kind='box', w=W, d=D, h=H, **dict(base, corr=True))
    if W and D and H:
        return dict(kind='box', w=W, d=D, h=H, **base)
    return None


DECOR = {
    'красн': (0.70, 0.06, 0.08), 'бел': (0.93, 0.92, 0.89), 'золот': (0.80, 0.60, 0.26),
    'розов': (0.92, 0.55, 0.64), 'коричнев': (0.30, 0.17, 0.10),
}


def decor(it):
    n = it['name'].lower()
    v = (it['variant'] or '').lower()
    if 'лента' in n:
        col = next(c for k, c in DECOR.items() if k in v)
        return dict(kind='ribbon', width=nums(v)[0], rgb=col, satin='атлас' in n)
    if 'свеча-цифра' in n:
        return dict(kind='candle_num', digit=str(int(nums(v)[0])))
    if 'тонкие' in n:
        return dict(kind='candles_thin', mix='разноцв' in v)
    if 'фонтан' in n:
        return dict(kind='fountain', h=nums(v)[0])
    if 'посыпк' in n:
        return dict(kind='sprinkles', grams=nums(v)[0])
    if 'топпер' in n:
        return dict(kind='topper', gold='золот' in v)
    return None


CHOC = {   # цвет каллет
    'dark': (0.055, 0.025, 0.014), 'bitter': (0.04, 0.018, 0.01), 'milk': (0.24, 0.115, 0.05),
    'white': (0.90, 0.82, 0.62), 'ruby': (0.62, 0.22, 0.28), 'caramel': (0.62, 0.36, 0.13),
    'wcaramel': (0.84, 0.62, 0.32), 'butter': (0.93, 0.84, 0.55),
}


def choc(it):
    n = it['name'].lower()
    if 'краситель' in n:
        rgb = {'красн': (0.75, 0.02, 0.04), 'син': (0.02, 0.12, 0.70), 'жёлт': (0.95, 0.70, 0.02)}
        return dict(kind='dye', rgb=next(c for k, c in rgb.items() if k in (it['variant'] or '').lower()))
    if 'порошок' in n and 'какао' in n:
        tone = {'noir': (0.06, 0.03, 0.02), 'extra': (0.16, 0.05, 0.03)}
        rgb = next((c for k, c in tone.items() if k in n), (0.20, 0.09, 0.05))
        return dict(kind='powder', rgb=rgb)
    if 'масло в порошке' in n:
        return dict(kind='powder', rgb=CHOC['butter'])
    if 'какао' in n and 'масло' in n:
        return dict(kind='chunks', rgb=CHOC['butter']) if 'каллет' not in n else dict(kind='callets', rgb=[CHOC['butter']])
    if 'вафельн' in n:
        return dict(kind='flakes')
    if 'масса' in n:
        return dict(kind='chunks', rgb=CHOC['white'] if 'бел' in n else CHOC['milk'])
    if 'термостаб' in n:
        return dict(kind='drops', rgb=CHOC['dark'])
    if 'смесь молочного и белого' in n:
        return dict(kind='callets', rgb=[CHOC['milk'], CHOC['white']])
    if 'розов' in n:
        c = [CHOC['ruby']]
    elif 'бел' in n and 'карамел' in n:
        c = [CHOC['wcaramel']]
    elif 'карамел' in n:
        c = [CHOC['caramel']]
    elif 'бел' in n:
        c = [CHOC['white']]
    elif 'молоч' in n:
        c = [CHOC['milk']]
    elif 'горьк' in n:
        c = [CHOC['bitter']]
    else:
        c = [CHOC['dark']]
    return dict(kind='callets', rgb=c)


def spec(it):
    """Параметры сцены или None. key — общий для одинаковых по виду позиций."""
    if it['category'] == 'Шоколад и ингредиенты':
        sp = choc(it)
    elif it['category'] in ('Свечи', 'Ленты', 'Декор'):
        sp = decor(it)
    else:
        sp = pack(it)
    if not sp:
        return None
    sp = {k: (round(v, 2) if isinstance(v, float) else v) for k, v in sp.items() if v is not None}
    sp['key'] = sp['kind'] + '-' + hashlib.sha1(json.dumps(sp, sort_keys=True).encode()).hexdigest()[:8]
    return sp
