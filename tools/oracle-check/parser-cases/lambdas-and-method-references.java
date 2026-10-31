import java.util.function.*;

class A {
  Runnable r = () -> {};
  Function<String, Integer> f = s -> s.length();
  BiFunction<Integer, Integer, Integer> g = (x, y) -> x + y;
  BiFunction<Integer, Integer, Integer> h = (Integer x, Integer y) -> { return x; };
  BiFunction<Integer, Integer, Integer> v = (var x, var y) -> x;
  Function<String, Integer> m = String::length;
  Supplier<A> n = A::new;
  Function<int[], Object> c = int[]::clone;
  Function<java.util.List<String>, Integer> p = java.util.List<String>::size;
  Supplier<Integer> s = super::hashCode;
}
