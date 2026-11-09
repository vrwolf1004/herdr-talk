"""Ed25519 signatures with the standard library only (RFC 8032, PureEdDSA, no context).

Same job as the `cryptography` package would do for signing rows, kept here so the Python edition needs no install.
shortcut: pure Python big-integer math is not constant time; a process on the same PC that can time it may learn bits of
the signing key. Upgrade to the `cryptography` package if that matters. The key never leaves this PC.
"""

import hashlib

P = 2**255 - 19
Q = 2**252 + 27742317777372353535851937790883648493
D = (-121665 * pow(121666, P - 2, P)) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)
KEY_BYTES = 32
SIG_BYTES = 64


def _sha512(data):
    return hashlib.sha512(data).digest()


def _inv(x):
    return pow(x, P - 2, P)


def _recover_x(y, sign):
    x2 = (y * y - 1) * _inv(D * y * y + 1) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


_BY = 4 * _inv(5) % P
_BX = _recover_x(_BY, 0)
_B = (_BX, _BY, 1, _BX * _BY % P)  # base point, extended coordinates (X, Y, Z, T)


def _add(p1, p2):
    x1, y1, z1, t1 = p1
    x2, y2, z2, t2 = p2
    a = (y1 - x1) * (y2 - x2) % P
    b = (y1 + x1) * (y2 + x2) % P
    c = t1 * 2 * D * t2 % P
    d = z1 * 2 * z2 % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _mul(scalar, point):
    result = (0, 1, 1, 0)
    addend = point
    while scalar:
        if scalar & 1:
            result = _add(result, addend)
        addend = _add(addend, addend)
        scalar >>= 1
    return result


def _encode(point):
    x, y, z, _ = point
    zinv = _inv(z)
    x, y = x * zinv % P, y * zinv % P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def _decode(data):
    if len(data) != 32:
        return None
    value = int.from_bytes(data, "little")
    sign = value >> 255
    y = value & ((1 << 255) - 1)
    if y >= P:
        return None
    x = _recover_x(y, sign)
    if x is None:
        return None
    return (x, y, 1, x * y % P)


def _expand(seed):
    digest = _sha512(seed)
    a = int.from_bytes(digest[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, digest[32:]


def public_key(seed):
    """32-byte public key for a 32-byte seed (the private key)."""
    if len(seed) != KEY_BYTES:
        raise ValueError("seed must be 32 bytes")
    a, _ = _expand(seed)
    return _encode(_mul(a, _B))


def sign(seed, message):
    """64-byte signature of `message` with the key made from `seed`."""
    if len(seed) != KEY_BYTES:
        raise ValueError("seed must be 32 bytes")
    a, prefix = _expand(seed)
    public = _encode(_mul(a, _B))
    r = int.from_bytes(_sha512(prefix + message), "little") % Q
    commitment = _encode(_mul(r, _B))
    k = int.from_bytes(_sha512(commitment + public + message), "little") % Q
    s = (r + k * a) % Q
    return commitment + s.to_bytes(32, "little")


def verify(public, message, signature):
    """True when `signature` is a valid signature of `message` by `public`. Never raises for bad input."""
    if len(public) != KEY_BYTES or len(signature) != SIG_BYTES:
        return False
    a_point = _decode(public)
    r_point = _decode(signature[:32])
    if a_point is None or r_point is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= Q:
        return False
    k = int.from_bytes(_sha512(signature[:32] + public + message), "little") % Q
    left = _encode(_mul(s, _B))
    right = _encode(_add(r_point, _mul(k, a_point)))
    return left == right
