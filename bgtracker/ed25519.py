"""Ed25519 signatures (RFC 8032) in plain Python, so updates can be checked without an extra library.

Who does what:
- verify(public_key, message, signature) -> True only if the signature was made with the matching
  secret key. The app uses it to check every update before installing it (see update.py).
- sign(secret_seed, message) and public_key(secret_seed) are used only by tools/release_key.py, on the
  maintainer's PC, where the secret key lives (never in the repository, never on GitHub).

This is the reference algorithm from RFC 8032 section 6, written for clarity rather than speed: one
verification takes a few milliseconds, which is nothing next to downloading the exe.
"""
import hashlib

P = 2 ** 255 - 19                                           # the field
L = 2 ** 252 + 27742317777372353535851937790883648493       # order of the base point
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _sha512_int(*parts):
    return int.from_bytes(hashlib.sha512(b"".join(parts)).digest(), "little")


# Points are kept in extended coordinates (X, Y, Z, T) with x = X/Z, y = Y/Z, x*y = T/Z.
def _add(p, q):
    a = (p[1] - p[0]) * (q[1] - q[0]) % P
    b = (p[1] + p[0]) * (q[1] + q[0]) % P
    c = 2 * p[3] * q[3] * D % P
    d = 2 * p[2] * q[2] % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _mul(s, p):
    q = (0, 1, 1, 0)                                        # neutral element
    while s > 0:
        if s & 1:
            q = _add(q, p)
        p = _add(p, p)
        s >>= 1
    return q


def _equal(p, q):
    return (p[0] * q[2] - q[0] * p[2]) % P == 0 and (p[1] * q[2] - q[1] * p[2]) % P == 0


def _recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P)
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


_GY = 4 * pow(5, P - 2, P) % P
_GX = _recover_x(_GY, 0)
G = (_GX, _GY, 1, _GX * _GY % P)


def _compress(p):
    zinv = pow(p[2], P - 2, P)
    x, y = p[0] * zinv % P, p[1] * zinv % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def _decompress(b):
    if len(b) != 32:
        return None
    y = int.from_bytes(b, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    return None if x is None else (x, y, 1, x * y % P)


def _expand(seed):
    if len(seed) != 32:
        raise ValueError("an Ed25519 secret key is 32 bytes")
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def public_key(seed):
    a, _ = _expand(seed)
    return _compress(_mul(a, G))


def sign(seed, message):
    a, prefix = _expand(seed)
    pub = _compress(_mul(a, G))
    r = _sha512_int(prefix, message) % L
    big_r = _compress(_mul(r, G))
    h = _sha512_int(big_r, pub, message) % L
    s = (r + h * a) % L
    return big_r + int.to_bytes(s, 32, "little")


def verify(public, message, signature):
    """True if `signature` (64 bytes) is a valid signature of `message` by `public` (32 bytes)."""
    if len(public) != 32 or len(signature) != 64:
        return False
    a = _decompress(public)
    r = _decompress(signature[:32])
    if a is None or r is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= L:
        return False
    h = _sha512_int(signature[:32], public, message) % L
    return _equal(_mul(s, G), _add(r, _mul(h, a)))
