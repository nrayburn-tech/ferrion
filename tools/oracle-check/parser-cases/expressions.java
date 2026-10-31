class A {
  Object f(Object o, int a, int b) {
    o = (a > 0 ? 1 : 2) + b * 3;
    o = ((Object) (o));
    o = (Runnable & java.io.Serializable) () -> {};
    o = a = b;
    o = -a + ~b - a++ - --b;
    o = a == b != true;
    o = o instanceof String s && s.isEmpty();
    a += 1;
    return o;
  }
}
