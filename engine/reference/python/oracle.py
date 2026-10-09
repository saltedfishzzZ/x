"""独立 T0 Oracle。不得导入或调用 Rust 正式实现。"""

from dataclasses import dataclass
from hashlib import sha256
from math import atan2, pi, sqrt, tan
from struct import pack
from unicodedata import category, normalize


@dataclass(frozen=True)
class Capacity:
    potential: float
    area: float
    volume: float


def linear_ramp_capacity(level: float) -> Capacity:
    """Triangle x>=0, y>=0, x+y<=1 with h=x+y."""
    if level <= 0.0:
        return Capacity(level, 0.0, 0.0)
    if level < 1.0:
        return Capacity(level, level * level / 2.0, level**3 / 6.0)
    return Capacity(level, 0.5, level / 2.0 - 1.0 / 3.0)


def unit_sphere_area() -> float:
    return 4.0 * pi


def _dot(left: tuple[float, float, float], right: tuple[float, float, float]) -> float:
    return sum(a * b for a, b in zip(left, right))


def _cross(
    left: tuple[float, float, float], right: tuple[float, float, float]
) -> tuple[float, float, float]:
    return (
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    )


def _unit(value: tuple[float, float, float]) -> tuple[float, float, float]:
    length = sqrt(_dot(value, value))
    return tuple(component / length for component in value)  # type: ignore[return-value]


def equiangular_face_vector(
    face: int, s: float, t: float
) -> tuple[float, float, float]:
    """独立实现 cs1 面坐标到单位球向量。"""
    horizontal = tan(pi * s / 4.0)
    vertical = tan(pi * t / 4.0)
    raw = (
        (1.0, horizontal, vertical),
        (-1.0, -horizontal, vertical),
        (-horizontal, 1.0, vertical),
        (horizontal, -1.0, vertical),
        (horizontal, vertical, 1.0),
        (horizontal, -vertical, -1.0),
    )[face]
    return _unit(raw)


def spherical_triangle_area(
    first: tuple[float, float, float],
    second: tuple[float, float, float],
    third: tuple[float, float, float],
) -> float:
    numerator = abs(_dot(first, _cross(second, third)))
    denominator = 1.0 + _dot(first, second) + _dot(second, third) + _dot(third, first)
    return 2.0 * atan2(numerator, denominator)


def cubed_sphere_cell_area(face: int, level: int, i: int, j: int) -> float:
    count = 1 << level
    s0, s1 = -1.0 + 2.0 * i / count, -1.0 + 2.0 * (i + 1) / count
    t0, t1 = -1.0 + 2.0 * j / count, -1.0 + 2.0 * (j + 1) / count
    corners = (
        equiangular_face_vector(face, s0, t0),
        equiangular_face_vector(face, s1, t0),
        equiangular_face_vector(face, s1, t1),
        equiangular_face_vector(face, s0, t1),
    )
    return spherical_triangle_area(*corners[:3]) + spherical_triangle_area(
        corners[0], corners[2], corners[3]
    )


def cubed_sphere_global_area(level: int) -> float:
    count = 1 << level
    return sum(
        cubed_sphere_cell_area(face, level, i, j)
        for face in range(6)
        for j in range(count)
        for i in range(count)
    )


def normalize_seed(seed: str) -> str:
    """独立实现 seed-format-v1 的可观察规范化部分。"""
    nfc = normalize("NFC", seed)
    if any(category(character) == "Cc" for character in nfc):
        raise ValueError("control character")
    result = " ".join(nfc.split())
    if not result or len(result) > 128:
        raise ValueError("invalid seed length")
    return result


def framed_sha256_u64(fields: list[bytes]) -> int:
    framed = b"".join(pack(">Q", len(field)) + field for field in fields)
    return int.from_bytes(sha256(framed).digest()[:8], "big")


def named_random_u64(
    seed_key: str,
    namespace: str,
    entity_id: str,
    property_name: str,
    sample_index: int,
) -> int:
    return framed_sha256_u64(
        [
            b"named-random-sha256-v1",
            seed_key.encode(),
            namespace.encode(),
            entity_id.encode(),
            property_name.encode(),
            pack(">Q", sample_index),
        ]
    )


def euler_velocity_micrometres_per_year(
    axis: tuple[float, float, float],
    position: tuple[float, float, float],
    radius_metres: int,
    rate_microdegrees_per_million_years: int,
) -> tuple[float, float, float]:
    scale = radius_metres * rate_microdegrees_per_million_years * pi / 180.0e6
    return tuple(component * scale for component in _cross(_unit(axis), _unit(position)))  # type: ignore[return-value]


def classify_boundary(normal: int, tangent: int, deadband: int = 5_000) -> str:
    if normal < -deadband:
        return "CONVERGENT"
    if normal > deadband:
        return "DIVERGENT"
    if abs(normal) <= deadband and abs(tangent) <= deadband:
        return "STABLE"
    return "UNRESOLVED"


def div_round_half_even(numerator: int, denominator: int) -> int:
    """独立整数实现有符号除法的半偶舍入。"""
    if denominator <= 0:
        raise ValueError("denominator")
    sign = -1 if numerator < 0 else 1
    quotient, remainder = divmod(abs(numerator), denominator)
    doubled = remainder * 2
    if doubled > denominator or (doubled == denominator and quotient % 2 == 1):
        quotient += 1
    return sign * quotient


def map_u53_to_range(sample_u53: int, minimum: int, maximum_exclusive: int) -> int:
    """独立整数实现U53到半开整数区间的映射。"""
    denominator = 1 << 53
    if maximum_exclusive <= minimum:
        raise ValueError("range")
    bounded = min(max(sample_u53, 0), denominator - 1)
    return minimum + bounded * (maximum_exclusive - minimum) // denominator
