import java.util.function.*;

class E<T> {
  int a, b;
  int[] xs;
  E o;
  Object v1 = 1L + 1.0f + 1.0 + 'c' + 0x1F + true + null;
  Object v2 = a | b ^ a & b;
  Object v3 = a << 1 >> 2 >>> 3;
  Object v4 = a < b == a <= b != a > b;
  Object v5 = a >= b || a != b && !true;
  Object v6 = -a + +a + ~a + ++a + --a + a-- + a++;
  Object v7 = a += a -= a *= a /= a %= a &= a |= a ^= a <<= a >>= a >>>= 1;
  Object v8 = xs[a][b];
  Object v9 = o.o.a;
  Object v10 = this.a + E.this.a + super.hashCode();
  Object v11 = int.class.getName() + void.class + int[][].class + java.lang.String.class;
  Object v12 = o.new B();
  Object v13 = new E<String>().new B();
  Object v14 = new <String>E<String>() {};
  Object v15 = (int) a + (java.util.List<String>) null + (int[]) null;
  Object v16 = o instanceof E;
  Object v17 = a > b ? a : b < a ? b : a;
  Object v18 = this.<String>f(1, 2).toString().length();
  Object v19 = ((a));
  BiFunction<String, int[], Integer> v20 = (String s, int... t) -> 1;
  IntUnaryOperator v21 = (final var q) -> 1;
  IntUnaryOperator v22 = x -> x;
  IntUnaryOperator v23 = _ -> 1;
  IntUnaryOperator v24 = (int x[]) -> 1;
  Object v25 = E[]::new;
  Object v26 = o::<String>f;
  Object v27 = E.super::toString;
  Object v28 = java.util.List<String>::size;
  Object v29 = (Runnable & java.io.Serializable) () -> f(1, 2);
  Object v30 = f(1, 2)[0];
  Object v31 = E.<String>f(1, 2);
  Object v32 = -1 + -1L - -0x1 - - 1 - -1.0 - -0 - - -2;
  native <U> Object[] f(int x, int y);
  class B {}
}
