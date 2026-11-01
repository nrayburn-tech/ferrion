// Wildcards: broken after `extends` or `super` when the bound doesn't fit.
class Wildcards {
  List<?> a;
  List<? extends Number> b;
  List<? super Integer> c;
  Map.Entry<?, ?> d;
  List<? extends Map.Entry<? super K, ? extends V>> e;
}

class OnOneLine<
    T extends
        Comparable<T>
            & Map<
                ? extends SomeVeryLongTypeArgumentNameNumberOneXXXXXXXXXXXXXXXXXXXXXXX, String>> {}

class AfterExtends<
    T extends
        Comparable<T>
            & Map<
                ? extends
                    SomeVeryLongTypeArgumentNameNumberOneXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX>> {}

class AfterSuper<
    T extends
        Comparable<T>
            & Map<
                ? super
                    SomeVeryLongTypeArgumentNameNumberOneXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX>> {}
