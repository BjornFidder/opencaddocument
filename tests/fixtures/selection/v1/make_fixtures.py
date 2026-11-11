"""Independent MessagePack fixture author. Uses only the Python standard library."""
import hashlib
import struct
from pathlib import Path


def pack(x):
    if x is None:
        return b'\xc0'
    if isinstance(x, bool):
        return bytes([0xc3 if x else 0xc2])
    if isinstance(x, int):
        if -32 <= x <= 127:
            return bytes([x & 255])
        if x >= 0:
            for code, fmt, maximum in [(0xcc,'B',255),(0xcd,'H',65535),
                                       (0xce,'I',2**32-1),(0xcf,'Q',2**64-1)]:
                if x <= maximum:
                    return bytes([code]) + struct.pack('>' + fmt, x)
        else:
            for code, fmt, minimum in [(0xd0,'b',-128),(0xd1,'h',-32768),
                                       (0xd2,'i',-2**31),(0xd3,'q',-2**63)]:
                if x >= minimum:
                    return bytes([code]) + struct.pack('>' + fmt, x)
    if isinstance(x, str):
        data = x.encode('utf-8')
        n = len(data)
        return (bytes([0xa0+n]) if n < 32 else b'\xd9'+bytes([n])) + data
    if isinstance(x, list):
        assert len(x) < 16
        return bytes([0x90+len(x)]) + b''.join(map(pack, x))
    if isinstance(x, dict):
        n = len(x)
        header = bytes([0x80+n]) if n < 16 else b'\xde'+struct.pack('>H',n)
        return header + b''.join(pack(k)+pack(v) for k,v in x.items())
    raise ValueError(x)


def frame(value):
    body = pack(value)
    return b'OCADSEL\0' + struct.pack('>HBI', 1, 1, len(body)) + body


def bits(x):
    return struct.unpack('>Q', struct.pack('>d', x))[0]


def vector(*x):
    return list(map(bits, x))


def key(kind, handle):
    return dict(kind=kind, handle=handle)


def common(handle, layer, layer_handle, linetype, linetype_handle):
    return dict(handle=handle, layer=layer, layer_handle=layer_handle,
                color=['by_layer'], line_weight=['value', -1], linetype=linetype,
                linetype_handle=linetype_handle, linetype_scale=bits(1),
                transparency=['explicit',128], color_name=None, invisible=False,
                owner_handle=5, material_flags=0, shadow_flags=0, plotstyle_flags=0,
                entity_mode=2)


def record(k, tag, payload):
    return dict(key=k, payload=[tag,payload])


def mline(c, h):
    return dict(common=c, flags=-32767, justification='zero', normal=vector(0,0,1),
                scale_factor=bits(1), start_point=vector(0,0,0), style_handle=h,
                style_name='Walls', style_element_count=2, vertices=[
                    dict(position=vector(0,0,0), direction=vector(1,0,0),
                         miter=vector(0,2,0), segments=[
                            dict(parameters=[0x7ff8000000000042,0x8000000000000000],
                                 area_fill_parameters=[]),
                            dict(parameters=[bits(2)], area_fill_parameters=[0x7ff0000000000000])
                         ])
                ])


def requirement(source, slot, handle, name, disposition):
    return dict(source=source, slot=slot, source_handle=handle,
                source_name=name, disposition=disposition)


def body(roots, records, requirements, report, units):
    return dict(namespace='fixture-v1', roots=roots, records=records,
                requirements=requirements, report=report,
                context=dict(insertion_units=units, linetype_scale=bits(1)))


def mixed():
    roots = [key('entity', h) for h in [16,9007199254740993,18]]
    layer, pattern, style = key('layer',32), key('linetype',33), key('object',34)
    records = [
        record(roots[0], 'line', dict(common=common(16,'L',32,'Dash',33),
               start=vector(0,0,0), end=vector(1,0,0), thickness=0x8000000000000000,
               normal=vector(0,0,1))),
        record(roots[1], 'mline', mline(common(roots[1]['handle'],'Ex',48,'',0),None)),
        record(roots[2], 'mline', mline(common(18,'Ex',48,'GHOST',68),34)),
        record(layer, 'layer', dict(handle=32, name='L',
               flags=dict.fromkeys(['frozen','locked','frozen_in_new_viewport','off'],False),
               color=['index',7], color_name=None, book_name=None, line_type='Dash',
               line_weight=['default'], is_plottable=True, transparency=['by_layer'],
               description='')),
        record(pattern, 'linetype', dict(handle=33, name='Dash', description='',
               elements=[dict(length=bits(1)),dict(length=bits(-.5))],
               pattern_length=bits(1.5), alignment=65)),
        record(style, 'mline_style', dict(handle=34, owner=6, name='Walls', description='',
               elements=[dict(offset=bits(x),color=['by_block'],linetype='Dash') for x in [-.5,.5]],
               start_angle=bits(0), end_angle=bits(0), fill_color=['none'],
               flags=dict.fromkeys(['fill_on','display_joints','start_square_cap',
                 'start_inner_arcs_cap','start_round_cap','end_square_cap',
                 'end_inner_arcs_cap','end_round_cap'],False))),
    ]
    req = requirement
    requirements = []
    for i,k in enumerate(roots):
        requirements.append(req(k,['owner'],5,'*Model_Space',['context','model_space']))
        requirements.append(req(k,['layer'],32,'L',['included',layer]) if i == 0
                            else req(k,['layer'],48,'Ex',['external',key('layer',48)]))
        requirements.append([
            req(k,['linetype'],33,'Dash',['included',pattern]),
            req(k,['linetype'],0,'',['context','by_layer']),
            req(k,['linetype'],68,'GHOST',['missing']),
        ][i])
        if i:
            requirements.append(req(k,['mline_style'],34,'Walls',['included',style]))
    requirements.extend([
        req(layer,['linetype'],None,'Dash',['included',pattern]),
        req(style,['style_registration'],6,'Walls',['context','mline_style_dictionary']),
        *[req(style,['style_element_linetype',i],None,'Dash',['included',pattern]) for i in range(2)],
    ])
    report = [
        dict(source=None, slot=None, code='unknown_units', message='fixture unknown units'),
        dict(source=roots[1], slot=['mline_style'], code='resolved_source_default',
             message='fixture source default'),
        dict(source=roots[2], slot=['linetype'], code='missing_dependency',
             message='fixture dangling pattern'),
    ]
    return body(roots,records,requirements,report,0)


if __name__ == '__main__':
    directory = Path(__file__).resolve().parent
    fixtures = [
        ('empty', body([],[],[],[],4), '3479f222bcc149f0379691175f9bad8b28b8d9e60c99dce4cf2b0625a06b136f'),
        ('mixed', mixed(), '8f5c819f7bfa1866620be675fc9ef4d3956bbacd681cd11897ca353db6852008'),
    ]
    for name,value,expected in fixtures:
        data = frame(value)
        assert hashlib.sha256(data).hexdigest() == expected, name
        (directory / (name+'.msgpack')).write_bytes(data)
        print(name,len(data),expected)
