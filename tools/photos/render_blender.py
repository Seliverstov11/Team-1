"""Рендер товаров в Blender (#56): blender -b -P render_blender.py -- specs.json out_dir [--samples N]

Параметры сцены — из specs.py (форма, размеры в см, материал, окно). На выходе PNG с прозрачным
фоном и мягкой тенью (shadow catcher); photos.py process кладёт их на подложку #F7F4EC.
Без текста, людей и логотипов: печать «полноцвет» и «в 1 краску» — абстрактный узор.
"""
import json
import math
import random
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index('--') + 1:]
SPECS, OUT = Path(argv[0]), Path(argv[1])
SAMPLES = int(argv[argv.index('--samples') + 1]) if '--samples' in argv else 96
ONLY = argv[argv.index('--only') + 1].split(',') if '--only' in argv else None
RES = 1100
FONT_SCRIPT = '/usr/share/fonts/gsfonts/Z003-MediumItalic.otf'
FONT_BOLD = '/usr/share/fonts/noto/NotoSans-Bold.ttf'


# ---------------------------------------------------------------- сцена

def setup():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'CYCLES'
    prefs = bpy.context.preferences.addons['cycles'].preferences
    for t in ('OPTIX', 'CUDA'):
        try:
            prefs.compute_device_type = t
            prefs.get_devices()
            if any(d.type == t for d in prefs.devices):
                for d in prefs.devices:
                    d.use = d.type == t
                sc.cycles.device = 'GPU'
                if t == 'OPTIX':
                    sc.cycles.denoiser = 'OPTIX'
                break
        except Exception:
            pass
    sc.cycles.samples = SAMPLES
    sc.cycles.use_denoising = True
    sc.cycles.max_bounces = 8
    sc.cycles.transmission_bounces = 8
    sc.render.film_transparent = True
    sc.cycles.film_transparent_glass = True
    sc.render.resolution_x = sc.render.resolution_y = RES
    sc.render.image_settings.file_format = 'PNG'
    sc.render.image_settings.color_mode = 'RGBA'
    sc.view_settings.view_transform = 'Standard'
    sc.view_settings.look = 'None'
    sc.view_settings.exposure = -0.15
    w = bpy.data.worlds.new('w'); sc.world = w
    w.use_nodes = True
    bg = w.node_tree.nodes['Background']
    bg.inputs[0].default_value = (0.95, 0.94, 0.91, 1); bg.inputs[1].default_value = 0.55


def clear():
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    for coll in (bpy.data.meshes, bpy.data.curves, bpy.data.lights, bpy.data.cameras):
        for b in list(coll):
            if b.users == 0:
                coll.remove(b)


def lights(scale):
    def area(name, loc, size, power, color=(1, 1, 1)):
        l = bpy.data.lights.new(name, 'AREA'); l.size = size * scale; l.energy = power * scale ** 2
        l.color = color
        o = bpy.data.objects.new(name, l); bpy.context.collection.objects.link(o)
        o.location = Vector(loc) * scale
        o.rotation_euler = (Vector((0, 0, 0)) - o.location).to_track_quat('-Z', 'Y').to_euler()
    area('key', (-5, -7, 11), 9, 1100, (1, 0.97, 0.93))
    area('fill', (9, -3, 4), 8, 260, (0.95, 0.97, 1))
    area('top', (0, 2, 14), 12, 420)
    area('rim', (2, 9, 6), 6, 300)


def camera(el=30, az=32, lens=70, target=None, objs=None):
    """Камера сверху-спереди-справа; дистанция — чтобы все объекты влезли в кадр."""
    pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    c = (lo + hi) / 2 if target is None else target
    r = (hi - lo).length / 2
    cam = bpy.data.cameras.new('cam'); cam.lens = lens; cam.sensor_width = 36
    o = bpy.data.objects.new('cam', cam); bpy.context.collection.objects.link(o)
    e, a = math.radians(el), math.radians(az)
    d = Vector((math.sin(a) * math.cos(e), -math.cos(a) * math.cos(e), math.sin(e)))
    fov = 2 * math.atan(18 / lens)
    o.location = c + d * (r / math.sin(fov / 2) * 1.02)
    o.rotation_euler = (c - o.location).to_track_quat('-Z', 'Y').to_euler()
    cam.clip_start = 0.01; cam.clip_end = 1e4
    bpy.context.scene.camera = o
    return r


def ground(z, size):
    bpy.ops.mesh.primitive_plane_add(size=size * 20, location=(0, 0, z))
    g = bpy.context.object; g.is_shadow_catcher = True; g.name = 'ground'
    return g


# ---------------------------------------------------------------- материалы

_mats = {}


def node_mat(name):
    m = bpy.data.materials.new(name); m.use_nodes = True
    nt = m.node_tree
    return m, nt, nt.nodes['Principled BSDF']


def coords(nt, scale):
    tc = nt.nodes.new('ShaderNodeTexCoord')
    mp = nt.nodes.new('ShaderNodeMapping'); mp.inputs['Scale'].default_value = (scale,) * 3
    nt.links.new(tc.outputs['Object'], mp.inputs['Vector'])
    return mp.outputs['Vector']


def bump(nt, p, tex_out, strength=0.15, dist=0.02):
    b = nt.nodes.new('ShaderNodeBump'); b.inputs['Strength'].default_value = strength
    b.inputs['Distance'].default_value = dist
    nt.links.new(tex_out, b.inputs['Height']); nt.links.new(b.outputs['Normal'], p.inputs['Normal'])


def mat(kind, rgb=None):
    key = (kind, tuple(rgb) if rgb else None)
    if key in _mats:
        return _mats[key]
    m, nt, p = node_mat(f'{kind}')
    P = p.inputs
    if kind in ('white', 'offwhite', 'laminated'):
        col = {'white': (0.80, 0.79, 0.76), 'offwhite': (0.66, 0.62, 0.53), 'laminated': (0.84, 0.83, 0.80)}[kind]
        P['Base Color'].default_value = col + (1,)
        P['Roughness'].default_value = 0.3 if kind == 'laminated' else 0.62
        n = nt.nodes.new('ShaderNodeTexNoise'); n.inputs['Scale'].default_value = 400
        nt.links.new(coords(nt, 1), n.inputs['Vector']); bump(nt, p, n.outputs['Fac'], 0.05)
    elif kind in ('kraft', 'kraft_dark', 'brown', 'beige'):
        a, b = {'kraft': ((0.36, 0.21, 0.105), (0.46, 0.29, 0.15)), 'kraft_dark': ((0.24, 0.13, 0.065), (0.31, 0.18, 0.09)),
                'brown': ((0.11, 0.055, 0.032), (0.14, 0.07, 0.04)), 'beige': ((0.55, 0.44, 0.31), (0.62, 0.50, 0.36))}[kind]
        n = nt.nodes.new('ShaderNodeTexNoise'); n.inputs['Scale'].default_value = 3.0; n.inputs['Detail'].default_value = 8
        nt.links.new(coords(nt, 1), n.inputs['Vector'])
        r = nt.nodes.new('ShaderNodeValToRGB')
        r.color_ramp.elements[0].color = a + (1,); r.color_ramp.elements[1].color = b + (1,)
        nt.links.new(n.outputs['Fac'], r.inputs['Fac']); nt.links.new(r.outputs['Color'], P['Base Color'])
        f = nt.nodes.new('ShaderNodeTexNoise'); f.inputs['Scale'].default_value = 300
        nt.links.new(coords(nt, 1), f.inputs['Vector']); bump(nt, p, f.outputs['Fac'], 0.12)
        P['Roughness'].default_value = 0.78
    elif kind == 'flute':                       # торец гофрокартона: волны
        w = nt.nodes.new('ShaderNodeTexWave'); w.wave_type = 'BANDS'; w.bands_direction = 'X'
        w.inputs['Scale'].default_value = 1.0
        nt.links.new(coords(nt, 3.0), w.inputs['Vector'])
        r = nt.nodes.new('ShaderNodeValToRGB')
        r.color_ramp.elements[0].color = (0.22, 0.13, 0.065, 1); r.color_ramp.elements[1].color = (0.50, 0.33, 0.18, 1)
        nt.links.new(w.outputs['Fac'], r.inputs['Fac']); nt.links.new(r.outputs['Color'], P['Base Color'])
        P['Roughness'].default_value = 0.85
    elif kind in ('full', 'one'):               # печать: горошек — без текста и логотипов
        v = nt.nodes.new('ShaderNodeTexVoronoi'); v.inputs['Scale'].default_value = 0.9
        v.inputs['Randomness'].default_value = 0.55
        nt.links.new(coords(nt, 1), v.inputs['Vector'])
        thr = nt.nodes.new('ShaderNodeMath'); thr.operation = 'LESS_THAN'; thr.inputs[1].default_value = 0.22
        nt.links.new(v.outputs['Distance'], thr.inputs[0])
        mix = nt.nodes.new('ShaderNodeMix'); mix.data_type = 'RGBA'
        nt.links.new(thr.outputs[0], mix.inputs['Factor'])
        if kind == 'full':
            bgc = (0.87, 0.74, 0.62, 1)                  # персиковый фон
            r = nt.nodes.new('ShaderNodeValToRGB'); r.color_ramp.interpolation = 'CONSTANT'
            pal = [(0.62, 0.13, 0.22), (0.14, 0.42, 0.30), (0.82, 0.35, 0.24), (0.16, 0.08, 0.05), (0.90, 0.88, 0.84)]
            r.color_ramp.elements[0].color = pal[0] + (1,)
            r.color_ramp.elements[1].position = 0.2; r.color_ramp.elements[1].color = pal[1] + (1,)
            for i, c in enumerate(pal[2:], 2):
                e = r.color_ramp.elements.new(i / len(pal)); e.color = c + (1,)
            nt.links.new(v.outputs['Color'], r.inputs['Fac'])
            nt.links.new(r.outputs['Color'], mix.inputs['B'])
        else:
            bgc = (0.80, 0.79, 0.76, 1)
            mix.inputs['B'].default_value = (0.20, 0.10, 0.06, 1)
        mix.inputs['A'].default_value = bgc
        nt.links.new(mix.outputs['Result'], P['Base Color'])
        P['Roughness'].default_value = 0.45
    elif kind == 'clear':
        P['Base Color'].default_value = (1, 1, 1, 1)
        P['Transmission Weight'].default_value = 1.0
        P['Roughness'].default_value = 0.03; P['IOR'].default_value = 1.5
    elif kind == 'frosted':
        P['Base Color'].default_value = (0.95, 0.95, 0.95, 1)
        P['Transmission Weight'].default_value = 0.85; P['Roughness'].default_value = 0.25
    elif kind in ('gold', 'silver'):
        P['Base Color'].default_value = ((0.86, 0.62, 0.26) if kind == 'gold' else (0.86, 0.87, 0.89)) + (1,)
        P['Metallic'].default_value = 1; P['Roughness'].default_value = 0.2
        n = nt.nodes.new('ShaderNodeTexNoise'); n.inputs['Scale'].default_value = 25; n.inputs['Detail'].default_value = 4
        nt.links.new(coords(nt, 1), n.inputs['Vector']); bump(nt, p, n.outputs['Fac'], 0.04)
    elif kind == 'mirror_gold':
        P['Base Color'].default_value = (0.72, 0.47, 0.16, 1); P['Metallic'].default_value = 1
        P['Roughness'].default_value = 0.22
    elif kind == 'glossy_black':
        P['Base Color'].default_value = (0.012, 0.012, 0.014, 1); P['Roughness'].default_value = 0.12
        P['Coat Weight'].default_value = 0.6
    elif kind == 'foil':
        P['Base Color'].default_value = (0.80, 0.81, 0.83, 1); P['Metallic'].default_value = 1
        P['Roughness'].default_value = 0.28
        n = nt.nodes.new('ShaderNodeTexVoronoi'); n.inputs['Scale'].default_value = 18
        nt.links.new(coords(nt, 1), n.inputs['Vector']); bump(nt, p, n.outputs['Distance'], 0.4)
    elif kind == 'choc':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 0.32
        P['Subsurface Weight'].default_value = 0.08 if sum(rgb) > 1.5 else 0.0
        P['Subsurface Radius'].default_value = (0.2, 0.1, 0.05)
    elif kind == 'powder':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 1.0
        n = nt.nodes.new('ShaderNodeTexNoise'); n.inputs['Scale'].default_value = 60; n.inputs['Detail'].default_value = 10
        nt.links.new(coords(nt, 1), n.inputs['Vector']); bump(nt, p, n.outputs['Fac'], 0.5)
    elif kind == 'wax':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 0.4
        P['Subsurface Weight'].default_value = 0.25; P['Subsurface Radius'].default_value = (0.3, 0.2, 0.15)
    elif kind == 'satin':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 0.22
        P['Anisotropic'].default_value = 0.6; P['Sheen Weight'].default_value = 0.1
    elif kind == 'grosgrain':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 0.65
        P['Sheen Weight'].default_value = 0.3
        w = nt.nodes.new('ShaderNodeTexWave'); w.inputs['Scale'].default_value = 40
        nt.links.new(coords(nt, 1), w.inputs['Vector']); bump(nt, p, w.outputs['Fac'], 0.25)
    elif kind == 'gel':
        P['Base Color'].default_value = tuple(c * 0.75 for c in rgb) + (1,); P['Roughness'].default_value = 0.15
        P['Coat Weight'].default_value = 0.6
    elif kind == 'bottle':
        P['Base Color'].default_value = (0.92, 0.92, 0.91, 1); P['Roughness'].default_value = 0.35
        P['Subsurface Weight'].default_value = 0.3; P['Subsurface Radius'].default_value = (0.5, 0.5, 0.5)
    elif kind == 'plain':
        P['Base Color'].default_value = tuple(rgb) + (1,); P['Roughness'].default_value = 0.5
    _mats[key] = m
    return m


def surf(spec):
    c = spec.get('color', 'white')
    return mat(c)


def inner_mat(spec):
    c = spec.get('color', 'white')
    if spec.get('corr') or c in ('full', 'one'):
        return mat('kraft') if spec.get('corr') else mat('white')
    return mat(c)


def assign(o, m, *more):
    o.data.materials.clear()
    o.data.materials.append(m)
    for x in more:
        o.data.materials.append(x)


# ---------------------------------------------------------------- геометрия

def cube(w, d, h, loc=(0, 0, 0), name='c'):
    """Брусок w×d×h, низ в z=loc.z."""
    bpy.ops.mesh.primitive_cube_add(size=1, location=(loc[0], loc[1], loc[2] + h / 2))
    o = bpy.context.object; o.scale = (w, d, h); o.name = name
    bpy.ops.object.transform_apply(scale=True)
    return o


def cyl(r, h, loc=(0, 0, 0), verts=96, name='cyl'):
    bpy.ops.mesh.primitive_cylinder_add(radius=r, depth=h, vertices=verts, location=(loc[0], loc[1], loc[2] + h / 2))
    o = bpy.context.object; o.name = name
    return o


def boolean(target, cutter, op='DIFFERENCE', keep_mat=False):
    m = target.modifiers.new('b', 'BOOLEAN'); m.operation = op; m.object = cutter; m.solver = 'EXACT'
    if keep_mat:
        m.material_mode = 'TRANSFER'
    bpy.context.view_layer.objects.active = target
    bpy.ops.object.modifier_apply(modifier=m.name)
    bpy.data.objects.remove(cutter, do_unlink=True)


def bevel(o, width, segs=2, angle=40):
    m = o.modifiers.new('bv', 'BEVEL'); m.width = width; m.segments = segs
    m.limit_method = 'ANGLE'; m.angle_limit = math.radians(angle)
    m.harden_normals = False
    return m


def smooth(o):
    for p in o.data.polygons:
        p.use_smooth = True


def join(objs, name='obj'):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    o = bpy.context.object; o.name = name
    return o


def shell(w, d, h, t, spec, open_top=False, z=0, name='shell'):
    """Полая коробка с толщиной стенки t; open_top — лоток без крышки. Внутренность — inner_mat."""
    o = cube(w, d, h, (0, 0, z), name)
    assign(o, surf(spec))
    if open_top:
        cut = cube(w - 2 * t, d - 2 * t, h, (0, 0, z + t))
    else:
        cut = cube(w - 2 * t, d - 2 * t, h - 2 * t, (0, 0, z + t))
    assign(cut, inner_mat(spec))
    boolean(o, cut, keep_mat=True)
    return o


def glass_pane(w, d, z, loc=(0, 0)):
    bpy.ops.mesh.primitive_plane_add(size=1, location=(loc[0], loc[1], z))
    g = bpy.context.object; g.scale = (w, d, 1); g.name = 'pane'
    bpy.ops.object.transform_apply(scale=True)
    s = g.modifiers.new('s', 'SOLIDIFY'); s.thickness = 0.03
    assign(g, mat('clear'))
    return g


def cut_window_top(o, w, d, h, t, k=0.62):
    ww, dd = w * k, d * k
    c = cube(ww, dd, t * 6, (0, 0, h - t * 3)); boolean(o, c)
    return glass_pane(ww + 0.6, dd + 0.6, h - t * 1.2)


def cut_window_front(o, w, d, h, t, side='front', z0=0.28, k=0.56):
    if side == 'front':
        c = cube(w * k, t * 8, h * (1 - z0 - 0.18), (0, -d / 2, h * z0))
        boolean(o, c)
        bpy.ops.mesh.primitive_plane_add(size=1, location=(0, -d / 2 + t * 1.2, h * z0 + h * (1 - z0 - 0.18) / 2))
        g = bpy.context.object; g.rotation_euler = (math.pi / 2, 0, 0)
        g.scale = (w * k + 0.6, h * (1 - z0 - 0.18) + 0.6, 1)
    else:
        c = cube(t * 8, d * k, h * (1 - z0 - 0.18), (w / 2, 0, h * z0))
        boolean(o, c)
        bpy.ops.mesh.primitive_plane_add(size=1, location=(w / 2 - t * 1.2, 0, h * z0 + h * (1 - z0 - 0.18) / 2))
        g = bpy.context.object; g.rotation_euler = (0, math.pi / 2, 0)
        g.scale = (h * (1 - z0 - 0.18) + 0.6, d * k + 0.6, 1)
    bpy.ops.object.transform_apply(scale=True, rotation=True)
    assign(g, mat('clear'))
    return g


def grid_for(n, w, d):
    rows_cols = {1: (1, 1), 2: (1, 2), 4: (2, 2), 6: (2, 3), 9: (3, 3), 12: (3, 4), 16: (4, 4)}[n]
    r, c = rows_cols
    if w < d:
        r, c = c, r
    return r, c    # r — по глубине (y), c — по ширине (x)


def holes(o, n, w, d, ztop, t, rk=0.37):
    r, c = grid_for(n, w, d)
    cw, cd = w / c, d / r
    rad = min(cw, cd) * rk
    cutters = []
    for i in range(c):
        for j in range(r):
            x = -w / 2 + cw * (i + 0.5); y = -d / 2 + cd * (j + 0.5)
            cutters.append(cyl(rad, t * 8, (x, y, ztop - t * 4), verts=64))
    cutter = join(cutters, 'holes'); boolean(o, cutter)


def thick(spec):
    return 0.3 if spec.get('corr') else 0.12


# ---------------------------------------------------------------- виды товаров

def build_lid(s):
    w, d, h, t = s['w'], s['d'], s['h'], thick(s)
    o = shell(w, d, h, t, s)
    parts = [o]
    if s['color'] == 'clear':
        assign(o, mat('clear'), mat('clear'))
        bevel(o, 0.25, 3)
        return parts
    if s.get('window') == 'top':
        parts.append(cut_window_top(o, w, d, h, t))
    elif s.get('window') == 'lace':
        cs = []
        R = min(w, d) * 0.36
        for ring, (rr, n, hr) in enumerate([(0, 1, 0.09), (0.42, 8, 0.07), (0.78, 14, 0.055), (1.0, 20, 0.04)]):
            for k in range(n):
                a = 2 * math.pi * k / n + ring * 0.2
                cs.append(cyl(R * hr * (1 if ring else 1.4), t * 8,
                              (math.cos(a) * R * rr, math.sin(a) * R * rr, h - t * 4), verts=32))
        boolean(o, join(cs))
    bevel(o, 0.08, 2)
    return parts


def build_box(s):
    w, d, h, t = s['w'], s['d'], s['h'], thick(s)
    lid_h = min(h * 0.32, 4.0) if h > 4 else h * 0.45
    base = shell(w, d, h - 0.05, t, s, open_top=True, name='base')
    lid = shell(w + 0.2, d + 0.2, lid_h, t, s, z=h - lid_h + 0.1, name='lid')
    parts = [base, lid]
    if s['color'] == 'clear':
        for o in parts:
            assign(o, mat('clear'), mat('clear')); bevel(o, 0.15, 3)
        return parts
    win = s.get('window', 'none')
    if win == 'top':
        parts.append(cut_window_top(lid, w + 0.2, d + 0.2, h + 0.1, t))
    if win in ('front', 'sides'):
        parts.append(cut_window_front(base, w, d, h - lid_h, t, 'front', z0=0.18, k=0.6))
    if win == 'sides':
        parts.append(cut_window_front(base, w, d, h - lid_h, t, 'right', z0=0.18, k=0.6))
    if s.get('holes'):
        ins = shell(w - 2 * t - 0.05, d - 2 * t - 0.05, h * 0.35, t, dict(s, color=s['color']), open_top=False, z=t)
        holes(ins, s['holes'], w, d, t + h * 0.35, t)
        parts.append(ins)
    for o in (base, lid):
        bevel(o, 0.06, 2)
    return parts


def build_flatbox(s):
    w, d, h, t = s['w'], s['d'], s['h'], thick(s)
    o = shell(w, d, h, t, s)
    # шов крышки-самосбора: узкая щель по переднему и боковым краям
    for c in (cube(w + 1, 0.06, 0.06, (0, -d / 2, h - 0.6)), cube(0.06, d + 1, 0.06, (w / 2, 0, h - 0.6)),
              cube(0.06, d + 1, 0.06, (-w / 2, 0, h - 0.6))):
        boolean(o, c)
    bevel(o, 0.08, 2)
    return [o]


def build_tray(s):
    w, d, h, t = s['w'], s['d'], s['h'], thick(s)
    o = shell(w, d, h, t, s, open_top=True)
    bevel(o, 0.06, 2)
    return [o]


def build_board(s):
    w, d, t = s['w'], s['d'], s['t']
    o = cube(w, d, t)
    edge = mat('flute') if s.get('corr') else (mat('kraft') if t >= 0.3 and s['color'] in ('gold', 'silver') else surf(s))
    top = mat('laminated') if t >= 0.35 and s['color'] == 'white' else surf(s)
    assign(o, top, edge)
    for p in o.data.polygons:
        p.material_index = 0 if abs(p.normal.z) > 0.5 else 1
    bevel(o, min(0.05, t * 0.3), 2)
    return [o]


def build_disk(s):
    w, t, sides = s['w'], s['t'], s.get('sides', 0)
    o = cyl(w / 2, t, verts=sides or 192)
    edge = mat('flute') if s.get('corr') else (mat('kraft') if t >= 0.3 and s['color'] in ('gold', 'silver') else surf(s))
    top = mat('laminated') if t >= 0.35 and s['color'] == 'white' else surf(s)
    assign(o, top, edge)
    for p in o.data.polygons:
        p.material_index = 0 if abs(p.normal.z) > 0.5 else 1
    if s['color'] in ('gold', 'silver') and not sides:
        # тиснение «ажурным» краем — канавка по окружности
        groove = cyl(w / 2 * 0.93, 0.03, (0, 0, t - 0.015), verts=192)
        inner = cyl(w / 2 * 0.91, 0.05, (0, 0, t - 0.02), verts=192)
        boolean(groove, inner); boolean(o, groove)
    bevel(o, min(0.04, t * 0.3), 2)
    return [o]


def build_bag(s):
    w, d, h = s['w'], s['d'], s['h']
    t = 0.06
    o = shell(w, d, h, t, s, open_top=True)
    # отворот сверху
    hem = shell(w + 0.04, d + 0.04, h * 0.06, t, dict(s, color='kraft_dark' if s['color'] == 'kraft' else s['color']),
                open_top=True, z=h * 0.94)
    hem.data.materials.clear(); hem.data.materials.append(surf(s))
    parts = [o, hem]
    # боковая складка: тонкая тёмная вертикаль
    crease = cube(0.08, 0.08, h * 0.86, (w / 2, 0, h * 0.07)); boolean(o, crease)
    if s.get('window_strip'):
        parts.append(cut_window_front(o, w, d, h, t, 'front', z0=0.12, k=0.45))
    if s.get('handles'):
        for y in (-d / 2 - 0.05, d / 2 + 0.05):
            cu = bpy.data.curves.new('h', 'CURVE'); cu.dimensions = '3D'
            cu.bevel_depth = max(0.18, w * 0.008); cu.bevel_resolution = 4
            sp = cu.splines.new('BEZIER'); sp.bezier_points.add(2)
            hw, hh = w * 0.2, max(6.0, w * 0.32)
            for bp, co in zip(sp.bezier_points, [(-hw, y, h - 2.5), (0, y, h + hh), (hw, y, h - 2.5)]):
                bp.co = co; bp.handle_left_type = bp.handle_right_type = 'AUTO'
            ob = bpy.data.objects.new('handle', cu); bpy.context.collection.objects.link(ob)
            cu.materials.append(mat('kraft_dark') if s['color'] == 'kraft' else mat('white'))
            parts.append(ob)
    if h > 2.6 * w:                       # длинный пакет — лёжа
        root = bpy.data.objects.new('root', None); bpy.context.collection.objects.link(root)
        for p in parts:
            p.parent = root
        root.rotation_euler = (0, math.pi / 2, 0); root.location = (h / 2, 0, d / 2)
        bpy.context.view_layer.update()
        parts.append(root)
    return parts


def build_sachet(s):
    w, h = s['w'], s['h']
    parts = []
    for i, (dx, dy, rot) in enumerate([(-0.18, 0.1, 0.18), (0.0, 0.0, 0.0), (0.2, -0.12, -0.16)]):
        o = cube(w, h, 0.12, (dx * w, dy * h, i * 0.14))
        o.rotation_euler = (0, 0, rot)
        assign(o, mat('frosted'))
        seam = cube(w, h * 0.08, 0.13, (dx * w, dy * h - h * 0.46, i * 0.14))
        seam.rotation_euler = (0, 0, rot); seam.parent = None
        assign(seam, mat('plain', (0.85, 0.85, 0.85)))
        parts += [o, seam]
    return parts


def tapered(w0, d0, w1, d1, h, t, m):
    bm = bmesh.new()
    vs = [bm.verts.new(v) for v in [(-w0 / 2, -d0 / 2, 0), (w0 / 2, -d0 / 2, 0), (w0 / 2, d0 / 2, 0), (-w0 / 2, d0 / 2, 0),
                                     (-w1 / 2, -d1 / 2, h), (w1 / 2, -d1 / 2, h), (w1 / 2, d1 / 2, h), (-w1 / 2, d1 / 2, h)]]
    bm.faces.new(vs[0:4][::-1])
    for i in range(4):
        j = (i + 1) % 4
        bm.faces.new([vs[i], vs[j], vs[4 + j], vs[4 + i]])
    me = bpy.data.meshes.new('t'); bm.to_mesh(me); bm.free()
    o = bpy.data.objects.new('t', me); bpy.context.collection.objects.link(o)
    so = o.modifiers.new('s', 'SOLIDIFY'); so.thickness = t; so.offset = -1
    bpy.context.view_layer.objects.active = o; bpy.ops.object.modifier_apply(modifier='s')
    assign(o, m)
    return o


def build_fries(s):
    w, d, h = s['w'], s['d'], s['h']
    o = tapered(w * 0.62, d * 0.55, w, d, h, 0.1, surf(s))
    c = cube(w * 3, d * 3, h, (0, 0, h * 0.78)); c.rotation_euler = (math.radians(-22), 0, 0)
    boolean(o, c)
    bevel(o, 0.04, 2)
    return [o]


def build_cupholder(s):
    w, d, h = s['w'], s['d'], s['h']
    deck = shell(w, d, 3.5, 0.15, s)
    for x in (-w / 4, w / 4):
        boolean(deck, cyl(4.0, 2, (x, 0, 2.5)))
    panel = cube(w * 0.84, 0.25, h, (0, 0, 0))
    assign(panel, surf(s))
    grip = cube(w * 0.26, 1, 1.8, (0, 0, h - 3)); bevel(grip, 0.8, 6)
    bpy.context.view_layer.objects.active = grip; bpy.ops.object.modifier_apply(modifier='bv')
    boolean(panel, grip)
    bevel(panel, 0.08, 2); bevel(deck, 0.06, 2)
    return [deck, panel]


def prism_top(w, d, h, z, m):
    bm = bmesh.new()
    pts = [(-w / 2, -d / 2, z), (w / 2, -d / 2, z), (w / 2, d / 2, z), (-w / 2, d / 2, z),
           (-w / 2, 0, z + h), (w / 2, 0, z + h)]
    v = [bm.verts.new(p) for p in pts]
    for f in ([v[0], v[1], v[5], v[4]], [v[2], v[3], v[4], v[5]], [v[0], v[4], v[3]], [v[1], v[2], v[5]], [v[3], v[2], v[1], v[0]]):
        bm.faces.new(f)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    me = bpy.data.meshes.new('p'); bm.to_mesh(me); bm.free()
    o = bpy.data.objects.new('p', me); bpy.context.collection.objects.link(o)
    assign(o, m)
    return o


def build_kidsbox(s):
    w, d, h = s['w'], s['d'], s['h']
    body = shell(w, d, h * 0.7, 0.12, s, open_top=True)
    roof = prism_top(w, d, h * 0.3, h * 0.7, surf(s))
    handle = cube(w * 0.9, 0.2, h * 0.32, (0, 0, h * 0.95))
    assign(handle, surf(s))
    g = cube(w * 0.35, 1, h * 0.09, (0, 0, h * 1.1)); bevel(g, h * 0.04, 6)
    bpy.context.view_layer.objects.active = g; bpy.ops.object.modifier_apply(modifier='bv')
    boolean(handle, g)
    for o in (body, roof, handle):
        bevel(o, 0.05, 2)
    return [body, roof, handle]


def build_handlebox(s):
    w, d, h = s['w'], s['d'], s['h']
    s2 = dict(s, h=h * 1.0)
    return build_kidsbox(dict(s2, w=w, d=d, h=h * 1.25))


def build_burger(s):
    w, d, h = s['w'], s['d'], s['h']
    lo = tapered(w * 0.86, d * 0.86, w, d, h * 0.42, 0.12, surf(s))
    top = cube(w, d, h * 0.56, (0, 0, h * 0.44)); assign(top, surf(s))
    b = top.modifiers.new('bv', 'BEVEL'); b.width = h * 0.22; b.segments = 6; b.limit_method = 'ANGLE'
    tab = cube(w * 0.25, 0.6, 0.12, (0, -d / 2 - 0.25, h * 0.42)); assign(tab, surf(s))
    return [lo, top, tab]


def build_pallet(s):
    w, d, h = s['w'], s['d'], s['h']
    o = tapered(w * 0.8, d * 0.8, w, d, h, 0.08, surf(s))
    if s.get('wave'):
        cs = []
        n = 7
        for i in range(n):
            x = -w / 2 + w * (i + 0.5) / n
            for y in (-d / 2, d / 2):
                c = cyl(w / n * 0.38, 3, (x, y, h * 0.92), verts=32); c.rotation_euler = (math.pi / 2, 0, 0)
                c.location = (x, y, h * 1.02); cs.append(c)
        nd = max(3, round(d / (w / n)))
        for j in range(nd):
            y = -d / 2 + d * (j + 0.5) / nd
            for x in (-w / 2, w / 2):
                c = cyl(d / nd * 0.38, 3, (x, y, h), verts=32); c.rotation_euler = (0, math.pi / 2, 0)
                c.location = (x, y, h * 1.02); cs.append(c)
        bpy.context.view_layer.update()
        boolean(o, join(cs))
    bevel(o, 0.04, 2)
    return [o]


def build_insert(s):
    w, d, h, t = s['w'], s['d'], s['h'], 0.15
    o = shell(w, d, h, t, s)
    holes(o, s['holes'], w, d, h, t, rk=0.37 if s['holes'] > 1 else 0.36)
    bevel(o, 0.05, 2)
    return [o]


def build_dividers(s):
    w, h = s['w'], s['h']
    a = cube(w, 0.1, h); b = cube(0.1, w, h)
    for o in (a, b):
        assign(o, surf(s)); bevel(o, 0.03, 1)
    return [a, b]


def build_candygrid(s):
    w, d, h, t = s['w'], s['d'], s['h'], 0.12
    tray = shell(w, d, h, t, s, open_top=True)
    walls = []
    for i in (1, 2, 3):
        walls.append(cube(w - 2 * t, 0.06, h * 0.85, (0, -d / 2 + d * i / 4, t)))
        walls.append(cube(0.06, d - 2 * t, h * 0.85, (-w / 2 + w * i / 4, 0, t)))
    for o in walls:
        assign(o, surf(s))
    lid = shell(w + 0.2, d + 0.2, h * 0.6, t, s)
    lid.location = (w * 0.45, d * 0.9, 0); lid.rotation_euler = (0, 0, 0.25)
    bevel(tray, 0.04, 2); bevel(lid, 0.04, 2)
    return [tray, lid] + walls


def build_sleevebox(s):
    w, d, h = s['w'], s['d'], s['h']
    box = shell(w, d, h, 0.12, s)
    sleeve = shell(w * 0.62, d + 0.08, h + 0.08, 0.04, dict(s, color='kraft_dark'), z=-0.04)
    assign(sleeve, mat('kraft_dark'), mat('kraft_dark'))
    bevel(box, 0.05, 2); bevel(sleeve, 0.03, 2)
    return [box, sleeve]


def build_folder(s):
    w, d = s['w'], s['d']
    m = mat(s['color'])
    back = cube(w, d, 0.06); assign(back, m)
    front = cube(w, d, 0.06, (w / 2, 0, 0)); assign(front, m)
    front.location = (-w / 2, 0, 0.06)
    # обложка приоткрыта: петля по левому краю
    bpy.context.view_layer.update()
    pivot = bpy.data.objects.new('pivot', None); bpy.context.collection.objects.link(pivot)
    pivot.location = (-w / 2, 0, 0.06)
    front.parent = pivot; front.location = (0, 0, 0)
    pivot.rotation_euler = (0, math.radians(-24), 0)
    parts = [back, front, pivot]
    if s.get('fastener'):
        strip = cube(1.4, d * 0.8, 0.05, (-w / 2 + 2.6, 0, 0.06)); assign(strip, mat('silver'))
        parts.append(strip)
        for y in (-4, 4):
            prong = cube(0.3, 0.6, 0.6, (-w / 2 + 2.6, y, 0.11)); assign(prong, mat('silver')); parts.append(prong)
    if s.get('ties'):
        for z, rot in ((0.07, 0.15), (0.0, -0.22)):
            tie = cube(9, 0.9, 0.02, (w / 2 + 4.2, 0, z)); tie.rotation_euler = (0, 0, rot)
            assign(tie, mat('plain', (0.82, 0.80, 0.76))); parts.append(tie)
    bpy.context.view_layer.update()
    return parts


# ---------------------------------------------------------------- декор

def build_ribbon(s):
    W = s['width'] / 10                       # мм → см
    m = mat('satin' if s.get('satin') else 'grosgrain', s['rgb'])
    core = cyl(2.6, W + 0.3); assign(core, mat('kraft'))
    hole = cyl(2.3, W + 1, (0, 0, -0.5)); boolean(core, hole)
    roll = cyl(4.6, W, (0, 0, 0.15)); assign(roll, m)
    hole = cyl(2.6, W + 1, (0, 0, -0.5)); boolean(roll, hole)
    # хвост ленты: от касательной точки по полу волной
    cu = bpy.data.curves.new('tail', 'CURVE'); cu.dimensions = '3D'; cu.extrude = W / 2
    cu.twist_mode = 'Z_UP'
    sp = cu.splines.new('BEZIER'); pts = [(4.6, 0, W / 2 + 0.15), (8, -3, W / 2), (11, -5.5, 0.08), (14, -4, 0.06), (16.5, -6.5, 0.06)]
    sp.bezier_points.add(len(pts) - 1)
    for i, (bp, co) in enumerate(zip(sp.bezier_points, pts)):
        bp.co = co; bp.handle_left_type = bp.handle_right_type = 'AUTO'
        bp.tilt = 0 if i < 2 else math.pi / 2
    ob = bpy.data.objects.new('tail', cu); bpy.context.collection.objects.link(ob)
    cu.materials.append(m)
    smooth(roll)
    return [core, roll, ob]


def build_candle_num(s):
    cols = {'1': (0.85, 0.42, 0.55), '3': (0.32, 0.62, 0.78), '7': (0.92, 0.62, 0.30)}
    cu = bpy.data.curves.new('t', 'FONT'); cu.body = s['digit']
    try:
        cu.font = bpy.data.fonts.load(FONT_BOLD)
    except Exception:
        pass
    cu.size = 8; cu.extrude = 0.7; cu.bevel_depth = 0.12; cu.bevel_resolution = 3; cu.align_x = 'CENTER'
    o = bpy.data.objects.new('digit', cu); bpy.context.collection.objects.link(o)
    o.rotation_euler = (math.pi / 2, 0, math.radians(-20))
    cu.materials.append(mat('wax', cols.get(s['digit'], (0.85, 0.42, 0.55))))
    bpy.context.view_layer.update()
    bb = [o.matrix_world @ Vector(c) for c in o.bound_box]
    zmin, zmax = min(p.z for p in bb), max(p.z for p in bb)
    o.location.z -= zmin
    wick = cyl(0.06, 0.9, (0, 0, zmax - zmin - 0.1)); assign(wick, mat('plain', (0.85, 0.85, 0.8)))
    stick = cyl(0.12, 2.2, (0, 0, -2.2)); assign(stick, mat('plain', (0.9, 0.9, 0.9)))
    o.location.z += 0; stick.location.z = -2.2 + 0.4
    for p in (o, wick, stick):
        p.location.z += 1.8
    return [o, wick, stick]


def build_candles_thin(s):
    pal = [(0.85, 0.20, 0.40), (0.15, 0.45, 0.85), (0.95, 0.75, 0.10), (0.20, 0.65, 0.40), (0.55, 0.30, 0.80),
           (0.95, 0.45, 0.20)] if s.get('mix') else [(0.92, 0.91, 0.88)]
    parts = []
    L = 7.5
    for i in range(9):
        a = math.radians(-32 + i * 8)
        c = cyl(0.28, L, (0, 0, -L / 2), verts=32)              # центр в нуле, вдоль Z
        wick = cyl(0.045, 0.7, (0, 0, L / 2 - 0.05), verts=12)
        assign(c, mat('wax', pal[i % len(pal)])); assign(wick, mat('plain', (0.92, 0.9, 0.84)))
        # укладываем веером: ось Z → направление a в плоскости пола, основания у центра
        rot = Matrix.Rotation(a, 4, 'Z') @ Matrix.Rotation(math.pi / 2, 4, 'Y')
        off = Matrix.Translation(Vector((math.cos(a), math.sin(a), 0)) * (L / 2 + 1.5) + Vector((0, 0, 0.28 + i * 0.001)))
        for o in (c, wick):
            o.matrix_world = off @ rot @ o.matrix_world
        smooth(c)
        parts += [c, wick]
    return parts


def build_fountain(s):
    h = s['h']
    parts = []
    head = 3.2 if h <= 12 else 4.5
    for i, a in enumerate((-14, 0, 14)):
        ar = math.radians(a)
        stick = cyl(0.12, h - head, (0, 0, 0), verts=24); assign(stick, mat('plain', (0.55, 0.42, 0.28)))
        top = cyl(0.42, head, (0, 0, h - head), verts=48); assign(top, mat('foil'))
        tip = cyl(0.1, 0.4, (0, 0, h)); assign(tip, mat('plain', (0.05, 0.05, 0.05)))
        for o in (stick, top, tip):
            o.rotation_euler = (0, math.pi / 2, ar)
            p = o.location.copy()
            o.location = Vector((p.z * math.cos(ar), p.z * math.sin(ar) + i * 0.9, 0.45))
        parts += [stick, top, tip]
    return parts


def mound(n, R, H, rnd, rmin):
    pts = []
    tries = 0
    while len(pts) < n and tries < n * 60:
        tries += 1
        r = R * math.sqrt(rnd.random()); a = rnd.random() * 2 * math.pi
        x, y = r * math.cos(a), r * math.sin(a)
        top = H * max(0.0, 1 - (r / R) ** 1.4)
        z = rnd.uniform(0, top)
        p = Vector((x, y, z))
        if all((p - q).length > rmin for q in pts):
            pts.append(p)
    return pts


def scatter(proto_fn, pts, mats_list, rnd, name='pile', tilt=0.6):
    bm = bmesh.new()
    for i, p in enumerate(pts):
        tmp = bmesh.new()
        proto_fn(tmp, rnd)
        rot = Matrix.Rotation(rnd.uniform(-tilt, tilt), 4, 'X') @ Matrix.Rotation(rnd.uniform(-tilt, tilt), 4, 'Y') @ \
            Matrix.Rotation(rnd.uniform(0, 6.3), 4, 'Z')
        bmesh.ops.transform(tmp, matrix=Matrix.Translation(p) @ rot, verts=tmp.verts)
        mi = rnd.randrange(len(mats_list))
        for f in tmp.faces:
            f.material_index = mi; f.smooth = True
        me = bpy.data.meshes.new('tmp'); tmp.to_mesh(me); tmp.free()
        bm.from_mesh(me); bpy.data.meshes.remove(me)
    me = bpy.data.meshes.new(name); bm.to_mesh(me); bm.free()
    o = bpy.data.objects.new(name, me); bpy.context.collection.objects.link(o)
    for m in mats_list:
        o.data.materials.append(m)
    return o


def callet(bm, rnd):
    bmesh.ops.create_uvsphere(bm, u_segments=20, v_segments=10, radius=0.75)
    for v in bm.verts:
        v.co.z *= 0.38 if v.co.z > 0 else 0.12
        v.co.x *= 1 + rnd.uniform(-0.04, 0.04)


def drop(bm, rnd):
    bmesh.ops.create_uvsphere(bm, u_segments=16, v_segments=10, radius=0.32)
    for v in bm.verts:
        if v.co.z > 0:
            k = v.co.z / 0.32
            v.co.z *= 2.0
            v.co.x *= 1 - 0.75 * k; v.co.y *= 1 - 0.75 * k
        else:
            v.co.z *= 0.3


def dragee(bm, rnd):
    bmesh.ops.create_icosphere(bm, subdivisions=2, radius=0.2)


def flake(bm, rnd):
    res = bmesh.ops.create_grid(bm, x_segments=6, y_segments=4, size=rnd.uniform(0.5, 0.9))
    for v in bm.verts:
        v.co.z = 0.6 * v.co.x ** 2 + rnd.uniform(-0.03, 0.03)
    bmesh.ops.solidify(bm, geom=bm.faces[:], thickness=0.04)


def build_callets(s):
    rnd = random.Random(3)
    pts = mound(230, 5.2, 3.0, rnd, 0.55) + [Vector((rnd.uniform(-7, 7), rnd.uniform(-7, -5.6), 0)) for _ in range(6)]
    return [scatter(callet, pts, [mat('choc', c) for c in s['rgb']], rnd)]


def build_drops(s):
    rnd = random.Random(4)
    pts = mound(520, 4.6, 2.5, rnd, 0.38)
    return [scatter(drop, pts, [mat('choc', s['rgb'])], rnd, tilt=0.25)]


def build_sprinkles(s):
    rnd = random.Random(5)
    pal = [(0.85, 0.12, 0.18), (0.95, 0.55, 0.05), (0.95, 0.85, 0.10), (0.15, 0.62, 0.28), (0.10, 0.35, 0.85),
           (0.55, 0.20, 0.70), (0.95, 0.95, 0.95), (0.92, 0.45, 0.65)]
    n = 600 if s.get('grams', 50) < 100 else 900
    pts = mound(n, 4.2 if n < 700 else 4.8, 2.4, rnd, 0.36)
    mats_l = []
    for c in pal:
        m = mat('gel', c); mats_l.append(m)
    return [scatter(dragee, pts, mats_l, rnd)]


def build_flakes(s):
    rnd = random.Random(6)
    pts = mound(260, 4.6, 2.4, rnd, 0.42)
    return [scatter(flake, pts, [mat('choc', (0.62, 0.38, 0.14)), mat('choc', (0.55, 0.32, 0.11))], rnd, tilt=1.5)]


def build_chunks(s):
    rnd = random.Random(8)
    parts = []
    for i, (x, y, sz) in enumerate([(0, 0, 3.4), (3.4, -1.2, 2.6), (-3.0, -1.6, 2.4), (1.0, 2.6, 2.2), (-1.2, -3.4, 1.6)]):
        bpy.ops.mesh.primitive_cube_add(size=1, location=(x, y, sz * 0.3))
        o = bpy.context.object; o.scale = (sz, sz * rnd.uniform(0.7, 1.0), sz * 0.55)
        o.rotation_euler = (rnd.uniform(-0.15, 0.15), rnd.uniform(-0.15, 0.15), rnd.uniform(0, 3))
        bpy.ops.object.transform_apply(scale=True)
        sub = o.modifiers.new('s', 'SUBSURF'); sub.levels = 4; sub.subdivision_type = 'SIMPLE'
        tex = bpy.data.textures.new(f'n{i}', 'VORONOI'); tex.noise_scale = sz * 0.45
        dm = o.modifiers.new('d', 'DISPLACE'); dm.texture = tex; dm.strength = sz * 0.12
        sm = o.modifiers.new('sm', 'SMOOTH'); sm.iterations = 3
        bv = o.modifiers.new('b', 'BEVEL'); bv.width = 0.05
        assign(o, mat('choc', s['rgb']))
        smooth(o); parts.append(o)
    return parts


def build_powder(s):
    R = 4.8
    bm = bmesh.new()
    bmesh.ops.create_grid(bm, x_segments=140, y_segments=140, size=R * 1.02)
    for v in list(bm.verts):
        r = math.hypot(v.co.x, v.co.y) / R
        if r > 1.0:
            bm.verts.remove(v)
            continue
        v.co.z = 3.8 * max(0.0, 1 - r ** 1.6) ** 1.3
    me = bpy.data.meshes.new('powder'); bm.to_mesh(me); bm.free()
    o = bpy.data.objects.new('powder', me); bpy.context.collection.objects.link(o)
    tex = bpy.data.textures.new('pn', 'CLOUDS'); tex.noise_scale = 0.9
    dm = o.modifiers.new('d', 'DISPLACE'); dm.texture = tex; dm.strength = 0.25
    sm = o.modifiers.new('sm', 'SMOOTH'); sm.iterations = 4
    assign(o, mat('powder', s['rgb'])); smooth(o)
    return [o]


def build_dye(s):
    body = cyl(1.45, 5.2, verts=64); assign(body, mat('bottle'))
    bv = bevel(body, 0.35, 4, 60)
    sh = cyl(0.95, 0.8, (0, 0, 5.2), verts=64); assign(sh, mat('bottle'))
    cap = cyl(0.7, 1.6, (0, 0, 6.0), verts=64); assign(cap, mat('plain', s['rgb']))
    tip = bpy.ops.mesh.primitive_cone_add(radius1=0.7, radius2=0.18, depth=1.0, location=(0, 0, 8.1), vertices=48)
    tip = bpy.context.object; assign(tip, mat('plain', s['rgb']))
    blob = cyl(1.0, 0.14, (3.0, -1.6, 0), verts=64); assign(blob, mat('gel', s['rgb']))
    bevel(blob, 0.06, 4, 30)
    blob2 = cyl(0.45, 0.12, (3.9, -0.6, 0), verts=48); assign(blob2, mat('gel', s['rgb'])); bevel(blob2, 0.05, 4, 30)
    for o in (body, sh, cap, tip):
        smooth(o)
    return [body, sh, cap, tip, blob, blob2]


def build_topper(s):
    m = mat('mirror_gold') if s.get('gold') else mat('glossy_black')
    parts = []
    font = None
    try:
        font = bpy.data.fonts.load(FONT_SCRIPT)
    except Exception:
        pass
    for line, z, size in (('С днём', 4.6, 4.6), ('рождения', 1.6, 4.6)):
        cu = bpy.data.curves.new('t', 'FONT'); cu.body = line
        if font:
            cu.font = font
        cu.size = size; cu.extrude = 0.15; cu.align_x = 'CENTER'; cu.space_character = 0.9
        o = bpy.data.objects.new('t', cu); bpy.context.collection.objects.link(o)
        o.rotation_euler = (math.pi / 2, 0, 0); o.location = (0, 0, z)
        cu.materials.append(m); parts.append(o)
    bar = cube(12.6, 0.3, 0.4, (0, 0, 0.72)); assign(bar, m)
    for x in (-3.5, 3.5):
        st = cube(0.5, 0.3, 5.5, (x, 0, -4.75)); assign(st, m); parts.append(st)
    parts.append(bar)
    root = bpy.data.objects.new('root', None); bpy.context.collection.objects.link(root)
    for p in parts:
        p.parent = root
    root.rotation_euler = (0, 0, math.radians(-10)); root.location.z = 3.95
    bpy.context.view_layer.update()
    return parts + [root]


BUILD = {k[6:]: v for k, v in globals().items() if k.startswith('build_')}
VIEW = {   # elevation, azimuth
    'disk': (38, 30), 'board': (36, 32), 'folder': (48, 20), 'sachet': (58, 15), 'ribbon': (40, 20),
    'callets': (34, 20), 'drops': (34, 20), 'sprinkles': (34, 20), 'flakes': (34, 20), 'chunks': (32, 20),
    'powder': (22, 20), 'topper': (6, 6), 'candle_num': (12, 10), 'dye': (18, 15), 'candles_thin': (50, 0),
    'fountain': (52, 0), 'bag': (18, 28), 'fries': (18, 25), 'cupholder': (24, 28), 'kidsbox': (22, 30),
}


def render(spec, out):
    clear()
    objs = BUILD[spec['kind']](spec)
    meshes = [o for o in objs if o.type in ('MESH', 'CURVE', 'FONT')]
    bpy.context.view_layer.update()
    pts = [o.matrix_world @ Vector(c) for o in meshes for c in o.bound_box]
    zmin = min(p.z for p in pts)
    size = max(max(p.x for p in pts) - min(p.x for p in pts), max(p.y for p in pts) - min(p.y for p in pts),
               max(p.z for p in pts) - zmin)
    g = ground(zmin, size)
    el, az = VIEW.get(spec['kind'], (28, 32))
    camera(el, az, objs=meshes)
    lights(size / 10)
    bpy.context.scene.render.filepath = str(out)
    bpy.ops.render.render(write_still=True)
    # рамка самого товара в пикселях (без тени) — по ней process кадрирует и центрирует
    from bpy_extras.object_utils import world_to_camera_view
    sc, cam = bpy.context.scene, bpy.context.scene.camera
    dg = bpy.context.evaluated_depsgraph_get()
    xs, ys = [], []
    for o in meshes:
        ev = o.evaluated_get(dg)
        try:
            me = ev.to_mesh()
        except RuntimeError:
            continue
        for v in me.vertices:
            p = world_to_camera_view(sc, cam, ev.matrix_world @ v.co)
            xs.append(p.x); ys.append(p.y)
        ev.to_mesh_clear()
    bbox = [min(xs) * RES, (1 - max(ys)) * RES, max(xs) * RES, (1 - min(ys)) * RES]
    out.with_suffix('.json').write_text(json.dumps({'bbox': [round(v, 1) for v in bbox], 'spec': spec}, ensure_ascii=False))


def main():
    setup()
    specs = json.loads(SPECS.read_text('utf8'))
    OUT.mkdir(parents=True, exist_ok=True)
    for i, sp in enumerate(specs, 1):
        if ONLY and sp['key'] not in ONLY and sp['kind'] not in ONLY:
            continue
        out = OUT / f"render-{sp['key']}.png"
        if out.exists() and '--refresh' not in argv:
            continue
        try:
            render(sp, out)
            print(f'RENDER {i}/{len(specs)} {sp["key"]}', flush=True)
        except Exception as e:
            import traceback; traceback.print_exc()
            print(f'FAIL {sp["key"]}: {e}', flush=True)


main()
