// Type arguments. Where they break, they're in a type parameter's bound, whose layout is the
// types rules' own.
class TypeArguments {
  Map<String, List<String>> a;
  Map<String, Map<Integer, List<String>>> b;
  Outer<String>.Inner<Integer> c;
  List<String> e = new ArrayList<>();

  void f() {
    Foo.<String>bar();
    this.<A, B>baz();
    Function<String, Integer> g = Integer::<String>parse;
  }
}

class Fits<
    T extends Map<SomeVeryLongTypeArgumentNameNumberOne, SomeVeryLongTypeArgumentNameTwoX>> {}

class OnTheNextLine<
    T extends
        Comparable<T>
            & Map<SomeVeryLongTypeArgumentNameNumberOne, SomeVeryLongTypeArgumentNameNumberTwo>> {}

class AllOnTheNextLine<
    T extends
        Comparable<T>
            & Map<SomeVeryLongTypeArgumentNameNumberOneXXXXXXXX, SomeVeryLongArgumentTwo>> {}

class OnePerLine<
    T extends
        Comparable<T>
            & Map<
                SomeVeryLongTypeArgumentNameNumberOneXXXXXXXXXXXXXXXXXX,
                SomeVeryLongTypeArgumentNameNumberTwo>> {}

class Nested<
    T extends
        Comparable<T>
            & Map<
                SomeVeryLongTypeArgumentNameNumberOneXXXXXXX,
                Map<
                    SomeVeryLongTypeArgumentNameNumberTwo,
                    SomeVeryLongTypeArgumentNameNumberThree>>> {}

class OneArgument<
    T extends
        Comparable<T>
            & SomeVeryLongGenericTypeNameXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX<
                String>> {}
