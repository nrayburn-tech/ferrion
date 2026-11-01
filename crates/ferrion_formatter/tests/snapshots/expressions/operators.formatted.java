// `?:`, assignment, casts, `instanceof` and patterns, and method references.
class Operators {
  void m() {
    x = a ? b : c ? d : e;
    x =
        someConditionHolds
            ? someVeryLongValueWhenTrueAAAAAAAAAAAAAA
            : someVeryLongValueWhenFalseBBBBBBBBBB;
    x =
        firstConditionHoldsAAAAAAA
            ? valueOneAAAAAAAAAAAAAAAA
            : secondConditionHoldsBBBBBBBB ? valueTwoBBBBBBBBB : valueThreeCC;
    x =
        aaaaaaaaaaaaaaaaaaa
            ? bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ? cccccccccccccccccccccc : ddddddddddddddddddd
            : eeeeee;
    x =
        a
            ? foo(
                aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
                bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)
            : c;
    foo(
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
            ? bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
            : cccccccc,
        d);
    x += 1;
    x -= 1;
    x <<= 1;
    x >>= 1;
    x >>>= 1;
    x =
        y =
            zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz;
    someVariableNameAAAAAAAAAAAAAAAAAAAAAAAA =
        someOtherValueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB.someMethod(xxxxx);
    this.someVariableNameAAAAAAAAAAAAAAAAAAAAAA =
        someOtherValueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB.someMethod(
            xxxxx, yyyyyyyyyyyyyyyyyyyyy, zzzzzzzzzzzzzzzzzzzz);
    array[indexAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA] =
        valueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB;
    x = (int) a + (long) (b) + (Foo<Bar>) c + (Foo & Bar) d;
    x =
        (SomeLongTypeName)
            someOtherValueBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB.someMethod(xxxxx);
    x =
        (Runnable)
            () -> {
              run();
            };
    x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
            instanceof Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb;
    x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
            instanceof final Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb bbbbbbbbbbbbbbbbbbb;
    x =
        o instanceof Point(var a, var b)
            && o instanceof Point(_, _)
            && o instanceof Point(int a, _);
    if (o
        instanceof
        Point(
            int xxxxxxxxxxxxxxxxxxxxxxx,
            int yyyyyyyyyyyyyyyyyyyyyyy,
            Point(var a, var bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb))) {}
    foo(
        String::valueOf,
        Integer::parseInt,
        someVeryLongReceiverName::someVeryLongMethodNameAAAAAAAAAAAAA);
    foo(super::bar, this::baz, Foo.Bar::new, List<String>::size, foo()::bar, x.y::<T>z, int[]::new);
    x =
        aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
            ::bbbbbbbbbbbbb;
    x = Foo.class;
    y = int.class;
    z = int[].class;
    w = Foo.Bar.class;
    v = void.class;
    x = Outer.this.y;
    y = Outer.super.foo();
    z = arr[i][j];
    w = foo().bar[0];
    v = ((a));
  }
}
