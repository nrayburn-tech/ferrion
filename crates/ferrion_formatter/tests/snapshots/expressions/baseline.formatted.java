// The first rule's output for expressions, before their own rules are written.
class A {
  Object[] a = {
    a + b * c - d, x = y += 1, -x, !b, ~i, ++i, i--, - -x, + +x, a ? b : c ? d : e, (int) x, (A & B) y, o instanceof String s && s.isEmpty(), o instanceof Point(int px, var py), obj.<String>method(1, 2).field[0], new A<>(), new int[] {
      1, 2
    }, new int[3][], x -> x, (a, b) -> {
      return a;
    }, String::valueOf, int[]::new, this::m, A.class, int.class, A.this, super.toString(), a >> b, a >>> b, a << b, a >= b, "a" + 'b' + 1L + 2.0f + 0x1p3 + true + null,
  };
}
