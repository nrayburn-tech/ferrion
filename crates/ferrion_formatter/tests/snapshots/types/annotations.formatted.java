// Type annotations: a space after each, and before one on a dimension or varargs.
class Annotations<@A T, @B @C U extends @D Number> {
  java.util.@Nullable List<String> a;
  java.util.Map.@A Entry<K, V> b;
  java.util.@A Map.@B Entry<@C K, @D V> @E [] c;
  Outer<String>.@A Inner<Integer> d;
  List<@A String> e;
  List<@A ? extends @B T> f;
  List<@A(1) @B(x = "y") String> g;
  @A int h;
  int @A [] i;
  String @A @B [] j;

  void m(@A int @B ... x) {}

  void n(String @A ... x) {}

  void o(String @A @B ... x) {}

  void p(@A java.lang.@B String q) {}
}
