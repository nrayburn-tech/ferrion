// The first rule's output for types, before their own rules are written.
class A<T, U extends Number & Comparable<U>> {
  Map<String, List<? extends T>> a;
  List<? super Integer>[] b;
  int[][] c;
  java.util.@Nullable List<String> d;
  String @NonNull [] e;
  <V> void m(V... v) throws E, F {}
  void n(@A int @B ... x) {}
}
