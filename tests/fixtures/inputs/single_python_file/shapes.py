"""Geometry module with basic shape classes and area calculations."""
from __future__ import annotations
import math
from dataclasses import dataclass


@dataclass
class Point:
    """A 2D point."""
    x: float
    y: float

    def distance_to(self, other: Point) -> float:
        """Euclidean distance to another point."""
        return math.sqrt((self.x - other.x) ** 2 + (self.y - other.y) ** 2)


class Shape:
    """Abstract base for all shapes."""

    def area(self) -> float:
        raise NotImplementedError

    def perimeter(self) -> float:
        raise NotImplementedError

    def describe(self) -> str:
        return f"{self.__class__.__name__}: area={self.area():.2f}, perimeter={self.perimeter():.2f}"


class Circle(Shape):
    """A circle defined by centre and radius."""

    def __init__(self, centre: Point, radius: float) -> None:
        self.centre = centre
        self.radius = radius

    def area(self) -> float:
        return math.pi * self.radius ** 2

    def perimeter(self) -> float:
        return 2 * math.pi * self.radius

    def contains(self, point: Point) -> bool:
        return self.centre.distance_to(point) <= self.radius


class Rectangle(Shape):
    """An axis-aligned rectangle."""

    def __init__(self, top_left: Point, width: float, height: float) -> None:
        self.top_left = top_left
        self.width = width
        self.height = height

    def area(self) -> float:
        return self.width * self.height

    def perimeter(self) -> float:
        return 2 * (self.width + self.height)

    def diagonal(self) -> float:
        return math.sqrt(self.width ** 2 + self.height ** 2)


class Triangle(Shape):
    """A triangle defined by three vertices."""

    def __init__(self, a: Point, b: Point, c: Point) -> None:
        self.a = a
        self.b = b
        self.c = c

    def _side_lengths(self) -> tuple[float, float, float]:
        return (
            self.a.distance_to(self.b),
            self.b.distance_to(self.c),
            self.c.distance_to(self.a),
        )

    def area(self) -> float:
        # Heron's formula
        a, b, c = self._side_lengths()
        s = (a + b + c) / 2
        return math.sqrt(s * (s - a) * (s - b) * (s - c))

    def perimeter(self) -> float:
        return sum(self._side_lengths())


def largest_shape(shapes: list[Shape]) -> Shape | None:
    """Return the shape with the greatest area."""
    if not shapes:
        return None
    return max(shapes, key=lambda s: s.area())


def bounding_box(shapes: list[Shape]) -> Rectangle | None:
    """Approximate bounding box of a list of circles (simplified)."""
    circles = [s for s in shapes if isinstance(s, Circle)]
    if not circles:
        return None
    min_x = min(c.centre.x - c.radius for c in circles)
    min_y = min(c.centre.y - c.radius for c in circles)
    max_x = max(c.centre.x + c.radius for c in circles)
    max_y = max(c.centre.y + c.radius for c in circles)
    return Rectangle(
        top_left=Point(min_x, min_y),
        width=max_x - min_x,
        height=max_y - min_y,
    )
