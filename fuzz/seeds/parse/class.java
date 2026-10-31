package com.example.shapes;

import java.util.List;
import static java.util.Objects.requireNonNull;

@FunctionalInterface
interface Area<T extends Shape> {
  double of(T shape);
}

public sealed interface Shape permits Circle, Square {}

record Circle(double r) implements Shape {}

non-sealed class Square implements Shape {
  private final double side;

  Square(double side) {
    this.side = side;
  }

  static double total(List<? extends Shape> shapes) {
    double sum = 0;
    for (var shape : shapes) {
      sum += switch (shape) {
        case Circle c when c.r() > 0 -> Math.PI * c.r() * c.r();
        case Square s -> s.side * s.side;
        default -> {
          yield 0;
        }
      };
    }
    int[] bits = {1 << 3, -1 >>> 28, 7 >> 1};
    bits[0] >>>= 1;
    Runnable r = () -> System.out.println(sum);
    return sum > 0 && bits.length != 0 ? sum : requireNonNull(null, "x");
  }
}
