// Long types in declarations, statements, and expressions, which GJF breaks around them first.
class InOtherConstructs
    extends SomeVeryLongBaseClassName<
        SomeVeryLongTypeNameNumberOne, SomeVeryLongTypeNameNumberTwo> {
  Map<
      SomeVeryLongTypeNameNumberOne,
      Map<SomeVeryLongTypeNameNumberTwo, SomeVeryLongTypeNameNumberThree>>
      a;
  Map<SomeVeryLongTypeNameNumberOneAAAAAAAAAAAAAAAAAA, SomeVeryLongTypeNameNumberOneBBBBBBBBBBBBB>
      b;
  com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever.and.ever
          .SomeTypeName<
      SomeVeryLongTypeArgumentOne, SomeVeryLongTypeArgumentTwo>
      c;

  public static <
          KeyTypeParameterName extends Comparable<KeyTypeParameterName>, ValueTypeParameterName>
      void d() {}

  void e()
      throws FirstVeryLongExceptionName,
          SecondVeryLongExceptionName,
          ThirdVeryLongExceptionName,
          FourthVeryLongExceptionName {}

  void f()
      throws
          com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever
              .SomeException {}

  void g(
      Map<SomeVeryLongTypeNameNumberOneXXXXXXXXXXXXXXXXXXXXXX, SomeVeryLongTypeNameNumberTwoXXXXXX>
          p) {
    Map<
        SomeVeryLongTypeNameNumberOne,
        Map<SomeVeryLongTypeNameNumberTwo, SomeVeryLongTypeNameNumberThree>> local;
    Object x =
        new HashMap<
            SomeVeryLongTypeNameNumberOneXXXXXXXXXXXXXX, SomeVeryLongTypeNameNumberTwoXXXXXXXXX>();
    Object o =
        (SomeVeryLongInterfaceNameNumberOne & SomeVeryLongInterfaceNameNumberTwo
            & ThirdInterfaceName & java.io.Serializable)
            x;
    try {
      x();
    } catch (A
        | com.example.some.very.lng.pkg.name.that.goes.on.and.on.forever.and.ever.and.ever
            .FirstException e) {
      x();
    }
  }
}
