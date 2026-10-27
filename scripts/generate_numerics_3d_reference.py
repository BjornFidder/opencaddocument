"""Manual, standard-library-only reference generation; never a build/CI step.

All inputs are frozen binary64 numbers. Geometry is evaluated with Decimal at
100 digits, independently of the Rust/CAD placement implementations. The conic
coefficient comparison is a numerical bound, not an interval certificate.
Run without arguments to regenerate the oracle; --init also regenerates cases.
"""
import argparse
from decimal import Decimal as D, getcontext
import hashlib
import json
import math
from pathlib import Path

getcontext().prec = 100
PI = D('3.1415926535897932384626433832795028841971693993751058209749445923078164062862089986280348253421170679')
ROOT = Path(__file__).resolve().parents[1] / 'tests' / 'fixtures'


def dec(x):
    return D.from_float(x) if isinstance(x, float) else D(x)


def vec(xs):
    return tuple(map(dec, xs))


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def mul(a, b):
    return tuple(x * dec(b) for x in a)


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def unit(a):
    return mul(a, 1 / dot(a, a).sqrt())


def sin(x):
    x = dec(x) % (2 * PI)
    if x > PI:
        x -= 2 * PI
    term = total = x
    n = 1
    while abs(term) > D('1e-105'):
        term *= -x*x / ((2*n)*(2*n+1))
        total += term
        n += 1
    return total


def cos(x):
    return sin(dec(x) + PI / 2)


ZERO = vec([0, 0, 0])
EYE = [vec([1, 0, 0]), vec([0, 1, 0]), vec([0, 0, 1])]


def linear(cols, p):
    return tuple(sum(cols[j][i]*p[j] for j in range(3)) for i in range(3))


def at(t, p):
    return add(t[1], linear(t[0], p))


def compose(a, b):
    return [linear(a[0], c) for c in b[0]], at(a, b[1])


def aaa(normal, reference):
    n = unit(vec(normal))
    # The discrete branch follows the normalized binary64 source, not a
    # hypothetical infinitely precise normalization at an exact threshold.
    ref = EYE[1] if reference == 'y' else EYE[2]
    x = unit(cross(ref, n))
    return [x, cross(n, x), n]


def cad_frame(p):
    axes = aaa(p['normal'], p['aaa_reference'])
    angle = dec(p['rotation']) + dec(p.get('extra_rotation', 0))
    c, s = cos(angle), sin(angle)
    return [add(mul(axes[0], c), mul(axes[1], s)),
            add(mul(axes[0], -s), mul(axes[1], c)), axes[2]], linear(axes, vec(p['origin']))


def native_frame(p):
    f = p['native']
    x, y = vec(f['x']), vec(f['y'])
    return [x, y, cross(x, y)], vec(f['origin'])


def transform(p, direction):
    axes, origin = cad_frame(p) if direction == 'cad' else native_frame(p)
    if direction == 'native':
        c, s = cos(p.get('extra_rotation', 0)), sin(p.get('extra_rotation', 0))
        axes = [add(mul(axes[0], c), mul(axes[1], s)), add(mul(axes[0], -s), mul(axes[1], c)), axes[2]]
    cols = [mul(a, s) for a, s in zip(axes, p['scale'])]
    return cols, add(origin, mul(linear(cols, vec(p['base'])), -1))


def pose(normal, origin=(100., 200., 300.), rotation=0., scale=(1., 1., 1.), base=(12.25, -6.75, 0.)):
    p = dict(normal=list(normal), origin=list(origin), rotation=rotation, scale=list(scale), base=list(base))
    length = math.sqrt(sum(n*n for n in normal))
    normalized = [n/length for n in normal]
    p['aaa_reference'] = 'y' if abs(normalized[0]) < 1/64 and abs(normalized[1]) < 1/64 else 'z'
    axes, o = cad_frame(p)
    p['native'] = dict(origin=list(map(float, o)), x=list(map(float, axes[0])), y=list(map(float, axes[1])))
    return p


def make_cases():
    cases = []
    def case(name, poses, budget=1e-6, unit='cm', mm=10., transports=False):
        cases.append(dict(name=name, poses=poses, budget_mm=budget, unit=unit, mm_per_unit=mm, transports=transports))
    nx = 1/64 - 2e-13
    case('aa_x_below', [pose([nx, 0., math.sqrt(1-nx*nx)])], transports=True)
    for axis in range(2):
        for sign in [-1, 1]:
            for label, x in [('below', 1/64-1e-13), ('above', 1/64+1e-13),
                             ('below_far', 1/64-2e-13), ('above_far', 1/64+2e-13),
                             ('exact', 1/64), ('prev', math.nextafter(1/64, 0)),
                             ('next', math.nextafter(1/64, 1))]:
                n = [0., 0., math.sqrt(1-x*x)]
                n[axis] = sign*x
                case(f'aa_{axis}_{sign}_{label}', [pose(n, rotation=.37)])
    for sign in [-1, 1]:
        case(f'pole_{sign}', [pose([0., 0., float(sign)], rotation=.61)])
        case(f'near_pole_{sign}', [pose([1e-14, -2e-14, float(sign)], rotation=.27)])
    for axis in range(3):
        scale = [2., 3., .5]
        scale[axis] *= -1
        case(f'mirror_{axis}', [pose([.3, .4, .8660254037844386], rotation=.37, scale=scale)])
    for unit, mm in [('mm', 1.), ('cm', 10.), ('m', 1000.), ('in', 25.4)]:
        f = 10/mm
        inner = pose([.3, .4, .8660254037844386], origin=[100*f, 200*f, 300*f], rotation=.37,
                     scale=[-2., 3., .5], base=[12.25*f, -6.75*f, 0.])
        outer = pose([.2, -.3, .9327379053088815], origin=[15500000*f, 46300000*f, 1200*f],
                     rotation=-.43, scale=[1., -1., 2.], base=[2*f, 3*f, 4*f])
        case('nested_'+unit, [inner, outer], 1e-5, unit, mm, unit == 'cm')
        cases[-1]['physical_equivalent'] = True
        # Fractional marker inverse qualification is deliberately PR 2 scope.
        cases[-1]['picture'] = unit == 'cm'
    case('three_levels', [pose([.3, .4, .8660254037844386], rotation=.37),
                         pose([-.4, .3, .8660254037844386], rotation=-.43, base=[2., 3., 4.]),
                         pose([.2, -.3, .9327379053088815], rotation=.27, scale=[-1., 2., 1.])], 1e-5)
    for scale, budget in [(.001, 1e-6), (1000., 1e-4)]:
        case('scale_'+str(scale), [pose([.3, .4, .8660254037844386], rotation=.37, scale=[scale]*3)], budget)
    separate = pose([.3, .4, .8660254037844386], rotation=.61, scale=[-2., .5, 1.])
    separate['extra_rotation'] = .27
    case('separate_rotation', [separate])
    return cases


def geometry(case, direction):
    t = (EYE, ZERO)
    for p in reversed(case['poses']):
        t = compose(t, transform(p, direction))
    factor = D(10)/dec(case['mm_per_unit']) if case.get('physical_equivalent') else D(1)
    out = {}
    def point(key, p):
        out[key] = {'point': at(t, mul(vec(p), factor))}
    def circle(key, center, radius):
        out[key] = dict(center=at(t, mul(vec(center), factor)), a=linear(t[0], vec([dec(radius)*factor, 0, 0])),
                        b=linear(t[0], vec([0, dec(radius)*factor, 0])), sweep=2*PI, circle=True)
    point('line/start', [0, 0, 0])
    point('line/end', [4000, 0, 0])
    circle('circle', [30, 5, 2], 3)
    start = dec(.5)
    r = D('7.5')*factor
    out['arc'] = dict(center=at(t, mul(vec([-10, 20, 0]), factor)),
                      a=linear(t[0], (r*cos(start), r*sin(start), D(0))),
                      b=linear(t[0], (-r*sin(start), r*cos(start), D(0))), sweep=D('1.5'), circle=False)
    point('dimension/first', [16, 8, 0])
    point('dimension/second', [4016, 8, 0])
    point('dimension/line', [16, 308, 0])
    if case.get('picture') is False:
        return out
    point('picture/line/start', [16, 18, 0])
    point('picture/line/end', [56, 18, 0])
    circle('picture/circle', [18, 12, 0], 1.5)
    point('picture/point', [19, 13, 1])
    # Dimension origin + marker origin + signed scale * (member - block base).
    point('picture/marker/start', [26, 15, 0])
    point('picture/marker/end', [24, 17, 0])
    return out


def write(path, obj):
    path.write_text(json.dumps(obj, indent=2, default=lambda d: float(d))+'\n', encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--init', action='store_true')
    args = parser.parse_args()
    inputs = ROOT / 'numerics-3d-cases.json'
    if args.init:
        write(inputs, make_cases())
    cases = json.loads(inputs.read_text(encoding='utf-8'))
    analytic = at(transform(pose([0., 0., 1.], [10., 20., 30.], scale=[2., -3., 1.], base=[4., 5., 6.]), 'cad'), vec([7, 8, 9]))
    assert max(abs(x-y) for x, y in zip(analytic, vec([16, 11, 33]))) < D('1e-90')
    write(ROOT / 'numerics-3d-oracle.json', dict(precision=100, input_sha256=hashlib.sha256(inputs.read_bytes()).hexdigest(),
          cases={c['name']: {d: geometry(c, d) for d in ['cad', 'native']} for c in cases}))


if __name__ == '__main__':
    main()
