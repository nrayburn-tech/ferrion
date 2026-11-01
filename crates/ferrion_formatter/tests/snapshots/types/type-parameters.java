// Type parameters: flat, then on the next line at +4, then one per line.
class Short < T , U extends Number&Comparable< U >> {
  < V > void m(V v) {}

  public static <K, V extends Comparable<V>> Map<K, V> n() {
    return null;
  }
}

class Fits<SomeVeryLongTypeParameterNameNumberOne, SomeVeryLongTypeParameterNameNumberTwoX> {}

class OnTheNextLine<KeyTypeParameterName extends Comparable<KeyTypeParameterName>, ValueTypeParameter> {}

class OnePerLine<KeyTypeParameterName extends Comparable<KeyTypeParameterName>, ValueTypeParameterName extends Comparable<ValueTypeParameterName>, ThirdTypeParameterName> {}

class Many<T, U, V, W, X, Y, Z, AA, BB, CC, DD, EE, FF, GG, HH, II, JJ, KK, LL, MM, NN, OO, PP, QQ, RR, SS, TT, UU, VV, WW, XX, YY> {}

interface Interface<SomeVeryLongTypeParameterNameNumberOne, SomeVeryLongTypeParameterNameNumberTwoXXX> {}

class Annotated<@SomeVeryLongAnnotationName(someValue = "some long value") @Other T extends Comparable<T>> {}

class AnnotatedOnTwoLines<@SomeVeryLongAnnotationName(someValue = "some long value") @OtherXXXXXXXXXXXXXXXXX T extends Comparable<T>> {}
