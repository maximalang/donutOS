import struct, sys, re
sys.path.insert(0, r'C:/Users/max/AppData/Local/hermes/hermes-agent/venv/Lib/site-packages')

def load(path):
    b = open(path, 'rb').read()
    e_lfanew = struct.unpack_from('<I', b, 0x3C)[0]
    nsec = struct.unpack_from('<H', b, e_lfanew+6)[0]
    opt_size = struct.unpack_from('<H', b, e_lfanew+20)[0]
    opt = e_lfanew + 24
    magic = struct.unpack_from('<H', b, opt)[0]
    image_base = struct.unpack_from('<Q', b, opt + 24)[0] if magic == 0x20b else struct.unpack_from('<I', b, opt + 28)[0]
    secs = []
    for i in range(nsec):
        o = opt + opt_size + i * 40
        name = b[o:o+8].rstrip(b'\x00').decode(errors='replace')
        vsize, va, rsize, raw = struct.unpack_from('<IIII', b, o+8)
        secs.append((name, va, vsize, raw, rsize))
    return b, image_base, secs

def off2va(b, ib, secs, off):
    for n, va, vs, raw, rs in secs:
        if raw <= off < raw + rs:
            return ib + va + (off - raw)

def va2off(b, ib, secs, va):
    rva = va - ib
    for n, v, vs, raw, rs in secs:
        if v <= rva < v + max(vs, rs):
            return raw + (rva - v)

path = sys.argv[1]
b, ib, secs = load(path)
text = next(s for s in secs if s[0] == '.text')

# 1) locate WAYFERN_TOKEN string (ASCII)
needle = b'WAYFERN_TOKEN\x00'
soff = b.find(needle)
assert soff != -1, 'WAYFERN_TOKEN not found'
sva = off2va(b, ib, secs, soff)
print(f'WAYFERN_TOKEN file-off={soff:#x} VA={sva:#x}')

# 2) find lea r?,[rip+disp] -> sva inside .text
cands = []
for op in (b'\x48\x8d\x0d', b'\x48\x8d\x15', b'\x4c\x8d\x05', b'\x4c\x8d\x0d', b'\x48\x8d\x05', b'\x4c\x8d\x15'):
    pos = b.find(op, text[3], text[3] + text[4])
    while pos != -1:
        disp = struct.unpack_from('<i', b, pos + 3)[0]
        if off2va(b, ib, secs, pos) + 7 + disp == sva:
            cands.append(pos)
        pos = b.find(op, pos + 1)
print('lea-xrefs:', [hex(c) for c in cands])

# 3) for each xref walk back to nearest cc-padding to find function start
def fn_start(off):
    i = off
    while i > text[3]:
        # look for 'cc cc' padding boundary within 0x800 bytes
        if b[i] == 0xcc and b[i-1] == 0xcc:
            j = i + 1
            while b[j] == 0xcc:
                j += 1
            return j
        i -= 1
    return None

for c in cands:
    st = fn_start(c)
    va = off2va(b, ib, secs, st) if st else None
    head = b[st:st+16].hex() if st else None
    ok = st is not None and b[st:st+5] == bytes.fromhex('564883ec60')
    print(f'xref {c:#x}: fn_start={hex(st) if st else None} VA={hex(va) if va else None} head={head} prologue-match={ok}')
